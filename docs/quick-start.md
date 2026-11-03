# Lamlet quick start

This guide is for readers who already know a functional language (Haskell, OCaml, Scheme, Elm, ...). It shows
how familiar ideas are spelled in Lamlet and where Lamlet deliberately differs. Every example here was run
against the interpreter. The full rules are in [spec.md](spec.md).

In one sentence: Lamlet is a strict, typed lambda calculus with Hindley–Milner inference, structural records and
variants, explicit compile-time evaluation, and hygienic macros, embedded in ordinary text documents.

## 1. Running code

```sh
cargo build --release
./target/release/lamlet file.lam          # run it: print the text, each region replaced by its value
./target/release/lamlet build file.lam -o file.md   # compile to Markdown, HTML, or JSON (documents.md)
./target/release/lamlet eval file.lam     # check, then evaluate; JSON results on stdout
./target/release/lamlet check file.lam    # types and stages only
```

A Lamlet file is a **document**. Only `$`-prefixed regions are code; everything else is text, printed as
written. `lamlet file.lam` prints this one as `This sentence is text.` and `The answer is 3.` (the import's
line disappears):

```text
This sentence is text.
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
${{^ {x} {* x x}} 7}              ;; 49: {^ {params} body} is a lambda ({λ ...} too)
$[# {square x} {* x x}]           ;; [# {name params} body] defines a function
${square 7}                       ;; 49
${{^ {} 42}}                      ;; a zero-argument function, called with {f}
```

A function definition puts the call shape where the name goes. It binds the name to a lambda that may call
itself:

```text
$[# {cube x} {* x {square x}}]    ;; the same as [# cube {^ cube {x} {* x {square x}}}]
$[# two 2  {twice f x} {f {f x}}] ;; one [# ...] may hold several bindings, in order
${twice cube two}                 ;; 512
```

Lambdas have an **exact arity**. There is no automatic currying or partial application, so `{+ 1}` is a type
error ("expects 2 arguments, but 1 were given"). Curry explicitly by returning a lambda:

```text
$[# {adder n} {^ {x} {+ x n}}]
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
${{^ {x} [# y {* x 2} z {+ y 1}] z} 5}   ;; the same, with both bindings in one declaration
```

A function definition's body may have declarations too: `[# {f x} [# y {* x 2}] {+ y 1}]`. Its body ends at
its first expression, where the next binding (if any) starts.

Bindings are immutable and lexically scoped, and a binding generalizes as in ML's `let`. Rebinding a name in
the same scope is an error; shadowing it in an inner scope is fine.

### Recursion

A function defined with `[# {f params...} body]` can call itself:

```text
$[# {fact n} {!if {= n 0} 1 {* n {fact {- n 1}}}}]
${fact 10}                        ;; 3628800
```

That form is short for a lambda with a name after `^`, which names the function inside its own body. Write it
directly for a recursive lambda that is not bound, or whose inner name differs:

```text
$[# sumTo {^ loop {n acc} {!if {= n 0} acc {loop {- n 1} {+ acc n}}}}]
${sumTo 100000 0}                 ;; 5000050000, and tail calls run in constant space
```

A plain binding cannot see itself: `[# f {^ {n} {f n}}]` reports "unknown name `f`". The name after `^` is
local to the lambda. Recursion is monomorphic inside the body, and there is no mutual recursion.

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

Types are inferred; you rarely write one. To check an expression against a type, use `{: e T}`. The type is
computed at compile time, so it may only use static names (§7):

```text
$[# addInts {: {^ {a b} {+ a b}} {Fn {' Int Int} Int}}]
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
$[# {area s} {match s {. circle {^ {r} {* 3.14 {* r r}}}
                          rect   {^ {wh} {* {.w wh} {.h wh}}}}}]
${area {#circle 1.0}}             ;; 3.14
${area {#rect {. w 2.0 h 3.0}}}   ;; 6.0
```

The handlers determine the variant type, and they must cover it exactly; a missing or extra case is a type
error. The std macro `!case` writes the same thing with one clause per alternative, each naming its payload:

```text
$[# {area s} {!case s {circle r {* 3.14 {* r r}}}
                       {rect wh {* {.w wh} {.h wh}}}}]
```

Options and pairs are not built in. They are ordinary definitions in std, made from these two forms:

- `some` / `none` build the variant `<none: Unit, some: a>`; `caseOption` and `withDefault` take it apart
- `pair` builds the record `{first, second}`

```text
${withDefault {some 3} 0}  ${withDefault none 0}      ;; 3  0
```

**Recursive types** are declared with `[Type ...]`. The head names the type and its parameters. Then come the
names of two functions, `roll` and `unroll`, and last the type's structure, an ordinary Type expression in
which the type's own name is the recursive occurrence:

```text
$[Type {Tree a} rollTree unrollTree
  {Variant {' {Field "leaf" Unit}
              {Field "node" {Struct {' {Field "l" Tree} {Field "v" a} {Field "r" Tree}}}}}}]
$[# leaf [% {rollTree {#leaf unit}}]]
$[# {node l v r} {rollTree {#node {. l l v v r r}}}]
$[# {sum t} {!case {unrollTree t} {leaf u 0} {node n {+ {+ {sum {.l n}} {.v n}} {sum {.r n}}}}}]
${sum {node {node leaf 1 leaf} 2 leaf}}   ;; 3
${Tree Int}                               ;; Tree(Int)
```

