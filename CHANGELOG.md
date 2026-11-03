# Changelog

All notable changes to this project are listed here. Versions follow [Semantic Versioning](https://semver.org);
while the version is `0.x`, a minor release may break the API. The language version (`LANGUAGE_VERSION`) and
bytecode version (`BYTECODE_VERSION`) are tracked separately from the crate version.

## Unreleased

Language version 0.3.0. Templates have one form, `f"{...}"`: documents using `[f "..."]` or `[latex "..."]` must
be rewritten, and prose with `$f"{` now opens a region.

### Added

- Templates are written `f"{Hello, I am "${.name p}", $age years old.}"`. The text runs, exactly as written, to
  the first `}"` outside a hole, with no escapes, so quotes, backslashes, and unbalanced brackets need none.
  Holes are balanced code and may contain Strings and other templates. In a document, `$f"{...}"` is a region
  (language change; spec §2.7, rationale D13).
- std's `latex`, `Fn([String], Latex)`, makes a formula from LaTeX source (`{latex f"{\frac{$a}{2}}"}`), and
  `latexEscape`, `Fn([String], String)`, turns text into LaTeX that typesets it literally.

### Changed

- A template is folded to a constant only when every hole is a constant. A template whose holes used no local
  variables used to be evaluated during the check, so a hole calling a Runtime native (`${time}`) was a stage
  error.

### Removed

- The bracket templates `[f "..."]` and `[latex "..."]` are retired, and are now unknown bracket forms (a syntax
  error). Write `f"{...}"` and `{latex f"{...}"}`. A String in a LaTeX hole is no longer escaped by itself; call
  `latexEscape`.
- The primitive `K::latexEscape`, which existed only for `[latex]` (std's `latexEscape` is written in Lamlet).

## 0.3.1 (2026-11-02)

### Fixed

- `lamlet` builds when vendored or packaged on its own. Its standard library was read from outside the crate's
  directory (`std/std.lam` at the repository root), which `cargo vendor` and `cargo package` do not copy, so a
  host that vendors Lamlet from git could not build it offline. The file is now `crates/lamlet/std/std.lam`.

## 0.3.0 (2026-11-02)

Language version 0.2.0. Documents written for 0.1.0 keep working, except one that uses `λ` as a name.

### Changed

- `λ` is reserved: it is the other spelling of the lambda head (below), so it can no longer be a binding name.
- An ascription's type and a native's signature are evaluated at compile time, as if written `[% T]`, so forms
  that used to be stage errors (`{: xs {List Int}}`) now check.

### Removed

- `lamlet prepare`: `lamlet build FILE --to ir --prepared` gives the same static values and runtime holes, as the
  document IR.
- The internal `Document::prepared` and `PreparedSpan`, replaced by `lamlet::embed`'s `Document::parts`, and
  `lamlet::run_source`, replaced by `Engine` and `Instance::evaluate`.
- The `run`, `docs`, and `timing` example programs, replaced by `lamlet FILE`, `lamlet check`, and
  `cargo bench -p lamlet --bench embed`.
- The speech backend (`lamlet build --to speech`, `--align`): it moved to the `lamlet-present` extension
  (`lamlet present speech`). lamlet-doc keeps the `Say`, `Anchor`, and `Pause` nodes.

### Added

- `λ` is another spelling of the lambda head: `{λ {x} {* x x}}` is `{^ {x} {* x x}}`. It is reserved, so `λ` can
  no longer be a binding name (language change).
- `lamlet FILE` (also `lamlet run FILE`) runs a document from top to bottom and prints it: its text as written,
  each expression region replaced by its value, and declarations removed with their lines.
- `lamlet-doc`, a new crate with no dependencies but `lamlet`: documents lowered to a format-neutral IR (`Doc`,
  `Block`, `Inline`), and backends from it to plain text, Markdown, HTML, and JSON (the IR itself). `lamlet FILE` goes through it,
  with the same output as before on every fixture and example.
- The `content` package (always available in the CLI, `lamlet_doc::CONTENT_SOURCE` for hosts): emphasis, links,
  images, code, formulas, headings, lists, tables, code blocks, named spans and groups, raw markup, and speech
  cues (`say`, `anchor`, `pause`), as typed values.
- Function definitions: `[# {f x y} body]` is `[# f {^ f {x y} body}]`, so a defined function can call itself
  without naming itself twice. The body may start with declarations, and ends at its first expression.
- Several bindings in one declaration: `[# a 1 {f x} {+ x a} b 2]` is `[# a 1] [# {f x} {+ x a}] [# b 2]`. A
  region's doc documents its first binding (language change).
- An ascription's type and a native's signature are evaluated at compile time: `{: xs {List Int}}` is
  `{: xs [% {List Int}]}`, and `[Native f {Fn {' Int} Int} …]` needs no `[% …]` (language change; both used to
  be stage errors).
- Splice groups in macro templates: `{... t1 t2}` instantiates its elements in place, so `{... k {f v}} ...`
  repeats two elements together, in one expansion.
- `!case`, a std macro for matching variants: `{!case s {circle r {* 3.0 {* r r}}} {rect wh {area wh}}}` is
  `{match s {. circle {^ {r} …} rect {^ {wh} …}}}`. It expands in one step, so the number of clauses does not
  count against the macro expansion depth.
- std, `content`, the examples, and the docs use function definitions, and generated native declarations no
  longer wrap their signatures in `[% ...]`.
- `docs/conventions.md`, the recommended naming conventions: camel case in code, `_` in package names, `-` in
  ids, files, and extension programs.
- Extensions: `lamlet NAME ARGS...` runs `lamlet-NAME ARGS...` from the PATH when NAME is not a command or a
  file, with the `lamlet` program's path in `LAMLET`. Narrated presentations are the extension `lamlet-present`.
- `lamlet build FILE [-o OUT] [--to text|md|html|ir]`: the format from `--to` or the extension of `OUT`.
  `--prose commonmark` parses prose as CommonMark (a subset), so one source builds to every format;
  `--standalone` writes an HTML page, and `--prepared` the IR without evaluating.

## 0.2.0 (2026-11-01)

Tagged `v0.2.0`; its crates still declared version 0.1.1.

### Removed

- `lamlet-present`, the render-lab adapter, moved to its own project. The Lamlet repository keeps general
  host contracts and synthetic hosts only; `docs/host.md` now holds just the general contract.

### Added

- `lamlet::embed`, the embedding surface for host programs: `Engine`, `SourceFile` (with a position in a host
  file), `Document::parts`/`render`/`render_with`/`binding`, `Instance` with persistent inputs and typed calls
  (`call_typed`), one `Error` type, `IntoLamlet`/`FromLamlet` conversions, and `Data`, a plain `Send + Sync`
  copy of values.
- Natives from Rust types: `NativeRegistry::function` takes the signature from a closure's argument tuple and
  result (`LamletType`), with a builder for availability, capabilities, version, and docs.
  `NativeRegistry::declarations` writes the matching `[NativeType]`/`[Native]` package. Typed opaque handles:
  `NativeType`, `Opaque<T>`, and `NativeRegistry::opaque`. `NativeCtx::input_as` reads a typed input.
- `NativeEntry::doc`.
- `lamlet::embed` catches panics and reports them as `ErrorKind::Internal`; an instance that panicked is
  marked unusable. `Data` conversions refuse values nested deeper than the type limit.
- `docs/embedding.md`, a guide to `lamlet::embed`, and four example hosts in `crates/lamlet/examples/` (`host_text`,
  `host_build_check`, `host_curve`, `host_threads`) that CI runs.
- Benchmarks (`cargo bench -p lamlet --bench embed`), with no dependencies. With `-- --gate`, which CI runs, they
  check that checking and evaluation scale linearly and that fixed workloads take no more VM instructions.
- `Engine` checks `std` once and starts each check from that state (`lamlet::Prelude`). Checking a small
  document that imports `std` is about 8.5× faster (1.14 ms to 0.13 ms), and a 1,000-line one about twice as
  fast. A document without `std` is about 10 µs slower (29 µs to 39 µs). Results are identical to checking
  from scratch, which a test compares over every fixture.
- An in-tree fuzzer (`crates/lamlet/tests/fuzz.rs`) and hostile-input tests (`crates/lamlet/tests/limits.rs`).
- `check_source` and `SourceFile::with_origin`: diagnostics can point into a larger host file.
- Continuous integration: tests, clippy, rustdoc, and a check that the `lamlet` library has no dependencies.

### Fixed

- The type checker accepted recursive types formed by unifying two type variables, for example
  `{^ {z} {!if c z {#node z}}}`. A host calling such a function could get an arbitrarily deep value. The occurs
  check now covers that case and constraints added to a variable.
- Documents could crash the host process with a stack overflow, through types that double in depth per binding
  or through computed `Type` values. Types are now limited to 100 levels and 100,000 parts (spec §7.1), and a
  violation is a `limit` diagnostic.
- A runtime error raised by a call in tail position (a failing native or builtin, an arity mismatch) now points
  at that call. It used to point at the caller's call site, or at nothing.
- Broken documentation link in `lamlet-present`; the `lamlet` CLI binary no longer collides with the `lamlet`
  library in `cargo doc`.

## 0.1.1 (2026-11-01)

### Changed

- Licensed under MIT only. The manifests previously declared `MIT OR Apache-2.0`, but only the MIT license was
  included. `lamlet-present` now inherits the workspace license, version, and edition.

## 0.1.0 (2026-11-01)

First tagged release.

- `lamlet`: scanner, region parser, type and capability checker, static preparation, closure conversion, and a
  bytecode VM, with no dependencies. Host API: `check_document`, `Document::prepared`, `Instance`, and
  `NativeRegistry`.
- `std/std.lam`: the standard library over the `K` primitives.
- `lamlet-cli`: the `lamlet` command, with JSON output.
- `lamlet-present`: an adapter from Lamlet presentation documents to render-lab timelines.
- Documentation: quick start, specification, rationale, host contract, implementation notes, and a programming
  languages course.
