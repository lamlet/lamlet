# Lamlet language specification

This is the normative description of Lamlet, language version 0.1.0, as implemented by the interpreter in
`crates/lamlet`. Anything marked **(future)** is not implemented. Supporting documents:

- [quick-start.md](quick-start.md): a tutorial
- [rationale.md](rationale.md): design decisions and the alternatives considered
- [host.md](host.md): the host boundary

## 1. Layers

```text
document text ──scan──▶ regions ──read──▶ syntax trees ──expand macros──▶ surface AST
   ──resolve + infer + stage-check──▶ core IR ──[% ...] run in static VM──▶ constants
   ──closure-convert──▶ bytecode ──VM──▶ immutable values ──▶ host
```

| Layer | Responsibility | Never does |
|---|---|---|
| Scanner (`source.rs`) | Finds `$name`, `${...}`, `$[...]`, `$$`, `$;{...}`, `$;[...]`; keeps prose spans | Parse code |
| Reader (`syntax.rs`) | Tokens → brace/bracket trees; comments → docs | Assign meaning |
| Macro expander (in `check.rs`) | `{!m ...}` trees → trees, hygienic, budgeted | Evaluate or type anything |
| Parser (`syntax.rs`) | Trees → surface AST; desugars list/record literals | Resolve names |
| Checker (`check.rs`, `types.rs`) | Resolution, HM inference with constraints, effect and stage checking, elaboration to core | Look at primitive names |
| Static evaluator | Runs checked `[% e]` in an isolated VM with a static budget | Run unchecked code |
| Compiler/VM (`compile.rs`, `vm.rs`) | Core → bytecode; eager evaluation with budgets | Re-check types |
| Host API (`lib.rs`, `host.rs`) | Linking natives, inputs, calling functions, decoding values | Ambient effects |

## 2. Surface syntax inside a region

### 2.1 Atoms

| Atom | Meaning |
|---|---|
| `42`, `-8`, `1.5`, `2e3` | Int / Float literal |
| `"text"` | String (escapes `\" \\ \n \t \r \0 \u{hex}`) |
| `true`, `false` | Bool |
| `x`, `pkg::x`, `+` | Variable (lexical or qualified) |
| `.label` | Field getter: a function value |
| `.label=` | Field setter: a function value |
| `#label` | Variant constructor: a function value |
| `r.f`, `r.pkg::f` (no space before the dot) | Call sugar, see §2.6 |

`::` separates module path components: `std::map`, `K::add`, `!std::if`, `$pkg::name` in prose. A dot never
names a module. Whether a dot starts a getter or a call depends only on what touches it. After whitespace or an
opener (`{`, `[`) it starts a getter (`{.x r}`, `{map rs .x}`). Directly after a token or a closing `}`/`]` it
is call sugar (`r.f`).

### 2.2 Braces are expressions

| Form | Meaning |
|---|---|
| `{f a1 ... an}` | Application, evaluated eagerly with exact arity. `{f}` calls a zero-argument function. |
| `{^ {x1 ... xn} decl* e}` | Lambda |
| `{^ self {x1 ... xn} decl* e}` | Recursive lambda: `self` names the function inside its body |
| `{: e T}` | Ascription: `T` must be a statically known Type |
| `{' e1 ... en}` | List literal |
| `{. l1 e1 ... ln en}` | Record literal (labels are not evaluated) |
| `{!m operands...}` | Macro use (syntax → syntax) |

The heads `^`, `:`, `'`, and `.` are reserved syntax. Every other brace form is an application.

### 2.3 Brackets are compiler directives

A bracket form's head is a keyword from a closed set. It is never a binding and is never called.

| Form | Class | Where | Value? | Binds? | Stage | Operands evaluated? |
|---|---|---|---|---|---|---|
| `[# x e]` | declaration | definition context | no | `x` | — | `e`, at the context's stage |
| `[Import p]`, `[Import* p]`, `[ImportAs p a]` | declaration | definition context | no | namespace (and exports) | compile | no |
| `[Native x T "id" Pure\|Static\|Runtime]` | declaration | definition context | no | `x` | compile | `T` statically |
| `[NativeType x "id"]` | declaration | definition context | no | `x : Type` | compile | no |
| `[Macro !m {pattern template}...]` | declaration | definition context | no | `!m` | compile | no (syntax) |
| `[% e]` | stage boundary | expression | yes | no | static | `e`, during checking |
| `[f "...$x..."]` | literal template | expression | String | no | context's stage | holes, left to right |
| `[latex "...$x..."]` | literal template | expression | Latex = `{latex: String}` | no | context's stage | holes, left to right |

