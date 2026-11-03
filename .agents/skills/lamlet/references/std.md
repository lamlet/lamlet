# std reference (generated from crates/lamlet/std/std.lam — do not edit by hand)

Each entry: name, then its doc (which starts with its type when it has one). `_names` are private and omitted.


## 1. Functions

- `id` — Fn([A], A). Returns its argument unchanged.
- `const` — Fn([A, B], A). Returns x; the second argument is still evaluated.
- `compose` — Fn([Fn([B], C), Fn([A], B)], Fn([A], C)). {compose f g} is the function x -> {f {g x}}.
- `flip` — Fn([Fn([A, B], C)], Fn([B, A], C)). The same function with its two arguments swapped.
- `error` — Fn([String], A). Aborts the whole evaluation with the message; it never returns, so it fits any type.
- `unit` — Unit. The only value of type Unit.
- `debug` — Fn([A], String). A deterministic display of any value, for diagnostics (not for presentation).

## 2. Booleans and control

- `caseBool` — Fn([Bool, Fn([], A), Fn([], A)], A). Calls the first thunk if the condition is true, otherwise the second. The first-class form of !if.
- `!if` — {!if c then else}: evaluates the Bool c, then only the selected branch. Both branches are checked and must have the same type.
- `!and` — {!and e...}: true when every operand is true; evaluates left to right and stops at the first false.
- `!or` — {!or e...}: true when some operand is true; evaluates left to right and stops at the first true.
- `not` — Fn([Bool], Bool). Logical negation.
- `choose` — Fn([Bool, A, A], A). Selects x if b is true, otherwise y. Both are already evaluated: use !if to avoid that.

## 3. Equality and order

- `=` — Eq A: Fn([A, A], Bool). Structural equality; functions and native values have none (a type error).
- `!=` — Eq A: Fn([A, A], Bool). True when the values differ.
- `<` — Ord A: Fn([A, A], Bool). Strictly less. Int, Float, and String (by Unicode scalar) are ordered.
- `>` — Ord A: Fn([A, A], Bool). Strictly greater.
- `<=` — Ord A: Fn([A, A], Bool). Less or equal.
- `>=` — Ord A: Fn([A, A], Bool). Greater or equal.
- `min` — Ord A: Fn([A, A], A). The smaller value (the first, on ties).
- `max` — Ord A: Fn([A, A], A). The larger value (the first, on ties).
- `clamp` — Ord A: Fn([A, A, A], A). x limited to the range from lo to hi.

## 4. Numbers

- `+` — Num N: Fn([N, N], N). Addition (Int overflow is an error; no mixing of Int and Float).
- `-` — Num N: Fn([N, N], N). Subtraction.
- `*` — Num N: Fn([N, N], N). Multiplication.
- `/` — Fn([Float, Float], Float). Division; dividing by zero is an error. Use div for Int.
- `neg` — Num N: Fn([N], N). Negation.
- `abs` — Num N: Fn([N], N). The magnitude; abs of the minimum Int overflows.
- `div` — Fn([Int, Int], Int). Euclidean quotient: a = b*q + r with 0 <= r < abs(b).
- `%` — Fn([Int, Int], Int). Euclidean remainder, always in [0, abs(b)): {% -8 3} is 1.
- `float` — Fn([Int], Float). Converts, rounding large integers to the nearest Float.
- `floor` — Fn([Float], Int). The largest Int not above x.
- `ceil` — Fn([Float], Int). The smallest Int not below x.
- `round` — Fn([Float], Int). The nearest Int, halves away from zero ({round 2.5} is 3).
- `truncate` — Fn([Float], Int). Drops the fraction, rounding toward zero.
- `fixed` — Fn([Float, Int], String). x with exactly n decimal places (0 to 20), correctly rounded, ties to even.
- `sqrt` — Fn([Float], Float). Square root; negative arguments are an error.
- `sin` — Fn([Float], Float). Sine (radians), computed identically on every platform.
- `cos` — Fn([Float], Float). Cosine (radians), computed identically on every platform.
- `exp` — Fn([Float], Float). e to the power x.
- `ln` — Fn([Float], Float). Natural logarithm; the argument must be positive.
- `pi` — Float. The ratio of a circle's circumference to its diameter.
- `pow` — Fn([Float, Float], Float). x to the power y, for x >= 0 (0 to any power is 0).
- `lerp` — Fn([Float, Float, Float], Float). Linear interpolation: a at t = 0, b at t = 1.

## 5. Lists

