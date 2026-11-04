//! Running a document's tests, and checking a document against `expect:` lines.
//!
//! Lamlet has no `catch`: an error aborts the whole evaluation. Tests are therefore isolated by the host. A
//! document binds a static list of tests to `_tests` (std's `!tests`, `!test`, `!testFails`), and [`run_tests`] calls
//! each test's thunk separately, so a failed check, a runtime error, or an exhausted budget fails that test
//! only. Checks are values (std's `Check`, `#pass` or `#fail`), so one test reports every check that failed.
//!
//! Whole-document expectations are prose lines of the document (see [`Expectations`]): `expect: TEXT` for
//! each expression region's value, `expect-error: KIND: TEXT` for a document that must fail, and
//! `input: KEY=INT` for host inputs. Errors during checking can only be tested this way, since checking stops
//! at the first error.
//!
//! Nothing here touches files or processes; hosts and the CLI supply the documents.
//!
//! ```
//! use lamlet::embed::*;
//! use lamlet::embed::testing::*;
//!
//! let engine = Engine::new();
//! let doc = engine.check(SourceFile::new("t.lam", r#"$[Import* std]
//! $[# _tests {!tests
//!   {!test "adds" {expectEqual {+ 1 2} 3}}
//!   {!test "wrong" {expectEqual {+ 1 2} 4}}
//!   {!testFails "overflows" "overflow" {* 9223372036854775807 2}}
//! }]"#)).unwrap();
//! let mut inst = engine.instance(&doc).unwrap();
//! let reports = run_tests(&doc, &mut inst, DEFAULT_TESTS).unwrap();
//! let passed: Vec<bool> = reports.iter().map(|r| r.outcome.passed()).collect();
//! assert_eq!(passed, [true, false, true]);
//! ```

use super::{display, Document, Engine, Error, Instance, SourceFile, Value};

/// The binding a document defines its tests in, unless told otherwise. It is private (`_`), so a library
/// can keep its tests inline without exporting them.
pub const DEFAULT_TESTS: &str = "_tests";

/// One test of a document: the record that std's `!test` and `!testFails` build.
#[derive(Clone, Debug)]
pub struct TestCase {
    pub name: String,
    /// The test's body: a function of no arguments returning a `List(Check)`.
    pub run: Value,
    /// For `!testFails`: the text the error message must contain.
    pub fails: Option<String>,
}

/// What happened when a test ran.
#[derive(Clone, Debug)]
pub enum Outcome {
    /// Every check passed (`checks` may be 0: a test with no checks).
    Passed { checks: usize },
    /// Some checks failed: their positions (from 1) and messages.
    Failed { checks: usize, failures: Vec<(usize, String)> },
    /// The body failed with an error (a runtime error or an exhausted budget).
    Errored(Error),
    /// A `!testFails` test passed: its body failed with an error containing the expected text.
    FailedAsExpected { message: String },
    /// A `!testFails` test failed: its body returned (`got` is `None`), or failed with another error.
    ExpectedError { expected: String, got: Option<Error> },
}

impl Outcome {
    pub fn passed(&self) -> bool {
        matches!(self, Outcome::Passed { .. } | Outcome::FailedAsExpected { .. })
    }
}

#[derive(Clone, Debug)]
pub struct TestReport {
    pub name: String,
    pub outcome: Outcome,
}