Why these need special syntax:

- **Declarations** change the scope seen by later code. A function cannot do this.
- **`[% e]`** changes *when* `e` runs. Applying a function cannot do that.
- **Templates** parse a literal at compile time. A function would have to parse a String at runtime, and
  evaluated Strings never become code. Each hole is typed separately: in `[f]` it must be a Scalar, and in
  `[latex]` it is a Latex value (inserted as source) or a Scalar (formatted, and escaped if it is a String).
  Per-hole typing is what lets one template mix values of different types without variadic functions.

Brackets are therefore exactly the places where the compiler acts on syntax. Braces are the places where
evaluation happens.

### 2.4 Bodies and definition contexts

A lambda body, a module, or a document is a sequence of declarations followed by one result expression. For a
module or document, the expression regions take the place of the result.

Scoping rules:

- A declaration binds the rest of its body, in source order.
- A duplicate name in the same scope is an error.
- An inner scope may shadow an outer one.
- Prose between regions is not a scope boundary.

### 2.5 Desugaring (surface → core)

| Surface | Core |
|---|---|
| `{' e1 ... en}` | `listₙ(e1, ..., en)`, a constant n-ary constructor (≡ `K::cons(e1, ... K::nil)`, but flat). The elements are evaluated left to right. |
| `{. l1 e1 ... ln en}` | `recordᵥ⟨l1..ln⟩(e1, ..., en)`, a constant constructor applied eagerly |
| `[# x e] rest` in a body | `let x = e in rest` |
| `{!if c a b}` (std macro) | `K::caseBool(c, λ().a, λ().b)` |
| `{!and ...}`, `{!or ...}` | nested `!if` |
| `{!assert a b}` | `[% {assertEqual a b}]` |
| `[f "a $x b"]` | `K::stringAppend` chain over literal text and `K::scalarString(x)` |
| `{: e T}` | `e`, after checking against `T` |
| `r.f` | `{f r}` |
| `{r.f a ...}` (at a brace head) | `{f r a ...}` |
| `r.f.g` | `{g {f r}}`; at a brace head, only the last step takes the arguments |

### 2.6 Call sugar

`r.f` is a purely syntactic call, desugared by the reader before macros and checking see the code, so macros,
hygiene, and every later stage see ordinary calls.

- **Lookup.** `f` is resolved by ordinary lexical lookup where it is written. It is never chosen by the type
  of `r`: there is no method dispatch, and no per-type method tables. `{"Hello!".substr 0 3}` is exactly
  `{substr "Hello!" 0 3}`.
- **Order.** The receiver is the first argument, so it is evaluated first.
- **Method names.** A method name is a word identifier, possibly qualified (`r.std::length`). Operators cannot
  follow the dot; write `{+ a b}`.
- **Receivers.** A receiver is any atom (a name, a literal, a label function) or a closed brace or bracket
  form: `{{f x}.g}`, `{3.neg}`, `{2.5.floor}`.
- **Fields.** `r.name` calls a function named `name`; it does not read a field. Fields are read with
  `{.name r}`. If no function `name` is in scope, the error says so.
- **Methods stored in fields** are ordinary fields holding functions: `{{.greet obj} obj}`. A record built from
  a prototype (`{.name= proto "Ada"}`) shares the prototype's function values. Record update copies only the
  record's field table, and values are reference counted, never deep-copied.

## 3. Core calculus

```text
e ::= c                       constants: literals, primitives, label functions, static values
    | x                       local variable
    | λ[f](x1..xn). e          lambda; f, when present, is bound to the lambda itself
    | e0(e1, ..., en)         application, n ≥ 0
    | let x = e1 in e2
    | if e0 then e1 else e2   produced only by inlining K::caseBool (see §3.3)
```

Values:

```text
v ::= () | true | false | n | r | "s" | [v...] | {l = v, ...} | #l v | Type | closure | primitive | native | opaque
```

### 3.1 Evaluation (call by value, left to right)

```text
e0 ⇓ f    e1 ⇓ v1 ... en ⇓ vn    apply(f, v1..vn) ⇓ v
─────────────────────────────────────────────────────   (App)
               e0(e1, ..., en) ⇓ v

apply(⟨λf(x1..xn).b, ρ⟩ as F, v1..vn) = b[ρ, f ↦ F, xi ↦ vi]   only when the arity is exactly n
apply(prim, v1..vn) = δ(prim, v1..vn)                          only when n equals prim's arity
```

