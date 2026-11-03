# Lamlet diagnostics: what they mean and how to fix them

These messages were captured from the interpreter, so their wording is exact. A diagnostic prints as
`file:line:col: <kind> error: message`, followed by the offending source and any notes. Read the notes: they
often give the fix.

| You wrote | Diagnostic | Fix |
|---|---|---|
| `{+ 1 2 3}` | `type error: this function expects 2 arguments, but 3 were given` | Operators are binary: `{+ {+ 1 2} 3}`, or `{fold xs 0 +}` |
| `{+ 1}` | `type error: this function expects 2 arguments, but 1 were given` | No partial application: `{^ {x} {+ 1 x}}` |
| `{+ 1 2.0}` | ``type error: argument 2: type mismatch: `Int` versus `Float` `` | Convert explicitly: `{+ {float 1} 2.0}` |
| `{map {' 1} +}` | `type error: argument 2: arity mismatch: a 1-argument function versus a 2-argument function` | Pass a function of the right arity: `{map xs {^ {x} {+ x 1}}}` |
| `{{^ {} 1} 2}` | `type error: this function expects 0 arguments, but 1 were given` | Zero-argument functions are called as `{f}` |
| `{' 1 "a"}` | ``type error: argument 2: type mismatch: `Int` versus `String` `` | Lists are homogeneous; use a record or variant to mix |
| `{!if true 1 "s"}` | `type error: both branches must have the same type` | Make both branches the same type |
| `{= id id}` | ``type error: argument 1: `Fn([a], a)` does not support equality`` | Functions and native values have no equality |
| `{.missing {. a 1}}` | ``type error: argument 1: record type `{a: Int}` has no field `missing` `` | Check the label; records are closed |
| `{match {#a 1} {. b ...}}` | ``type error: argument 2: the handlers do not cover exactly the matched alternatives: variant type `<b: a>` has no alternative `#a` `` | Give exactly one handler per alternative |
| `{match {#none unit} {. none {^ {} 0} ...}}` | ``type error: argument 2: handler `none` must be a one-argument function: arity mismatch: a 0-argument function versus a 1-argument function`` | Every handler takes the payload, even Unit: `{^ {u} 0}` (or `{!case o {none u 0} ...}`) |
| `{{^ {f} {pair {f 1} {f "a"}}} id}` | ``type error: argument 1: type mismatch: `Int` versus `String` `` | A lambda parameter has one type (no higher-rank types): let-bind the function, or pass one function per type |
| `{map xs !if}` | ``syntax error: a macro must be applied: write {!if ...}`` | Macros are not values; wrap the use in a lambda |
| `[# λ 1]` | ``syntax error: expected a binding name, or a function head `{name params...}` (an unqualified, unreserved identifier)`` | `λ` is reserved (the lambda head, like `^`); rename the binding |
| A list holding records of different shapes | ``type mismatch: `{age: Int, name: String}` versus `{age: Int, other: Bool}` `` | One element type per list; apply the polymorphic function to each record instead |
| `[# f {^ {n} {f n}}]` | ``name error: unknown name `f` `` | Recursion needs a named lambda: `{^ f {n} {f n}}` |
| `{if c a b}` | ``name error: unknown name `if` ``, noting that `` `!if` is a macro `` | `{!if c a b}` |
| `{p.name}` (to read a field) | ``name error: unknown name `name` ``, noting `` to read a field, write `{.name x}` `` | `{.name p}`; `x.f` calls the *function* `f` |
| `{std.map ...}` (a dot instead of `::`) | ``name error: `std` is a package namespace, not a value (an export is written `std::name`)`` | `std::map` |
| `[# x 1] [# x 2]` in one scope | ``name error: `x` is already bound in this scope`` | Rename the binding, or shadow it in an inner scope |
| `[# b {+ 1 2}] [% b]` | ``stage error: a static expression cannot use the runtime binding `b` `` | Stage the definition: `[# b [% {+ 1 2}]]` |
| `[# T {List Int}] {: xs T}` | ``stage error: a static expression cannot use the runtime binding `T` `` | `[# T [% {List Int}]]`, or write the type in place: `{: xs {List Int}}` |
| A Runtime native inside `[% ...]` | `stage error: ... requires Runtime capabilities ...` | Call it at runtime, or use a Pure or Static native |
| `[f "${str "x"}"]` | ``syntax error: a `$` hole in a template cannot contain a String literal`` | Use a raw template, `f"{${str "x"}}"`, or bind the String first: `[# s "x"] [f "$s"]` |
| `${f"{hi}"}`, or `f"{I am ${age}}"` | ``type error: a value of type String is not callable`` | `${...}` holds a form, and `{x}` calls `x`: write `$f"{hi}"` in prose and `$age` in a hole |
| `f"{text` (no closing `}"`) | ``syntax error: unterminated template: its text ends at the first `}"` outside a `$` hole`` | Close it with `}"`; a literal `}"` in the text is a hole, `${str "}\""}` |
| `{1.+ 2}` | ``syntax error: a method name after `.` must be a word identifier`` | `{+ 1 2}` |
| `{. a}` | `syntax error: a record literal ... needs a value after every label` | `{. a 1}` |
| `{head {'}}` | `runtime error: head of an empty list` | Use `caseList` or check `isEmpty` |
| An infinite loop | `limit error: instruction budget (50000000) exhausted` | Fix the termination; budgets are `Limits` in the host API |
| Deep non-tail recursion | `limit error: call depth limit (200000 frames) exceeded` | Use an accumulator and a tail call, as std's list functions do |
| Expressions nested more than 96 deep | `limit error: expression nesting limit (96) exceeded` | Bind intermediate values with `[# ...]` |

## The diagnostic kinds

| Kind | Raised by |
|---|---|
| `syntax` | scanner, reader, parser |
| `name` | resolution, imports |
| `type` | inference |
| `stage` | static/runtime separation |
| `package` | resolution, cycles |
| `native` | native contracts |
| `static-evaluation` | an error raised while running `[% ]` |
| `runtime` | `error`/`K::fail`, overflow, division by zero |
| `limit` | an exhausted budget |

Checking stops at the first diagnostic in a document.