/// The tests a document binds to `binding`: a static `List` of `{name, run, fails}` records, as std's
/// `!tests` builds.
pub fn tests(doc: &Document, binding: &str) -> Result<Vec<TestCase>, Error> {
    let b = doc.binding(binding).map_err(|_| {
        Error::value(format!(
            "the document has no static binding `{}`; define it with {{!tests ...}}, or use only static names in its tests",
            binding
        ))
    })?;
    let shape = || Error::value(format!("`{}` must be a list of tests built with !test or !testFails, not {}", binding, b.type_text));
    let Value::List(items) = &b.value else { return Err(shape()) };
    items
        .iter()
        .map(|item| {
            let Value::Record(fields) = &item else { return Err(shape()) };
            let field = |l: &str| fields.iter().find(|(k, _)| &**k == l).map(|(_, v)| v);
            let (Some(Value::Str(name)), Some(run), Some(Value::Variant(fails))) = (field("name"), field("run"), field("fails")) else {
                return Err(shape());
            };
            let fails = match (&*fails.0, &fails.1) {
                ("none", _) => None,
                ("some", Value::Str(m)) => Some(m.to_string()),
                _ => return Err(shape()),
            };
            if !run.is_callable() {
                return Err(shape());
            }
            Ok(TestCase { name: name.to_string(), run: run.clone(), fails })
        })
        .collect()
}

/// Run one test in `inst`, under the instance's limits (each call has its own budget).
pub fn run_test(inst: &mut Instance, test: &TestCase) -> Outcome {
    let result = inst.call_value(&test.run, vec![]);
    match (&test.fails, result) {
        (None, Err(e)) => Outcome::Errored(e),
        (None, Ok(v)) => checks(&v),
        (Some(expected), Err(e)) => {
            let message = e.diagnostics().first().map(|d| d.message.clone()).unwrap_or_default();
            if message.contains(expected.as_str()) {
                Outcome::FailedAsExpected { message }
            } else {
                Outcome::ExpectedError { expected: expected.clone(), got: Some(e) }
            }
        }
        (Some(expected), Ok(_)) => Outcome::ExpectedError { expected: expected.clone(), got: None },
    }
}

/// Run every test `doc` binds to `binding`, in order.
pub fn run_tests(doc: &Document, inst: &mut Instance, binding: &str) -> Result<Vec<TestReport>, Error> {
    Ok(tests(doc, binding)?.iter().map(|t| TestReport { name: t.name.clone(), outcome: run_test(inst, t) }).collect())
}

