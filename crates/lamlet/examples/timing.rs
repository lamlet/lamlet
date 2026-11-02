//! Rough timings (not a benchmark suite): `cargo run --release -p lamlet --example timing`.

use lamlet::*;
use std::rc::Rc;
use std::time::Instant;

fn bench(label: &str, src: &str) {
    let natives = Rc::new(NativeRegistry::new());
    let t = Instant::now();
    let doc = check_document("<bench>", src, MemoryResolver::new().into_rc(), natives.clone(), &Options::default()).unwrap();
    let checked = t.elapsed();
    let mut inst = Instance::new(doc.program.clone(), natives).unwrap();
    let t = Instant::now();
    let r = inst.evaluate(HostContext::default(), Limits::default()).unwrap();
    let ran = t.elapsed();
    let st = inst.stats();
    println!(
        "{label}: check {:?}, run {:?}, {} instructions ({:.1} ns/instr), alloc ~{} KiB, max frames {} -> {:?}",
        checked, ran, st.instructions, ran.as_nanos() as f64 / st.instructions.max(1) as f64, st.alloc_bytes / 1024, st.max_frames, r.last().map(|x| &x.1)
    );
}

fn main() {
    bench("std only", "$[Import* std] ${+ 1 2}");
    bench("100k range", "$[Import* std] ${length {range 0 100000}}");
    bench("100k range+map+fold", "$[Import* std] ${fold {map {range 0 100000} {^ {x} {* x 2}}} 0 +}");
    bench(
        "10k-char string",
        "$[Import* std] $[# s [% {concat {\' \"ab{}cd\" \"ab{}cd\" \"ab{}cd\" \"ab{}cd\"}}]] $[# go {^ go {n acc} {!if {= n 0} acc {go {- n 1} {++ acc s}}}}] ${{go 400 \"\"}.chars.length}",
    );
}
