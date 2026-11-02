# Lamlet quick start

This guide is for readers who already know a functional language (Haskell, OCaml, Scheme, Elm, ...). It shows
how familiar ideas are spelled in Lamlet and where Lamlet deliberately differs. Every example here was run
against the interpreter. The full rules are in [spec.md](spec.md).

In one sentence: Lamlet is a strict, typed lambda calculus with Hindley–Milner inference, structural records and
variants, explicit compile-time evaluation, and hygienic macros, embedded in ordinary text documents.

## 1. Running code

```sh
cargo build --release
./target/release/lamlet eval file.lam     # check, then evaluate; JSON results on stdout
./target/release/lamlet check file.lam    # types and stages only
cargo run -q -p lamlet --example run -- file.lam   # plain one-line-per-result output
```

A Lamlet file is a **document**. Only `$`-prefixed regions are code; everything else is prose that the
language ignores:

```text
This sentence is prose and is ignored.
$[Import* std]
The answer is ${+ 1 2}.
```

| Spelling | Meaning |
|---|---|
| `${...}` | An expression; its value is a result of the document |
| `$[...]` | A compiler form, such as a declaration or an import |
| `$name` | Read a binding |
| `$$` | A literal dollar sign |

Inside a region you write plain code with no more dollars. Almost every file starts with `$[Import* std]` to
bring the standard library's names into scope.

## 2. Functions and application

Application is written in braces, function first. There are no infix operators: `+` is an ordinary name.

```text
${+ 1 2}                          ;; 3
$[# square {^ {x} {* x x}}]       ;; [# name value] binds a name; {^ {params} body} is a lambda
${square 7}                       ;; 49
${{^ {} 42}}                      ;; a zero-argument function, called with {f}
```

Lambdas have an **exact arity**. There is no automatic currying or partial application, so `{+ 1}` is a type
error ("expects 2 arguments, but 1 were given"). Curry explicitly by returning a lambda:

```text
$[# adder {^ {n} {^ {x} {+ x n}}}]
${{adder 10} 5}                   ;; 15
${map {' 1 2 3} {adder 1}}        ;; [2, 3, 4]
${fold {' 1 2 3 4} 0 +}           ;; 10, and + is first-class
```

Evaluation is strict, call by value: the function, then each argument left to right, each exactly once.

`;;` starts a line comment inside code, and `;{...}` or `;[...]` comments out one balanced form. Use
`$;{...}` or `$;[...]` to disable a whole document region. A lone `;` inside code is a syntax error;
semicolons in Strings and ordinary prose remain literal text.

```text
${+ 1 ;{this form is ignored} 2}
${+ 1 ;[this note is ignored] 2}
${+ 1 ;; ignore the rest of this line, including } and ]
    2}
```

### Local bindings

A function body is a series of declarations followed by one result expression. Each declaration binds the rest
of the body; this is `let*`.

```text
${{^ {x}
    [# y {* x 2}]
    [# z {+ y 1}]
    z} 5}                         ;; 11
```

Bindings are immutable and lexically scoped, and a binding generalizes as in ML's `let`. Rebinding a name in
the same scope is an error; shadowing it in an inner scope is fine.

### Recursion

A plain binding cannot see itself: `[# f {^ {n} {f n}}]` reports "unknown name `f`". To recurse, give the
lambda a name after `^`:

```text
$[# fact {^ fact {n} {!if {= n 0} 1 {* n {fact {- n 1}}}}}]
${fact 10}                        ;; 3628800

$[# sumTo {^ loop {n acc} {!if {= n 0} acc {loop {- n 1} {+ acc n}}}}]
${sumTo 100000 0}                 ;; 5000050000, and tail calls run in constant space
```

The name is local to the lambda. Recursion is monomorphic inside the body, and there is no mutual recursion.

## 3. Values and types

