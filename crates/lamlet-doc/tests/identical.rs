//! The text writer gives exactly what the renderer it replaces gave (`Document::render_with` with scalars as
//! text and everything else in its written form), on every fixture and example and on edge cases of line removal.

use lamlet::embed::*;
use std::path::Path;

/// The output `lamlet FILE` had before the IR: scalars as text, other values in their written form.
fn old(engine: &Engine, name: &str, source: &str) -> Result<String, String> {
    let doc = engine.check(SourceFile::new(name, source)).map_err(|e| e.render())?;
    let mut instance = prepared(engine, &doc, source)?;
    doc.render_with(&mut instance, |r, v| text(r, v).or_else(|_| Ok(display(v)))).map_err(|e| e.render())
}

/// An instance with a fixed time and the inputs a fixture asks for (`input: KEY=INT` lines).
fn prepared(engine: &Engine, doc: &Document, source: &str) -> Result<Instance, String> {
    let mut instance = engine.instance(doc).map_err(|e| e.render())?;
    instance.set_time(1_700_000_000);
    for line in source.lines() {
        if let Some((k, v)) = line.strip_prefix("input: ").and_then(|r| r.split_once('=')) {
            instance.set_input(k, v.trim().parse::<i64>().unwrap()).map_err(|e| e.render())?;
        }
    }
    Ok(instance)
}

fn new(engine: &Engine, name: &str, source: &str) -> Result<String, String> {
    let doc = engine.check(SourceFile::new(name, source)).map_err(|e| e.render())?;
    let mut instance = prepared(engine, &doc, source)?;
    let ir = lamlet_doc::evaluate_and_lower(&doc, &mut instance, lamlet_doc::TEXT).map_err(|e| e.render())?;
    lamlet_doc::backend::text::write(&ir).map_err(|e| e.render())
}

fn same(engine: &Engine, name: &str, source: &str) -> bool {
    let (a, b) = (old(engine, name, source), new(engine, name, source));
    match (&a, &b) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "{name}: the text differs"),
        (Err(_), Err(_)) => {}
        _ => panic!("{name}: one succeeded and the other failed:\nold: {a:?}\nnew: {b:?}"),
    }
    a.is_ok()
}

#[test]
fn every_fixture_and_example_prints_the_same() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let engine = Engine::new();
    let (mut files, mut printed) = (0, 0);
    for dir in ["tests/fixtures", "examples"] {
        let mut paths: Vec<_> = std::fs::read_dir(root.join(dir)).unwrap().map(|e| e.unwrap().path()).collect();
        paths.sort();
        for path in paths.into_iter().filter(|p| p.extension().is_some_and(|e| e == "lam")) {
            let source = std::fs::read_to_string(&path).unwrap();
            files += 1;
            printed += same(&engine, &path.display().to_string(), &source) as usize;
        }
    }
    eprintln!("{printed} of {files} documents printed and matched");
    assert!(files > 50, "found only {files} documents");
    // Most fixtures test errors and fail in both; those that print are the comparison's substance.
    assert!(printed >= 25, "only {printed} of {files} documents printed; the comparison proves little");
}

#[test]
fn line_removal_edge_cases_print_the_same() {
    let engine = Engine::new();
    let cases = [
        "$[# a 1]\nx $a\n",
        "$[# a 1]   \n\n$a\n",
        "  $[# a 1] $;{note}  \nend",
        "$[% \"a\\n\"]$[# y 2]\nnext\n",
        "$[% \"a\\n   \"]$[# y 2]\nnext\n",
        "$[% \"\"]$[# y 2]\nkept\n",
        "$[Import* std]$K::unit$[# y 2]\nunit line\n",
        "$[Import* std]$[# y 2]$K::unit\n${' 1 2} $map\n",
        "no regions at all",
        "",
        "$$ and $$5\n$[# z \"q\"]\n$z\n",
        "$[Import* std]\n${^ {x} x} ${some {. a 1 b \"t\"}}\n",
    ];
    for (i, source) in cases.iter().enumerate() {
        assert!(same(&engine, &format!("case {i}"), source), "case {i} failed to print: {:?}", new(&engine, "case", source));
    }
}

#[test]
fn values_lower_to_text_value_and_code_nodes() {
    use lamlet_doc::{Block, Inline};
    let engine = Engine::new();
    let doc = engine.check(SourceFile::new("d", "$[Import* std]${+ 1 2}|${' 1 2}|$map|$K::unit")).unwrap();
    let mut instance = engine.instance(&doc).unwrap();
    let ir = lamlet_doc::evaluate_and_lower(&doc, &mut instance, lamlet_doc::TEXT).unwrap();
    let [Block::Plain(inlines)] = &ir.blocks[..] else { panic!("{ir:?}") };
    let kinds: Vec<&str> = inlines
        .iter()
        .map(|i| match i {
            Inline::Text { .. } => "text",
            Inline::Raw { .. } => "raw",
            Inline::Code { .. } => "code",
            Inline::Value { .. } => "value",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["text", "raw", "value", "raw", "code", "raw"]);
    assert_eq!(inlines[0].at().map(|a| (a.line, a.column)), Some((1, 15)));
}

#[test]
fn raw_markup_of_another_format_is_refused_with_its_place() {
    let engine = Engine::new();
    let doc = engine.check(SourceFile::new("page.lam", "Some *prose*.")).unwrap();
    let mut instance = engine.instance(&doc).unwrap();
    let ir = lamlet_doc::evaluate_and_lower(&doc, &mut instance, "md").unwrap();
    let e = lamlet_doc::backend::text::write(&ir).unwrap_err();
    assert_eq!(e.render(), "page.lam:1:1: this is markup for `md`, which cannot be written as plain text");
}

#[test]
fn a_runtime_region_without_a_value_is_an_error() {
    let engine = Engine::new();
    let doc = engine.check(SourceFile::new("d.lam", "$[Import* std]\nnow ${+ 1 2}")).unwrap();
    let e = lamlet_doc::lower(&doc, &Default::default(), lamlet_doc::TEXT).unwrap_err();
    assert!(e.render().starts_with("d.lam:2:5: region"), "{}", e.render());
}
