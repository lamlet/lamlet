//! A host adapter from Lamlet presentation descriptions to render-lab timelines.
//!
//! The boundary has three parts (docs/host.md):
//!
//! 1. **Pure computation (Lamlet).** A document using the `present` package defines `script`, the sentences to
//!    speak, and `show`, a function from measured narration to a description of beats, captions, and actions.
//! 2. **Description of effects (this crate).** [`Presentation`] decodes those values into typed Rust data and
//!    samples animation curves at frame times by calling the Lamlet closures with
//!    [`lamlet::Instance::call`]. The document is never re-evaluated per frame.
//! 3. **Execution (the host).** Speech synthesis is a [`Synthesizer`] the host supplies. Playback, blending,
//!    seeking, and recording stay in render-lab, which consumes the [`Timeline`] JSON this crate writes.
//!
//! The crate depends on `lamlet` and `serde_json` only. render-lab is not a dependency: its `Timeline` schema
//! (lab/src/shot.rs) is the contract.

use lamlet::value::{List, Value};
use lamlet::*;
use serde_json::{json, Map, Value as Json};
use std::collections::HashMap;
use std::rc::Rc;

/// The `present` package's Lamlet source.
pub const PRESENT_SOURCE: &str = include_str!("present.lam");

/// A resolver with `std` and the `present` package.
pub fn resolver() -> MemoryResolver {
    let mut r = MemoryResolver::new();
    r.add("present", PRESENT_SOURCE, Exports::All);
    r
}

/// A sentence to speak, as the document describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sentence {
    pub id: String,
    pub text: String,
    /// Cue name and the index of the spoken word it precedes.
    pub cues: Vec<(String, usize)>,
}

/// What a synthesizer measured for one sentence: its duration and each word's start time (seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct Narration {
    pub seconds: f64,
    pub words: Vec<f64>,
}

/// A speech synthesizer: turns text into audio the host keeps, and reports its timing. Lamlet never calls it.
pub trait Synthesizer {
    fn synthesize(&mut self, text: &str) -> Result<Narration, String>;
}

/// A deterministic stand-in for a real voice (render-lab's Piper pipeline): each word lasts in proportion to
/// its length. It produces no audio; it exists so the boundary can be exercised and tested offline.
#[derive(Clone, Debug)]
pub struct EstimatedVoice {
    pub seconds_per_char: f64,
    pub seconds_per_word: f64,
}

impl Default for EstimatedVoice {
    fn default() -> Self {
        EstimatedVoice { seconds_per_char: 0.055, seconds_per_word: 0.08 }
    }
}

impl Synthesizer for EstimatedVoice {
    fn synthesize(&mut self, text: &str) -> Result<Narration, String> {
        let mut t = 0.0;
        let mut words = Vec::new();
        for w in text.split(' ').filter(|w| !w.is_empty()) {
            words.push(t);
            t += self.seconds_per_word + self.seconds_per_char * w.chars().count() as f64;
        }
        Ok(Narration { seconds: t, words })
    }
}

/// Synthesizes each distinct text at most once. Evaluating or rebuilding a presentation therefore never
/// repeats synthesis (or playback, which the host starts from its own audio track).
pub struct Cached<S> {
    pub inner: S,
    cache: HashMap<String, Narration>,
    pub syntheses: usize,
}

impl<S: Synthesizer> Cached<S> {
    pub fn new(inner: S) -> Self {
        Cached { inner, cache: HashMap::new(), syntheses: 0 }
    }
}

impl<S: Synthesizer> Synthesizer for Cached<S> {
    fn synthesize(&mut self, text: &str) -> Result<Narration, String> {
        if let Some(n) = self.cache.get(text) {
            return Ok(n.clone());
        }
        self.syntheses += 1;
        let n = self.inner.synthesize(text)?;
        self.cache.insert(text.to_string(), n.clone());
        Ok(n)
    }
}

/// A host action at a time, decoded from the Lamlet `Action` variant.
#[derive(Clone, Debug)]
pub enum Action {
    Set { at: f64, blend: f64, params: Vec<(String, f64)> },
    Demo { at: f64, name: String },
    Formula { at: f64, tag: String },
    /// `curve` is a Lamlet function of local time; it is sampled, not interpreted.
    Animate { at: f64, seconds: f64, channel: String, curve: Value },
}

#[derive(Clone, Debug)]
pub struct Caption {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Beat {
    pub id: String,
    pub headline: String,
    pub kind: String,
    pub start: f64,
    pub seconds: f64,
    pub captions: Vec<Caption>,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug)]
pub struct Show {
    pub title: String,
    pub beats: Vec<Beat>,
}

/// Output settings for a render-lab timeline.
#[derive(Clone, Debug)]
pub struct TimelineOptions {
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub engine_label: String,
}

impl Default for TimelineOptions {
    fn default() -> Self {
        TimelineOptions { fps: 30, width: 1920, height: 1080, engine_label: "Lamlet".into() }
    }
}

