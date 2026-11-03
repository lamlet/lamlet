---
name: lamlet
description: >-
  Write, read, debug, and extend Lamlet, the typed functional language embedded in text documents (.lam files;
  this repository's crates/lamlet). Use this skill whenever you work with .lam files, `$[...]`/`${...}` document
  regions, the Lamlet standard library (crates/lamlet/std/std.lam), the K primitives, Lamlet macros, the lamlet CLI,
  or the interpreter in crates/lamlet. For documents built into pages (the `content` package, `lamlet build`),
  also use `lamlet-documents`; for Rust hosts and extensions, `lamlet-embedding`.
---

# Lamlet expert guide (language version 0.3)

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

Related skills: **`lamlet-documents`** (pages: headings, tables, `content`, Markdown and HTML output) and
**`lamlet-embedding`** (Rust hosts, natives, extensions).

## 1. The mental model, in seven rules

1. **Documents.** Only `$`-regions are code: `${expr}`, `$[form]`, `$name`, `$f"{…}"`. Everything else is prose,
   and `$$` is a literal dollar. Inside a region you write plain code, with no more dollars.
2. **Braces are expressions; brackets are compiler directives.** `{f a b}` applies `f`. `[...]` declares,
   imports, or stages. (Templates, `f"{…}"`, are the one literal the compiler parses.)
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
;; line comment         ;{ disabled form }      ;[ disabled form ]       $;{...} / $;[...] disabled region (in prose)

42  -8  1.5  2e3  "text\n\u{41}"  true  false   literals (no implicit Int↔Float)
x   pkg::x   K::add   +   std::+                names; `::` separates module paths, never `.`
.label   .label=   #tag                          getter / setter / variant constructor (function values)

{f a b}                 application              {f}            call a zero-argument function
{^ {x y} decl* body}    lambda (λ is the same head: {λ {x} …})   {^ name {x} …} recursive lambda (name = itself)
{: e T}                 type ascription (T runs at compile time, as [% T]: it may use only static names)
{' a b c}   {'}         list literal / empty list
{. a 1 b 2}  {.}        record literal (labels are word identifiers, not evaluated)
{!m operands…}          macro use (always marked with !)
x.f   {x.f a b}   x.f.g call sugar: {f x}, {f x a b}, {g {f x}}  (no space before the dot!)

[# name expr]           binding (in a document or a lambda body, before the result expression)
[# {f x y} decl* body]  function definition: [# f {^ f {x y} decl* body}], so f may recurse
[# a 1 {f x} body b 2]  several bindings in one declaration, in source order (let*)
[% expr]                evaluate at compile time; the result is embedded as a constant
f"{Hi "$name", ${e}}"   template, a String: text exactly as written up to the first }" outside a hole (no
                        escapes); holes $name, ${form}, $[form] are typed (Scalar); $$ is a literal $
{latex f"{\frac{$a}{2}}"} a formula: std's Latex = {latex: String} (no LaTeX template form)
[Import p]  [Import* p]  [ImportAs p alias]            imports (lexical, source order)
[Macro !name {pattern template} ...]                   pattern macro
[Native name {Fn {' A B} R} "api.id/x" Pure|Static|Runtime]   typed host function (signature runs as [% …])
[NativeType Name "type.id"]                            opaque host type
[Type {T a} rollT unrollT body]  [Type T rollT unrollT body]   iso-recursive type (body runs as [% …]; §3)
```

Spacing is significant around the dot:

- `{r .x}` applies `r` to the getter `.x`.
- `{.x r}` reads field `x` of `r`.
- `{r.x}` calls a function named `x` with `r`.

Every token must be separated by whitespace, a delimiter, or a comment; the dot of the call sugar is the
only exception. A lone `;` inside code is a syntax error. Balanced comments contain raw text with balanced
braces/brackets and closed Strings; `;;` has no special meaning inside them. Strings and prose retain literal
semicolons. `~` is an ordinary symbolic identifier.

### What `#` means

`#` has three unrelated roles, told apart by what touches it:

| Written | Meaning |
|---|---|
| `[# x 1]`, `[# {f x} …]` (a bracket head, then a space) | a declaration: binds names for the rest of the scope |
| `#tag` (touching a word, in code) | a variant constructor: `{#circle 1.0}` builds a tagged value; `match`/`!case` takes it apart |
| `# Title` (at the start of a prose line, outside any region) | not Lamlet at all: a Markdown heading, which `lamlet-documents` builds into a heading |

### Bodies

A lambda body is `[# ...]` declarations followed by exactly one result expression. Each declaration binds the
rest of the body (`let*` semantics) and generalizes (let-polymorphism). Rebinding a name in the same scope is an
error; shadowing it in an inner scope is allowed. A document is a module: its declarations are in source order,
and its expression regions are its results.

### Document level

A `.lam` file is a document (spec §10). Only `$`-spellings are active:

| Spelling | Meaning |
|---|---|
| `$name`, `$pkg::name` | reads a binding, no call. A following `.` is prose: `I am $AGE.` reads `AGE` |
| `${…}` | an expression region |
| `$[…]` | a declaration, import, macro, native, `[% ]`, or template |
| `$f"{…}"` | a template as a region: `I am $f"{${.age p} years}" old` |
| `$;{…}`, `$;[…]` | a disabled region: inert text, dropped from the output |
| `$$` | a literal `$`; any other `$` not followed by a name, `{`, or `[` is prose (`$5`, `$ {x}`) |
| ```` ``` ```` / `~~~` fence | verbatim prose: no regions inside, `$$` stays `$$`; an unclosed fence is an error |

- Once `${` or `$[` starts, the scanner commits: an unterminated region is an error, never prose.
- Inside code, `$` is an error except in Strings and template holes.
- **Doc comments:** the prose paragraph directly above a declaration (no blank line between, the declaration at
  the start of its line), a disabled region directly before it, or `;;` lines directly before it inside code.
  Docs never change checking; hosts read them (`Document::doc`).
- `lamlet FILE` prints the document: prose as written, each region replaced by its value, and lines holding
  only declarations removed.

## 3. Types

```text
Unit Bool Int Float String Type   List(a)   Fn([a, b], r)   {x: Int, y: a}   <none: Unit, some: a>   Native("id")
Tree(a)                           a declared recursive type
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

### Recursive types

`[Type {Tree a} rollTree unrollTree body]` declares an *iso-recursive* type. The body is a static Type
expression in which `Tree` (with no arguments) is the recursive occurrence and `a` the parameter:

```text
[Type {Tree a} rollTree unrollTree
  {Variant {' {Field "leaf" Unit} {Field "node" {Struct {' {Field "l" Tree} {Field "v" a} {Field "r" Tree}}}}}}]
```

- It binds `Tree : Fn([Type], Type)` (with no parameters, `Tree` is a Type), and `rollTree` and `unrollTree`, which
  convert between `Tree(a)` and its structure `<leaf: Unit, node: {l: Tree(a), r: Tree(a), v: a}>`.
- **A `Tree(a)` is not a variant.** Build it with `{rollTree {#node …}}`, and take it apart with
  `{!case {unrollTree t} …}`. Matching a value or reading its fields directly is a type error that says so.
- Both functions are the identity at run time, and a value displays as its structure. They are calls, so
  `[# leaf {rollTree {#leaf unit}}]` is a runtime binding; write `[# leaf [% {rollTree {#leaf unit}}]]` for a
  static one.
- **Structural identity.** Two declarations with the same body are the same type.
- **Regular only.** Inside the body, `Tree` is a Type, not a constructor: `{Tree {List a}}` there is an error.
  Recursive types nest freely through arguments (`{List Tree}`, `{Option Tree}`, another declared type).
- **Functions are allowed** (lazy streams: `{Fn {'} Stream}`), and their capabilities survive rolling.
- `Eq` holds when the body supports it. There are no mutually recursive groups, and no recursive types in native
  signatures.

### Types as values

`Type` values are ground data: no variables, and never cyclic (a recursive type is named, as `Tree(Int)`). Build
them with these functions:

| Function | Builds |
|---|---|
| `Int`, `Float`, `Bool`, `String`, `Unit`, `Type`, `Never` | the base types |
| `{List T}` | a list type |
| `{Fn {' A B} R}` | a function type |
| `{Struct {' {Field "x" Int} ...}}` | a record type |
| `{Variant {' {Field "a" T} ...}}` | a variant type |
| `{Option T}`, `{Pair A B}` | the std option and pair types |
| `{Tree T}` | a declared recursive type, applied |

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

`references/examples.lam` is a verified tour of each feature, and `references/patterns.lam` solves the problems
that need several together: sorting, lookup tables, trees without recursive types, replacing mutual recursion,
polymorphism through bindings, parsing with options, state machines, `!pipe`/`!let`-style macros, types computed
from data, and curve combinators. `cargo test` checks every `expect:` line in both. Patterns to reach for:

- **Loops:** use `fold`, `map`, `filter`, and `range`, or a tail-recursive named lambda with an accumulator.
- **Text:** use `f"{...}"` for typed interpolation: `f"{${.name p} said "${.words p}"}"`. It is expanded
  during the check into concatenation, so it is a plain String (a constant when its holes are). A hole holds a
  *form*, so `${age}` calls `age`: read a binding with `$age`, a field with `${.age p}`. Likewise `${f"{…}"}`
  calls the String; in prose write `$f"{…}"`. A literal `}"` in the text is a hole: `${str "}\""}`.
- **Formulas:** `{latex f"{\frac{$a}{2}}"}` (a `Latex`); put a name in one with `${latexEscape name}`. `format` takes `List(String)`, so convert values with `str`
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
- **Trees:** declare a recursive type with `[Type …]` (§3). **Graphs** with sharing or cycles: store nodes in a
  list and refer to them by index (patterns §3).
- **Failure a caller can handle:** return an option or a variant with a reason; `error` aborts everything.

### Names

Recommended names (`docs/conventions.md`; not checked by the language):

- Values, functions, fields, and variant tags in lower camel case (`isEmpty`, `startFrame`, `#lightOnly`);
  types in upper camel case (`Content`); macros `!camelCase`; private names with a leading `_`.
- Predicates start with `is` or `has`. Two forms of one function: the plain name takes the simple argument, and
  `…Of` the general one (`emph "word"`, `emphOf content`).
- Packages in lower case with `_` (`garden_tools`); files, ids, cue names, and extension programs with `-`
  (`point-light.lam`, `{#bounced-light}`, `lamlet-present`); native ids `pkg.part/dashed-name`.

### Reading printed types

- Constraints print on the variable: `Fn([a:Scalar, String], Content)` means "any Scalar `a`".
- Capabilities print after the type: `Fn([], Int) !Runtime`.
- Records and variants print fields sorted, so `{. b 1 a 2}` is `{a: Int, b: Int}`.
- A package's named types are printed in full, structurally (a `content` value is a long
  `List(<close: String, …>)`); structural types have no names.

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
| `[f "Hi $name"]`, `[latex "x^2"]` (before 0.3) | `f"{Hi $name}"`, `{latex "x^2"}` |
| `f"{I am ${age}}"`, `${f"{…}"}` | `f"{I am $age}"`; `$f"{…}"` in prose (a hole or `${}` holds a form, which calls) |
| Static code using `[# x {...}]` | `[# x [% {...}]]` |


## 7. What Lamlet does not have

Don't emulate these by guessing at syntax:

- **Not in the language:**
  - mutation, references, or exceptions you can catch
  - partial application, variadic functions, or default arguments
  - parentheses or infix operators
  - type classes, method dispatch by type, mutual recursion, mutually recursive or non-regular types, or
    equi-recursive types (a recursive type is never its structure: roll and unroll it)
  - polymorphic computed types (`{List a}`)
  - higher-rank types
  - string indexing primitives (use `chars`, `substr`, `nth`)
  - procedural macros
- **Not in the tooling:** portable compiled artifacts, and multiple diagnostics per check (checking stops at the
  first error).

## 8. Workflow: always verify

1. Write the document. Start with `$[Import* std]`.
2. Run it with `./target/release/lamlet FILE.lam`. It prints the document with each region's value filled in, or
   the first diagnostic with its location. For each region's value and type, use
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

- **A library function** goes in `crates/lamlet/std/std.lam`:
  - Write a doc paragraph directly above the definition, starting with its type, e.g.
    `Fn([List(A)], Int). ...`.
  - Define it before any use (source order).
  - Prefix private helpers with `_`.
  - Add a fixture, then regenerate `references/std.md` with
    `python3 .agents/skills/lamlet/scripts/gen_std_reference.py`.
- **A K primitive** is a last resort, only for what Lamlet cannot express with reasonable performance. Add one
  `Intrinsic` to `crates/lamlet/src/builtins.rs` with a scheme (for example `forall a:Eq. Fn([a], Int)`), and
  justify it in `docs/spec.md` §8. The checker must never get per-primitive code.
- **Host functionality** is a native (`[Native ...]` plus a registration in Rust), never a primitive. See the
  `lamlet-embedding` skill and its `references/host-api.md`.
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
| `crates/lamlet/std/std.lam` | The standard library source; also the best style reference |
| `references/std.md` | Every std export with its type (generated) |
| `references/examples.lam` | Verified idioms, one per feature |
| `references/patterns.lam` | Verified solutions that combine features |
| `references/errors.md` | Diagnostics, and their fixes |
| `docs/conventions.md` | Naming conventions |
| `docs/documents.md` | Documents and `lamlet build` (the `lamlet-documents` skill) |
| `docs/embedding.md` | The Rust API (the `lamlet-embedding` skill, with `references/host-api.md` there) |
| `crates/lamlet/src/` | Interpreter internals (see `docs/implementation.md`) |
