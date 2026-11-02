# Embedding Lamlet in a host program (plan)

Lamlet already runs in process: `check_document`, `Document::prepared`, and `Instance` are a library API, and the
CLI is one host of it. This plan turns that API into a supported embedding surface, so that an application can
check, prepare, and evaluate Lamlet documents inside its own process (at build time, in tools, and at run time),
expose its own typed functions to them, and ship the result to users. It names no consumer: requirements are
stated as general workloads, and the tests use synthetic hosts.

Status: plan (2026-11-01). Nothing here is implemented unless marked "exists".

## Workloads it has to serve

| Workload | What the host does | What it needs from Lamlet |
|---|---|---|
| Text with computed regions, prepared before a slow step (speech synthesis, typesetting, translation) | Check a document, evaluate every region, splice the values into its prose | A prepared-span API that gives text plus values with positions; region values as plain data; diagnostics with positions in the host's file |
| Build-time validation | Check documents without the runtime (no device, no window), harvest what they reference | Metadata-only checking (`Options::require_linked = false`, exists); a crate with no dependencies, usable from a build script |
| Host functions | Expose typed functions (registry lookups, measurements, counts) to scripts, some at preparation time | Typed native registration from Rust types; `Pure` natives callable during static preparation (exists); capabilities (exist) |
| Interactive evaluation | Call compiled closures many times per second with explicit inputs (time, events) | Reusable `Instance` (exists), cheap calls, budgets and cancellation (exist), no panics |
| Several threads | Prepare on a worker thread, use results on another | Compiled programs and plain results that are `Send + Sync`; instances stay per thread |
| Shipping | Depend on a released crate in an offline, locked build | A versioned repository and release, a stable API, a semver policy |

## Where it stands today

What the plan builds on, as of this date (`crates/lamlet/src`):

| Piece | Today | Gap for embedding |
|---|---|---|
| Checking | `check_document(name, source, Rc<dyn Resolver>, Rc<NativeRegistry>, &Options) -> Result<Document, Failure>` (`lib.rs`) | Five arguments, all `Rc`; `Failure` holds exactly one diagnostic |
| Prepared spans | `Document::prepared() -> Vec<PreparedSpan>` with `Text`, `Declaration`, `Value`, `Residual`, `Comment` | Values are `Value` (`Rc`-based, may hold closures); positions are byte `Span`s into a `SourceMap` the host must resolve itself |
| Static bindings and docs | `Document::static_bindings`, `static_binding(name)`, `docs` | Exists; not yet promised stable |
| Instances | `Instance::new(Rc<Program>, Rc<NativeRegistry>)`, `initialize`, `evaluate`, `call`, `reset`, `cancel_flag`, `stats` | Every call passes `HostContext` and `Limits` by value (cloned per call); arguments and results are untyped `Value`s |
| Natives | `NativeRegistry::register_fn` with explicit `TypeDesc`s, `Availability`, capabilities; `interface_only()`; `fingerprint()` (`host.rs`) | Callbacks are `Rc<dyn Fn>`; types written by hand; stubs written by hand |
| Opaque host types | `register_type`, `host::opaque(type_id, data, release)` | `Box<dyn Any>` with no typed access back |
| Budgets | `Limits` (`vm.rs`), `Limits::static_default()` for preparation, cancellation through `AtomicBool` | Exists |
| JSON | `lamlet-cli` converts values to and from JSON (`value_json`, `data_json`, `json_to_typed` in `main.rs`) | Lives in the CLI, not the library |
| Caching identity | `Program::fingerprint` (source, dependencies, language and bytecode versions, native metadata) | Nothing is persisted |
| Panic sites | 81 `unwrap`/`expect`/`panic!` in the library: `syntax.rs` 34, `builtins.rs` 21, `check.rs` 9, `vm.rs` 5, `compile.rs` 4, others 8 | Not audited |
| Repository | Git on GitHub, MIT-licensed, tagged `v0.1.1`, changelog, CI | Not on crates.io (phase 0) |

Two facts shape phase 2. First, `Code::consts` (`bytecode.rs`) stores `Value`s, so a compiled `Program` holds
`Rc`s through its constants and not only through `Rc<Code>`. Second, static bindings and static region values
can be closures (`Value::Closure`), which capture `Rc` values and cannot be converted to plain data.

## Non-goals

- A C ABI, WebAssembly build, or bindings for other languages. The embedding surface is Rust; others can be
  built on it later.
- Running untrusted documents with OS-level isolation. Budgets, cancellation, and capabilities bound a document's
  work and reach; process isolation is the host's choice.