/// A checked presentation document and an instance to call its functions.
pub struct Presentation {
    pub doc: Document,
    instance: Instance,
    limits: Limits,
}

type R<T> = Result<T, String>;

impl Presentation {
    /// Check a presentation document. Static values (the script, computed text) are prepared here, once.
    pub fn compile(name: &str, source: &str) -> R<Presentation> {
        let natives = Rc::new(NativeRegistry::new());
        let doc = check_document(name, source, resolver().into_rc(), natives.clone(), &Options::default()).map_err(|f| f.render())?;
        let instance = Instance::new(doc.program.clone(), natives).map_err(|d| d.render(&doc.sources))?;
        Ok(Presentation { doc, instance, limits: Limits::default() })
    }

    fn binding(&self, name: &str) -> R<Value> {
        self.doc.static_binding(name).cloned().ok_or_else(|| format!("the presentation must define a static `{}`", name))
    }

    fn call(&mut self, f: &Value, args: Vec<Value>) -> R<Value> {
        self.instance.call(f, args, HostContext::default(), self.limits.clone()).map_err(|e| e.to_diagnostic().render(&self.doc.sources))
    }

    /// The sentences to synthesize, in order.
    pub fn script(&self) -> R<Vec<Sentence>> {
        list(&self.binding("script")?)?
            .iter()
            .map(|s| {
                let cues = list(get(s, "cues")?)?.iter().map(|c| Ok((string(get(c, "name")?)?, int(get(c, "word")?)? as usize))).collect::<R<_>>()?;
                Ok(Sentence { id: string(get(s, "id")?)?, text: string(get(s, "text")?)?, cues })
            })
            .collect()
    }

    /// Synthesize the script (through the host's synthesizer) and build the show from the measurements.
    pub fn build(&mut self, voice: &mut dyn Synthesizer) -> R<(Vec<Sentence>, Vec<Narration>, Show)> {
        let script = self.script()?;
        let narration = script.iter().map(|s| voice.synthesize(&s.text)).collect::<R<Vec<_>>>()?;
        let show = self.show(&narration)?;
        Ok((script, narration, show))
    }

    /// Call the document's `show` with measured narration and decode the result.
    pub fn show(&mut self, narration: &[Narration]) -> R<Show> {
        let arg = Value::List(List::from_vec(
            narration
                .iter()
                .map(|n| {
                    Value::record(vec![
                        (Rc::from("seconds"), Value::Float(n.seconds)),
                        (Rc::from("words"), Value::List(List::from_vec(n.words.iter().map(|w| Value::Float(*w)).collect()))),
                    ])
                })
                .collect(),
        ));
        let f = self.binding("show")?;
        let v = self.call(&f, vec![arg])?;
        decode_show(&v)
    }

    /// Sample an animation curve at a local time.
    pub fn sample(&mut self, curve: &Value, t: f64) -> R<f64> {
        let v = self.call(curve, vec![Value::Float(t)])?;
        float(&v)
    }

    /// Lower a show to render-lab's `Timeline` JSON (lab/src/shot.rs). Animations are sampled at every frame
    /// they cover and written as keys that set the channel; render-lab blends and plays them.
    pub fn to_timeline(&mut self, show: &Show, o: &TimelineOptions) -> R<Json> {
        let fps = o.fps as f64;
        let frame = |t: f64| (t * fps).round().max(0.0) as u64;
        let mut beats = Vec::new();
        let mut keys: Vec<Json> = Vec::new();
        let mut start = 0u64;
        for b in &show.beats {
            let begin = frame(b.start);
            if begin < start {
                return Err(format!("beat `{}` starts before the previous beat ends", b.id));
            }
            // Beats are contiguous: a gap becomes part of the previous beat.
            if let Some(prev) = beats.last_mut() {
                let prev: &mut Json = prev;
                prev["frames"] = json!(begin - prev["startFrame"].as_u64().unwrap());
            } else if begin != 0 {
                return Err("the first beat must start at time 0".into());
            }
            let end = frame(b.start + b.seconds).max(begin + 1);
            let captions: Vec<Json> = b.captions.iter().map(|c| json!({"start": frame(c.start), "end": frame(c.end), "text": c.text})).collect();
            let kind = match b.kind.as_str() {
                "3d" | "2d" => b.kind.clone(),
                other => return Err(format!("beat `{}` has kind {:?}; render-lab expects \"3d\" or \"2d\"", b.id, other)),
            };
            beats.push(json!({"id": b.id, "chapter": 1, "kind": kind, "startFrame": begin, "frames": end - begin,
                "headline": b.headline, "captions": captions}));
            for a in &b.actions {
                match a {
                    Action::Set { at, blend, params } => {
                        let mut m = Map::new();
                        for (n, v) in params {
                            m.insert(n.clone(), json!(v));
                        }
                        keys.push(json!({"frame": frame(*at), "blend": frame(*blend), "params": m, "source": b.id}));
                    }
                    Action::Demo { at, name } => keys.push(json!({"frame": frame(*at), "blend": 0, "demo": name, "source": b.id})),
                    Action::Formula { at, tag } => keys.push(json!({"frame": frame(*at), "blend": 0, "formula": tag, "source": b.id})),
                    Action::Animate { at, seconds, channel, curve } => {
                        let (f0, f1) = (frame(*at), frame(at + seconds));
                        for f in f0..=f1 {
                            let v = self.sample(curve, (f - f0) as f64 / fps)?;
                            keys.push(json!({"frame": f, "blend": 0, "params": {channel.clone(): v}, "source": b.id}));
                        }
                    }
                }
            }
            start = begin;
        }
        let total = match beats.last() {
            Some(b) => b["startFrame"].as_u64().unwrap() + b["frames"].as_u64().unwrap(),
            None => return Err("a show needs at least one beat".into()),
        };
        keys.sort_by_key(|k| k["frame"].as_u64().unwrap());
        Ok(json!({
            "fps": o.fps, "width": o.width, "height": o.height, "totalFrames": total,
            "title": show.title, "engineLabel": o.engine_label,
            "chapters": [{"number": 1, "title": show.title, "startFrame": 0, "endFrame": total}],
            "beats": beats, "keys": keys,
        }))
    }
}

