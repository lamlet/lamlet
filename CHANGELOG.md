# Changelog

All notable changes to this project are listed here. Versions follow [Semantic Versioning](https://semver.org);
while the version is `0.x`, a minor release may break the API. The language version (`LANGUAGE_VERSION`) and
bytecode version (`BYTECODE_VERSION`) are tracked separately from the crate version.

## Unreleased

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
