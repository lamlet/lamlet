# Design rationale

The governing question: what is the smallest coherent functional language that can compute a host's
descriptions (such as an animated, narrated presentation) without putting presentation semantics into the
language? Each decision below records the alternatives that were considered.

## D1. Every primitive is an ordinary value with an ordinary type

**Decision.** There are no special bindings. Every `K` primitive has an ordinary, possibly constrained, type
scheme. When an operation needs static information (a field name, an alternative, a recursion target), the
information is *syntax*:

- `.l`, `.l=`, `#l` for fields and alternatives
- `{. ...}` for record literals
- `{' ...}` for list literals
- `{^ self ...}` for recursion

The alternative is to put the requirement into a scheme as a constraint, as `K::match` does with its case
constraint.

**Why.** A function's meaning should not depend on how it is spelled at the call site. Wrapping a primitive in a
lambda, aliasing it, or passing it to a higher-order function must preserve its behavior and its type. Labels
are static by construction (they are tokens), so no "this argument must happen to be a compile-time constant"
rule is needed.

**Alternatives considered.**

- *Primitives that take static arguments, such as `{get r "field"}` with a String.* These become intrinsics that
  cannot be wrapped (`{^ {r k} {get r k}}` cannot be typed), and they need dependent typing in disguise.
- *Dependent function types.* That would be unrestricted dependent typing, which is out of scope.

## D2. No variadic functions; one list literal

**Decision.** Operators are binary, collection functions take Lists, and `{' ...}` is the only variable-length
form. Heterogeneous formatting is done by `[f "..."]`, whose holes are typed one by one *because it is syntax*.

**Why.** A variadic calling convention needs a flag in function types, special unification between variadic and
fixed arities, and, for heterogeneous arguments, something like rank-2 argument vectors. Such a function cannot
be written by users, stored, or passed where a list function is expected.

**Alternatives considered.**

- *Record-based argument packs.* These need records with optional fields.
- *Implicit currying.* This changes exact-arity evaluation and error behavior.
- *Homogeneous variadics only.* These still need the calling-convention flag.

**Cost.** Code like `{+ 1 2 3}` must nest, or use `fold`.

## D3. Structural records and labeled variants

**Decision.**

- Records `{l: τ}` are closed in their literal type. Field access is row-polymorphic through constraints.
- Variants `<l: τ>` are eliminated by `K::match` with a record of handlers. Its typing relation is solved in
  whichever direction information arrives: from the variant, or from the handler record.
- Option, Pair, Never, and Latex are library definitions.

**Why.** Labeled variants allow two alternatives of the same type (`#left 1` versus `#right 1`), and they compose
with first-class functions.

**Alternatives considered.**

- *Unions indexed by type (`Union Int String`).* These cannot hold two alternatives of the same type, and they
  force injection and projection to take Type arguments. That is D1's problem again.
- *Nominal wrapper types with a tag registry.* They need global mutable registration, which makes type
  construction order-dependent.

**Limitation.** An unannotated top-level `#tag` defaults to the closed variant of what it has seen, just as `Num`
defaults to Int. *Abstract* types, with a hidden representation, would need module-private constructors; they
are future work.

## D4. Recursion is a binding form

**Decision.** `{^ name {params} body}` binds `name`, monomorphically, to the function itself. The compiler emits
a `SelfClosure` instruction, so the closure never captures itself.

**Why.**

- In a typed call-by-value calculus with exact arity, a generic fixpoint combinator cannot have an ordinary type
  over all arities.
- A builder-based `fix` needs a special typing rule, runs again on each use, and has an "used while being
  constructed" failure mode.
- Making a closure refer to itself through its frame keeps reference counting acyclic, at no cost.

**Sugar.** `[# {f x} body]` is `[# f {^ f {x} body}]`. The head declares `f`, so in the body `f` is the function
being defined, as with Scheme's `define` or Python's `def`, and recursion needs no second spelling of the name.
This differs from the plain form exactly when `f` shadows an outer binding: `[# {g x} {+ {g x} 1}]` calls
itself, while `[# g {^ {x} {+ {g x} 1}}]` wraps the outer `g`, because a plain binding's expression cannot see
the name it binds. Both are deliberate: the function form defines, and the plain form extends. It is pure
syntax: the reader produces the same lambda, and the checker has no rule for it. One `[# ...]` may also
hold several bindings (`[# a 1 b 2]`), which is the same as one declaration per binding; a function head's body
ends at its first expression, so the forms stay unambiguous without separators.