Evaluation rules:

- **Arity.** An arity mismatch is a type error, caught before execution. The runtime check is a backstop.
- **Errors.** An error aborts the whole evaluation immediately. Later arguments are not evaluated, and there is
  no catch.
- **Closures.** A closure captures its free variables by value; all values are immutable. A caller's binding
  of the same name cannot affect a closure.
- **Shadowing.** An inner binding hides an outer one only within its own scope.
- **Recursion.** `λf` is the only source of recursion. `f` refers to the closure being applied, so there is no
  "initialization before use" state. Recursion is monomorphic inside its own body.
- **Tail calls.** A call in tail position does not grow the frame stack.
- **Termination.** Termination is not guaranteed. Every evaluation runs under an instruction, frame, stack,
  allocation, string-size, and native-call budget. Running out is a `limit` diagnostic, never a hang or a host
  stack overflow.

### 3.2 Laziness

There is none. The only way to delay evaluation is a lambda. Eliminators take handler functions and call
exactly one of them:

- `K::caseBool` for Bool
- `K::caseList` for List
- `K::match` for variants

### 3.3 The only optimization with semantic preconditions

A call whose callee is the primitive `K::caseBool` is compiled as `if c then a else b` when both handler
arguments are literal zero-parameter, non-recursive lambdas. The compiler checks that the callee is
`K::caseBool` by the identity of the resolved constant, never by its name, so aliases are inlined and shadowing
definitions are not.

The rewrite is unobservable: in both forms exactly one branch runs, and creating a closure has no visible
effect.

## 4. Types

```text
τ ::= Unit | Bool | Int | Float | String | Type
    | List(τ)
    | Fn([τ1..τn], τ) ! ε            ε: capability set ⊆ {Static, Runtime}, or a variable
    | {l1: τ1, ..., ln: τn}         structural record, labels sorted
    | <l1: τ1, ..., ln: τn>         structural variant; <> is Never
    | Native("id")                  opaque host type
    | α                             inference variable, possibly constrained
```

Constraints on variables:

| Constraint | Satisfied by |
|---|---|
| `Num` | Int, Float |
| `Ord` | Int, Float, String |
| `Eq` | anything except functions and opaque values, recursively |
| `Scalar` | Int, Float, String, Bool, Unit |
| `has .l: τ` | a record with field `l: τ` |
| `has #l: τ` | a variant with alternative `l: τ` |
| `cases(h, r, ε)` | on a variant `<li: τi>`: `h` = `{li: Fn([τi], r) ! ε}` |

Inference is Hindley–Milner with let-generalization. Constraints are attached to variables and checked when
the variable is bound. A generalized binding keeps its constraints, so every use instantiates them again.
Constraints never depend on the *name* of the function they came from. A lambda that wraps `K::match`, or `.x`,
or `K::add` has the same constrained type as the primitive.

### 4.1 Label functions

These are first-class functions. Each label `l` determines one value:

```text
.l   : ∀ r a.   (r has .l: a) ⇒ Fn([r], a)
.l=  : ∀ r a.   (r has .l: a) ⇒ Fn([r, a], r)
#l   : ∀ v a.   (v has #l: a) ⇒ Fn([a], v)
K::match : ∀ v h r ε. cases(h, r, ε) on v ⇒ Fn([v, h], r) ! ε
```

A record literal `{. l1 e1 ... }` has the closed type `{l1: τ1, ...}`.

A variant variable that is still unresolved at a top-level region defaults to the closed variant made of
exactly its observed alternatives. This parallels `Num` defaulting to `Int`. Within a function it stays
polymorphic.

### 4.2 Effects (capabilities)

Every function type carries a capability variable:

- **Calling** a function adds its capabilities to the caller's context.
- **Creating** a closure adds nothing.
- **Sources.** `Static` and `Runtime` come only from natives declared with that availability. `Pure` natives
  and all primitives have none.
- **Static code.** A static expression must not require `Runtime`.
- **Runtime code.** A runtime region must not require `Static`.
- **Ascription.** Ascribing a function type cannot remove capabilities: an ascribed type gets a fresh
  capability variable, which is then unified with the expression's.

### 4.3 Type values and computed types

