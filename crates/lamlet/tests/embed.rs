//! The embedding surface (`lamlet::embed`), exercised by synthetic hosts. Each host uses only `lamlet::embed`,
//! as a real host would.

use lamlet::embed::*;
use std::collections::BTreeMap;

/// A host that registers a pure registry lookup (callable during preparation) and a runtime clock.
fn engine() -> Engine {
    let mut e = Engine::new();
    let n = e.natives_mut();
    n.register_fn("host.counts/leaves", vec![TypeDesc::String], TypeDesc::Int, Availability::Pure, |_, a| match &a[0] {
        Value::Str(s) if &**s == "oak" => Ok(Value::Int(42_289)),
        Value::Str(s) => Err(format!("no tree named {:?}", s)),
        _ => Err("expected a String".into()),
    })
    .unwrap();
    n.register_fn("host.clock/seconds", vec![], TypeDesc::Float, Availability::Runtime, |ctx, _| ctx.input("clock")).unwrap();
    e
}

const STUBS: &str = "$[Import* std]\
$[Native leaves [% {Fn {' String} Int}] \"host.counts/leaves\" Pure]\
$[Native seconds [% {Fn {'} Float}] \"host.clock/seconds\" Runtime]";

// ---------------------------------------------------------------------------------------------------------
// Host 1: text with computed regions, prepared before a slow step.

#[test]
fn text_preparer_renders_prose_with_values() {
    let doc = engine()
        .check(SourceFile::new(
            "story.txt",
            format!("{STUBS}\n$[# n [% {{leaves \"oak\"}}]]\nThe oak has $n leaves.\n$[# half [% {{div n 2}}]]\nHalf of them is $half.\n"),
        ))
        .unwrap();
    assert_eq!(doc.render(text).unwrap(), "The oak has 42289 leaves.\nHalf of them is 21144.\n");
}

#[test]
fn parts_list_prose_declarations_and_regions_in_order() {
    let doc = engine().check(SourceFile::new("p.txt", "$[Import* std]$[# a 2]x $[% {+ a 1}] $;{off} y")).unwrap();
    let kinds: Vec<String> = doc
        .parts()
        .iter()
        .map(|p| match p {
            Part::Text { text, .. } => format!("text {:?}", text),
            Part::Comment { .. } => "comment".into(),
            Part::Declaration { region } => format!("decl {}", region.index),
            Part::Region { region, value } => format!("region {} {} {}", region.index, region.type_text, value.as_ref().map(display).unwrap_or_default()),
        })
        .collect();
    assert_eq!(kinds, ["decl 0", "decl 1", "text \"x \"", "region 2 Int 3", "text \" \"", "comment", "text \" y\""]);
    assert_eq!(doc.region(2).unwrap().ty, Some(TypeDesc::Int));
}

#[test]
fn render_drops_lines_that_hold_only_declarations() {
    let doc = engine().check(SourceFile::new("d.txt", "$[Import* std]Title\n$[# a 1] $[# b 2]\n  $;{note}\nValue: $[% {+ a b}]\n\nEnd")).unwrap();
    assert_eq!(doc.render(text).unwrap(), "Title\nValue: 3\n\nEnd");
}

#[test]
fn diagnostics_point_into_the_host_file() {
    // The document starts at line 40, column 5 of the host's file.
    let err = engine().check(SourceFile::new("chapter.md", "$[Import* std]ok ${+ 1 2}\nbad ${+ 1 true}").at(40, 5)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Check);
    let at = err.locations()[0].clone().unwrap();
    assert_eq!((at.file.as_str(), at.line), ("chapter.md", 41));
    assert!(err.to_string().starts_with("chapter.md:41:"), "{}", err);

    let doc = engine().check(SourceFile::new("chapter.md", "ab $[% 1]").at(40, 5)).unwrap();
    let Part::Region { region, .. } = &doc.parts()[1] else { panic!() };
    assert_eq!((region.at.line, region.at.column), (40, 8));
}