**Limitation.** There is no mutual recursion. It can be encoded by dispatching on a variant tag inside one
function. `letrec` groups are future work.

## D5. One eliminator per native data type; laziness only through lambdas

**Decision.**

- The eliminators are `K::caseBool`, `K::caseList`, and `K::match`; each takes handler functions.
- `!if`, `!and`, and `!or` are std macros over `K::caseBool`.
- `K::caseBool` applied to two literal thunks compiles to a jump. The compiler recognizes the primitive by the
  identity of the resolved constant, never by its name, and the optimization is unobservable.

**Why.** Uniform elimination means the optimization is a property of the eliminator rather than of one macro's
output, and laziness is always visible as a lambda.

## D6. Explicit, syntactic staging; types are ordinary data

**Decision.** A binding is static by a syntactic rule (spec §5), never because an optimizer could fold it. Type
values may be computed at any stage, because building a Type has no effect. Only the checker's *reading* of a
Type requires a static constant. The checker evaluates code to find one only at an explicit stage boundary:
`[% e]`, the type operand of an ascription `{: e T}`, or a native's signature `[Native f T …]`. Those operands
can only ever be static Types, so they are stage boundaries by their position, and `{: xs {List Int}}` and
`[Native f {Fn {' Int} Int} …]` need no `[% ...]`. The rule is still syntactic: `T`
may use only static bindings, exactly as inside `[% ...]`.

**Supported fragment.** Computed types are ground, first-order descriptors: no type variables (`{List a}` cannot
be written) and no recursive types. This keeps the checker free of dependent typing.

**Alternative considered.** *A separate type-expression sublanguage.* It would duplicate the expression language,
and lose computed types built by ordinary functions, which is the feature worth having.

## D7. No type-directed values

**Decision.** There are no polymorphic numeric constants and no type classes. Numeric polymorphism dispatches on
the runtime representation of operands, so it exists only where an operand exists. Folds take an explicit
initial value.

**Why.** A constant whose meaning depends on a type that is not known at runtime would make a value's meaning
depend on how a host chooses to look at it.

**Alternative considered.** *Type classes with dictionary passing.* That is a large feature, and it changes how
calls are elaborated.

## D8. Deterministic transcendental functions

**Decision.** `sin`, `cos`, `exp`, and `ln` are implemented with only IEEE `+ − × ÷` and exact scaling. `sqrt`
is IEEE, so it is correctly rounded.

**Why.** Deterministic replay and offline rendering should not depend on the platform's math library. The results
are pinned bit-for-bit in tests. They are accurate to a few ulps, not correctly rounded; for example, `exp 1.0`
is 1 ulp above e.

## D9. Pattern macros only

**Decision.** Macros are hygienic pattern macros, with budgets. There are no procedural macros.

**Why.** No use case needs them; computation over data uses `[% ...]`.

**Splice groups.** A template's `...` repeats one element, so producing flat pairs (`{. a fa b fb}` from
`{a fa} {b fb}`) once needed a recursive helper macro, one expansion level per pair, which ran into the depth
limit for large or nested uses. A splice group `{... t1 t2}` is instantiated in place, so `{... t1 t2} ...`
repeats several elements in one expansion. It reuses the reserved `...` in a position that had no meaning, so
no existing macro changes. Patterns do not accept it yet: matching a flat sequence in chunks (`{... k v} ...`
against `a 1 b 2`) is the natural extension, deferred until a macro needs it.

**Limits.** Macro expansion depth (64), checker expression nesting (96), and reader nesting (200) are limited
deliberately. Together they bound the host stack, so even unoptimized builds report a `limit` diagnostic rather
than overflowing a 2 MiB thread stack.

## D10. Host boundary: descriptions, not effects