- Sharing one `Instance` across threads, or a concurrent VM.
- Hot reload of a running instance. A host rechecks and creates a new instance; artifacts (phase 5) make that
  cheap.
- Changes to the decisions in `docs/rationale.md`.

## Phase 0: a releasable crate

1. Put the repository under version control, with a changelog, the MIT `LICENSE` file, and a tag per release.
   (Done: git repository on GitHub, `LICENSE`, every crate declares `license = "MIT"`, tags `v0.1.0` and
   `v0.1.1`, `CHANGELOG.md`.)
2. Publish `lamlet` (the library) so consumers can depend on a version and vendor it: crates.io, or a tagged git
   URL until then. The library keeps zero dependencies; `serde` support is an optional feature.
3. Host adapters that serve one consumer do not belong in this repository (only synthetic hosts and examples do).
   A consumer-specific presentation adapter (`crates/lamlet-present`) moves to its consumer. (Done: it is now
   the separate project `lamlet-present`.)
4. A semver policy: the embedding surface below is the public API; `check`, `compile`, `vm`, `bytecode`, and the
   internal AST stay `#[doc(hidden)]` or private and may change in minor versions. `LANGUAGE_VERSION` and
   `BYTECODE_VERSION` are versioned separately from the crate: a language change that rejects previously
   accepted documents is a breaking crate release.
5. CI: `cargo test`, `cargo clippy -D warnings`, `cargo doc` without warnings, and a build of `lamlet` alone
   with `--locked --offline` after `cargo vendor`, to prove the zero-dependency claim. (Done:
   `.github/workflows/ci.yml` runs tests, clippy, and rustdoc with warnings as errors, and fails if `cargo tree`
   shows any dependency of `lamlet`.)

Item 4 waits for phase 1: hiding `check`, `vm`, and the other modules before `lamlet::embed` exists would hide the
only API there is. Until then the policy is the one in `CHANGELOG.md`: any `0.x` minor release may break the API.

**Done when** a scratch crate outside the repository depends on a tagged `lamlet` by version or git tag, builds
offline from a vendored copy, and runs `run_source`.

## Phase 1: the embedding surface

**Status: implemented (2026-11-01)** in `crates/lamlet/src/embed.rs`, tested by `crates/lamlet/tests/embed.rs`
(17 tests). It differs from the sketch below in these ways:

- **Names.** The pieces of a document are `Part` (not `Span`, which the crate already uses for byte ranges), and
  positions are `Location`s with line, column, and byte offsets. The registry is reached with
  `engine.natives_mut()`, and the resolver set with `engine.set_resolver(...)`; there are no `EngineOptions`.
- **Inputs live on the instance.** `inst.set_input(key, v)`, `grant`, and `set_limits` persist between calls,
  so `call_typed(&f, args)` takes no inputs, and a call per frame changes one input instead of rebuilding a
  context.
- **Bindings hold values.** `doc.binding(name)` returns the prepared value. Resolving bindings by name inside an
  instance waits for phase 2, where it is needed.
- **Not done in this phase:**
  - JSON stays in the CLI (open question 2).
  - `Data` has no opaque handles: `Data::from_value` refuses opaque values, which typed handles in phase 3 cover.
  - The CLI is not rewritten yet (phase 7).
- **A gap found while testing.** `check_interface` cannot prepare a `[% ...]` that calls a native, because
  preparation needs the implementation, so such documents fail the build-time check. Fixing it means letting
  interface checks leave those regions unprepared (a checker change), or requiring build tools to register
  `Pure` implementations. See open question 5.
- **A fact hosts need.** Only `$name` (a static binding) and `$[% e]` regions are prepared during the check. A
  `${...}` region is an application and stays residual even without runtime inputs. `docs/embedding.md`
  (phase 7) must say so.

A small facade over the existing modules, `lamlet::embed`:

```rust
let mut engine = Engine::new(EngineOptions::default());     // std included
engine.resolver(resolver);                                   // packages: Box<dyn Resolver>
engine.natives(|r| r.register_fn(/* id, params, ret, availability, callback */))?; // typed `function` is phase 3
let doc = engine.check(SourceFile::new("story.txt", text))?; // Result<Document, Failure>
for span in doc.spans() {                                    // prose, declarations, regions, comments, in order
    match span {
        Span::Text(t) => …,
        Span::Region { value, ty, at, .. } => …,             // value: Option<Data>; None for a residual region
        Span::Declaration { .. } | Span::Comment { .. } => {}
    }
}
let text = doc.render(|value| value.as_text())?;             // the common "splice values into prose" case
let mut inst = engine.instance(&doc)?;                       // runtime calls
let f = doc.binding("curve")?;                               // a static binding, by name
let y: f64 = inst.call_typed(&f, (0.5f64,), Inputs::new().with("time", 1.25))?;
```

