# Host interface and the render-lab integration

Lamlet computes values. Hosts decide what values mean and when effects happen. This document has three parts:

- the general contract between Lamlet and any host
- the presentation vocabulary (`present`)
- the render-lab adapter (`crates/lamlet-present`), which is implemented and tested

## 1. Three layers

| Layer | Owner | Examples | Can it have effects? |
|---|---|---|---|
| Pure computation | Lamlet | Text with computed values, decisions, easing curves, timing arithmetic | No. It is deterministic given its inputs. |
| Description of effects | Lamlet values with host-defined types | `#set`, `#formula`, `#animate`, sentences to speak | No: they are data. |
| Execution | The host | Synthesizing speech, playing audio, blending channels, rendering frames, seeking | Yes |

Lamlet never calls the speech engine, the renderer, or a clock that the host did not pass in explicitly. A host
reads a computed value, and reading it never re-runs the source that produced it.

## 2. When things are evaluated

```text
compile (once)       check_document: types, stages; [% ...] runs; script and computed text are prepared
synthesize (once)    host: each distinct sentence → audio + Narration{seconds, words}   (cached by text)
build (once/edit)    Instance::call(show, [narration]) → Show (beats, captions, actions)
lower (once)         adapter: seconds → frames; Animate curves sampled per frame via Instance::call
play (every frame)   render-lab: Timeline::state_at(frame), unchanged
```

These are the properties the requirements asked for:

- **The document is not re-evaluated per frame.** Per-frame work is a call to an already compiled closure, or
  none at all once the timeline has been lowered.
- **Pause, resume, and seek** belong to render-lab. A Timeline is a pure function of the frame, and Lamlet
  curves are pure functions of time.
- **Deterministic replay and offline rendering** follow from purity. The same document and the same narration
  measurements give byte-identical timeline JSON, and this is tested.
- **Audio/visual sync** comes from cue words. `@name` in a sentence marks the next spoken word, and the cue's
  time is that word's measured start. Visual timing is computed from measured speech, never estimated
  separately.
- **No replay of speech.** Speech playback cannot be triggered by evaluation: evaluation only returns
  descriptions. Synthesis happens in the host, through `Cached<S>`, once per distinct text. Rebuilding a show
  does not synthesize again, and this is tested.

## 3. The `present` package (Lamlet)

The package is `crates/lamlet-present/src/present.lam`, an ordinary Lamlet package. Its vocabulary:

| Function | Type (informal) | Meaning |
|---|---|---|
| `say id text` | `→ {id, text, cues: List({name, word})}` | A sentence; each `@name` word becomes a cue |
| `timeline script narration pause` | `→ List(Timed)` | Lays sentences end to end using measured narration |
| `at timed name`, `end timed` | `→ Float` | The absolute time of a cue, or of the sentence's end |
| `caption timed` | `→ {start, end, text}` | A caption shown while the sentence is spoken |
| `set t blend params`, `demo t name`, `formula t latex`, `hideFormula t` | `→ Action` | Descriptions of host actions |
| `animate t seconds channel curve` | `→ Action` | `curve : Fn([Float], Float)` over local time |
| `tween a b seconds ease`, `wave c amp period`, `smooth`, `easeInOut` | curves | Animation as pure functions of time (`Animation<T> ≈ Time → T`) |
| `beat id headline kind sentences actions tail` | `→ Beat` | One presentation step |
| `inlineFormula latex` | `→ String` | A render-lab rich-text formula run |

Everything above is plain Lamlet code over `std` and needs no compiler support. `Action` is a variant:

```text
<set: {at, blend, params}, demo: {at, name}, formula: {at, tag}, animate: {at, seconds, channel, curve}>
```

A presentation document defines two static bindings:

- `script`, the list of sentences
- `show : Fn([List({seconds: Float, words: List(Float)})], {title, beats})`

## 4. The render-lab adapter (Rust)

The adapter is `crates/lamlet-present`, which depends on `lamlet` and `serde_json` only:

- **`Presentation::compile(name, source)`** checks the document and prepares its static values.
- **`Presentation::build(&mut dyn Synthesizer)`** synthesizes the script and calls `show`. It returns the
  script, the narration measurements, and the decoded `Show`.