#[test]
fn residual_regions_need_an_instance_to_render() {
    let e = engine();
    let doc = e.check(SourceFile::new("t.txt", format!("{STUBS}At ${{seconds}} s."))).unwrap();
    let err = doc.render(text).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Value);
    assert!(err.to_string().contains("evaluated at runtime"), "{}", err);

    let mut inst = e.instance(&doc).unwrap();
    inst.set_input("clock", 1.5).unwrap();
    assert_eq!(doc.render_with(&mut inst, text).unwrap(), "At 1.5 s.");
}

#[test]
fn a_formatter_decides_how_non_text_values_appear() {
    let doc = engine().check(SourceFile::new("i.txt", "See $[% {#image {. path \"floor.png\" width 400}}].")).unwrap();
    assert!(doc.render(text).is_err());
    let html = doc
        .render(|_, v| match Data::from_value(v)? {
            Data::Variant(tag, payload) if tag == "image" => match *payload {
                Data::Record(f) => Ok(format!("<img src={:?} width={:?}>", f["path"], f["width"])),
                _ => Err("bad image".into()),
            },
            _ => text_of(v),
        })
        .unwrap();
    assert_eq!(html, "See <img src=String(\"floor.png\") width=Int(400)>.");
}

fn text_of(v: &Value) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s.to_string()),
        other => Ok(display(other)),
    }
}

// ---------------------------------------------------------------------------------------------------------
// Host 2: build-time validation, without implementations.

#[test]
fn build_time_check_uses_metadata_only() {
    let mut e = engine();
    e.natives_mut().register_type("host.ui/widget", "1").unwrap();
    let interface = Engine::new();
    // The interface check accepts a stub with no implementation at all.
    let doc = interface
        .check_interface(SourceFile::new("ui.lam", "$[Import* std]$[Native ping [% {Fn {'} Int}] \"host.net/ping\" Runtime] ${ping}"))
        .unwrap();
    assert!(doc.parts().iter().any(|p| matches!(p, Part::Region { value: None, .. })));
    // Natives registered on the engine are never called: a runtime call checks against the metadata...
    let doc = e.check_interface(SourceFile::new("b.lam", format!("{STUBS}${{leaves \"pine\"}}"))).unwrap();
    assert!(doc.parts().iter().any(|p| matches!(p, Part::Region { value: None, .. })));
    // ...but a static region cannot be prepared without an implementation, so the interface check fails.
    let err = e.check_interface(SourceFile::new("b.lam", format!("{STUBS}$[% {{leaves \"oak\"}}]"))).unwrap_err();
    assert!(err.to_string().contains("has no implementation"), "{}", err);
    // The full check runs it during preparation, so the bad argument fails the check.
    let err = e.check(SourceFile::new("b.lam", format!("{STUBS}$[# x [% {{leaves \"pine\"}}]]"))).unwrap_err();
    assert!(err.to_string().contains("no tree named"), "{}", err);
}

#[test]
fn linking_reports_missing_natives() {
    let doc = Engine::new()
        .check_interface(SourceFile::new("u.lam", "$[Import* std]$[Native ping [% {Fn {'} Int}] \"host.net/ping\" Runtime] ${ping}"))
        .unwrap();
    let err = Engine::new().instance(&doc).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Link);
    assert!(err.to_string().contains("host.net/ping"), "{}", err);
}

#[test]
fn require_linked_rejects_unregistered_stubs() {
    let mut e = Engine::new();
    e.options_mut().require_linked = true;
    let err = e.check(SourceFile::new("u.lam", "$[Import* std]$[Native ping [% {Fn {'} Int}] \"host.net/ping\" Runtime]")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Check);
}

#[test]
fn bindings_and_docs_describe_what_a_document_defines() {
    let doc = engine()
        .check(SourceFile::new("d.lam", "$[Import* std]\nScales a length.\n$[# scale {^ {x} {* 2.0 x}}]\n$[# name \"oak\"]"))
        .unwrap();
    let scale = doc.binding("scale").unwrap();
    assert_eq!(scale.ty, Some(TypeDesc::Func(vec![TypeDesc::Float], TypeDesc::Float.into())));
    assert_eq!(scale.doc.as_deref(), Some("Scales a length."));
    assert_eq!(doc.doc("scale"), Some("Scales a length."));
    assert!(doc.bindings().iter().any(|b| b.name == "name" && b.type_text == "String"));
    assert_eq!(doc.binding("missing").unwrap_err().kind(), ErrorKind::Value);
    assert!(doc.dependencies().iter().any(|(n, _)| n == "std"));
}

