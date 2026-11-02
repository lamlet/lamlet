//! An animation curve evaluator: the document describes motion as functions of time, and the host calls them
//! once per frame. The document is checked once; each frame is only a call to a compiled function.
//!
//! `cargo run --release -p lamlet --example host_curve`

use lamlet::embed::*;
use std::time::Instant;

const CURVES: &str = r#"$[Import* std]$[Import* stage]
Eases from 0 to 1 over the unit interval: slow, fast, slow.
$[# ease {^ {t} {* t {* t {- 3.0 {* 2.0 t}}}}}]
The light's brightness at time t (seconds): it rises over two seconds, scaled by the stage's power.
$[# brightness {^ {t} {* {ease {clamp {/ t 2.0} 0.0 1.0}} {power}}}]
"#;

fn main() -> Result<(), Error> {
    let mut engine = Engine::new();
    // The stage's power level: a runtime input the host changes between frames.
    engine.natives_mut().function("stage.grid/power", |ctx, ()| ctx.input_as::<f64>("power")).register()?;
    let mut packages = MemoryResolver::new();
    packages.add("stage", &engine.natives().declarations("stage")?, Exports::All);
    engine.set_resolver(packages);

    let doc = engine.check(SourceFile::new("curves.lam", CURVES))?;
    let brightness = doc.binding("brightness")?;
    println!("brightness : {}", brightness.type_text);
    println!("doc: {}\n", doc.doc("brightness").unwrap_or(""));

    let mut instance = engine.instance(&doc)?;
    let fps = 24.0;
    let mut samples = Vec::new();
    for frame in 0..=72 {
        let t = frame as f64 / fps;
        // Inputs persist on the instance; each frame changes only what moved.
        instance.set_input("power", if frame % 12 == 6 { 0.8 } else { 1.0 })?;
        let b: f64 = instance.call_typed(&brightness, (t,))?;
        samples.push(b);
        if frame % 6 == 0 {
            println!("t = {:4.2}s  {:5.3}  {}", t, b, "#".repeat((b * 40.0).round() as usize));
        }
    }
    assert_eq!(samples[0], 0.0);
    assert_eq!(samples[24], 0.5);
    assert_eq!(samples[48], 1.0);
    assert_eq!(samples[54], 0.8);

    // Calls are cheap enough for every frame of an interactive host.
    let calls = 100_000;
    let started = Instant::now();
    for i in 0..calls {
        let _: f64 = instance.call_typed(&brightness, (i as f64 / calls as f64,))?;
    }
    let per_call = started.elapsed() / calls;
    println!("\n{} calls, {:?} per call", calls, per_call);
    Ok(())
}