| Type | Literals / constructors | Notes |
|---|---|---|
| `Int` | `42`, `-8` | Signed 64-bit; overflow is an error |
| `Float` | `1.5`, `2e3` | Finite binary64; there is no implicit conversion from Int (use `float`) |
| `Bool` | `true`, `false` | |
| `String` | `"text\n"` | Unicode scalars; `++` concatenates two Strings |
| `Unit` | `unit` | |
| `List(a)` | `{' 1 2 3}`, `{'}` | `'` is list syntax, not quotation |
| Records | `{. name "Ada" age 36}` | Structural |
| Variants | `{#circle 1.0}` | Structural, tagged |
| Functions | `{^ {x} ...}` | |
| `Type` | `Int`, `{List Int}`, ... | Types are values too (§6) |

Numeric operators are binary and polymorphic over Int and Float, but you cannot mix the two:

```text
${+ 1.5 2.0}  ${float 3}  ${round 2.5}  ${% -7 3}     ;; 3.5  3.0  3  2
${+ 1 2 3}    ;; type error: this function expects 2 arguments, but 3 were given; write {+ {+ 1 2} 3}
${+ 1 2.0}    ;; type error: type mismatch: `Int` versus `Float`
```

Types are inferred; you rarely write one. To check an expression against a type, use `{: e T}`:

```text
$[# addInts {: {^ {a b} {+ a b}} [% {Fn {' Int Int} Int}]}]
${addInts 2 3}                    ;; 5
```

## 4. Records and variants

**Records** are built with `{. label value ...}`. The label syntax gives you first-class functions:

- `.name` reads the field `name`
- `.name=` returns a copy of the record with `name` replaced

```text
$[# p {. name "Ada" age 36}]
${.name p}                        ;; "Ada"
${.age= p 37}                     ;; {age: 37, name: "Ada"}
${map {' p p} .age}               ;; [36, 36]
```

Field access is row-polymorphic: `{^ {r} {.age r}}` accepts any record that has an `age` field.

**Variants** are built with `#tag payload` and taken apart with `match`. `match` takes a record holding one
handler per alternative:

```text
$[# area {^ {s} {match s {. circle {^ {r} {* 3.14 {* r r}}}
                            rect   {^ {wh} {* {.w wh} {.h wh}}}}}}]
${area {#circle 1.0}}             ;; 3.14
${area {#rect {. w 2.0 h 3.0}}}   ;; 6.0
```

The handlers determine the variant type, and they must cover it exactly; a missing or extra case is a type
error.

Options and pairs are not built in. They are ordinary definitions in std, made from these two forms:

- `some` / `none` build the variant `<none: Unit, some: a>`; `caseOption` and `withDefault` take it apart
- `pair` builds the record `{first, second}`

```text
${withDefault {some 3} 0}  ${withDefault none 0}      ;; 3  0
```

## 5. Call sugar: `x.f`

A dot written directly after a value is a call: `x.f` means `{f x}`. At the head of a brace, the receiver
becomes the first argument:

```text
${"Hello!".substr 0 3}               ;; {substr "Hello!" 0 3}  → "Hel"
${"Hello!".chars.length}             ;; {length {chars "Hello!"}} → 6
${{' 1 2 3}.map {^ {x} {* x x}}}     ;; [1, 4, 9]
${3.neg}                             ;; -3
```

This is syntax, not dispatch. `f` is looked up in scope like any name, whatever the type of `x`, so it works on
every type and wrapping it in a lambda changes nothing. Keep these distinctions in mind:

- **Spacing matters.** `{r .x}` (with a space) applies `r` to the getter `.x`.
- **Fields are separate.** `{p.name}` calls a *function* `name`; read a field with `{.name p}`. The error
  message tells you this if you mix them up.
- **Methods in fields.** Objects are records whose fields can hold functions. Build instances from a prototype
  so they share one closure:

```text
$[# _greet {^ {self} [f "Hi, I am ${.name self}."]}]
$[# Person {. name "nobody" greet _greet}]
$[# ada {.name= Person "Ada"}]
${{.greet ada} ada}                  ;; "Hi, I am Ada."
```

## 6. Control flow is macros

The language is strict, so `if` cannot be a function. `!if`, `!and`, and `!or` are std macros: syntax-to-syntax
rewrites, always marked with `!`. Only the chosen branch runs, but both branches are type-checked:

```text
${!if {< 1 2} "yes" {error "never runs"}}             ;; "yes"
```

