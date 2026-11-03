---
name: lamlet
description: >-
  Write, read, debug, and extend Lamlet, the typed functional language embedded in text documents (.lam files;
  this repository's crates/lamlet). Use this skill whenever you work with .lam files, `$[...]`/`${...}` document
  regions, the Lamlet standard library (std/std.lam), the K primitives, Lamlet macros, the lamlet CLI, or embedding
  Lamlet from Rust.
---

# Lamlet expert guide (language version 0.1)

Lamlet is a strict, typed lambda calculus with:

- Hindley–Milner inference
- structural records and variants
- explicit compile-time evaluation (`[% ...]`)
- hygienic pattern macros
- typed host natives

It is embedded in prose documents. This guide is self-contained for writing correct code. When the exact
specification matters, consult `docs/spec.md`; it is normative.

**Before you claim any Lamlet code works, run it** (see §8). Never invent syntax or library functions: if it is
not in this guide, `references/std.md`, or `docs/spec.md`, it does not exist.

## 1. The mental model, in seven rules

1. **Documents.** Only `$`-regions are code: `${expr}`, `$[form]`, `$name`. Everything else is ignored prose,
   and `$$` is a literal dollar. Inside a region you write plain code, with no more dollars.
2. **Braces are expressions; brackets are compiler directives.** `{f a b}` applies `f`. `[...]` declares,
   imports, stages, or parses a template.
3. **Strict and exact.** Arguments are evaluated once, left to right, before the call. Arity is exact: there is
   no currying, no partial application, and no variadics.
4. **One meaning per name.** Names resolve lexically. No dispatch depends on types, so wrapping anything in a
   lambda never changes its behavior. `x.f` is pure syntax for `{f x}`.
5. **Data is immutable and structural.**
   - Records: `{. x 1}`, read with `.x`, updated with `.x=`.
   - Variants: `#tag v`, taken apart with `match` and a record of handlers.
   - Lists: `{' ...}`.
   - Equal structure means an equal type.
6. **Explicit staging.** `[% e]` runs at compile time. A binding is static only by a syntactic rule (§4.4). An
   optimizer never makes something static.
7. **No ambient effects.** Code reads the outside world only through declared natives and host inputs, so
   evaluation is deterministic and the math is bit-identical across platforms.

## 2. Syntax, complete

```text
; line comment          ~{ disabled form }      ~[ disabled form ]       $~{...} disabled region (in prose)

42  -8  1.5  2e3  "text\n\u{41}"  true  false   literals (no implicit Int↔Float)
x   pkg::x   K::add   +   std::+                names; `::` separates module paths, never `.`
.label   .label=   #tag                          getter / setter / variant constructor (function values)

{f a b}                 application              {f}            call a zero-argument function
{^ {x y} decl* body}    lambda                   {^ name {x} …} recursive lambda (name = itself)
{: e T}                 type ascription (T runs at compile time, as [% T]: it may use only static names)
{' a b c}   {'}         list literal / empty list
{. a 1 b 2}  {.}        record literal (labels are word identifiers, not evaluated)
{!m operands…}          macro use (always marked with !)
x.f   {x.f a b}   x.f.g call sugar: {f x}, {f x a b}, {g {f x}}  (no space before the dot!)

[# name expr]           binding (in a document or a lambda body, before the result expression)
[# {f x y} decl* body]  function definition: [# f {^ f {x y} decl* body}], so f may recurse
[# a 1 {f x} body b 2]  several bindings in one declaration, in source order (let*)
[% expr]                evaluate at compile time; the result is embedded as a constant
[f "Hi $name, ${expr}"] string template: each hole is typed (Scalar); $$ is a literal $
[latex "\frac{$a}{2}"]  LaTeX template: raw backslashes; result type Latex = {latex: String}
[Import p]  [Import* p]  [ImportAs p alias]            imports (lexical, source order)
[Macro !name {pattern template} ...]                   pattern macro
[Native name {Fn {' A B} R} "api.id/x" Pure|Static|Runtime]   typed host function (signature runs as [% …])
[NativeType Name "type.id"]                            opaque host type
```

Spacing is significant around the dot:

- `{r .x}` applies `r` to the getter `.x`.
- `{.x r}` reads field `x` of `r`.
- `{r.x}` calls a function named `x` with `r`.

Every token must be separated by whitespace or a delimiter; the dot of the call sugar is the only exception.

### Bodies

A lambda body is `[# ...]` declarations followed by exactly one result expression. Each declaration binds the
rest of the body (`let*` semantics) and generalizes (let-polymorphism). Rebinding a name in the same scope is an
error; shadowing it in an inner scope is allowed. A document is a module: its declarations are in source order,
and its expression regions are its results.

## 3. Types

```text
Unit Bool Int Float String Type   List(a)   Fn([a, b], r)   {x: Int, y: a}   <none: Unit, some: a>   Native("id")
```

- **Int** is signed 64-bit, and overflow is a runtime error. **Float** is finite binary64. Numeric operators are
  `Num`-polymorphic (Int or Float, but never mixed). Use `float`, `round`, `floor`, `ceil`, and `truncate` to
  convert.
- **Constraints:**
  - `Num` (Int, Float)
  - `Ord` (Int, Float, String)
  - `Eq` (everything except functions and native values)
  - `Scalar` (String, Int, Float, Bool, Unit)
  - `has .l: τ`, from field access
  - `has #l: τ`, from a variant constructor
  - a case relation, from `match`

  They generalize with the function that uses them, so `{^ {r} {.age r}}` accepts *any* record with `age`.
- **Records are closed** at their literal: `{. a 1}` has type exactly `{a: Int}`.
- **Variants.** A variant's type is fixed by the handlers of the `match` that consumes it, or by an ascription.
  An unconstrained top-level `{#tag v}` defaults to the closed variant of what it saw, just as `Num` defaults to
  Int.
- **Lists are homogeneous.** Records of different shapes cannot share a list.
- **Never.** `error : Fn([String], a)` fits any type. Never is `<>`, the empty variant.
- **Effects.** Function types carry capabilities (`!Runtime` or `!Static`) that come from natives. Calling adds
  them to the caller, ascription cannot remove them, and they show in printed types (`Fn([], Float) !Runtime`).
- **No higher-rank types.** A lambda *parameter* used at two types must be let-bound instead.

### Types as values

`Type` values are ground data: no variables, no recursive types. Build them with these functions:

| Function | Builds |
|---|---|
| `Int`, `Float`, `Bool`, `String`, `Unit`, `Type`, `Never` | the base types |
| `{List T}` | a list type |
| `{Fn {' A B} R}` | a function type |
| `{Struct {' {Field "x" Int} ...}}` | a record type |
| `{Variant {' {Field "a" T} ...}}` | a variant type |
| `{Option T}`, `{Pair A B}` | the std option and pair types |

Use computed types for checking. An ascription's type and a native's signature are evaluated at compile time
(`{: xs {List Int}}`, `[Native f {Fn {' Int} Int} …]`); a *named* type must be a static binding:
`[# P [% {Struct {' {Field "x" Int}}}]] {: v P}`.

## 4. Semantics an expert must know

### 4.1 Evaluation

- **Order.** The callee is evaluated first, then the arguments left to right. Record fields and list elements
  are evaluated in source order.
- **Errors abort the whole evaluation.** There is no catch.
- **Closures** capture by value, and lexical scope is never affected by the caller.
- **Laziness exists only through lambdas** (thunks):
  - `{!if c a b}`, `{!and ...}`, and `{!or ...}` are std macros that expand to `K::caseBool` with thunks, so only
    the chosen branch runs. Both branches are still type-checked.
  - `choose` is eager.
  - The first-class form is `{caseBool c {^ {} a} {^ {} b}}`.

### 4.2 Recursion

- **Only named lambdas recurse:** `[# {fact n} ...]` (short for `[# fact {^ fact {n} ...}]`). A plain binding
  `[# f {^ {n} ...}]` cannot see itself, which makes it the way to wrap a shadowed outer `f`:
  `[# g {^ {x} {+ {g x} 1}}]` calls the outer `g`, while `[# {g x} {+ {g x} 1}]` calls itself forever.
- **Recursion is monomorphic** within its own body, and **there is no mutual recursion**. To work around that,
  dispatch on a variant tag inside one function.
- **Tail calls** run in constant VM frames. Non-tail recursion is limited to 200k frames, so write long loops
  with an accumulator, as std's `map`, `filter`, `range`, `take`, and `foldr` do.

### 4.3 Eliminators

These are the only primitive ways to inspect data:

- `K::caseBool b thenThunk elseThunk`
- `K::caseList xs ifEmpty {^ {head tail} ...}`
- `K::match v {. tag1 handler1 tag2 handler2}`: exactly one handler per alternative, and every handler is a
  1-argument function. Unit payloads take `{^ {u} ...}`. The std macro `!case` is the shorter form:
  `{!case v {tag1 x body1} {tag2 y body2}}`.

### 4.4 Staging

A binding `[# x e]` is **static** when `e` is one of:

- a literal
- a static name
- a lambda whose free variables are static
- a label function
- a list or record literal, or a template, whose parts are all static
- a `[% ...]`
- a native or import

Anything else is a **runtime** binding, even if it could be folded, so `[# b {+ 1 2}]` is runtime.

`[% e]` may use only static bindings, and must not require `Runtime`. A static closure may *contain* a Runtime
native and still be called at runtime. Runtime code may not call `Static` natives.

`{!assert a b}` is `[% {assertEqual a b}]`: a compile-time equality check that returns `a`.

### 4.5 Macros

- **Rewrites, not values.** A macro is a syntax-to-syntax rewrite, and it is never a value: `{map xs !if}` is an
  error.
- **Clauses.** `[Macro !name {pattern template} ...]` tries its clauses in order.
- **Patterns.** An identifier is a pattern variable, `_` matches anything, literals match exactly, nested lists
  match by shape, and `p ...` repeats.
- **Templates.** `t ...` repeats one element. To repeat several together, use a splice group, which expands in
  place: `{... k {f v}} ...` (std's `!case` builds its handler record this way).
- **Hygiene.** Names a template introduces resolve where the macro was defined, and binders it introduces cannot
  capture user operands. Labels (`{. x a}`, `.x`, `#t`) are syntax, so hygiene leaves them alone.
- **Macro names** must be word identifiers (`!pipe`, not `!->`).
- **Limits.** Expansion depth is limited to 64, and expression nesting to 96. Each `!and`/`!or` operand costs a
  few levels.
- **Expansions are checked** exactly like hand-written code.

Useful std-free idiom, a pipeline:

```text
[Macro !pipe {{x} x} {{x {f args ...} rest ...} {!pipe {f x args ...} rest ...}}]
{!pipe xs {map g} {filter p} {fold 0 +}}
```

### 4.6 Packages

- **`std` is a package.** `[Import* std]` brings its names into scope. The export list is every top-level name
  not starting with `_`.
- **Conflicts.** Two wildcard imports that define the same name are an error. Use `ImportAs`, or qualify:
  `a::map`.
- **`K` is always in scope** (in an enclosing prelude, so a module may shadow it). Every `K::` primitive is an
  ordinary function with an ordinary type, so it can be aliased, wrapped, or passed. The full list is in
  `docs/spec.md` §8.

## 5. Writing idiomatic Lamlet

`references/examples.lam` is a verified tour: every line in it is checked by `cargo test`. Patterns to reach
for:

- **Loops:** use `fold`, `map`, `filter`, and `range`, or a tail-recursive named lambda with an accumulator.
- **Text:** use `[f "..."]` for typed interpolation. `format` takes `List(String)`, so convert values with `str`
  first. `++` concatenates two Strings; `concat` and `join` work on lists. `chars` splits a String into scalars,
  and `substr`/`split` work on Strings.
- **Options:** `some`, `none`, `withDefault`, `caseOption`.
- **Pairs:** `pair`, `.first`, `.second`.
- **State machines:** an `update : Fn([State, Event], State)`, where events are variants. Replay events with
  `{fold events initial update}`.
- **Objects:**
  - Objects are records whose fields hold functions. Methods take `self` explicitly:
    `{{.greet obj} obj}`.
  - Build defaults with a prototype record plus `.field=`; instances share closures, and nothing is deep-copied.
- **Animation:** a curve is a function of time, `Fn([Float], a)`. Compose curves with ordinary functions
  (`lerp`, `clamp`, `sin`, `cos`, `pow`, `pi`).
- **Pipelines:** `xs.f.g` reads left to right, but only the last step of a head chain takes arguments. For
  multi-argument stages, nest calls or use `!pipe`.

## 6. Pitfalls

The top ten (more, with exact diagnostics, in `references/errors.md`):

| Wrong | Right |
|---|---|
| `{+ 1 2 3}`, `{- x}` | `{+ {+ 1 2} 3}`, `{neg x}` |
| `{+ 1}` (partial application) | `{^ {y} {+ 1 y}}` |
| `{+ 1 2.0}` | `{+ {float 1} 2.0}` |
| `[# f {^ {n} … {f …}}]` | `[# {f n} … {f …}]` |
| `{if c a b}` | `{!if c a b}` |
| `{p.name}` to read a field | `{.name p}` |
| `std.map`, `K.add` | `std::map`, `K::add` |
| `{list 1 2}`, `{concat "a" "b"}` | `{' 1 2}`, `{++ "a" "b"}` |
| `[f "${str "x"}"]` (quote in a hole) | `[# s "x"] [f "$s"]` |
| Static code using `[# x {...}]` | `[# x [% {...}]]` |


## 7. What Lamlet does not have

Don't emulate these by guessing at syntax:

- **Not in the language:**
  - mutation, references, or exceptions you can catch
  - partial application, variadic functions, or default arguments
  - parentheses or infix operators
  - type classes, method dispatch by type, mutual recursion, or recursive types (List is the only one)
  - polymorphic computed types (`{List a}`)
  - higher-rank types
  - string indexing primitives (use `chars`, `substr`, `nth`)
  - procedural macros
- **Not in the tooling:** portable compiled artifacts, and multiple diagnostics per check (checking stops at the
  first error).

## 8. Workflow: always verify

1. Write the document. Start with `$[Import* std]`.
2. Run it with `cargo run -q --release -p lamlet --example run -- FILE.lam`. It prints `[region] value`, or the
   first diagnostic with its location. For structured output, use
   `./target/release/lamlet eval FILE.lam` (JSON), or `lamlet check` for types only.
3. On an error, look up its kind and message in `references/errors.md`, and read the diagnostic's notes.
4. To check the type of something, put it alone in a `${...}` region and run `lamlet eval`: the JSON shows each
   region's `type`. Alternatively, ascribe the type you expect with `{: e T}`.
5. For a library-level change, also run `cargo test` (unit, semantic, fixture, CLI, and adapter suites, in
   debug and release).

### Tests in this repository

A fixture is a document in `tests/fixtures/*.lam`. Its prose lines `expect: <display>` (one per expression
region, in order) or `expect-error: <kind>: <text>` state the expected outcome, and `input: KEY=INT` supplies a
host input. Adding a file adds a test. Semantic rules have Rust tests in `crates/lamlet/tests/semantics.rs`.

Display formats used in `expect:` lines:

| Value | Displays as |
|---|---|
| Strings | `"s"` (quoted) |
| Records | `{a: 1, b: "x"}`, fields sorted |
| Variants | `#tag(payload)`, or `#tag` when the payload is unit |
| Lists | `[1, 2]` |
| Unit | `()` |
| Types | `List(Int)`, `{a: Int}`, `<a: Int>`, `Never` |

## 9. Extending Lamlet

- **A library function** goes in `std/std.lam`:
  - Write a doc paragraph directly above the definition, starting with its type, e.g.
    `Fn([List(A)], Int). ...`.
  - Define it before any use (source order).
  - Prefix private helpers with `_`.
  - Add a fixture, then regenerate `references/std.md` with
    `python3 .agents/skills/lamlet/scripts/gen_std_reference.py`.
- **A K primitive** is a last resort, only for what Lamlet cannot express with reasonable performance. Add one
  `Intrinsic` to `crates/lamlet/src/builtins.rs` with a scheme (for example `forall a:Eq. Fn([a], Int)`), and
  justify it in `docs/spec.md` §8. The checker must never get per-primitive code.
- **Host functionality** is a native (`[Native ...]` plus `NativeRegistry::register_fn`), never a primitive. See
  `references/host-api.md`.
- **Language changes** must keep the invariants of §1: no name- or type-directed special cases, explicit
  staging, determinism. Record the decision in `docs/rationale.md`, and update `docs/spec.md` and
  `docs/quick-start.md`.

## 10. Map

| Path | What it holds |
|---|---|
| `docs/spec.md` | Normative semantics, the full K table, limits |
| `docs/quick-start.md` | Tutorial |
| `docs/host.md` | The host boundary |
| `docs/rationale.md` | Why things are the way they are |
| `std/std.lam` | The standard library source; also the best style reference |
| `references/std.md` | Every std export with its type (generated) |
| `references/examples.lam` | Verified idioms |
| `references/errors.md` | Diagnostics, and their fixes |
| `references/host-api.md` | The Rust API and CLI |
| `crates/lamlet/src/` | Interpreter internals (see `docs/implementation.md`) |
