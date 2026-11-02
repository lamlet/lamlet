//! End-to-end tests of the presentation boundary, without render-lab: the Lamlet document drives the timeline
//! through typed descriptions, and the adapter only lowers and samples them.

use lamlet_present::*;
use serde_json::Value as Json;

const LIGHTING: &str = include_str!("../examples/lighting.lam");

fn build(src: &str) -> (Presentation, Vec<Sentence>, Vec<Narration>, Show, Json) {
    let mut p = Presentation::compile("lighting.lam", src).unwrap_or_else(|e| panic!("{}", e));
    let mut voice = EstimatedVoice::default();
    let (script, narration, show) = p.build(&mut voice).unwrap();
    let timeline = p.to_timeline(&show, &TimelineOptions::default()).unwrap();
    (p, script, narration, show, timeline)
}

fn channel(timeline: &Json, name: &str) -> Vec<(u64, f64)> {
    timeline["keys"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|k| k["params"].get(name).map(|v| (k["frame"].as_u64().unwrap(), v.as_f64().unwrap())))
        .collect()
}

#[test]
fn script_marks_cues_without_speaking_them() {
    let (_, script, ..) = build(LIGHTING);
    assert_eq!(script.len(), 3);
    assert_eq!(script[0].text, "This room is lit by one point light.");
    assert_eq!(script[0].cues, vec![("light".to_string(), 5)]);
    // A computed value (the window at half radius, evaluated in Lamlet) is inserted into the spoken text.
    assert_eq!(script[1].text, "Its light fades smoothly: halfway to the radius, 88 percent remains.");
}

#[test]
fn timeline_follows_render_lab_schema() {
    let (_, _, _, show, t) = build(LIGHTING);
    // Beats are contiguous and cover totalFrames (Timeline::validate).
    let mut frame = 0;
    for b in t["beats"].as_array().unwrap() {
        assert_eq!(b["startFrame"].as_u64().unwrap(), frame);
        frame += b["frames"].as_u64().unwrap();
        assert!(["3d", "2d"].contains(&b["kind"].as_str().unwrap()));
    }
    assert_eq!(frame, t["totalFrames"].as_u64().unwrap());
    assert_eq!(show.beats.len(), 3);
    // Keys use only fields of render-lab's `Key` (it denies unknown fields) and are sorted by frame.
    let allowed = ["frame", "blend", "params", "source", "demo", "formula"];
    let keys = t["keys"].as_array().unwrap();
    for k in keys {
        for f in k.as_object().unwrap().keys() {
            assert!(allowed.contains(&f.as_str()), "unexpected key field {}", f);
        }
    }
    assert!(keys.windows(2).all(|w| w[0]["frame"].as_u64() <= w[1]["frame"].as_u64()));
    assert_eq!(keys[0]["demo"], "room");
}

#[test]
fn speech_timing_drives_visuals() {
    let (_, script, narration, show, t) = build(LIGHTING);
    // The intensity animation starts at the cue word ("one"), as measured by the voice.
    let intro_start = 0.0;
    let cue = intro_start + narration[0].words[script[0].cues[0].1];
    let first = channel(&t, "lightIntensity").into_iter().find(|(f, _)| *f > 0).unwrap();
    assert_eq!(first.0, (cue * 30.0).round() as u64);
    // Captions are timed to their sentence.
    let c = &show.beats[1].captions[0];
    assert!((c.end - c.start - narration[1].seconds).abs() < 1e-9);
    // The formula appears at its cue, with LaTeX built in Lamlet.
    let f = t["keys"].as_array().unwrap().iter().find(|k| k["formula"].as_str().is_some_and(|s| !s.is_empty())).unwrap();
    assert!(f["formula"].as_str().unwrap().starts_with("{@ formula=\"w = \\big(1"));
}

#[test]
fn animation_is_a_pure_function_of_time() {
    let (mut p, _, _, show, t) = build(LIGHTING);
    // The eased tween rises monotonically from 0 to the configured intensity.
    let samples: Vec<f64> = channel(&t, "lightIntensity").into_iter().map(|x| x.1).collect();
    assert!(samples.windows(2).all(|w| w[0] <= w[1] + 1e-12), "{:?}", samples);
    assert!((samples.last().unwrap() - 6.0).abs() < 1e-9);
    // Sampling in any order (seeking backwards, replay) gives identical values: curves are plain functions.
    let curve = show.beats[1].actions.iter().find_map(|a| match a {
        Action::Animate { curve, .. } => Some(curve.clone()),
        _ => None,
    });
    let curve = curve.unwrap();
    let forward: Vec<f64> = (0..20).map(|i| p.sample(&curve, i as f64 * 0.1).unwrap()).collect();
    let backward: Vec<f64> = (0..20).rev().map(|i| p.sample(&curve, i as f64 * 0.1).unwrap()).collect();
    assert_eq!(forward, backward.into_iter().rev().collect::<Vec<_>>());
}

#[test]
fn rebuilding_is_deterministic_and_never_resynthesizes() {
    let mut p = Presentation::compile("lighting.lam", LIGHTING).unwrap();
    let mut voice = Cached::new(EstimatedVoice::default());
    let (_, _, a) = p.build(&mut voice).unwrap();
    let ta = p.to_timeline(&a, &TimelineOptions::default()).unwrap();
    let (_, _, b) = p.build(&mut voice).unwrap();
    let tb = p.to_timeline(&b, &TimelineOptions::default()).unwrap();
    assert_eq!(ta, tb, "the same inputs give the same timeline");
    assert_eq!(voice.syntheses, 3, "each sentence is synthesized once, however often the show is rebuilt");
}

#[test]
fn presentation_decisions_are_computed() {
    let (_, script, _, show, t) = build(LIGHTING);
    assert_eq!(show.beats[2].headline, "A bright room");
    assert!(script[2].text.contains("glow warmly"));
    assert_eq!(channel(&t, "attenuationRadius").last().unwrap().1, 9.0);
    // Change one setting: the same document makes the other decision.
    let dim = LIGHTING.replace("$[# intensity 6.0]", "$[# intensity 3.0]");
    let (_, script, _, show, t) = build(&dim);
    assert_eq!(show.beats[2].headline, "A dim room");
    assert!(script[2].text.contains("stays dim"));
    assert_eq!(channel(&t, "attenuationRadius").last().unwrap().1, 4.0);
}

#[test]
fn errors_are_reported_through_the_boundary() {
    // A type error in the presentation is a checked diagnostic, before anything runs.
    let bad = LIGHTING.replace("{p \"cameraDrift\" 1.0}", "{p \"cameraDrift\" 1}");
    let e = Presentation::compile("lighting.lam", &bad).err().unwrap();
    assert!(e.contains("type error"), "{}", e);
    // A cue that does not exist fails when the show is built, with the Lamlet call stack.
    let bad = LIGHTING.replace("{at fall \"window\"}", "{at fall \"nope\"}");
    let mut p = Presentation::compile("lighting.lam", &bad).unwrap();
    let e = p.build(&mut EstimatedVoice::default()).err().unwrap();
    assert!(e.contains("no cue named nope"), "{}", e);
}