`{!and a b ...}` and `{!or a b ...}` short-circuit. For first-class use there is `caseBool`, which takes thunks:
`{caseBool c {^ {} a} {^ {} b}}`. There is also `choose`, which is eager.

You can define your own macros. Expansion is hygienic: names the template introduces cannot capture the
caller's names.

```text
$[Macro !unless {{c a b} {!if c b a}}]
${!unless false "ran" "skipped"}                      ;; "ran"
```

## 7. Compile-time evaluation and types as values

`[% e]` evaluates `e` while the document is being checked, and embeds the result as a constant:

```text
$[# table [% {map {range 0 5} square}]]
$table                            ;; [0, 1, 4, 9, 16], computed once, at compile time
${!assert {length table} 5}       ;; a compile-time check
```

Static code may only use static bindings. A binding is static when it is one of these:

- a literal
- a lambda over static values
- a list, record, or template literal made of static parts
- a `[% ]`

Anything else is a runtime binding, even if it could be folded:

```text
$[# b {+ 1 2}] $[% b]     ;; stage error: a static expression cannot use the runtime binding `b`
$[# b [% {+ 1 2}]] $[% b] ;; fine
```

Types are ordinary values of type `Type`, built by ordinary functions. Positions where the checker reads a type
need a static one, so computed types are wrapped in `[% ]`:

```text
$[# Person [% {Struct {' {Field "name" String} {Field "age" Int}}}]]
${: p Person}                     ;; checks p against the computed record type
```

Other type builders are `List`, `Fn`, `Variant`, `Option`, and `Pair`. Types are ground: you cannot write
`{List a}` with a type variable.

## 8. Strings and templates

`[f "..."]` is a string template. Holes are `$name` or `${expr}`; each hole is type-checked and must be a
scalar.

```text
$[# who "Ada"]
$[f "Hello, $who! 2 + 2 = ${+ 2 2}."]                 ;; "Hello, Ada! 2 + 2 = 4."
```

A hole cannot contain a string literal, because the quote would end the template; bind the String first. There
is also a runtime formatter and a joiner:

```text
${format "{} and {}" {' "a" {str 1}}}     ;; "a and 1"
${join ", " {' "x" "y"}}                  ;; "x, y"
```

`[latex "..."]` is the same idea for formulas. Backslashes are literal, and the result has type `Latex`.

## 9. Packages and the primitives

| Import | Gives you |
|---|---|
| `[Import std]` | Qualified names: `std::map` |
| `[Import* std]` | The names directly |
| `[ImportAs std s]` | A short alias: `s::map` |

Two wildcard imports that define the same name are an error, not a silent shadowing.

Every module can also see `K`, the built-in primitives: `K::add`, `K::caseList`, `K::match`, ... Each is an
ordinary function with an ordinary type, so you can alias it, wrap it, or pass it around freely. Everything in
`std/std.lam` is plain Lamlet over `K`, and is worth reading as a tour of idiomatic code.

## 10. Common pitfalls

| You wrote | Problem | Write instead |
|---|---|---|
| `{+ 1 2 3}` | Operators are binary | `{+ {+ 1 2} 3}` or `{fold {' 1 2 3} 0 +}` |
| `{+ 1}` | No partial application | `{^ {x} {+ 1 x}}` |
| `{+ 1 2.0}` | No implicit Int to Float | `{+ {float 1} 2.0}` |
| `[# f {^ {n} {f n}}]` | A plain binding is not recursive | `[# f {^ f {n} {f n}}]` |
| `{if c a b}` | `if` is a macro | `{!if c a b}` |
| `{. x 1 y}` | A label without a value | `{. x 1 y 2}` |
| `$[% x]` where `x` is computed | `x` is a runtime binding | `[# x [% ...]]` |
| `{p.name}` | That calls a function `name` | `{.name p}` |
| `std.map`, `K.add` | Module paths use `::` | `std::map`, `K::add` |
| `{x .f a}` | The space makes `.f` a getter | `{x.f a}` |

## 11. Where to go next

- [spec.md](spec.md): the precise semantics, the primitive list, and the staging rules
- `std/std.lam`: the whole standard library, in Lamlet
- `tests/fixtures/*.lam`: small documents that state their expected results
- [host.md](host.md): the contract between Lamlet and a host application
