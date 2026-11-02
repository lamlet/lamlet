# Changelog

All notable changes to this project are listed here. Versions follow [Semantic Versioning](https://semver.org);
while the version is `0.x`, a minor release may break the API. The language version (`LANGUAGE_VERSION`) and
bytecode version (`BYTECODE_VERSION`) are tracked separately from the crate version.

## Unreleased

### Removed

- `lamlet-present`, the render-lab adapter, moved to its own project. The Lamlet repository keeps general
  host contracts and synthetic hosts only; `docs/host.md` now holds just the general contract.

### Added

- Continuous integration: tests, clippy, rustdoc, and a check that the `lamlet` library has no dependencies.

### Fixed

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
