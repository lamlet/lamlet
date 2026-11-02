//! Compile a Lamlet presentation and write a render-lab timeline plus its narration manifest.
//!
//! cargo run -p lamlet-present --example render_lab_timeline -- [PRESENTATION.lam] [OUT_DIR]
//!
//! Then, in render-lab: `render-lab --investigation o3de/lighting render OUT_DIR/timeline.json --snapshot 0,60`.

use lamlet_present::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/examples/lighting.lam").into());
    let out = std::path::PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "lamlet-timeline".into()));
    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {}", path, e));
    let run = || -> Result<(), String> {
        let mut p = Presentation::compile(&path, &source)?;
        let mut voice = Cached::new(EstimatedVoice::default());
        let (script, narration, show) = p.build(&mut voice)?;
        let timeline = p.to_timeline(&show, &TimelineOptions::default())?;
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        let write = |name: &str, j: &serde_json::Value| std::fs::write(out.join(name), serde_json::to_string_pretty(j).unwrap()).map_err(|e| e.to_string());
        write("timeline.json", &timeline)?;
        write("narration.json", &narration_manifest(&script, &narration, &show))?;
        eprintln!(
            "{} beats, {} keys, {} frames, {} sentences synthesized -> {}",
            show.beats.len(),
            timeline["keys"].as_array().unwrap().len(),
            timeline["totalFrames"],
            voice.syntheses,
            out.display()
        );
        Ok(())
    };
    if let Err(e) = run() {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}