- **Engine.** Owns the resolver, the native registry, `Options`, and default `Limits`, so the five arguments of
  `check_document` become one receiver. `engine.check_interface(...)` checks against `interface_only()` for
  build-time validation.
- **Values out.** `Data`, a plain, owned, `Send + Sync` mirror of `Value` without closures (unit, bool, int,
  float, string, list, record, variant, type description, and opaque handles by id). `Value::to_data` returns an
  error naming the path to the first closure or native function it meets; `Data::to_value`. JSON conversion
  moves from the CLI into the library behind the `json` feature (or as a dependency-free writer), so the CLI and
  hosts share one encoding.
- **Functions out.** A static binding that is a function is reached by name (`doc.binding("show")`), returning a
  `Binding` handle that an instance resolves. Hosts never need to hold a closure `Value` themselves, which keeps
  phase 2 possible.
- **Values in and typed calls.** `IntoLamlet`/`FromLamlet` traits for Rust scalars, `String`, `Vec<T>`, tuples
  (as `{first, second}` pairs), `Option<T>` (as `<none, some>`), and derive-free records via a builder; with the
  `serde` feature, any `Serialize`/`Deserialize` type through `Data`. `call_typed` checks the argument and result
  types against the binding's `ty_desc` before calling, so a mismatch is an error, never a wrong value.
- **Inputs and limits per call.** `Inputs` replaces building a `HostContext`; the instance keeps its context and
  limits between calls and only applies what changed, so a per-frame call does not clone two maps.
- **One render helper.** `Document::render` covers the most common host task, prose with values spliced in, with
  the policy for declarations left on their own line (they leave no blank line) and a per-type formatter. It
  fails if any region is residual; `render_with(&mut inst, ...)` evaluates residual regions first.
- **Positions in the host's file.** A host that extracts a document from a larger file passes a `SourceFile` with a
  line/column origin, so diagnostics point into the host's file. Every span the facade returns carries resolved
  `(line, column)` pairs as well as byte offsets.
- **Errors.** One error type, `lamlet::embed::Error`, for check failures, link failures, and runtime errors, each
  with diagnostics, a `Display` that renders them, and `std::error::Error`.

**Tests.** A synthetic text preparer (a document embedded at line 40 of a host file; diagnostics name line 40+),
a build-time checker using `check_interface`, and round trips `Value → Data → Value` and `Data → JSON → Data` over
every fixture in `tests/fixtures` that has no closures in its results.

**Done when** the three synthetic hosts compile against `lamlet::embed` only (no `check`, `vm`, or `bytecode`
imports) and the CLI's JSON output is unchanged for every fixture.

## Phase 2: threads

- `Program`, its code, and `TypeDesc` move from `Rc` to `Arc` and become `Send + Sync`, so a document checked on
  one thread can be instantiated on another. Measure the cost; the VM's per-call values stay `Rc`.
- Constants: `Code::consts` holds `Value`s today. They change to a `Const` type of `Send + Sync` scalars and
  strings (`Arc<str>`), turned into `Value`s when the VM loads a code object. Static values that the compiler
  folds into code must be expressible as `Const` or `Data`; a closure produced during preparation is recompiled
  from its code and captured `Data`, or the binding stays residual and is evaluated per instance. Which of the
  two is a design decision for this phase (see open questions).
- `Document` splits into a `Send + Sync` part (program, spans, region types, `Data` values, docs, dependencies)
  and per-thread state. `Binding` handles from phase 1 work across threads because they name a binding, not a
  value.
- `Instance` stays `!Send`: one per thread, cheap to create from a shared `Program`.
- Native callbacks: `Arc<dyn Fn + Send + Sync>` when registered on an engine shared by threads; `Rc` remains for
  single-threaded registries. Opaque values stay per thread; only their type metadata is shared.
- A test checks a document on one thread and evaluates it on four, comparing results with a single-threaded run.

**Done when** `static_assertions`-style compile tests (no dependency: a generic `fn assert_send_sync<T>()`)
cover `Engine` (with a thread-safe registry), `Program`, the shared `Document`, `Data`, and `Error`, and the
four-thread test passes. Checking and calling benchmarks (phase 5) regress by at most 5%.