/// Interpret a test body's result, a `List(Check)`.
fn checks(v: &Value) -> Outcome {
    let bad = |what: String| Outcome::Errored(Error::value(format!("a test must return a list of checks, not {}", what)));
    let Value::List(items) = v else { return bad(display(v)) };
    let mut failures = Vec::new();
    let mut n = 0;
    for (i, c) in items.iter().enumerate() {
        n += 1;
        match &c {
            Value::Variant(p) if &*p.0 == "pass" => {}
            Value::Variant(p) if &*p.0 == "fail" => match &p.1 {
                Value::Str(m) => failures.push((i + 1, m.to_string())),
                other => failures.push((i + 1, display(other))),
            },
            other => return bad(display(other)),
        }
    }
    if failures.is_empty() {
        Outcome::Passed { checks: n }
    } else {
        Outcome::Failed { checks: n, failures }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Whole-document expectations

/// The expectation lines of a document. Each starts at the beginning of a prose line:
///
/// - `expect: TEXT`: the next expression region's value displays as TEXT. When a document has any `expect`
///   line, every expression region must have one.
/// - `expect-error: KIND: TEXT`: checking or evaluation fails with a diagnostic of KIND (`syntax`, `name`,
///   `type`, `stage`, `package`, `native`, `static-evaluation`, `runtime`, `limit`) whose rendering contains
///   TEXT. It cannot be combined with `expect` lines.
/// - `input: KEY=INT`: a host input for the evaluation (for example `input: std.time=1700000000`).
#[derive(Clone, Debug, Default)]
pub struct Expectations {
    /// Line number and expected display, one per expression region in order.
    pub values: Vec<(usize, String)>,
    /// Line number, diagnostic kind, and text.
    pub error: Option<(usize, String, String)>,
    pub inputs: Vec<(String, i64)>,
}

impl Expectations {
    /// Read the expectation lines of a document's text.
    pub fn parse(text: &str) -> Result<Expectations, String> {
        let mut s = Expectations::default();
        for (i, line) in text.lines().enumerate() {
            let n = i + 1;
            if let Some(rest) = line.strip_prefix("expect: ") {
                s.values.push((n, rest.to_string()));
            } else if let Some(rest) = line.strip_prefix("expect-error: ") {
                let (kind, msg) = rest.split_once(": ").ok_or_else(|| format!("line {}: expect-error needs `KIND: TEXT`", n))?;
                if s.error.is_some() {
                    return Err(format!("line {}: a document can expect only one error", n));
                }
                s.error = Some((n, kind.to_string(), msg.to_string()));
            } else if let Some(rest) = line.strip_prefix("input: ") {
                let (k, v) = rest.split_once('=').ok_or_else(|| format!("line {}: input needs `KEY=INT`", n))?;
                let v = v.trim().parse().map_err(|_| format!("line {}: input `{}` must be an Int", n, k))?;
                s.inputs.push((k.trim().to_string(), v));
            }
        }
        if s.error.is_some() && !s.values.is_empty() {
            return Err("a document cannot combine expect and expect-error lines".into());
        }
        Ok(s)
    }

    /// Whether the document states any outcome.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.error.is_none()
    }

    /// Compare an outcome with the expectations: either the displayed values of the expression regions, or
    /// the failure's diagnostic kind and rendering. Returns one line per problem; empty when it matches.
    pub fn compare(&self, outcome: &Result<Vec<String>, (String, String)>) -> Vec<String> {
        match (&self.error, outcome) {
            (Some((line, kind, msg)), Err((k, rendered))) => {
                if k != kind || !rendered.contains(msg.as_str()) {
                    vec![format!("line {}: expected {} error containing {:?}, got {} error:\n{}", line, kind, msg, k, rendered)]
                } else {
                    vec![]
                }
            }
            (Some((line, kind, msg)), Ok(values)) => {
                vec![format!("line {}: expected {} error {:?}, but evaluation produced {:?}", line, kind, msg, values)]
            }
            (None, Err((k, rendered))) => vec![format!("unexpected {} error:\n{}", k, rendered)],
            (None, Ok(values)) => {
                let mut problems = Vec::new();
                for (i, (line, want)) in self.values.iter().enumerate() {
                    match values.get(i) {
                        Some(got) if got == want => {}
                        Some(got) => problems.push(format!("line {}: expected {}, got {}", line, want, got)),
                        None => problems.push(format!("line {}: expected {}, but there is no result #{}", line, want, i + 1)),
                    }
                }
                if values.len() > self.values.len() {
                    problems.push(format!(
                        "{} result(s) have no expect line: {:?}",
                        values.len() - self.values.len(),
                        &values[self.values.len()..]
                    ));
                }
                problems
            }
        }
    }
}

/// The kind (as `DiagKind::as_str`) and rendering of an error's first diagnostic.
fn failure(e: &Error) -> (String, String) {
    let kind = e.diagnostics().first().map(|d| d.kind.as_str()).unwrap_or("internal");
    (kind.to_string(), e.render())
}

/// Check and evaluate a document in `engine`, and compare the result with the document's expectation lines.
/// Returns one line per problem; empty when the document behaves as its lines say.
pub fn check_expectations(engine: &Engine, file: SourceFile) -> Vec<String> {
    let spec = match Expectations::parse(&file.text) {
        Ok(s) => s,
        Err(e) => return vec![e],
    };
    let outcome = engine.check(file).map_err(|e| failure(&e)).and_then(|doc| {
        let mut inst = engine.instance(&doc).map_err(|e| failure(&e))?;
        for (k, v) in &spec.inputs {
            inst.set_input(k, *v).map_err(|e| failure(&e))?;
        }
        let values = inst.evaluate().map_err(|e| failure(&e))?;
        Ok(values.iter().map(|(_, v)| display(v)).collect::<Vec<_>>())
    });
    spec.compare(&outcome)
}
