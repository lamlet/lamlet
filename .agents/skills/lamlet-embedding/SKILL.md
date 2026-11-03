---
name: lamlet-embedding
description: >-
  Embed Lamlet in a Rust program, or write a Lamlet extension. Use this skill whenever Rust code uses
  `lamlet::embed` (Engine, Document, Instance, natives, opaque host types, Data, Limits, resolvers), when a host
  checks documents at build time, calls a document's functions (curves, rules) from Rust, gives documents
  functions of its own, or when you build a `lamlet-NAME` extension program that depends on lamlet and
  lamlet-doc. Also use it when changing crates/lamlet/src/embed.rs or the example hosts. For the language,
  use the `lamlet` skill; for building pages, `lamlet-documents`.
---

# Embedding Lamlet

A **host** is a Rust program that checks Lamlet documents, uses their values, calls their functions, and gives
them functions of its own (*natives*). Lamlet computes; the host decides what values mean and does every
effect. The supported API is one module, `lamlet::embed`; everything else in the crate is internal and may
change between minor versions.

Read these for detail; this skill is the map and the rules:

| Path | Holds |
|---|---|
| `docs/embedding.md` | The embedding guide (normative for the API) |
| `docs/host.md` | The host contract every host follows |
| `references/host-api.md` | The API and the CLI on one page, including the internal API under `embed` |
| `crates/lamlet/examples/host_*.rs` | Four complete hosts that check their own output in CI; copy from these |
| `crates/lamlet/tests/embed.rs`, `natives.rs` | The API's tests: the precise behavior of each call |

**Before you claim host code works, compile and run it.** The examples run with
`cargo run -p lamlet --example host_text` (also `host_build_check`, `host_curve`, `host_threads`).

## 1. The shape of a host

```rust
use lamlet::embed::*;

let mut engine = Engine::new();                                    // std, plus std's natives
// natives and packages first (§2, §3): changing them makes the engine check std again
let doc = engine.check(SourceFile::new("room.lam", source))?;      // 1. types, stages, and static values
let text = doc.render(text)?;                                      // 2. use the values prepared during the check
let mut instance = engine.instance(&doc)?;                         // 3. run the rest
let area = doc.binding("area")?;                                   //    a static top-level binding
let a: f64 = instance.call_typed(&area, (3.0, 4.0))?;              //    Rust arguments as a tuple
```

- **One engine, many documents.** Set the engine up once. It checks `std` once and starts every later check
  from that state.
- **Compile once, run many times.** A `Document` is immutable. An instance evaluates or calls any number of
  times, and a failed call leaves it usable (except after an `Internal` error).
- **Which regions are prepared during the check:** `$name` of a static binding and `$[% e]`. Every `${...}` is
  a call and runs in an instance, even with no inputs. `doc.render` fails on those; use
  `doc.render_with(&mut instance, format)`.
- **Text inside another file:** `SourceFile::new("chapter.md", block).at(line, 1)` makes every diagnostic name
  the line of the outer file.

## 2. Natives

```rust
let natives = engine.natives_mut();
natives.function("garden.count/leaves", |_ctx, (tree,): (String,)| Ok(tree.len() as i64))
    .pure()                                                        // callable in [% ...] and at runtime
    .doc("Fn([String], Int). Leaves on one of the garden's trees.")
    .register()?;
natives.function("garden.calendar/today", |ctx, ()| ctx.input_as::<i64>("day")).register()?;   // Runtime
```

| Availability | Builder | Callable | For |
|---|---|---|---|
| Pure | `.pure()` | during the check and at runtime | deterministic lookups and computations |
| Runtime | `.runtime()`, the default | at runtime only | anything reading inputs or the outside world |
| Static | `.static_()` | during the check only | values fixed when the document is prepared |

- **The Lamlet type comes from the closure's Rust types.** Arguments arrive as one tuple (`()` for none,
  `(x,)` for one). The closure returns `Result<T, String>`; an `Err` becomes a diagnostic naming the native and
  the calling line.