`Type` is an ordinary, first-order data type of ground descriptors: no variables, no quantifiers, no recursion.
Type values can be built at any stage with ordinary pure functions: `K::listType`, `K::functionType`,
`K::recordType`, and `K::variantType`.

The supported fragment of computed types:

- Computed types appear only where the checker reads a Type, which is two places: the type operand of `{: e T}`
  and the signature of `[Native ...]`.
- There, the operand must elaborate to a **static constant**: the name of a static binding, or `[% e]`.
- The checker reads the constant and converts it to a type. It never evaluates a function application to
  obtain a type.

Hence:

- There are no runtime-dependent types and no dependent function types.
- Types cannot be polymorphic: `{List a}` cannot be written, because a Type value contains no variables.
- A Type computation that does not terminate is a static `limit` error.

Equality of Type values is structural equality of normalized descriptors: record fields and variant
alternatives are sorted by label.

## 5. Stages

Every binding has a stage, S (static) or R (runtime).

| A binding `[# x e]` is static (S) when `e` is | Otherwise |
|---|---|
| a literal, a static name, `[% e']`, a lambda whose free variables are all S, a label function, a list/record literal or `[f]`/`[latex]` template whose parts are all S, or a Native / NativeType / import | R |

This is a syntactic, predictable rule. The checker never decides to evaluate early on its own: `[# x {+ 1 2}]`
is R, and `[# x [% {+ 1 2}]]` is S.

Rules:

1. `[% e]`: `e` is checked first. Its free variables must all be S, and its capability must not include
   `Runtime`. It is then evaluated in the static VM, under the static budget, and its result is embedded as a
   constant of type τ.
2. Any S value can be used at runtime, where it is a constant. A static closure called at runtime must not
   require `Static`.
3. A static closure can *contain* a reference to a Runtime native. Calling that closure statically is then a
   stage error, found by checking, before anything runs.
4. Runtime arguments are never visible to static code: a `[% ]` nested in a lambda cannot mention the lambda's
   parameters.
5. Static evaluation is isolated: there is no host state and no runtime inputs. Static natives read only the
   inputs a host declares for preparation.

## 6. Errors

Diagnostics carry a kind, a primary span, and notes. The notes record macro-expansion sites and call stacks.

| Kind | Raised by |
|---|---|
| `syntax` | the scanner, reader, or parser |
| `name` | resolution, duplicate bindings, import conflicts |
| `type` | inference, arity, constraints |
| `stage` | static/runtime capability or availability violations |
| `package` | resolution, cycles, exports |
| `native` | contract, linking, or capability failures |
| `static-evaluation` | an error raised while running `[% ]` |
| `runtime` | `K::fail`, overflow, division by zero, empty `head` |
| `limit` | an exhausted budget: instructions, frames, stack, allocation, strings, natives, macro depth/count/size, or nesting depth |

## 7. Macros

A macro maps syntax to syntax. It is never a value, and its use is always marked with `!`. Pattern macros are
an ordered list of `{pattern template}` clauses:

- **Matching.** A clause matches identifiers as variables, `_` as anything, literals exactly, nested lists by
  shape, and `p ...` as a repetition.
- **Hygiene.** Names a template introduces resolve where the macro was defined, and binders it introduces
  cannot capture the user's operands. This is implemented with marks.
- **Checking.** An expansion is parsed and checked exactly like hand-written code, so it gets no shortcut
  around types, stages, or effects.
- **Budgets.** Expansion depth (64), expansion count (1,000,000), and produced syntax size (4,000,000
  nodes) are all limited.

### 7.1 Static resource limits

| Limit | Value | Why |
|---|---|---|
| Bracket/brace nesting in source | 200 | Bounds the host stack used by parsing and dropping trees. |
| Expression nesting seen by the checker, through expansions | 96 | Bounds checker/compiler recursion; about 2 MiB of host stack in unoptimized builds. |
| Static evaluation | 10M instructions, 100k frames, 256 MiB allocation | The same VM as runtime, with a separate budget. |

Exceeding any of them is a `limit` diagnostic. The defaults for runtime evaluation are in `vm::Limits`.

Procedural macros are **(future)**. No use case has appeared that `[% ]` and pattern macros cannot cover.

## 8. Primitive boundary (the `K` package)

`K` is built in and imported by default in a prelude scope that a module may shadow. Every export is an
ordinary value with an ordinary constrained type scheme: there is no special binding class. Aliases,
wrappers, and higher-order uses therefore behave identically.