## Phase 3: native functions from Rust types

**Status: implemented (2026-11-01)** in `crates/lamlet/src/embed.rs`, tested by `crates/lamlet/tests/natives.rs`
(8 tests). Notes:

- **Arguments as a tuple.** The closure takes the context and one tuple: `|ctx, (w, h): (f64, f64)| Ok(w * h)`,
  and `|ctx, ()| ...` for no arguments. A single method then infers the signature for any arity (up to six),
  and the closure's types come from its bound. The result is always `Result<R, String>`.
- **Availability defaults to Runtime.** `.pure()` must be asked for, because a Pure native runs during
  preparation. `.register()` adds the entry; the builder is `#[must_use]`.
- **Declarations.** Names come from the last segment of the id. The package uses `std` qualified
  (`std::Fn`, `std::Int`), so it defines only the natives and can be imported with `[Import* ...]` next to `std`.
  Name collisions are an error. There is no per-entry name override yet.
- **Opaque types.** `NativeType` is a trait with a `TYPE_ID`. `Opaque<T>` derefs to `T`, and releasing is `T`'s
  `Drop`, so the registry's release hook is not needed for typed handles.
- **Fixed along the way.** A native failing in tail position (`{^ {x} {leaves x}}`) reported no position,
  because the VM releases the frame before a tail call. Builtins had the same problem. Call-time errors of a tail
  call now carry its site.

- `NativeRegistry::function(api_id, f)` infers parameter and result `TypeDesc`s from the closure's types through
  `IntoLamlet`/`FromLamlet`, and returns a builder for availability (`.pure()`, `.static_()`, `.runtime()`),
  capabilities, version, and documentation. A closure may take `&mut NativeCtx` first to read inputs.
- The matching `[Native ...]` declaration can be generated: `registry.declarations("host")` writes a Lamlet
  package of `[Native x T "id" Pure]` stubs with docs, so authors import a checked interface instead of writing
  stubs by hand, and build-time checking uses the same stubs with `interface_only()`.
- Host opaque types (`NativeType`) get typed handles: `registry.opaque::<T>("host.image")`, with release, and
  `Opaque<T>` implements `IntoLamlet`/`FromLamlet` with a checked downcast.
- Errors from a native carry the API id and are diagnostics with the calling region's position.

**Tests.** A synthetic host registers a pure lookup, a runtime measurement reading an input, and an opaque
handle type; its generated package is checked into the test as a snapshot and must check cleanly; calling the
lookup during preparation and the measurement at run time both work; a native returning `Err` produces a
diagnostic pointing at the calling region.

## Phase 4: robustness

- **No panics on any input.** Audit the 81 `unwrap`/`expect`/`panic!` sites (counted above); the ones input can
  reach return diagnostics, and the rest get a comment stating the invariant. The facade wraps each entry point
  in `catch_unwind` as a last guard and reports an internal error.
- **Fuzzing.** `cargo fuzz` targets for the scanner, reader, parser, and checker (any byte string), and for the VM
  with random well-typed programs from the fixtures. The fuzz crate lives outside the workspace so the library
  stays dependency-free.
- **Several diagnostics.** The checker reports every independent error of a module, not only the first (today's
  limitation), with stable codes (`E0102 unknown name`) for hosts to match on. Codes refine today's `DiagKind`
  and are listed in `docs/spec.md`; a code is never reused.
- **Budgets everywhere.** Static evaluation and runtime calls are bounded (exists); the facade exposes per-call
  limits and the cancellation flag. Checking itself (macro expansion, type inference) is bounded too, so a
  hostile document cannot hang a build.

**Done when** the audit is complete, each fuzz target has run 24 hours without a crash, and every `err-*`
fixture carries its expected code.

## Phase 5: speed and caching

- **Compiled artifacts.** A versioned binary format for a checked `Program` plus its static values and spans,
  keyed by the existing fingerprint (source, dependencies, language and bytecode versions, native metadata), so a
  host loads a prepared document without rechecking. Verification on load (exists for bytecode). The format is
  written by hand (no serde in the core) and depends on phase 2's `Const`/`Data` split, since closures and
  opaque values cannot be stored. An artifact from another version or fingerprint is rejected, never migrated.
- **Benchmarks** (`benches/`): checking a 1,000-line document with 200 regions, preparing it, loading its
  artifact, and 10⁶ calls of a small closure. Targets to set from the first measurements; a regression gate in
  CI.

**Done when** loading an artifact is at least 10× faster than checking the same document, and an artifact
round trip produces the same `spans()` and the same call results as a fresh check.