A `Tree(Int)` is not a variant: `rollTree` makes one from its structure, and `unrollTree` gives the structure
back, so a `match` needs the `unrollTree` first. At run time both functions are the identity. Two declarations
with the same structure define the same type. A type with no parameters is written `[Type Nat rollNat unrollNat
{Option Nat}]`.

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
$[# {_greet self} f"{Hi, I am ${.name self}.}"]
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

In a template, `t ...` repeats `t` for each match of a repeated pattern. A splice group `{... a b}` stands for
its elements in place, so `{... a b} ...` repeats two elements together:

```text
$[Macro !rec {{{k v} ...} {. {... k v} ...}}]
${!rec {x 1} {y 2}}                                   ;; {x: 1, y: 2}
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

Types are ordinary values of type `Type`, built by ordinary functions. Where the checker reads a type (an
ascription's type, a native's signature), it evaluates it at compile time, as if written `[% ]`. A type you
name with `[# ...]` is an ordinary binding, so it needs `[% ]` to be static:

```text
${: {' 1.0} {List Float}}         ;; computed in place: no [% ] needed
$[# Person [% {Struct {' {Field "name" String} {Field "age" Int}}}]]
${: p Person}                     ;; checks p against the computed record type
```

Other type builders are `List`, `Fn`, `Variant`, `Option`, `Pair`, and the constructors of `[Type ...]`
declarations (§4). Types are ground: you cannot write `{List a}` with a type variable.

## 8. Strings and templates

`f"{...}"` is a string template. Holes are `$name` or `${expr}`; each hole is type-checked and must be a
scalar. The text, exactly as written, runs to the first `}"` outside a hole: quotes and backslashes need no
escapes, and a hole may hold a String. A template is expanded during the check into String concatenation, so it
is an ordinary String.

```text
$[# who "Ada"]
$f"{Hello, $who! 2 + 2 = ${+ 2 2}.}"                ;; Hello, Ada! 2 + 2 = 4.
$[# {quote p} f"{${.name p} said "${.words p}"}"]   ;; Ada said "hi" (for {. name "Ada" words "hi"})
$f"{a ${str "x"} b}"                                 ;; a x b: a hole may hold a String
```

A hole `${...}` holds a form, so `${age}` *calls* `age`; read a binding with `$age`. There is also a runtime
formatter and a joiner:

```text
${format "{} and {}" {' "a" {str 1}}}     ;; "a and 1"
${join ", " {' "x" "y"}}                  ;; "x, y"
```

A formula is a `Latex`, std's record `{latex: String}`. Make one from a template, whose backslashes are text:
`{latex f"{\frac{$who}{2}}"}`. To show a name in a formula, escape it: `{latex f"{\text{${latexEscape name}}}"}`.

## 9. Packages and the primitives

| Import | Gives you |
|---|---|
| `[Import std]` | Qualified names: `std::map` |
| `[Import* std]` | The names directly |
| `[ImportAs std s]` | A short alias: `s::map` |

Two wildcard imports that define the same name are an error, not a silent shadowing.

Every module can also see `K`, the built-in primitives: `K::add`, `K::caseList`, `K::match`, ... Each is an
ordinary function with an ordinary type, so you can alias it, wrap it, or pass it around freely. Everything in
`crates/lamlet/std/std.lam` is plain Lamlet over `K`, and is worth reading as a tour of idiomatic code.

## 10. Common pitfalls

| You wrote | Problem | Write instead |
|---|---|---|
| `{+ 1 2 3}` | Operators are binary | `{+ {+ 1 2} 3}` or `{fold {' 1 2 3} 0 +}` |
| `{+ 1}` | No partial application | `{^ {x} {+ 1 x}}` |
| `{+ 1 2.0}` | No implicit Int to Float | `{+ {float 1} 2.0}` |
| `[# f {^ {n} {f n}}]` | A plain binding is not recursive | `[# {f n} {f n}]` |
| `[# {g x} {+ {g x} 1}]` to extend an outer `g` | In a function definition, `g` is the new function, so it recurses forever | `[# g {^ {x} {+ {g x} 1}}]` |
| `{if c a b}` | `if` is a macro | `{!if c a b}` |
| `{. x 1 y}` | A label without a value | `{. x 1 y 2}` |
| `$[% x]` where `x` is computed | `x` is a runtime binding | `[# x [% ...]]` |
| `{p.name}` | That calls a function `name` | `{.name p}` |
| `std.map`, `K.add` | Module paths use `::` | `std::map`, `K::add` |
| `{x .f a}` | The space makes `.f` a getter | `{x.f a}` |
| `{!case t ...}` on a `[Type]` value | A recursive type is not its structure | `{!case {unrollT t} ...}` |

## 11. Where to go next

- [spec.md](spec.md): the precise semantics, the primitive list, and the staging rules
- `crates/lamlet/std/std.lam`: the whole standard library, in Lamlet
- `tests/fixtures/*.lam`: small documents that state their expected results
- [host.md](host.md): the contract between Lamlet and a host application