| Primitive | Type | Why it is a primitive |
|---|---|---|
| `add`, `subtract`, `multiply` | `∀n:Num. Fn([n,n],n)` | Machine arithmetic with checked overflow |
| `negate` | `∀n:Num. Fn([n],n)` | Machine arithmetic |
| `divideFloat` | `Fn([Float,Float],Float)` | IEEE division with a zero check |
| `quotientEuclidean`, `remainderEuclidean` | `Fn([Int,Int],Int)` | Needs wider-than-Int intermediate values |
| `toFloat` | `Fn([Int],Float)` | Representation conversion |
| `floor`, `ceil`, `round`, `truncate` | `Fn([Float],Int)` | Representation conversion with a range check |
| `sqrt`, `sin`, `cos`, `exp`, `ln` | `Fn([Float],Float)` | Transcendental functions; implemented deterministically (only IEEE `+ − × ÷ √`), so every platform gets identical bits |
| `fixed` | `Fn([Float,Int],String)` | Correctly rounded decimal rendering of binary64 |
| `equal`, `less`, `hash` | `∀a:Eq` / `∀a:Ord` | Structural over every representation; no type-directed dispatch exists in the language |
| `caseBool` | `Fn([Bool, Fn([],a)!e, Fn([],a)!e], a)!e` | Bool is a native representation and needs an eliminator |
| `nil`, `cons`, `caseList` | List constructors and eliminator | List is the only recursive type; general recursive types are (future) |
| `match` | see §4.1 | Variant eliminator |
| `stringAppend` | `Fn([String,String],String)` | String representation |
| `stringScalars` | `Fn([String],List(String))` | String representation (Unicode scalars) |
| `scalarString` | `∀s:Scalar. Fn([s],String)` | Number formatting; dispatches on the runtime representation |
| `latexEscape` | `Fn([String],String)` | Elaboration target of `[latex]`; the compiler may reference only `K` |
| `debugString` | `∀a. Fn([a],String)` | Deterministic display of any value, for diagnostics |
| `fail` | `∀a. Fn([String],a)` | Aborting is not expressible otherwise |
| `unit` | `Unit` | The Unit value |
| `intType`, `floatType`, `boolType`, `stringType`, `unitType`, `typeType` | `Type` | Base descriptors |
| `listType` | `Fn([Type],Type)` | Type-descriptor construction |
| `functionType` | `Fn([List(Type),Type],Type)` | Type-descriptor construction |
| `recordType`, `variantType` | `Fn([List({name:String,type:Type})],Type)` | Type-descriptor construction; validates and sorts |

Label functions (`.l`, `.l=`, `#l`, and record constructors) are families of constants indexed by syntax, in
the same way numeric literals are.

Everything else lives in `std`, written in Lamlet:

- Option, Pair, and Never are defined from records and variants.
- `if`, `and`, `or`, `not`, `choose`, the comparison operators, `min`, `max`, `abs`, and `pow` are ordinary
  definitions.
- All list functions are ordinary definitions.
- `format`, `concat`, `join`, and `assertEqual` are ordinary definitions.
- `Struct`, `Variant`, `Option`, `Pair`, and `Fn` are ordinary functions that build Type values.

### 8.1 The hash encoding

`K::hash` is FNV-1a (64-bit) over a canonical byte encoding, returned as the two's-complement Int. Equal values
have equal hashes on every platform and in every run. The encoding is part of the language contract, so changing
it is a language version change.

| Value | Encoding |
|---|---|
| Unit | tag `0` |
| Bool | tag `1`, then byte `0` or `1` |
| Int | tag `2`, then 8 bytes little-endian |
| Float | tag `3`, then the IEEE binary64 bits, little-endian (zero is normalized) |
| String | tag `4`, then a body |
| List | tag `5`, then the element count (u64 LE), then each element |
| Record | tag `6`, then the field count, then for each field in label order its label as a body and its value |
| Variant | tag `7`, then the label as a body, then the payload |
| Type | tag `8`, then the type's canonical display text as a body |

A body is its byte length as a u64 LE, followed by the UTF-8 bytes. Functions and native values have no hash.

## 9. Natives and host capabilities

- **Declaration.** `[Native x T "api-id" A]` declares a callable whose concrete type `T` (static, fixed arity,
  no function arguments in v1) and availability `A` must match the host registry exactly.
