//! Runs every Lamlet document in `tests/fixtures/`.
//!
//! A fixture is an ordinary Lamlet document. Its prose (a comment to Lamlet) carries the expectations, one per
//! line, starting at the beginning of a line (`lamlet::embed::testing::Expectations`):
//!
//! - `expect: TEXT` — the next expression region's value displays as TEXT (all regions must be listed).
//! - `expect-error: KIND: TEXT` — checking or evaluation fails with a diagnostic of KIND (`syntax`, `name`,
//!   `type`, `stage`, `package`, `native`, `static-evaluation`, `runtime`, `limit`) whose rendering contains TEXT.
//! - `input: KEY=INT` — host input for the evaluation (for example `input: std.time=1700000000`).
//!
//! A fixture with `expect-error` must not also contain `expect` lines.

use lamlet::embed::testing::check_expectations;
use lamlet::embed::*;
use std::path::PathBuf;

fn engine() -> Engine {
    let mut r = MemoryResolver::new();
    r.add("fx-ctl", "$[Import std] $[Macro !unless {{c a b} {!std::if c b a}}] $[Macro !_hidden {{} 0}] $[# limit 10]", Exports::All);
    r.add("fx-shapes", "$[Import* std] $[# Point [% {Struct {' {Field \"x\" Int} {Field \"y\" Int}}}]] $[# origin {: [% {. x 0 y 0}] Point}]", Exports::All);
    let mut e = Engine::new();
    e.set_resolver(r);
    e.limits_mut().max_instructions = 5_000_000;
    e
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
    let engine = engine();
    let mut failures = Vec::new();
    for p in &paths {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(p).unwrap();
        let problems = check_expectations(&engine, SourceFile::new(name.clone(), text));
        if !problems.is_empty() {
            failures.push(format!("--- {}\n{}", name, problems.join("\n")));
        }
    }
    assert!(failures.is_empty(), "{} of {} fixtures failed:\n{}", failures.len(), paths.len(), failures.join("\n"));
    eprintln!("{} fixtures passed", paths.len());
}
