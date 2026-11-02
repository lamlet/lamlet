# Embedding Lamlet in a Rust program

This guide is for a Rust program (a *host*) that uses Lamlet documents: it checks them, fills in their
computed values, calls their functions, and gives them functions of its own. Everything here is in one module,
`lamlet::embed`. The rest of the crate is internal and may change between minor versions.

Four complete programs accompany this guide in `crates/lamlet/examples/`. They check their own output and run
in CI:

| Example | What the host does |
|---|---|
| [`host_text.rs`](../crates/lamlet/examples/host_text.rs) | Prepares a text: computed values, a host function, a picture the host turns into markup |
| [`host_build_check.rs`](../crates/lamlet/examples/host_build_check.rs) | Checks the Lamlet blocks of a Markdown file at build time, with errors at Markdown lines |
| [`host_curve.rs`](../crates/lamlet/examples/host_curve.rs) | Calls animation curves once per frame, with the time and other inputs passed in |
| [`host_threads.rs`](../crates/lamlet/examples/host_threads.rs) | Prepares documents on worker threads and collects the results |

Run one with `cargo run -p lamlet --example host_text`.

## 1. The shape of a host

```rust
use lamlet::embed::*;

let engine = Engine::new();                                       // std, plus the standard natives
let doc = engine.check(SourceFile::new("room.txt", source))?;     // 1. check: types, and static values
let text = doc.render(text)?;                                     // 2. use the prepared values
let mut instance = engine.instance(&doc)?;                        // 3. run what needs the runtime
let area = doc.binding("area")?;
let a: f64 = instance.call_typed(&area, (3.0, 4.0))?;
```

