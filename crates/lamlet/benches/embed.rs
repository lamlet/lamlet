//! Benchmarks of the embedding surface, with no dependencies: `cargo bench -p lamlet --bench embed`.
//!
//! The workloads are a host's typical ones: checking a 1,000-line document with 200 regions,
//! evaluating and rendering it, 10⁶ calls of a small closure, plus a VM loop.
//!
//! `-- --gate` also checks properties that do not depend on the machine, so CI can run it on shared runners:
//! checking and evaluation scale linearly with document size, and the instruction counts of fixed workloads do
//! not grow. Timings are printed, never gated.

use lamlet::embed::*;
use std::time::{Duration, Instant};

/// The VM instructions one call of `ease` takes, and one call of `sum` (`{fold {range 0 n} 0 +}`) for n = 10⁶.
/// These are deterministic. If a change lowers them, lower the numbers here; if it raises them, that is a regression to
/// justify.
const EASE_INSTRUCTIONS: u64 = 13;
const FOLD_INSTRUCTIONS: u64 = 28_000_032;

/// How much slower doubling the document may make checking or evaluation (2.0 is linear).
const MAX_DOUBLING_FACTOR: f64 = 2.6;

const PROSE: [&str; 4] = [
    "The quick brown fox jumps over the lazy dog, and the dog does not mind at all.",
    "A paragraph of ordinary prose sits between the computed values of the document.",
    "Nothing here is code: only dollar regions are active, and this line has none.",
    "Hosts keep this text as it is and splice the computed values in between.",
];

/// A document of `blocks` blocks; each block is 20 lines with 4 regions (2 declarations, 1 value computed during
/// the check, 1 computed at runtime). 50 blocks are 1,000 lines and 200 regions.
fn document(blocks: usize) -> String {
    let mut s = String::from("$[Import* std]\n");
    let prose = |s: &mut String| {
        for p in PROSE {
            s.push_str(p);
            s.push('\n');
        }
    };
    for i in 0..blocks {
        s.push_str(&format!("$[# f{i} {{^ {{x}} {{+ {{* x x}} {i}}}}}]\n"));
        prose(&mut s);
        s.push_str(&format!("Block {i} squares three into $[% {{f{i} 3}}] during the check.\n"));
        prose(&mut s);
        s.push_str(&format!("$[# r{i} [% {{. name \"item {i}\" size {{f{i} 2}}}}]]\n"));
        prose(&mut s);
        s.push_str(&format!("At runtime, item {i} has size ${{f{i} {{.size r{i}}}}}.\n"));
        prose(&mut s);
    }
    s
}

const CURVES: &str = "$[Import* std]$[# ease {^ {t} {* t {* t {- 3.0 {* 2.0 t}}}}}]$[# sum {^ {n} {fold {range 0 n} 0 +}}]";

/// Run `f` `n` times; returns the median and the minimum.
fn time(n: usize, mut f: impl FnMut()) -> (Duration, Duration) {
    let mut ts: Vec<Duration> = (0..n)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    ts.sort();
    (ts[n / 2], ts[0])
}

fn row(name: &str, (median, min): (Duration, Duration), note: &str) {
    println!("{:<34} {:>12?} {:>12?}   {}", name, median, min, note);
}

fn main() {
    let gate = std::env::args().any(|a| a == "--gate");
    let engine = Engine::new();
    let mut failures = Vec::new();

    println!("{:<34} {:>12} {:>12}", "workload", "median", "min");

    // Checking (which includes preparing static regions).
    let small = document(50);
    let large = document(100);
    assert_eq!(small.lines().count(), 1001);
    let check = |src: &str| engine.check(SourceFile::new("bench.txt", src)).unwrap();
    let doc = check(&small);
    assert_eq!(doc.parts().iter().filter(|p| !matches!(p, Part::Text { .. })).count(), 201);
    // Every check also checks `std`, which costs the same for any document; it is measured on its own.
    let check_std = time(21, || drop(check("$[Import* std]")));
    let check_small = time(21, || drop(check(&small)));
    let check_large = time(21, || drop(check(&large)));
    row("check `$[Import* std]` alone", check_std, "the fixed cost of every check");
    row("check 1,000 lines, 200 regions", check_small, "");
    row("check 2,000 lines, 400 regions", check_large, "");

    // Evaluating and rendering (the runtime regions run; the rest is already prepared).
    let render = |doc: &Document| {
        let mut inst = engine.instance(doc).unwrap();
        doc.render_with(&mut inst, text).unwrap()
    };
    let doc_large = check(&large);
    assert!(render(&doc).contains("At runtime, item 49 has size 2858."));
    let render_small = time(21, || drop(render(&doc)));
    let render_large = time(21, || drop(render(&doc_large)));
    row("instance + evaluate + render 1,000", render_small, "");
    row("instance + evaluate + render 2,000", render_large, "");

    // 10⁶ calls of a small closure.
    let curves = check(CURVES);
    let ease = curves.binding("ease").unwrap();
    let sum = curves.binding("sum").unwrap();
    let mut inst = engine.instance(&curves).unwrap();
    let _: f64 = inst.call_typed(&ease, (0.5,)).unwrap();
    let ease_instructions = inst.stats().instructions;
    let calls = 1_000_000;
    let calls_time = time(3, || {
        for i in 0..calls {
            let _: f64 = inst.call_typed(&ease, (i as f64 / calls as f64,)).unwrap();
        }
    });
    row("10^6 calls of a Float curve", calls_time, &format!("{:?} per call, {} instructions each", calls_time.0 / calls, ease_instructions));

    // A VM loop: allocation, closures, and tail calls in std's fold and range.
    let n = 1_000_000i64;
    let mut sum_result = 0i64;
    let fold_time = time(5, || sum_result = inst.call_typed(&sum, (n,)).unwrap());
    let fold_instructions = inst.stats().instructions;
    assert_eq!(sum_result, n * (n - 1) / 2);
    let per_element = fold_instructions as f64 / n as f64;
    row(
        "fold over range 0..10^6",
        fold_time,
        &format!("{} instructions ({:.1} per element), {:.0} M instructions/s", fold_instructions, per_element, fold_instructions as f64 / fold_time.0.as_secs_f64() / 1e6),
    );

    if gate {
        // Minimums (the least noisy measure) of the larger runs against the smaller ones, without the fixed cost
        // of checking std: linear scaling is a factor of 2.
        let std_cost = check_std.1.as_secs_f64();
        for (what, small, large, fixed) in [
            ("checking", check_small.1, check_large.1, std_cost),
            ("evaluation", render_small.1, render_large.1, 0.0),
        ] {
            let factor = (large.as_secs_f64() - fixed) / (small.as_secs_f64() - fixed);
            println!("gate: doubling the document multiplies {} time by {:.2}", what, factor);
            if factor > MAX_DOUBLING_FACTOR {
                failures.push(format!("{} scales worse than linearly: doubling the document took {:.2}× as long", what, factor));
            }
        }
        if ease_instructions > EASE_INSTRUCTIONS {
            failures.push(format!("a call of `ease` takes {} instructions, more than the recorded {}", ease_instructions, EASE_INSTRUCTIONS));
        }
        if fold_instructions > FOLD_INSTRUCTIONS {
            failures.push(format!("the fold takes {} instructions, more than the recorded {}", fold_instructions, FOLD_INSTRUCTIONS));
        }
        if failures.is_empty() {
            println!("gate: ok");
        } else {
            for f in &failures {
                eprintln!("gate failed: {}", f);
            }
            std::process::exit(1);
        }
    }
}