**Decision.**

- Presentation behavior is data: variants of actions, and curves as functions of time.
- Speech is a description, timed by host measurements.
- A host lowers the descriptions to its own timeline (render-lab, for example, to its `Timeline`), so the host
  remains the only scheduler.

**Why.** Replay, seeking, and offline rendering come for free, because both sides are pure functions of time.
Evaluation can never trigger playback.

**Alternative considered.** *Lamlet calling a host's APIs per frame through natives.* That couples the language
runtime to the frame loop, and makes determinism depend on native behavior.

## D11. `::` for modules, `.` for lexical call sugar

**Decision.** Module paths use `::`. A dot directly after a token is call sugar: `r.f` means `{f r}`, with `f`
resolved lexically.

**Why.** The sugar lets pipelines read left to right without adding semantics. Keeping module paths off the dot
means `p.name` cannot mean either "a package export" or "a call" depending on what `p` is, and a sentence-ending
dot in prose is never part of a name.

**Alternatives considered.**

- *Type-directed member functions.* The function would depend on the receiver's inferred type. That fails, or
  needs type classes, when the type is unknown, as in `{^ {x} {x.m}}`, and it would break D1's
  wrapping-preserves-meaning rule.
- *Falling back to a field when no function is in scope.* Then an unrelated definition elsewhere would change
  what `{p.name}` means.

**Objects.** Objects are records of functions. A prototype record plus `.f=` gives defaults and shared methods.
Values are reference counted, so instances share closures, and an update copies only the field table.

**Cost.** Whitespace is significant around the dot: `{x.f}` and `{x .f}` mean different things.

## D12. Semicolon-prefixed comments

**Decision.** Inside code, `;{...}` and `;[...]` remove a balanced form, and `;;` starts a line comment.
At document level, `$;{...}` and `$;[...]` disable a region. The marker must touch the opener; a lone `;`
inside code is an error. Strings and ordinary document prose retain literal semicolons.

**Why.** One prefix groups all comment forms. The next character selects a balanced comment or a line
comment, so the reader can distinguish them without interpreting the commented text as code. Balanced
comments keep braces/brackets balanced and Strings closed; their bodies otherwise remain raw text.

**Compatibility.** This replaces the former tilde-prefixed balanced comments and single-semicolon line
comments. Tilde remains available as an ordinary symbolic identifier. Comment text can still document the
following declaration.

## D13. Raw templates, `f"{...}"`

**Decision.** `f"{text}"` and `latex"{text}"` are templates whose text runs, exactly as written, to the first
`}"` outside a hole. Their holes are balanced code, so they may hold Strings. `[f "..."]` and `[latex "..."]`
stay. In a document, `$f"{...}"` is a region.

**Why.** In `[f "..."]` a quote ends the template, so prose with quotes needs `\"`, and a hole cannot hold a
String literal at all. Text without delimiters (`[f Hello, ${...}]`) would need balanced brackets and an escape
for a lone `]`. `}"` is a terminator that ordinary text almost never contains, and the one text it excludes is
still expressible without an escape character, as a hole: `${str "}\""}`.

**Alternatives.** `[f "{...}"]` would keep templates in brackets, but `[f "{a} b"]` is already a valid template
whose text starts with a brace, so it would change meaning. `f"` was a syntax error before (a token must be
followed by whitespace or a delimiter), so no document meant anything by it.

**Cost.** Templates were the bracket forms that parse a literal at compile time; the raw form is an atom
instead, an exception to "brackets are where the compiler acts on syntax" (§2.3), justified as a literal, like
a String or a number. In prose, `$f"{` now opens a region where it used to read the binding `f` followed by
the text `"{`.

## Known limitations

- No mutual recursion, recursive types (other than List), polymorphic computed types, abstract types, or type
  classes.
- No higher-rank polymorphism: a lambda parameter used at two types must be let-bound instead.
- Checking stops at the first diagnostic.
- No portable compiled artifacts: programs are in-memory only.
- Some `match` type errors report a structural record mismatch rather than naming the missing handler.
- Expression nesting is limited to 96 levels, including macro expansion.
