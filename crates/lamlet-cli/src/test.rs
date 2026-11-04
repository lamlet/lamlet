//! `lamlet test PATH...`: run the tests of documents and check their expectation lines.
//!
//! For each `.lam` file (a directory means every `.lam` file under it, recursively, in sorted order):
//!
//! 1. Its expectation lines (`expect:`, `expect-error:`, `input:`; `lamlet::embed::testing::Expectations`)
//!    are compared with the outcome of checking and evaluating it.
//! 2. When it checks and has a static binding `_tests` (std's `{!tests ...}`), each test runs on its own:
//!    a failed check, runtime error, or exhausted budget fails that test only.
//!
//! A file that does not check is one failure, and the run goes on with the next file. Exit status: 0 when
//! everything passed, 1 when anything failed, 2 for a usage or I/O error.

use lamlet::embed::testing::{run_test, tests, Expectations, Outcome};
use lamlet::embed::{display, Engine, Error, Instance, SourceFile, Value, TIME_INPUT};
use serde_json::{json, Value as Json};
use std::path::{Path, PathBuf};

pub struct TestOptions {
    pub paths: Vec<String>,
    /// Run only the tests whose name contains this text.
    pub filter: Option<String>,
    /// The binding that holds the tests.
    pub binding: String,
    pub json: bool,
}

impl Default for TestOptions {
    fn default() -> TestOptions {
        TestOptions { paths: vec![], filter: None, binding: lamlet::embed::testing::DEFAULT_TESTS.into(), json: false }
    }
}