- **`Presentation::to_timeline(&Show, &TimelineOptions)`** writes render-lab's `Timeline` JSON
  (`lab/src/shot.rs`):
  - beats are contiguous
  - captions are given in frames
  - `#set`, `#demo`, and `#formula` become `Key`s
  - each `#animate` is sampled at every frame it covers
- **`narration_manifest`** describes the speech for a TTS host: text, start, duration, and word times.
- **Synthesizers.** `Synthesizer` is the host's speech engine. `EstimatedVoice` is a deterministic stand-in
  that produces no audio. `Cached<S>` adds once-per-text caching.

Example:

```sh
cargo run -p lamlet-present --example render_lab_timeline -- crates/lamlet-present/examples/lighting.lam out
render-lab --investigation o3de/lighting render --timeline out/timeline.json --snapshot 0,120,260 --resolution 720p
```

The example presentation is `examples/lighting.lam`. It has a rich-text document (prose plus computed regions)
and these parts:

1. **A computed value.** O3DE's attenuation window at half radius is computed in Lamlet and inserted into
   speech and captions ("88 percent").
2. **A conditional decision.** `bright` selects the closing line, the beat headline, and the final
   `attenuationRadius`.
3. **Two time-dependent animations.** An eased intensity tween starts at the spoken cue `@light`, and a camera
   sway is sampled per frame.
4. **Speech with synchronization.** Cue words place the formula card and the radius change.

### Verification performed

- **`crates/lamlet-present/tests/present.rs`**, 7 tests:
  - the Key fields follow render-lab's schema
  - beats are contiguous
  - cue-to-frame timing is correct
  - curves are monotone and give the same values in any sampling order
  - rebuilds are byte-identical and never resynthesize
  - one document makes both bright and dim decisions
  - errors surface through the boundary
- **render-lab itself.** It loaded and validated the generated timeline with its own `Timeline::load`, which
  checks channel names and actions against the investigation. It then rendered frames 0, 120, and 260 on the
  GPU:
  - frame 0: light at 0.0 cd
  - frame 120: 6.0 cd after the Lamlet tween, the falloff formula card, and the computed caption
  - frame 260: the "A bright room" headline, the formula hidden, and the radius mid-blend toward 9 m

  This was a manual run, not part of `cargo test`, because it needs render-lab's build and a GPU.

### Not done (known limitations)

- **No real speech.** No audio is produced: `EstimatedVoice` stands in for render-lab's Piper pipeline
  (`tools/video/narration.py`). A real host implements `Synthesizer` by:
  - running Piper
  - storing `<sha256(text+speed+voice)>.wav` as narration.py does
  - returning `{seconds, words}` from the phoneme durations

  `narration.json` then tells the muxer where each sentence plays.
- **Dense keys.** Animations are written as one key per frame. render-lab's `Motion::Path` could represent
  them more compactly; that is an optimization, not a semantic change.
- **No live hook.** render-lab has no runtime hook for live computed text: captions are fixed when the timeline
  is built. Live per-frame Lamlet evaluation would need an `#[action]` in a render-lab package that calls
  `Instance::call` with `ActionFrame.elapsed`. That seam exists in render-lab, but this prototype does not use
  it.
- **No interactive state.** Interactive `update : State → Event → State` is supported by the language (records
  plus a pure function), but there is no demonstration. render-lab dispatches click events only to Rust
  handlers.

## 5. General host contract (any host)

These rules apply to every host, not just render-lab:

- **Inputs** are explicit: function arguments, or `HostContext` inputs read by natives. There are no ambient
  reads.
- **Capabilities** are granted per evaluation (`HostContext::grant`) and checked on every native call.
- **Budgets** (`Limits`) apply to every evaluation and call. A failure is atomic: no partial outputs are
  returned.
- **Values** cross the boundary as `lamlet::Value`:
  - records have sorted fields and can be read with `Value::field`
  - variants are `(label, payload)`
  - functions are opaque, and the host calls them with `Instance::call`
- **Types** of static bindings are available as `TypeDesc` (`StaticBinding::ty_desc`), so a host can check that
  a document defines what it expects before calling it.
