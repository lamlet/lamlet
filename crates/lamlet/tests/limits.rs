//! Inputs designed to exhaust the host: deep or huge types, deep host data. Each must end in a diagnostic,
//! never a crash, within an ordinary test thread's 2 MiB stack and in a debug build.

use lamlet::embed::*;

/// `f0 = x ↦ [x]` and `f(i) = x ↦ f(i-1)(f(i-1)(x))`: binding `i` has a type `2^i` levels deep.
fn doubling(n: usize, wrap: &str) -> String {
    let mut s = format!("$[Import* std]$[# f0 {{^ {{x}} {}}}]", wrap);
    for i in 1..=n {
        s.push_str(&format!("$[# f{i} {{^ {{x}} {{f{p} {{f{p} x}}}}}}]", p = i - 1));
    }
    s
}

#[test]
fn exponentially_deep_types_hit_the_nesting_limit() {
    let err = Engine::new().check(SourceFile::new("hm.lam", doubling(20, "{' x}"))).unwrap_err();
    assert_eq!(err.diagnostics()[0].kind, DiagKind::Limit);
    assert!(err.to_string().contains("type nesting limit"), "{}", err);
}

#[test]
fn exponentially_large_types_hit_the_size_limit() {
    // Each binding doubles the size of the type but adds only one level.
    let mut s = String::from("$[Import* std]$[# g0 {^ {x} x}]");
    for i in 1..=40 {
        s.push_str(&format!("$[# g{i} {{^ {{x}} {{pair {{g{p} x}} {{g{p} x}}}}}}]", p = i - 1));
    }
    let err = Engine::new().check(SourceFile::new("size.lam", s)).unwrap_err();
    assert!(err.to_string().contains("type size limit"), "{}", err);
}

#[test]
fn composing_deep_types_inside_one_expression_hits_the_limit() {
    // f6's type is 65 levels deep, under the limit. Composing it inside one expression would build a type
    // 60 × 64 levels deep without any binding in between; every expression's type is bounded, so it fails.
    let mut s = doubling(6, "{' x}");
    let mut e = String::from("1");
    for _ in 0..60 {
        e = format!("{{f6 {}}}", e);
    }
    s.push_str(&format!("$[# g {{^ {{}} {{length {}}}}}]", e));
    let err = Engine::new().check(SourceFile::new("deep.lam", s)).unwrap_err();
    assert!(err.to_string().contains("type nesting limit"), "{}", err);
    // Within the limit, composition works: 65 + 33 levels.
    let mut s = doubling(6, "{' x}");
    s.push_str("$[# g {^ {} {length {f6 {f5 1}}}}]");
    let engine = Engine::new();
    let doc = engine.check(SourceFile::new("ok.lam", s)).unwrap();
    let mut inst = engine.instance(&doc).unwrap();
    assert_eq!(inst.call_typed::<_, i64>(&doc.binding("g").unwrap(), ()).unwrap(), 1);
}

#[test]
fn computed_types_hit_the_nesting_limit() {
    let src = "$[Import* std]$[# T [% {fold {range 0 100000} Int {^ {t i} {List t}}}]]";
    let err = Engine::new().check(SourceFile::new("t.lam", src)).unwrap_err();
    assert!(err.to_string().contains("type nesting limit"), "{}", err);
    // A hundred levels is still fine.
    let ok = "$[Import* std]$[# T [% {fold {range 0 99} Int {^ {t i} {List t}}}]]$T";
    assert!(Engine::new().check(SourceFile::new("t.lam", ok)).is_ok());
}

#[test]
fn recursive_types_are_rejected_wherever_they_form() {
    for src in [
        "$[Import* std]$[# a {^ {n} {fold {range 0 n} {#leaf 0} {^ {t i} {#node t}}}}]",
        "$[# a {^ {z} {K::caseBool true {^ {} z} {^ {} {#node z}}}}]",
        "$[# a {^ {z} {K::caseBool true {^ {} z} {^ {} {. next z}}}}]",
        "$[# a {^ {z} {K::caseBool true {^ {} {. x 1}} {^ {} {.x= z z}}}}]",
        "$[# h {^ h {t} {h {#node t}}}]",
    ] {
        let err = Engine::new().check(SourceFile::new("r.lam", src)).unwrap_err();
        assert!(err.to_string().contains("recursive type"), "{}: {}", src, err);
    }
}

#[test]
fn deep_host_data_is_refused() {
    // Deeper than any well-typed value, but shallow enough for Rust to drop recursively.
    let mut d = Data::Int(0);
    let mut t = TypeData::Int;
    for _ in 0..1_000 {
        d = Data::List(vec![d]);
        t = TypeData::List(Box::new(t));
    }
    assert!(d.to_value().unwrap_err().contains("nesting limit"), "{:?}", d.to_value().err());
    assert!(t.to_desc().unwrap_err().contains("nesting limit"));
    // A list 98 levels deep, near the limit, converts both ways.
    let mut s = doubling(6, "{' x}");
    s.push_str("$[# deep [% {f6 {f5 0}}]]");
    let doc = Engine::new().check(SourceFile::new("l.lam", s)).unwrap();
    let v = doc.binding("deep").unwrap().value;
    let back = Data::from_value(&v).unwrap().to_value().unwrap();
    assert_eq!(display(&back), display(&v));
}

#[test]
fn panics_become_internal_errors_and_poison_the_instance() {
    let mut e = Engine::new();
    e.natives_mut().function("host.bug/boom", |_, (n,): (i64,)| -> Result<i64, String> { if n > 0 { panic!("boom {}", n) } else { Ok(0) } }).pure().register().unwrap();
    std::panic::set_hook(Box::new(|_| {}));
    let stub = "$[Import* std]$[Native boom [% {Fn {' Int} Int}] \"host.bug/boom\" Pure]";
    // During preparation.
    let err = e.check(SourceFile::new("b.lam", format!("{stub}$[% {{boom 1}}]"))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert!(err.to_string().contains("boom 1"), "{}", err);
    // During a call: the instance is unusable afterwards, and a new one works.
    let doc = e.check(SourceFile::new("b.lam", format!("{stub}$[# f {{^ {{n}} {{boom n}}}}]"))).unwrap();
    let f = doc.binding("f").unwrap();
    let mut inst = e.instance(&doc).unwrap();
    assert_eq!(inst.call_typed::<_, i64>(&f, (1i64,)).unwrap_err().kind(), ErrorKind::Internal);
    let again = inst.call_typed::<_, i64>(&f, (0i64,)).unwrap_err();
    assert!(again.to_string().contains("unusable"), "{}", again);
    assert_eq!(e.instance(&doc).unwrap().call_typed::<_, i64>(&f, (0i64,)).unwrap(), 0);
    let _ = std::panic::take_hook();
}