/// Every `.lam` file a path names: the file itself, or the files under a directory, sorted.
fn files(path: &str) -> Result<Vec<PathBuf>, String> {
    let p = Path::new(path);
    if !p.is_dir() {
        return Ok(vec![p.to_path_buf()]);
    }
    let mut out = Vec::new();
    let mut stack = vec![p.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| format!("reading {}: {}", dir.display(), e))? {
            let path = entry.map_err(|e| format!("reading {}: {}", dir.display(), e))?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "lam") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The result of one file.
struct FileReport {
    name: String,
    /// Problems that are not a single test's: the file does not check, or its expectation lines do not hold.
    problems: Vec<String>,
    tests: Vec<(String, Outcome)>,
    /// Whether the file had anything to test.
    tested: bool,
    /// Whether the file has expectation lines.
    expects: bool,
}

impl FileReport {
    fn ok(&self) -> bool {
        self.problems.is_empty() && self.tests.iter().all(|(_, o)| o.passed())
    }
}

fn failure(e: &Error) -> (String, String) {
    let kind = e.diagnostics().first().map(|d| d.kind.as_str()).unwrap_or("internal");
    (kind.to_string(), e.render())
}

/// An instance with the request's inputs and time, then the file's own inputs.
fn instance(engine: &Engine, doc: &lamlet::embed::Document, inputs: &[(String, Value)], time: i64, spec: &Expectations) -> Result<Instance, Error> {
    let mut inst = engine.instance(doc)?;
    inst.set_time(time);
    for (k, v) in inputs {
        inst.set_input(k, v.clone())?;
    }
    for (k, v) in &spec.inputs {
        inst.set_input(k, *v)?;
    }
    Ok(inst)
}

fn test_file(engine: &Engine, opts: &TestOptions, name: String, text: String, inputs: &[(String, Value)], time: i64) -> FileReport {
    let mut report = FileReport { name: name.clone(), problems: vec![], tests: vec![], tested: false, expects: false };
    let spec = match Expectations::parse(&text) {
        Ok(s) => s,
        Err(e) => {
            report.problems.push(e);
            report.tested = true;
            return report;
        }
    };
    report.tested = !spec.is_empty();
    report.expects = !spec.is_empty();
    let doc = match engine.check(SourceFile::new(name, text)) {
        Ok(d) => d,
        Err(e) => {
            report.tested = true;
            report.problems.extend(if spec.is_empty() { vec![e.render()] } else { spec.compare(&Err(failure(&e))) });
            return report;
        }
    };
    if !spec.is_empty() {
        let outcome = instance(engine, &doc, inputs, time, &spec)
            .and_then(|mut inst| inst.evaluate())
            .map(|values| values.iter().map(|(_, v)| display(v)).collect::<Vec<_>>())
            .map_err(|e| failure(&e));
        report.problems.extend(spec.compare(&outcome));
    }
    if doc.bindings().iter().any(|b| b.name == opts.binding) {
        report.tested = true;
        let cases = match tests(&doc, &opts.binding) {
            Ok(c) => c,
            Err(e) => {
                report.problems.push(e.render());
                return report;
            }
        };
        let mut inst = match instance(engine, &doc, inputs, time, &spec) {
            Ok(i) => i,
            Err(e) => {
                report.problems.push(e.render());
                return report;
            }
        };
        for case in cases.iter().filter(|c| opts.filter.as_ref().is_none_or(|f| c.name.contains(f.as_str()))) {
            report.tests.push((case.name.clone(), run_test(&mut inst, case)));
        }
    }
    report
}

fn indent(text: &str, by: &str) -> String {
    text.lines().map(|l| format!("{}{}", by, l)).collect::<Vec<_>>().join("\n")
}

fn print_text(reports: &[FileReport]) {
    for r in reports {
        println!("{}", r.name);
        if !r.tested {
            println!("  no tests (bind `_tests` with {{!tests ...}}, or add expect: lines)");
        }
        for p in &r.problems {
            println!("  FAIL   {}", indent(p, "         ").trim_start());
        }
        if r.expects && r.problems.is_empty() {
            println!("  ok     expect lines");
        }
        for (name, o) in &r.tests {
            match o {
                Outcome::Passed { checks: 0 } => println!("  ok     {} (no checks)", name),
                Outcome::Passed { .. } => println!("  ok     {}", name),
                Outcome::FailedAsExpected { .. } => println!("  ok     {} (failed as expected)", name),
                Outcome::Failed { checks, failures } => {
                    println!("  FAIL   {}", name);
                    for (i, m) in failures {
                        println!("         check {} of {}: {}", i, checks, m);
                    }
                }
                Outcome::Errored(e) => {
                    println!("  ERROR  {}", name);
                    println!("{}", indent(&e.render(), "         "));
                }
                Outcome::ExpectedError { expected, got } => {
                    println!("  FAIL   {}", name);
                    match got {
                        None => println!("         expected an error containing {:?}, but it returned", expected),
                        Some(e) => {
                            println!("         expected an error containing {:?}, got:", expected);
                            println!("{}", indent(&e.render(), "         "));
                        }
                    }
                }
            }
        }
    }
    let tests: Vec<&Outcome> = reports.iter().flat_map(|r| r.tests.iter().map(|(_, o)| o)).collect();
    let passed = tests.iter().filter(|o| o.passed()).count();
    let errored = tests.iter().filter(|o| matches!(o, Outcome::Errored(_))).count();
    let failed = tests.len() - passed - errored;
    let bad_files = reports.iter().filter(|r| !r.problems.is_empty()).count();
    let mut summary = format!(
        "{} file{}, {} test{}: {} passed, {} failed, {} errored",
        reports.len(),
        if reports.len() == 1 { "" } else { "s" },
        tests.len(),
        if tests.len() == 1 { "" } else { "s" },
        passed,
        failed,
        errored
    );
    if bad_files > 0 {
        summary.push_str(&format!("; {} file{} failed", bad_files, if bad_files == 1 { "" } else { "s" }));
    }
    println!("{}", summary);
}

fn outcome_json(name: &str, o: &Outcome) -> Json {
    match o {
        Outcome::Passed { checks } => json!({"name": name, "outcome": "passed", "checks": checks}),
        Outcome::FailedAsExpected { message } => json!({"name": name, "outcome": "passed", "error": message}),
        Outcome::Failed { checks, failures } => json!({
            "name": name, "outcome": "failed", "checks": checks,
            "failures": failures.iter().map(|(i, m)| json!({"check": i, "message": m})).collect::<Vec<_>>(),
        }),
        Outcome::Errored(e) => json!({"name": name, "outcome": "errored", "error": e.render()}),
        Outcome::ExpectedError { expected, got } => json!({
            "name": name, "outcome": "failed", "expected_error": expected, "error": got.as_ref().map(|e| e.render()),
        }),
    }
}

/// Run `lamlet test`; returns the exit status.
pub fn run(engine: &Engine, opts: &TestOptions, inputs: &[(String, Value)], time: i64) -> i32 {
    let mut paths = Vec::new();
    for p in &opts.paths {
        match files(p) {
            Ok(f) => paths.extend(f),
            Err(e) => {
                eprintln!("lamlet: {}", e);
                return 2;
            }
        }
    }
    let time = inputs.iter().find(|(k, _)| k == TIME_INPUT).and_then(|(_, v)| if let Value::Int(t) = v { Some(*t) } else { None }).unwrap_or(time);
    let mut reports = Vec::new();
    for path in &paths {
        let name = path.display().to_string();
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("lamlet: reading {}: {}", name, e);
                return 2;
            }
        };
        reports.push(test_file(engine, opts, name, text, inputs, time));
    }
    let ok = reports.iter().all(|r| r.ok());
    if opts.json {
        let files: Vec<Json> = reports
            .iter()
            .map(|r| {
                json!({
                    "file": r.name, "ok": r.ok(), "problems": r.problems,
                    "tests": r.tests.iter().map(|(n, o)| outcome_json(n, o)).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json!({"command": "test", "ok": ok, "files": files})).unwrap());
    } else {
        print_text(&reports);
    }
    if ok {
        0
    } else {
        1
    }
}
