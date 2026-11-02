# Implementation notes

This describes the code as it is. For what the language means, see [spec.md](spec.md); for why it is designed
this way, see [rationale.md](rationale.md).

The library has no dependencies. The CLI and the presentation adapter use `serde_json`.

## Layout

| Module | Role |
|---|---|
| `source.rs` | Source map, spans, diagnostics, and the `$` document scanner |
| `syntax.rs` | Lexer (including `.l`, `.l=`, `#l`, `::` paths, method tokens), reader (trees, docs, depth limit, `x.f` call sugar), parser to the surface AST, and macro pattern matching and instantiation |
| `types.rs` | `TypeDesc` (Type values) and inference types; constraints `Num`/`Eq`/`Ord`/`Scalar`, fields, alternatives, and case relations |
| `check.rs` | Resolution, packages, HM inference, effect and stage checking, macro expansion, elaboration to core, static evaluation |
| `core.rs`, `compile.rs` | Core IR; closure conversion to bytecode (`SelfClosure` for recursive lambdas) |
| `bytecode.rs`, `vm.rs` | Verified stack bytecode; iterative VM with explicit frames, tail calls, and budgets |
| `builtins.rs` | The `K` table (each entry has a name, scheme, arity, and implementation), label functions, deterministic math, hash |
| `host.rs`, `package.rs` | Native registry, host context, resolver, the embedded std |
| `std/std.lam` | The standard library, written in Lamlet |
| `crates/lamlet-present` | The `present` package (Lamlet) and the render-lab adapter (Rust) |

## Pipeline

A module is checked declaration by declaration in source order. A `[% e]` is fully checked (types, effects,
free variables) and then run in the session's static VM, so its value (for example a Type) is available to
later declarations. Checked code is compiled once. `Instance` runs it any number of times, with explicit inputs
and limits.

Elaboration targets:

| Source | Elaborates to |
|---|---|
| `{' ...}` | A call of the constant `LabelFn::List(n)` |
| `{. ...}` | A call of `LabelFn::Record(labels)` |
| `.l`, `.l=`, `#l` | Constants (`LabelFn::Get`/`Set`/`Tag`) |
| `{^ f {...} ...}` | `Core::Lambda { self_id }` |
| `K::caseBool` applied to two literal thunks | `Core::If` (by identity) |
| Templates | `K::stringAppend` chains over `K::scalarString`/`K::latexEscape` |

List, record, and template literals whose parts are constants are folded into constants. This is also what
makes them statically available (spec §5).

## Status

| Area | Status |
|---|---|
| Scanner, reader, parser, scopes, packages, imports, natives, opaque types, capabilities | Implemented and tested |
| Core calculus, eager evaluation, recursive lambdas, tail calls | Implemented and tested |
| HM inference with constraints; records and variants with row-style field, alternative, and case constraints | Implemented and tested |
| Static evaluation, syntactic stage rule, effect checking | Implemented and tested |
| Pattern macros with hygiene and budgets | Implemented and tested |
| Deterministic math, hash | Implemented and tested (bits pinned) |
| CLI (`check`/`prepare`/`eval`/`call`/`batch`) | Implemented and tested |
| render-lab adapter | Implemented; tested offline, and validated and rendered once by render-lab itself (manual run) |
| Procedural macros, portable artifacts, benchmark suite, multiple diagnostics, mutual recursion, abstract types | Not implemented |

## Tests

| Location | Contents |
|---|---|
| `crates/lamlet/src/*` unit tests (21) | Schemes, numeric primitives, deterministic math, hash, constraint solving, scanner, parser |
| `crates/lamlet/tests/semantics.rs` (56) | One rule of the spec per test, including evaluation order (observed through a logging native), primitive aliasing and wrapping, staging, budgets, natives, packages, macros, positions |
| `tests/fixtures/*.lam` (94) | Documents whose prose states `expect:` / `expect-error:` lines; run by `crates/lamlet/tests/fixtures.rs` |
| `crates/lamlet-cli/tests/cli.rs` (8) | Protocol, exit codes, typed JSON calls |
| `crates/lamlet-present/tests/present.rs` (7) | The presentation boundary end to end |

All of them pass under both `cargo test` (debug) and `cargo test --release`. Debug builds matter here: the static
limits (spec §7.1) are chosen so that unoptimized builds stay within a 2 MiB test-thread stack.

## Limits

The static limits are in spec §7.1. Runtime defaults (`vm::Limits::default`):

- 50M instructions
- 200k frames
- 4M stack slots
- a 1 GiB cumulative allocation budget
- 16 MiB per String
- 1M native calls

The memory limit is a cumulative allocation budget, not live-heap accounting.

## Adding a primitive

1. Add one `Intrinsic` to `builtins::INTRINSICS` with a scheme.
2. Justify it in spec §8: it must not be reasonably expressible in Lamlet.
3. Bind it in std if users should see it under a plain name.

There is no other place to register it. In particular, the checker has no per-primitive code.