## Phase 6: language features general hosts need

Kept to what the rationale allows. The rationale's decisions stand: no variadic functions (D2; a list literal is
the one variadic form) and no nominal wrappers with a global registry (D3). Hosts express "a value of a host
kind" with a record and a tag field, a variant, or a native opaque type.

| Feature | Why a general host needs it | Notes |
|---|---|---|
| Mutual recursion (`[#rec ...]` groups) | Recursive descriptions (nested sequences, trees) | Listed as a limitation today (rationale D4) |
| String keys into records (`{get r "field"}` when the field set is known statically) | Hosts pass attribute maps | Or a `Map(String, A)` type in std |
| Float text without exponents | Text output for people | `fixed : Fn([Float, Int], String)` exists for a fixed number of places; default Float display uses Rust's `{:?}` and switches to exponent form for large and small magnitudes. Add a shortest-decimal form without exponents |
| Abstract types in packages | A package hides a record's representation behind functions | Listed as a limitation today |
| Package submodules (`[ImportAs host.text t]`) | Larger host libraries | `Exports::Names` re-exports exist; formalize |
| Doc comments on bindings exported to hosts (`Document::docs`, exists) | Generated reference pages | Already present; stabilize |

Each feature goes through the usual path: a rationale entry if it touches a decision, a spec change, fixtures
(including `err-*` fixtures), and an update to the lamlet skill.

## Phase 7: documentation and examples

- `docs/embedding.md`: the facade, threads, natives, artifacts, error handling, with a synthetic host for each
  workload above (a text preparer, a build-time checker, an animation curve evaluator, a two-thread host). The
  hosts live in `crates/lamlet/examples/` and run in CI, so the documentation cannot drift.
- `docs/host.md` keeps the general host contract and points to `docs/embedding.md`. (Its render-lab sections
  moved with `lamlet-present`.)
- The CLI is rewritten on the facade, so it is the reference host.

## Order and size

| Phase | Depends on | Size |
|---|---|---|
| 0 Releasable crate | none | small |
| 1 Embedding surface | 0 | medium |
| 2 Threads | 1 | medium to large (an `Rc` to `Arc` audit, plus the constant and static-closure split) |
| 3 Typed natives | 1 | medium |
| 4 Robustness | 1 | medium, ongoing (fuzzing) |
| 5 Artifacts and benchmarks | 1, 2 | medium |
| 6 Language features | 1 | each small to medium, independent |
| 7 Docs | 1 to 3 | small |

A consumer can start on phases 0 and 1 alone: in process, single-threaded, with hand-written native stubs.
Benchmarks (the second half of phase 5) do not depend on phase 2 and should land early, so phase 2's cost can be
measured against a baseline.

## Risks

| Risk | Effect | Mitigation |
|---|---|---|
| `Arc` everywhere slows the VM | Interactive hosts lose throughput | Only shared, immutable data becomes `Arc`; per-call values stay `Rc`; benchmarks before and after |
| Static closures cannot cross threads or be stored | Phase 2 and 5 cannot cover documents whose static bindings are functions | Bindings by name, re-created per instance; documented as the cost of a function-valued static binding |
| Freezing the facade too early | Semver churn for consumers | Release phase 1 as `0.x`; promise stability only after one consumer and the CLI use it |
| Several diagnostics produce cascades | Hosts see noise after the first error | Report only errors independent of an earlier one (an unknown name is not also a type error) |

## Open questions

1. **Static closures across threads (phase 2).** Recompile closures from code plus captured `Data` (keeps
   preparation results, needs every capture to be data), or re-evaluate function-valued bindings per instance
   (simpler, repeats preparation work)? Leaning to the second, with the first as an optimization.
2. **JSON in the library.** A `json` feature with `serde_json`, or a small dependency-free writer and reader in
   the core? The zero-dependency claim argues for the latter.
3. **`Document::render` policy.** Is the declaration line rule right for every host, or should `render` take a
   policy value? As implemented, a line holding only declarations, comments, and whitespace is removed with its
   line break.
4. **Crate split.** Does the facade live in `lamlet` (`lamlet::embed`) or in a thin `lamlet-embed` crate that
   re-exports a stable subset? One crate is simpler; two make the semver boundary physical. Phase 1 chose
   `lamlet::embed`.
5. **Static natives in build-time checks.** Should `check_interface` leave a `[% ...]` that needs a native
   unprepared (a checker change), or should build tools register `Pure` implementations? The first keeps build
   scripts free of host code.