// ---------------------------------------------------------------------------------------------------------
// Host 3: an animation curve evaluated many times with explicit inputs.

#[test]
fn curve_evaluator_calls_a_closure_with_typed_arguments() {
    let e = engine();
    let doc = e
        .check(SourceFile::new("curve.lam", format!("{STUBS}$[# ease {{^ {{t}} {{* t {{* t {{- 3.0 {{* 2.0 t}}}}}}}}}}]$[# drift {{^ {{t}} {{+ t {{seconds}}}}}}]")))
        .unwrap();
    let ease = doc.binding("ease").unwrap();
    let drift = doc.binding("drift").unwrap();
    let mut inst = e.instance(&doc).unwrap();
    let samples: Vec<f64> = (0..=4).map(|i| inst.call_typed(&ease, (i as f64 / 4.0,)).unwrap()).collect();
    assert_eq!(samples, [0.0, 0.15625, 0.5, 0.84375, 1.0]);
    for frame in 0..3 {
        inst.set_input("clock", frame as f64 * 0.5).unwrap();
        let y: f64 = inst.call_typed(&drift, (1.0,)).unwrap();
        assert_eq!(y, 1.0 + frame as f64 * 0.5);
    }
}

#[test]
fn typed_calls_reject_wrong_arguments_and_results() {
    let e = engine();
    let doc = e.check(SourceFile::new("c.lam", "$[Import* std]$[# f {^ {x} {* 2 x}}]$[# k 3]")).unwrap();
    let f = doc.binding("f").unwrap();
    let mut inst = e.instance(&doc).unwrap();
    assert_eq!(inst.call_typed::<_, i64>(&f, (21i64,)).unwrap(), 42);
    let wrong_arg = inst.call_typed::<_, i64>(&f, (1.5,)).unwrap_err();
    assert_eq!(wrong_arg.kind(), ErrorKind::Value);
    assert!(wrong_arg.to_string().contains("argument 1 of `f` must be Int"), "{}", wrong_arg);
    let arity = inst.call_typed::<_, i64>(&f, (1i64, 2i64)).unwrap_err();
    assert!(arity.to_string().contains("takes 1 arguments"), "{}", arity);
    let wrong_result = inst.call_typed::<_, String>(&f, (1i64,)).unwrap_err();
    assert!(wrong_result.to_string().contains("expected String"), "{}", wrong_result);
    let not_fn = inst.call(&doc.binding("k").unwrap(), vec![]).unwrap_err();
    assert!(not_fn.to_string().contains("not a function"), "{}", not_fn);
}

#[test]
fn runtime_errors_and_budgets_surface_as_errors() {
    let e = engine();
    let doc = e.check(SourceFile::new("r.lam", "$[Import* std]$[# quot {^ {a b} {div a b}}]$[# spin {^ spin {n} {spin {+ n 1}}}]")).unwrap();
    let mut inst = e.instance(&doc).unwrap();
    let err = inst.call_typed::<_, i64>(&doc.binding("quot").unwrap(), (1i64, 0i64)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Runtime);
    inst.set_limits(Limits { max_instructions: 10_000, ..Limits::default() });
    let err = inst.call_typed::<_, i64>(&doc.binding("spin").unwrap(), (0i64,)).unwrap_err();
    assert_eq!(err.diagnostics()[0].kind, DiagKind::Limit);
    // The instance stays usable after a failure.
    assert_eq!(inst.call_typed::<_, i64>(&doc.binding("quot").unwrap(), (9i64, 3i64)).unwrap(), 3);
}