/// The narration manifest a TTS host needs: each sentence's text and where it plays.
pub fn narration_manifest(script: &[Sentence], narration: &[Narration], show: &Show) -> Json {
    let starts: Vec<f64> = show.beats.iter().flat_map(|b| b.captions.iter().map(|c| c.start)).collect();
    Json::Array(
        script
            .iter()
            .zip(narration)
            .enumerate()
            .map(|(i, (s, n))| json!({"id": s.id, "text": s.text, "start": starts.get(i), "seconds": n.seconds, "words": n.words}))
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------------------
// Decoding Lamlet values (the checked types make these failures host bugs, but they are reported, not panics)

fn get<'a>(v: &'a Value, f: &str) -> R<&'a Value> {
    v.field(f).ok_or_else(|| format!("expected a record with field `{}`, found {:?}", f, v))
}

fn string(v: &Value) -> R<String> {
    match v {
        Value::Str(s) => Ok(s.to_string()),
        o => Err(format!("expected a String, found {:?}", o)),
    }
}

fn float(v: &Value) -> R<f64> {
    match v {
        Value::Float(f) => Ok(*f),
        o => Err(format!("expected a Float, found {:?}", o)),
    }
}

fn int(v: &Value) -> R<i64> {
    match v {
        Value::Int(n) => Ok(*n),
        o => Err(format!("expected an Int, found {:?}", o)),
    }
}

fn list(v: &Value) -> R<Vec<Value>> {
    match v {
        Value::List(l) => Ok(l.to_vec()),
        o => Err(format!("expected a List, found {:?}", o)),
    }
}

fn decode_action(v: &Value) -> R<Action> {
    let (tag, p) = match v {
        Value::Variant(x) => (&*x.0, &x.1),
        o => return Err(format!("expected an action variant, found {:?}", o)),
    };
    let at = float(get(p, "at")?)?;
    Ok(match tag {
        "set" => Action::Set {
            at,
            blend: float(get(p, "blend")?)?,
            params: list(get(p, "params")?)?.iter().map(|x| Ok((string(get(x, "name")?)?, float(get(x, "value")?)?))).collect::<R<_>>()?,
        },
        "demo" => Action::Demo { at, name: string(get(p, "name")?)? },
        "formula" => Action::Formula { at, tag: string(get(p, "tag")?)? },
        "animate" => Action::Animate {
            at,
            seconds: float(get(p, "seconds")?)?,
            channel: string(get(p, "channel")?)?,
            curve: get(p, "curve")?.clone(),
        },
        other => return Err(format!("unknown action #{}", other)),
    })
}

fn decode_show(v: &Value) -> R<Show> {
    let beats = list(get(v, "beats")?)?
        .iter()
        .map(|b| {
            Ok(Beat {
                id: string(get(b, "id")?)?,
                headline: string(get(b, "headline")?)?,
                kind: string(get(b, "kind")?)?,
                start: float(get(b, "start")?)?,
                seconds: float(get(b, "seconds")?)?,
                captions: list(get(b, "captions")?)?
                    .iter()
                    .map(|c| Ok(Caption { start: float(get(c, "start")?)?, end: float(get(c, "end")?)?, text: string(get(c, "text")?)? }))
                    .collect::<R<_>>()?,
                actions: list(get(b, "actions")?)?.iter().map(decode_action).collect::<R<_>>()?,
            })
        })
        .collect::<R<_>>()?;
    Ok(Show { title: string(get(v, "title")?)?, beats })
}
