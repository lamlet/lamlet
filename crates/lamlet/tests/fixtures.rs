//! Runs every Lamlet document in `tests/fixtures/`.
//!
//! A fixture is an ordinary Lamlet document. Its prose (a comment to Lamlet) carries the expectations, one per
//! line, starting at the beginning of a line:
//!
//! - `expect: TEXT` — the next expression region's value displays as TEXT (all regions must be listed).
//! - `expect-error: KIND: TEXT` — checking or evaluation fails with a diagnostic of KIND (`syntax`, `name`,
//!   `type`, `stage`, `package`, `native`, `static-evaluation`, `runtime`, `limit`) whose rendering contains TEXT.
//! - `input: KEY=INT` — host input for the evaluation (for example `input: std.time=1700000000`).
//!
//! A fixture with `expect-error` must not also contain `expect` lines.

use lamlet::value::display;
use lamlet::*;
use std::path::PathBuf;
use std::rc::Rc;

struct Spec {
    expects: Vec<(usize, String)>,
    error: Option<(usize, String, String)>,
    ctx: HostContext,
}

fn spec(text: &str) -> Spec {
    let mut s = Spec { expects: vec![], error: None, ctx: HostContext::default() };
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if let Some(rest) = line.strip_prefix("expect: ") {
            s.expects.push((n, rest.to_string()));
        } else if let Some(rest) = line.strip_prefix("expect-error: ") {
            let (kind, msg) = rest.split_once(": ").unwrap_or_else(|| panic!("line {}: expect-error needs `KIND: TEXT`", n));
            s.error = Some((n, kind.to_string(), msg.to_string()));
        } else if let Some(rest) = line.strip_prefix("input: ") {
            let (k, v) = rest.split_once('=').expect("input: KEY=INT");
            s.ctx = std::mem::take(&mut s.ctx).with_input(k, Value::Int(v.trim().parse().expect("Int input")));
        }
    }
    s
}

fn resolver() -> MemoryResolver {
    let mut r = MemoryResolver::new();
    r.add("fx-ctl", "$[Import std] $[Macro !unless {{c a b} {!std::if c b a}}] $[Macro !_hidden {{} 0}] $[# limit 10]", Exports::All);
    r.add("fx-shapes", "$[Import* std] $[# Point [% {Struct {' {Field \"x\" Int} {Field \"y\" Int}}}]] $[# origin {: [% {. x 0 y 0}] Point}]", Exports::All);
    r
}

/// Run one fixture; returns a failure description.
fn run(name: &str, text: &str) -> Result<(), String> {
    let spec = spec(text);
    if spec.error.is_some() && !spec.expects.is_empty() {
        return Err("a fixture cannot combine expect and expect-error".into());
    }
    let natives = Rc::new(NativeRegistry::new());
    let outcome = check_document(name, text, resolver().into_rc(), natives.clone(), &Options::default())
        .map_err(|f| (f.diagnostics[0].kind, f.render()))
        .and_then(|doc| {
            let mut inst = Instance::new(doc.program.clone(), natives).map_err(|d| (d.kind, d.render(&doc.sources)))?;
            let out = inst
                .evaluate(spec.ctx.clone(), Limits { max_instructions: 5_000_000, ..Limits::default() })
                .map_err(|e| (e.kind, e.to_diagnostic().render(&doc.sources)))?;
            Ok(out.iter().map(|(_, v)| display(v)).collect::<Vec<_>>())
        });
    match (&spec.error, outcome) {
        (Some((line, kind, msg)), Err((k, rendered))) => {
            if k.as_str() != kind || !rendered.contains(msg.as_str()) {
                return Err(format!("line {}: expected {} error containing {:?}, got {} error:\n{}", line, kind, msg, k.as_str(), rendered));
            }
            Ok(())
        }
        (Some((line, kind, msg)), Ok(values)) => Err(format!("line {}: expected {} error {:?}, but evaluation produced {:?}", line, kind, msg, values)),
        (None, Err((k, rendered))) => Err(format!("unexpected {} error:\n{}", k.as_str(), rendered)),
        (None, Ok(values)) => {
            let mut problems = Vec::new();
            for (i, (line, want)) in spec.expects.iter().enumerate() {
                match values.get(i) {
                    Some(got) if got == want => {}
                    Some(got) => problems.push(format!("line {}: expected {}, got {}", line, want, got)),
                    None => problems.push(format!("line {}: expected {}, but there is no result #{}", line, want, i + 1)),
                }
            }
            if values.len() > spec.expects.len() {
                problems.push(format!("{} result(s) have no expect line: {:?}", values.len() - spec.expects.len(), &values[spec.expects.len()..]));
            }
            if problems.is_empty() {
                Ok(())
            } else {
                Err(problems.join("\n"))
            }
        }
    }
}

#[test]
fn fixtures() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {}", dir.display(), e))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "lam"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures found");
    let mut failures = Vec::new();
    for p in &paths {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(p).unwrap();
        if let Err(e) = run(&name, &text) {
            failures.push(format!("--- {}\n{}", name, e));
        }
    }
    assert!(failures.is_empty(), "{} of {} fixtures failed:\n{}", failures.len(), paths.len(), failures.join("\n"));
    eprintln!("{} fixtures passed", paths.len());
}