#[test]
fn conversions_cover_options_pairs_lists_and_records() {
    let e = engine();
    let doc = e
        .check(SourceFile::new(
            "v.lam",
            "$[Import* std]$[# firstOr {^ {xs d} {caseList xs {^ {} {pair d none}} {^ {h t} {pair h {some {length t}}}}}}]$[# label {^ {r} [f \"${.name r}#${.id r}\"]}]",
        ))
        .unwrap();
    let mut inst = e.instance(&doc).unwrap();
    let f = doc.binding("firstOr").unwrap();
    let r: (i64, Option<i64>) = inst.call_typed(&f, (vec![7i64, 8, 9], 0i64)).unwrap();
    assert_eq!(r, (7, Some(2)));
    let r: (i64, Option<i64>) = inst.call_typed(&f, (Vec::<i64>::new(), 5i64)).unwrap();
    assert_eq!(r, (5, None));
    let card = record().field("name", "oak").field("id", 3i64).build().unwrap();
    let s: String = inst.call_typed(&doc.binding("label").unwrap(), (card,)).unwrap();
    assert_eq!(s, "oak#3");
    assert!(record().field("a", 1i64).field("a", 2i64).build().is_err());
    assert!(record().field("x", f64::NAN).build().is_err());
}

// ---------------------------------------------------------------------------------------------------------
// Data: plain values for hosts.

#[test]
fn data_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Data>();
    assert_send_sync::<TypeData>();
}

#[test]
fn data_refuses_functions_and_names_where_they_are() {
    let doc = engine().check(SourceFile::new("f.lam", "$[Import* std]${. ok 1 f {' id}}")).unwrap();
    let Part::Region { value: Some(v), .. } = &doc.parts()[1] else { panic!() };
    let err = Data::from_value(v).unwrap_err();
    assert!(err.contains("at .f[0]"), "{}", err);
    assert!(Data::Float(f64::INFINITY).to_value().is_err());
    assert!(Data::Variant("not a label".into(), Box::new(Data::Unit)).to_value().is_err());
    let mut fs = BTreeMap::new();
    fs.insert("x".to_string(), Data::Float(-0.0));
    let v = Data::Record(fs).to_value().unwrap();
    assert_eq!(display(&v), "{x: 0.0}");
}

/// Every value in every passing fixture survives `Value → Data → Value` and `Data → Value → Data`.
#[test]
fn data_round_trips_fixture_values() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut resolver = MemoryResolver::new();
    resolver.add("fx-ctl", "$[Import std] $[Macro !unless {{c a b} {!std::if c b a}}] $[Macro !_hidden {{} 0}] $[# limit 10]", Exports::All);
    resolver.add("fx-shapes", "$[Import* std] $[# Point [% {Struct {' {Field \"x\" Int} {Field \"y\" Int}}}]] $[# origin {: [% {. x 0 y 0}] Point}]", Exports::All);
    let mut e = Engine::new();
    e.set_resolver(resolver);
    let (mut values, mut functions) = (0, 0);
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|f| f.unwrap().path()).collect();
    names.sort();
    for path in names {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.starts_with("err-") || !name.ends_with(".lam") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let doc = e.check(SourceFile::new(name.clone(), text.clone())).unwrap_or_else(|err| panic!("{}: {}", name, err));
        let mut inst = e.instance(&doc).unwrap();
        for line in text.lines() {
            if let Some((k, v)) = line.strip_prefix("input: ").and_then(|r| r.split_once('=')) {
                inst.set_input(k, v.trim().parse::<i64>().unwrap()).unwrap();
            }
        }
        for (region, v) in inst.evaluate().unwrap_or_else(|err| panic!("{}: {}", name, err)) {
            match Data::from_value(&v) {
                Ok(d) => {
                    let back = d.to_value().unwrap_or_else(|m| panic!("{} region {}: {}", name, region, m));
                    assert_eq!(display(&back), display(&v), "{} region {}", name, region);
                    assert_eq!(Data::from_value(&back).unwrap(), d, "{} region {}", name, region);
                    values += 1;
                }
                Err(_) => functions += 1,
            }
        }
    }
    assert!(values > 250, "only {} values round-tripped ({} functions skipped)", values, functions);
}

