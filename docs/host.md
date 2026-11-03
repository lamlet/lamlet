# Host interface

Lamlet computes values. Hosts decide what values mean and when effects happen. This document is the general
contract between Lamlet and any host. For the Rust API that implements it, see [embedding.md](embedding.md).
Narrated presentations, which apply this contract to speech-timed content, are the extension
`lamlet-present` ([documents.md](documents.md), section 6).

## 1. Three layers

| Layer | Owner | Examples | Can it have effects? |
|---|---|---|---|
| Pure computation | Lamlet | Text with computed values, decisions, easing curves, timing arithmetic | No. It is deterministic given its inputs. |
| Description of effects | Lamlet values with host-defined types | `#set`, `#formula`, `#animate`, sentences to speak | No: they are data. |
| Execution | The host | Synthesizing speech, playing audio, blending channels, rendering frames, seeking | Yes |

Lamlet never calls the speech engine, the renderer, or a clock that the host did not pass in explicitly. A host
reads a computed value, and reading it never re-runs the source that produced it.

## 2. General host contract (any host)

These rules apply to every host:

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