- **Conversions:** `()`, `bool`, `i64`, `f64` (finite), `String`, `Vec<T>`, `Option<T>` (std's option),
  `(A, B)` (std's pair), `Value`, `Data`, `Opaque<T>`. Records: `record().field("x", 1i64).build()?`.
- **No function-typed native arguments** (v1). A native cannot call back into Lamlet.
- **Inputs, not ambient reads.** A native reads the world only through `ctx.input(...)` values the host set
  with `instance.set_input`, or through its own captured state. Keep Pure natives deterministic: documents are
  meant to give the same result every time.
- **Capabilities:** `.capability("network")` makes the call require `instance.grant("network")`.
- **Ids** are a dotted package, `/`, and a dashed name: `garden.tree/make-widget`. The Lamlet name is the last
  segment in camel case (`makeWidget`). Two natives with the same Lamlet name are an error.

### Declarations: how documents see natives

A document must declare what it calls. Let the registry write the declarations and serve them as a package:

```rust
let mut packages = MemoryResolver::new();
packages.add("garden", &engine.natives().declarations("garden")?, Exports::All);
engine.set_resolver(packages);          // documents: $[Import* garden] ... {leaves "oak"}
```

To keep `content` too, start from `lamlet_doc::resolver()` and `add` to it. A declaration written by hand
(`[Native leaves {Fn {' String} Int} "garden.count/leaves" Pure]`) must match the registration exactly, types and
availability; a mismatch is a `Link` error when the instance is made.

### Host objects

```rust
struct Tree { name: String }
impl NativeType for Tree { const TYPE_ID: &'static str = "garden.tree"; }

natives.opaque::<Tree>()?;                                         // before functions that use it
natives.function("garden.tree/plant", |_, (name,): (String,)| Ok(Opaque::new(Tree { name }))).register()?;
natives.function("garden.tree/name", |_, (t,): (Opaque<Tree>,)| Ok(t.name.clone())).pure().register()?;
```

Lamlet can store and pass an opaque value but not look inside it, compare it, or hash it. `Opaque<T>` derefs
to `T`; `T` drops when the last handle is gone.

## 3. Packages and resolvers

- A package is a document checked as a module. The host supplies packages through a `Resolver`
  (`MemoryResolver` is the ready one); **the core never reads files or searches directories.** A host that
  keeps packages in files reads them itself and adds the text.
- `Exports::All` exports every top-level name not starting with `_`, macros as `!name`. `Exports::Names` lists
  them, and may re-export imported names (submodules).
- `std` is always supplied; `K` is built in and cannot be replaced.

## 4. Checking at build time

`engine.check_interface(file)` checks against the natives' declarations only and never calls them, so a build
script or CI job can check documents without the host's runtime. Register natives with bodies that are never
called (`host_build_check.rs` uses `unreachable!`). Its one limit: a `[% ...]` that calls a native needs the
implementation, so it fails there.

`engine.options_mut().require_linked = true` (or `lamlet check --require-linked`) fails the check on a declared
native that is not registered, instead of at `engine.instance`.

## 5. Values, errors, threads, budgets

- **`Value`** holds functions and host objects and stays on its thread. **`Data`** (`Data::from_value`) is a
  plain `Send + Sync` copy with no functions or opaques. Send `Data`, Strings, or your own types between threads.
- **Errors** are one `Error`: `kind()` is `Check`, `Link`, `Runtime`, `Value`, `Registry`, or `Internal`;
  `diagnostics()` and `locations()` give detail; `Display` renders them. `Internal` is a caught panic (a bug in
  Lamlet or a native): make a new instance.
- **Threads:** `Engine`, `Document`, and `Instance` stay on their thread. Give each worker its own engine, built
  by one shared setup function (`host_threads.rs`). Sharing a checked document is not available.
- **Budgets:** `instance.set_limits(Limits { max_instructions: …, ..Limits::default() })` bounds every
  evaluation and call (instructions, frames, stack, allocation, string size, native calls); preparation has its
  own (`engine.options_mut().static_limits`). `instance.cancel_flag()` stops a running call from another thread.
- **Untrusted documents** cannot crash the host or run forever, but cap their byte size and consider checking on
  a worker thread (the slowest known input takes about 1.2 s to hit the macro budget).
- **Per-frame calls** are cheap (about half a microsecond for a small curve, `host_curve.rs`): set the inputs
  that changed, call again; nothing is rebuilt.

## 6. Extensions

Tools for one kind of document are extensions, not features of Lamlet:

- **The program** is `lamlet-NAME`, in its own git repository named after it (`lamlet/lamlet-present`).
  `lamlet NAME ARGS...` runs it from the PATH when NAME is neither a command nor a file, with the `lamlet`
  program's path in the `LAMLET` environment variable.
- **Dependencies by git tag**, never by path, and the tags of `lamlet` and `lamlet-doc` must be the same
  release:

  ```toml
  lamlet = { git = "https://github.com/lamlet/lamlet", tag = "v0.3.1" }
  lamlet-doc = { git = "https://github.com/lamlet/lamlet", tag = "v0.3.1" }
  ```

  A host that depends on both Lamlet and an extension's crate must use the tag the extension uses, or the build
  holds two copies of Lamlet whose types do not match.
- **Install** with `cargo install --git URL`. There is no registry and no package manager.
- **Its Lamlet packages** live inside the program (`include_str!`), and it can print them for other tools
  (`lamlet present source > present.lam`, then `lamlet build deck.lam --package present=present.lam`).
- **Names** follow `docs/conventions.md`: package `snake_case` (`present`, `garden_tools`), ids and files with
  dashes, JSON keys in lower camel case with a versioned `lamlet…` key (`{"lamletSchedule": 1, ...}`).

## 7. Rules for changing the embedding surface

- **The core stays embeddable, like Lua.** `crates/lamlet` has no dependencies and no file, environment, process,
  or network access. Never add search paths, manifests, or fetching to it; such conveniences belong in the CLI
  or an opt-in resolver a host chooses. `lamlet-doc` depends only on `lamlet`.
- **No host-specific features in Lamlet.** A need of one host becomes a general mechanism (a native, an input,
  a resolver) or lives in that host or an extension. Docs and examples describe hosts generally.
- **Host functionality is a native, never a K primitive.**
- Changing `embed.rs`: update `docs/embedding.md`, `references/host-api.md`, and the example hosts, and run
  `cargo test` and each example.
