//! A dependency-free fuzzer that runs inside `cargo test`: mutated fixture documents go through checking,
//! preparation, evaluation, and calls with arbitrary arguments, and no input may panic. Every outcome must be
//! a value or a diagnostic.
//!
//! `LAMLET_FUZZ_ITERS` (default 3000) sets the number of inputs and `LAMLET_FUZZ_SEED` the starting seed, so a
//! longer run is `LAMLET_FUZZ_ITERS=200000 cargo test --release --test fuzz`. A failing input is printed
//! with its seed.

use lamlet::embed::*;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

/// xorshift64*: small, deterministic, and good enough to pick mutations.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// Fragments that steer mutations toward the grammar: delimiters, region openers, literals at the edges of
/// their ranges, and forms with their own rules (macros, staging, templates, imports, natives).
const TOKENS: &[&str] = &[
    "{", "}", "[", "]", "$", "$$", "${", "$[", "$;{", "$;[", ";;", "\"", "\\", "\\u{", "#", ".", ".x", ".x=", "'", "^", "%",
    "!", "...", "::", "_", " ", "\n", "\r\n", "\t", "0", "-0", "-9223372036854775808", "9223372036854775807",
    "99999999999999999999", "1e308", "1e309", "-1.5", "0.0", "true", "false", "Int", "Float", "{'}", "{.}", "{' 1 2}",
    "{. a 1}", "[% ", "[f \"", "[latex \"", "[Macro !m {{x} x}]", "[Macro !m {{x ...} {f x ...}}]", "{!m 1}", "[# x ",
    "[# f {^ f {n} {f n}}]", "{^ {x} x}", "{^ {} 1}", "[Import* std]", "[Import std]", "[ImportAs std s]", "s::map",
    "K::", "K::match", "K::caseList", "{match ", "#none", "#some", "[NativeType T \"t\"]",
    "[Native n [% {Fn {'} Int}] \"host.x/n\" Pure]", "{: 1 Int}", "{Struct {' {Field \"x\" Int}}}", "{error \"e\"}",
    "{range 0 1000000}", "{fold ", "é", "\u{0}", "\u{FEFF}", "𝔸", "$a.b.c",
    // Shapes behind past bugs: types that double per binding, computed types, and values that contain
    // themselves.
    "[# f0 {^ {x} {' x}}]", "[# f1 {^ {x} {f0 {f0 x}}}]", "[# f2 {^ {x} {f1 {f1 x}}}]", "{f2 {f2 {f2 1}}}",
    "[# T [% {fold {range 0 500} Int {^ {t i} {List t}}}]]", "{!if true z {#node z}}", "{.x= z z}", "{^ {z} ",
];

fn mutate(rng: &mut Rng, base: &str, other: &str) -> String {
    let mut s: Vec<u8> = base.as_bytes().to_vec();
    for _ in 0..1 + rng.below(4) {
        let at = rng.below(s.len() + 1);
        match rng.below(6) {
            0 if !s.is_empty() => {
                let end = (at + 1 + rng.below(16)).min(s.len());
                s.drain(at.min(end)..end);
            }
            1 if !s.is_empty() => {
                let end = (at + 1 + rng.below(32)).min(s.len());
                let chunk = s[at.min(end)..end].to_vec();
                s.splice(at..at, chunk);
            }
            2 if !other.is_empty() => {
                let o = other.as_bytes();
                let start = rng.below(o.len());
                let end = (start + 1 + rng.below(64)).min(o.len());
                s.splice(at..at, o[start..end].iter().copied());
            }
            3 if !s.is_empty() => {
                // Replace one byte with a delimiter, which unbalances or rebalances the document.
                let i = rng.below(s.len());
                s[i] = b"{}[]$\" "[rng.below(7)];
            }
            _ => {
                let t = TOKENS[rng.below(TOKENS.len())];
                s.splice(at..at, t.bytes());
            }
        }
    }
    String::from_utf8_lossy(&s).into_owned()
}

/// Arbitrary values, including ill-typed ones, as a host might pass them.
fn arbitrary(rng: &mut Rng, depth: u32) -> Value {
    match rng.below(if depth > 2 { 6 } else { 9 }) {
        0 => Value::Unit,
        1 => Value::Bool(rng.below(2) == 0),
        2 => Value::Int([0, 1, -1, i64::MAX, i64::MIN, 7][rng.below(6)]),
        3 => Value::Float([0.0, 1.5, -2.0, 1e308, f64::MIN_POSITIVE][rng.below(5)]),
        4 => Value::str(["", "x", "é𝔸", "{"][rng.below(4)]),
        5 => Value::Type(TypeDesc::Int.into()),
        6 => Value::List(List::from_vec((0..rng.below(4)).map(|_| arbitrary(rng, depth + 1)).collect())),
        7 => record().field("x", arbitrary(rng, depth + 1)).field("first", 1i64).build().unwrap(),
        _ => Value::variant(["some", "none", "x"][rng.below(3)], arbitrary(rng, depth + 1)),
    }
}