1. **Check.** `engine.check` parses the document and checks its types. It also computes everything that does not
   depend on the runtime (§2). A failure is an [`Error`](#7-errors) listing diagnostics.
2. **Use prepared values.** A checked `Document` lists its prose and regions in order, with the values computed
   so far.
3. **Run.** An `Instance` evaluates the rest and calls the document's functions, any number of times.

An engine is set up once and checks any number of documents. It checks `std` the first time it is needed,
and starts every later check from that state. A small document that imports `std` then checks in about
0.13 ms instead of 1.1 ms, with the same result as checking from scratch. Changing the engine's resolver, natives, or options makes it
check `std` again. A document can have any number of instances.

## 2. Documents

A document is prose with active regions. `doc.parts()` returns them in order:

| `Part` | Meaning |
|---|---|
| `Text { text, at }` | Prose (with `$$` already turned into `$`) |
| `Region { region, value }` | An expression region. `value` is `Some` if it was computed during the check. |
| `Declaration { region }` | A `$[# ...]`, import, macro, or native declaration. It has no value. |
| `Comment { text, at }` | A disabled region (`$;{...}`). Hosts normally skip it. |

Each `Region` has an `index` (its position among all regions), its type (`ty` as a `TypeDesc` when it is
ground, `type_text` always), and a `Location` with line, column, and byte offsets.

### Which regions are computed during the check

| Region | Computed during the check? |
|---|---|
| `$name`, naming a static binding | Yes |
| `$[% expr]` | Yes. The expression must not need runtime input. |
| `${f a b}` | No: it is a call, and calls run in an instance, even when they need no input |

A binding is static when it is a literal, a lambda, a `[% ...]`, or built from static parts (spec §4.4). So
`$[# total [% {fold xs 0 +}]]` followed by `$total` is prepared; `${fold xs 0 +}` is not.

### Rendering

`doc.render(format)` gives the prose with each region replaced by `format(region, value)`. A line that held
only declarations, comments, and spaces is removed, so the declarations at the top of a document leave no
blank lines. `text` is the formatter for plain text. It writes strings as they are, numbers and booleans in
their usual form, and fails on anything else.

```rust
let prose = doc.render(text)?;                          // fails if a region needs the runtime
let prose = doc.render_with(&mut instance, text)?;      // evaluates the document first
```

The formatter is where the host decides what values mean. A document can describe a picture as
`$[% {#picture {. file "garden.png" caption "Spring"}}]`, and the host's formatter turns that variant into an
image tag, a typesetter command, or a pause in speech. `host_text.rs` does this.

### Documents inside other files

When the Lamlet text comes from a larger file, such as a block in Markdown, give its starting position:

```rust
engine.check(SourceFile::new("chapter.md", block_text).at(40, 1))?;
```

Every location and diagnostic then names the line in `chapter.md`, not in the block.

### What else a document tells the host

- `doc.binding("name")` returns a static top-level binding: its value, type, and doc comment.
  `doc.bindings()` lists them all.
- `doc.doc("name")` returns the doc comment of any name in scope.
- `doc.dependencies()` lists the packages the document loaded.
- `doc.fingerprint()` identifies the compiled program (source, packages, versions, natives), for caches.

## 3. Instances

```rust
let mut instance = engine.instance(&doc)?;
instance.set_input("clock", 1.25)?;             // read by natives; stays until changed
instance.set_time(1_700_000_000);               // what std's `time` returns
instance.grant("network");                      // a capability natives may require
instance.set_limits(Limits { max_instructions: 1_000_000, ..Limits::default() });

let all: Vec<(u32, Value)> = instance.evaluate()?;           // every expression region, by index
let y: f64 = instance.call_typed(&curve, (0.5,))?;           // Rust arguments as a tuple, Rust result
let v: Value = instance.call(&curve, vec![Value::Float(0.5)])?;
```

- **Inputs live on the instance** and stay set between calls. A program calling a curve every frame changes
  one input and calls again; nothing is rebuilt.
- **`call_typed`** converts the arguments, checks them against the function's type, and converts the result. A
  wrong type is an error, never a wrong value.
- **Budgets** (`Limits`) bound every evaluation and call: instructions, call depth, stack, allocation, string
  size, and native calls. A failure returns nothing partial, and the instance stays usable.
- **Cancellation.** `instance.cancel_flag()` returns an `Arc<AtomicBool>`. Setting it from any thread stops the
  running call.

Calls are cheap. `host_curve.rs` measures about half a microsecond for a small curve in a release build.

## 4. Host functions

A host gives documents functions of its own, called *natives*. The Lamlet type comes from the Rust types:

```rust
let natives = engine.natives_mut();
natives
    .function("garden.count/leaves", |_ctx, (tree,): (String,)| Ok(tree.len() as i64))
    .pure()
    .doc("Fn([String], Int). Leaves on one of the garden's trees.")
    .register()?;
natives.function("garden.calendar/today", |ctx, ()| ctx.input_as::<i64>("day")).register()?;
```

The closure gets the native context and its arguments as one tuple (`()` for none). It returns
`Result<T, String>`, where an `Err` becomes a diagnostic naming the function and the calling line. When the
function may run is set on the builder:

| Builder | Callable | Use it for |
|---|---|---|
| `.pure()` | During the check (`[% ...]`) and at runtime | Deterministic functions: lookups, measurements |
| `.runtime()` (the default) | At runtime only | Anything that reads inputs or the outside world |
| `.static_()` | During the check only | Values fixed when the document is prepared |

`.capability("name")` additionally requires the instance to grant `name`.

### Declarations

A document has to declare the natives it uses. The registry writes those declarations:

```rust
let package = engine.natives().declarations("garden")?;   // Lamlet source, one declaration per native
let mut packages = MemoryResolver::new();
packages.add("garden", &package, Exports::All);
engine.set_resolver(packages);
// Documents write `$[Import* garden]` and call `{leaves "oak"}`.
```

Names come from the last part of the id: `garden.count/leaves` is `leaves`, and `make-widget` would be
`makeWidget`. Two natives with the same name are an error.

### Host objects

A Rust value can live inside Lamlet values as an opaque handle. Lamlet can store it and pass it back, but not
look inside it.

```rust
struct Tree { name: String }
impl NativeType for Tree { const TYPE_ID: &'static str = "garden.tree"; }

natives.opaque::<Tree>()?;
natives.function("garden.tree/plant", |_, (name,): (String,)| Ok(Opaque::new(Tree { name }))).register()?;
natives.function("garden.tree/name", |_, (t,): (Opaque<Tree>,)| Ok(t.name.clone())).register()?;
```

`Opaque<T>` derefs to `T`. The value is dropped when the last handle, in Rust or in Lamlet, is gone.

## 5. Checking at build time

`engine.check_interface(file)` checks a document against the natives' declarations only, and never calls them.
A build script or CI job can use it without the host's runtime (no device, no window, no network).
`host_build_check.rs` checks the blocks of a Markdown file this way.

One limit: a `[% ...]` that calls a native needs the native to run, so `check_interface` reports it as an
error.

## 6. Values

Lamlet values cross the boundary as `Value`, or as `Data`, a plain copy:

| | `Value` | `Data` |
|---|---|---|
| Holds functions and host objects | Yes | No (converting one is an error naming where it is) |
| Can be sent to another thread | No | Yes (`Send + Sync`) |
| Pattern matching in Rust | `Value::Record`, `Value::Variant`, `v.field("x")` | `Data::Record(BTreeMap)`, `Data::Variant(tag, payload)` |

`Data::from_value(&v)` and `data.to_value()` convert. Rust values convert directly where `call_typed` and
natives need them:

| Rust | Lamlet |
|---|---|
| `()`, `bool`, `i64`, `f64`, `String` | `Unit`, `Bool`, `Int`, `Float` (finite), `String` |
| `Vec<T>` | `List(T)` |
| `Option<T>` | `<none: Unit, some: T>` (std's option) |
| `(A, B)` | `{first: A, second: B}` (std's pair) |
| `Opaque<T>` | `Native("T::TYPE_ID")` |

Records are built with `record().field("name", "oak").field("height", 12.5).build()?`.

## 7. Errors

Every function returns `lamlet::embed::Error`. It displays as rendered diagnostics, implements
`std::error::Error`, and tells what went wrong:

| `err.kind()` | Meaning |
|---|---|
| `Check` | The document did not check: syntax, names, types, stages, packages, or a failure while preparing |
| `Link` | An instance could not be made: a native the document declares is not registered, or differs |
| `Runtime` | Evaluation or a call failed: a runtime error, an exhausted budget, or cancellation |
| `Value` | A value crossing the boundary had the wrong type (an argument, a result, a region for `render`) |
| `Registry` | A native or host type could not be registered |
| `Internal` | A bug: Lamlet or a native panicked. The panic was caught. |

`err.diagnostics()` gives the details, and `err.locations()` gives each diagnostic's file, line, and column.
After an `Internal` error, the instance it happened in is unusable; create a new one. (Rust's default panic hook
still prints the panic message to stderr.)

## 8. Threads

`Engine`, `Document`, and `Instance` stay on the thread that created them. To prepare documents in parallel,
give each worker thread its own engine, built by one shared function. Send results back as plain values:
rendered `String`s, `Data`, or your own types. `host_threads.rs` shows this.

Sharing one checked document between threads is planned (phase 2 of `plans/embedding.md`).

## 9. Untrusted documents

A document cannot crash the host or run forever:

- **Evaluation** is bounded by `Limits`, and preparation by `Options::static_limits`.
- **Checking** is bounded by limits on source nesting, expression nesting, macro expansion, and type size and
  depth (spec §7.1). Exceeding one is a `limit` diagnostic.
- **Panics** are caught (`Internal`). A fuzzer in `cargo test` looks for inputs that panic.

The slowest input found so far takes about 1.2 seconds (release build) to reach the macro expansion budget.
Hosts that accept documents from untrusted sources should also cap their size, and can run checks on a worker
thread.

## 10. Not yet available

- Sharing documents between threads, and storing compiled documents so they need no rechecking (phases 2 and
  5).
- More than one diagnostic per check, and stable error codes (phase 4).
- JSON conversion in the library. The CLI's JSON encoding is not yet exposed.