- **Effects.** `A = Runtime` or `A = Static` adds that capability to the function type. `Pure` adds nothing.
- **Validation.** Arguments and results are checked against the declared descriptors at the boundary.
- **Capabilities.** Named capabilities ("secrets") are granted per evaluation by the host context, and are
  checked by the VM on every call.
- **Opaque types.** `[NativeType x "id"]` declares an opaque type; its values are host handles.

Native calls are synchronous and are not preempted by the VM. They count against the native-call budget.

## 10. Documents and comments

A Lamlet source file is a document. Only these spellings are active; all other text is prose. Prose introduces no
bindings, produces no values, and is not a scope boundary.

| Spelling | Meaning |
|---|---|
| `$name`, `$pkg::name` | A lookup region: reads a binding (no call) |
| `${...}` | An expression region |
| `$[...]` | A bracket form: a declaration, import, `[% ]`, or template |
| `$;{...}`, `$;[...]` | A disabled region: inert, and not part of the displayed prose |
| `$$` | A literal `$` in prose |
| Any other `$` | Prose: `$50`, `$ {x}` |

Scanning rules:

- **Commitment.** Once `${` or `$[` is seen, the scanner commits to code. An unterminated or malformed region is a
  diagnostic, never silently treated as prose.
- **Balancing.** Delimiter balancing respects Strings, so `"}"` and `"$x"` inside code do not end a region or
  open another one.
- **Disabled regions.** Their bodies are raw text, with balanced braces/brackets and closed Strings, just
  like block comments inside code. Dollars and semicolons in the body have no special meaning.
- **Strings are never re-scanned.** An evaluated String is data and never becomes code.
- **Dollars in code.** Inside code, `$` is an error, except inside String literals and template holes.
- **Lookups and punctuation.** A lookup takes a word identifier and its `::name` components. A following `.` is
  prose, so `I am $AGE.` reads `AGE`.

Comments inside code:

| Spelling | Meaning |
|---|---|
| `;; text` | A line comment, to the end of the line. It also hides delimiters on that line. |
| `;{...}`, `;[...]` | Removes one balanced form. Its body is raw text, with balanced delimiters and closed Strings. |

The comment marker must touch its opener: `; { ... }` is not a block comment. A lone `;` inside code is a
syntax error. A block comment's body does not interpret `;;` as a line comment. Semicolons inside Strings
are literal data, and unprefixed semicolons in document prose are ordinary text. `~` is an ordinary symbolic
identifier and no longer starts a comment.

**Doc comments.** A declaration (`[# ...]`, `[Macro ...]`, `[Native ...]`, `[NativeType ...]`) gets a doc from
one of these places:

- Inside code, a comment directly before the declaration (consecutive `;;` lines join).
- At document level, the disabled region directly before it, or the prose paragraph directly above it. The
  declaration must start its line, and there must be no blank line in between.

Docs never affect checking or evaluation. They are recorded by definition identity, so they follow a binding
through imports and aliases (`Document::docs`, `StaticBinding::doc`).

## 11. Packages

A package is a document checked as a module. The host supplies packages through an injected `Resolver`; the core
never searches directories.

| Form | Binds |
|---|---|
| `[Import p]` | The namespace `p`, so exports are read as `p::name` |
| `[Import* p]` | The namespace `p` and every export under its own name |
| `[ImportAs p a]` | The namespace `a` only |

Rules:

- **Exports.** `Exports::All` exports every top-level name not starting with `_`, including macros as `!name`.
  `Exports::Names` lists exports explicitly; it may re-export imported bindings or namespaces, which is how
  submodules are published.
- **Scope.** Imports are declarations: they follow lexical, source-order scope, and may appear in lambda bodies.
  They are resolved at compile time.
- **Conflicts.** Two different bindings for one name in one scope are an error. A wildcard import is checked as
  a whole before any of its names are bound. Re-importing the same binding is idempotent, and the namespace name
  itself counts as a binding. An inner scope may shadow an outer one.
- **Identity.** Aliases and re-exports preserve a definition's identity: its type, stage, and capabilities.
- **Cycles.** Import cycles are rejected, and the diagnostic gives the cycle's path.
- **Initialization.** A package's runtime bindings are initialized once per `Instance`, in dependency order,
  including through diamond imports. A runtime-initialized binding is not static.
- **`std`.** `std` is an ordinary package that the standard resolver always supplies.
- **`K`.** `K` is built in: resolvers cannot replace it. Every module imports it in a prelude scope enclosing the
  module, so a module may shadow it.
