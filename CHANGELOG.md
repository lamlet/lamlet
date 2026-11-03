# Changelog

All notable changes to this project are listed here. Versions follow [Semantic Versioning](https://semver.org);
while the version is `0.x`, a minor release may break the API. The language version (`LANGUAGE_VERSION`) and
bytecode version (`BYTECODE_VERSION`) are tracked separately from the crate version.

## Unreleased

### Removed

- `lamlet-present`, the render-lab adapter, moved to its own project. The Lamlet repository keeps general
  host contracts and synthetic hosts only; `docs/host.md` now holds just the general contract.

### Added

- `λ` is another spelling of the lambda head: `{λ {x} {* x x}}` is `{^ {x} {* x x}}`. It is reserved, so `λ` can
  no longer be a binding name (language change).
- `lamlet FILE` (also `lamlet run FILE`) runs a document from top to bottom and prints it: its text as written,
  each expression region replaced by its value, and declarations removed with their lines.
- `lamlet-doc`, a new crate with no dependencies but `lamlet`: documents lowered to a format-neutral IR (`Doc`,
  `Block`, `Inline`), and backends from it to plain text, Markdown, HTML, JSON (the IR itself), and speech
  (utterances with anchors between words, and a translation alignment check). `lamlet FILE` goes through it,
  with the same output as before on every fixture and example (`plans/formats.md`).
- The `content` package (always available in the CLI, `lamlet_doc::CONTENT_SOURCE` for hosts): emphasis, links,
  images, code, formulas, headings, lists, tables, code blocks, named spans and groups, raw markup, and speech
  cues (`say`, `anchor`, `pause`), as typed values.
- Extensions: `lamlet NAME ARGS...` runs `lamlet-NAME ARGS...` from the PATH when NAME is not a command or a
  file, with the `lamlet` program's path in `LAMLET`. Narrated presentations are the extension `lamlet-present`.
- `lamlet build FILE [-o OUT] [--to text|md|html|ir|speech]`: the format from `--to` or the extension of `OUT`.
  `--prose commonmark` parses prose as CommonMark (a subset), so one source builds to every format;
  `--standalone` writes an HTML page, `--prepared` the IR without evaluating, and `--align` checks a translation
  against its original.
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