fn engine() -> Engine {
    let mut e = Engine::new();
    let mut resolver = MemoryResolver::new();
    resolver.add("fx-ctl", "$[Import std] $[Macro !unless {{c a b} {!std::if c b a}}] $[Macro !_hidden {{} 0}] $[# limit 10]", Exports::All);
    resolver.add("fx-shapes", "$[Import* std] $[# Point [% {Struct {' {Field \"x\" Int} {Field \"y\" Int}}}]] $[# origin {: [% {. x 0 y 0}] Point}]", Exports::All);
    e.set_resolver(resolver);
    e.natives_mut().function("host.x/n", |_, ()| Ok(1i64)).pure().register().unwrap();
    let small = Limits { max_instructions: 200_000, max_frames: 2_000, max_alloc_bytes: 16 << 20, ..Limits::default() };
    e.options_mut().static_limits = small.clone();
    *e.limits_mut() = small;
    e
}

/// Fail on an `Internal` error: the embedding surface turns panics into those.
fn no_bug<T>(r: Result<T, Error>) -> Result<T, Error> {
    if let Err(e) = &r {
        assert_ne!(e.kind(), ErrorKind::Internal, "{}", e);
    }
    r
}

/// Check, render, evaluate, and call everything the document defines.
fn exercise(e: &Engine, rng: &mut Rng, name: &str, text: &str) {
    let Ok(doc) = no_bug(e.check(SourceFile::new(name, text))) else { return };
    let _ = no_bug(doc.render(text_or_display));
    let _ = no_bug(e.check_interface(SourceFile::new(name, text)));
    let Ok(mut inst) = no_bug(e.instance(&doc)) else { return };
    inst.set_time(1_700_000_000);
    let _ = no_bug(doc.render_with(&mut inst, text_or_display));
    for b in doc.bindings() {
        let _ = Data::from_value(&b.value);
        if b.value.is_callable() {
            let arity = match &b.ty {
                Some(TypeDesc::Func(ps, _)) => ps.len(),
                _ => rng.below(4),
            };
            let args: Vec<Value> = (0..arity).map(|_| arbitrary(rng, 0)).collect();
            let _ = no_bug(inst.call(&b, args.clone()));
            // `call_value` skips the binding's type check, so ill-typed arguments reach the VM.
            let _ = no_bug(inst.call_value(&b.value, args));
        }
    }
}

fn text_or_display(region: &Region, v: &Value) -> Result<String, String> {
    text(region, v).or_else(|_| Ok(display(v)))
}

#[test]
fn no_input_panics() {
    let iters: u64 = std::env::var("LAMLET_FUZZ_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(3000);
    let seed: u64 = std::env::var("LAMLET_FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x5eed);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut corpus: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|f| f.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "lam"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    corpus.sort();
    let e = engine();
    std::panic::set_hook(Box::new(|_| {}));
    let mut failures = Vec::new();
    let (mut slowest, mut slowest_input) = (Duration::ZERO, String::new());
    for i in 0..iters {
        let mut rng = Rng(seed.wrapping_add(i).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let (name, base) = &corpus[rng.below(corpus.len())];
        let (_, other) = &corpus[rng.below(corpus.len())];
        let input = mutate(&mut rng, base, other);
        let started = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(|| exercise(&e, &mut rng, name, &input)));
        let took = started.elapsed();
        if took > slowest {
            (slowest, slowest_input) = (took, input.clone());
        }
        if let Err(p) = outcome {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            failures.push(format!("seed {} (from {}): {}\n--- input ---\n{}\n-------------", seed.wrapping_add(i), name, msg, input));
            if failures.len() >= 5 {
                break;
            }
        }
    }
    let _ = std::panic::take_hook();
    eprintln!("fuzz: {} inputs, slowest {:?} ({} bytes)", iters, slowest, slowest_input.len());
    if std::env::var("LAMLET_FUZZ_SHOW_SLOWEST").is_ok() {
        eprintln!("--- slowest input ---\n{}\n---------------------", slowest_input);
    }
    assert!(failures.is_empty(), "{} panicking inputs:\n{}", failures.len(), failures.join("\n"));
}