- `cons` — Fn([A, List(A)], List(A)). Prepends one element.
- `caseList` — Fn([List(A), Fn([], R), Fn([A, List(A)], R)], R). Calls the first handler for the empty list, or the second with the head and tail.
- `fold` — Fn([List(A), B, Fn([B, A], B)], B). Left fold: combines the accumulator with each element, first to last.
- `reverse` — Fn([List(A)], List(A)). The elements in reverse order.
- `foldr` — Fn([List(A), B, Fn([A, B], B)], B). Strict right fold: callbacks run from the last element to the first.
- `map` — Fn([List(A), Fn([A], B)], List(B)). Applies f to each element, in order.
- `filter` — Fn([List(A), Fn([A], Bool)], List(A)). The elements for which p is true, in order.
- `isEmpty` — Fn([List(A)], Bool). True for the empty list.
- `head` — Fn([List(A)], A). The first element; errors on an empty list.
- `tail` — Fn([List(A)], List(A)). Everything after the first element; errors on an empty list.
- `length` — Fn([List(A)], Int). The number of elements.
- `append` — Fn([List(A), List(A)], List(A)). xs followed by ys.
- `flatten` — Fn([List(List(A))], List(A)). Concatenates a list of lists, one level deep.
- `nth` — Fn([List(A), Int], A). The element at a zero-based index; errors when it is out of range.
- `range` — Fn([Int, Int], List(Int)). The Ints from `from` up to, but not including, `to`.
- `take` — Fn([List(A), Int], List(A)). At most the first n elements.
- `drop` — Fn([List(A), Int], List(A)). Everything after the first n elements.
- `any` — Fn([List(A), Fn([A], Bool)], Bool). True when p holds for some element (stops at the first).
- `all` — Fn([List(A), Fn([A], Bool)], Bool). True when p holds for every element (stops at the first failure).
- `zip` — Fn([List(A), List(B)], List({first: A, second: B})). Pairs elements up to the shorter list's length.
- `contains` — Eq A: Fn([List(A), A], Bool). True when some element equals the value.
- `unique` — Eq A: Fn([List(A)], List(A)). Keeps the first occurrence of each value, in order.
- `union` — Eq A: Fn([List(A), List(A)], List(A)). Concatenates two lists and keeps the first occurrence of each value.

## 6. Records, variants, options, and pairs

- `match` — Fn([V, H], R). {match v {. a fa b fb}} calls the handler of v's alternative with its payload. The handlers must cover exactly the variant's alternatives.
- `!case` — {!case v {alt x body} ...}: matches v, binding the payload of each alternative to x in its body. It is {match v {. alt {^ {x} body} ...}}, so the clauses must cover exactly the variant's alternatives. A Unit payload still has a name: {!case o {none u 0} {some x x}}.
- `pair` — Fn([A, B], {first: A, second: B}). A pair is an ordinary record.
- `first` — Fn([{first: A, ...}], A). The first component of a pair (or any record with a `first` field).
- `second` — Fn([{second: B, ...}], B). The second component of a pair.
- `some` — Fn([A], <none: Unit, some: A>). A present optional value.
- `none` — <none: Unit, some: A>. The absent optional value.
- `caseOption` — Fn([<none: Unit, some: A>, Fn([], R), Fn([A], R)], R). Calls ifNone for none, or ifSome with the payload.
- `withDefault` — Fn([<none: Unit, some: A>, A], A). The payload, or the default for none.

## 7. Types

- `Int` — The Type of signed 64-bit integers.
- `Float` — The Type of finite binary64 numbers.
- `Bool` — The Type of true and false.
- `String` — The Type of Unicode scalar sequences.
- `Unit` — The Type whose only value is unit.
- `Type` — The Type of Type values.
- `List` — Fn([Type], Type). The Type of lists of T.
- `Fn` — Fn([List(Type), Type], Type). {Fn {' A B} R} is the Type of functions from A and B to R.
- `Field` — Fn([String, Type], {name: String, type: Type}). A named member, for Struct and Variant.
- `Struct` — Fn([List({name: String, type: Type})], Type). A structural record Type; member order does not matter.
- `Variant` — Fn([List({name: String, type: Type})], Type). A structural variant Type; member order does not matter.
- `Never` — The empty variant: the Type of an expression that never returns.
- `Option` — Fn([Type], Type). The Type of optional values of T.
- `Pair` — Fn([Type, Type], Type). The Type of pairs.

## 8. Strings

- `str` — Scalar A: Fn([A], String). Formats a String, Int, Float, Bool, or Unit (Unit gives the empty String).
- `++` — Fn([String, String], String). Concatenates two Strings.
- `chars` — Fn([String], List(String)). The String's Unicode scalars, each as a one-scalar String.
- `concat` — Fn([List(String)], String). Concatenates a list of Strings.
- `join` — Fn([String, List(String)], String). Concatenates with sep between neighbours.
- `substr` — Fn([String, Int, Int], String). The len scalars starting at index start (zero-based), clipped to the String: {"Hello!".substr 0 3} is "Hel".
- `split` — Fn([String, String], List(String)). The pieces between occurrences of sep, a one-scalar String: {"a,b,,c".split ","} is ["a", "b", "", "c"].
- `format` — Fn([String, List(String)], String). Replaces each {} in the template by the next argument; {{ and }} are literal braces. Too few or too many arguments is an error. For typed holes, prefer a template f"{...}".
- `Latex` — The Type of formulas: a record holding LaTeX source, so hosts can render it as math.
- `latex` — Fn([String], Latex). A formula from its LaTeX source, often a template: {latex f"{\frac{1}{2}}"}.
- `latexSource` — Fn([Latex], String). The LaTeX source of a formula.
- `latexEscape` — Fn([String], String). Text as LaTeX source that typesets it literally, for putting a name or a label into a formula: {latexEscape "a_b"} is "a\_b".

## 9. Assertions, hashing, and host time

- `assertEqual` — Eq A: Fn([A, A], A). Returns a if the values are equal, otherwise fails with both values.
- `!assert` — {!assert a b}: checks during preparation that a equals b and returns a. Both must be statically available.
- `hash` — Eq A: Fn([A], Int). A stable hash: equal values hash equally on every platform and in every run.
- `time` — Fn([], Int), Runtime. The Unix time (seconds) that the host supplies for this evaluation; constant within it.
