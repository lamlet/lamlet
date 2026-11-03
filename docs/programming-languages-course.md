# Introduction to Programming Languages, through Lamlet

This course is for someone who can program but is new to language theory. Its goal is to help you explain,
evaluate, and deliberately change Lamlet's design. It assumes no previous knowledge of lambda calculus,
type theory, or compiler construction.

There are twelve sessions. A suggested pace is one or two sessions per week, with time to work the exercises.
The notes introduce the ideas; deriving rules and trying counterexamples is where most of the learning happens.
This is a guided introduction, rather than a replacement for a full textbook or a formal verification of Lamlet.

The [executable companion](../examples/language-theory.lam) has one section for each session. From the
repository root, run:

```sh
cargo run -q -p lamlet-cli -- examples/language-theory.lam        # the document, each region's value filled in
cargo run -q -p lamlet-cli -- eval examples/language-theory.lam   # each region's value and type, as JSON
```

For inferred types and stage information, run:

```sh
cargo build -p lamlet-cli
./target/debug/lamlet check examples/language-theory.lam
```

In each session: read the explanation, predict the relevant companion results, run it, do the exercise,
and then read the indicated source. You do not need to understand an entire Rust module before moving on.
Keep a design notebook recording a rule, its purpose, an example, and a possible alternative.

## Course map

| Session | Main question | What you will practice |
|---|---|---|
| 1. Syntax and semantics | What does defining a language involve? | Separate structure, checking, and execution |
| 2. Grammars and representations | How does text become a program? | Write a grammar and draw syntax trees |
| 3. Binding and scope | Which definition does a name refer to? | Track environments and avoid variable capture |
| 4. Lambda calculus and evaluation | What does calling a function mean? | Trace evaluation and explain delayed execution |
| 5. Type systems | What can checking establish? | Construct typing derivations |
| 6. Type inference and polymorphism | How can types be discovered? | Solve constraints and instantiate type schemes |
| 7. Data types | How do programs represent combinations and alternatives? | Model data with products, sums, and recursion |
| 8. Recursion and termination | How is repeated computation defined? | Identify tail calls and reason about termination |
| 9. Effects and state | What can happen besides producing a value? | Distinguish values, stores, inputs, and capabilities |
| 10. Macros and staging | What happens before runtime? | Separate syntax transformation from evaluation |
| 11. Language implementation | How do these rules become executable? | Follow closures, bytecode, and VM frames |
| 12. Correctness and design | How do we justify a language change? | Use equivalence, tests, and small proofs |

## 1. Syntax and semantics

A language definition answers at least three questions:

1. **Syntax:** what structures can a program have?
2. **Static semantics:** which programs are accepted before ordinary runtime execution?
3. **Dynamic semantics:** how do accepted programs execute?

Consider this complete Lamlet document:

```text
$[Import* std]
${+ 1 2}
```

The document scanner finds two code regions. The expression region contains an application: the function
named `+` receives two arguments, `1` and `2`. Name resolution finds the imported definition. Type checking
establishes that this application has type `Int`. Evaluation produces the value `3`.

Here are three different things: the source text `{+ 1 2}`, its type `Int`, and its resulting value `3`.
A type describes possible values and uses; it is not the result of running the expression.

We will use two common **judgments**, which are statements justified by rules:

```text
Γ ⊢ e : τ       In type environment Γ, expression e has type τ.
ρ ⊢ e ⇓ v       In value environment ρ, expression e evaluates to value v.
```

Read `Γ` as “gamma,” `ρ` as “rho,” and `τ` as “tau.” A type environment maps names to types or type schemes;
a value environment maps names to values. The symbols are bookkeeping, not additional Lamlet syntax.
The second judgment describes evaluations that finish with a value; errors and divergence need additional
rules or judgments.

A syntactically valid program can fail name resolution or typing. A well-typed program can still divide by
zero, exhaust a budget, or compute an unintended answer. These are different questions.

**Exercise.** Classify each as a syntax, name, type, or runtime issue, assuming `std` is imported:
an unclosed brace; `{missingName 1}`; `{+ 1 true}`; `{div 1 0}`. State what stage can discover each issue.

**Read afterward:** [specification §1](spec.md#1-layers) and the small `Core` enum in
[`core.rs`](../crates/lamlet/src/core.rs).

## 2. Grammars and representations

A grammar describes allowed program shapes. Here is a deliberately small language, written in
**metasyntax**: notation for describing a language. This is not Lamlet code.

```text
e ::= n | x | add(e, e) | lambda(x, e) | app(e, e) | let(x, e, e)
```

`e` stands for an expression, `n` for an integer literal, `x` for a variable name, and `|` separates
alternatives. The grammar gives recursive structure: an addition contains two expressions, which may
themselves contain additions.

An abstract syntax tree (**AST**) stores this structure without irrelevant punctuation. The expression
`{+ 1 {* 2 3}}` has a call at its root, with another call as its second argument. It is not a flat list of
words to execute in textual order.

A lexer recognizes tokens. A parser recognizes structure. Lamlet also has a reader that groups tokens
into brace and bracket trees before parsing. Macro expansion transforms those syntax trees. Resolution
connects uses of names to definitions. Elaboration turns convenient surface forms into a smaller internal
language. These responsibilities may be interleaved in an implementation.

**Syntactic sugar** adds convenient spelling with a defined translation. In Lamlet, `3.neg` means
`{neg 3}`. It introduces no new dispatch rule. Changing braces to parentheses would change concrete syntax;
changing argument evaluation order would change dynamic semantics.

**Exercise.** Draw the AST of `{+ 1 {* 2 3}}`. Then translate `xs.reverse.length` into ordinary nested calls.
Explain why a parser cannot determine whether `xs` has been defined just from expression structure.

**Read afterward:** `Syntax`, `Expr`, and `parse_expr` in
[`syntax.rs`](../crates/lamlet/src/syntax.rs). Keep the surface `Expr` and checked `Core` representations distinct.

## 3. Binding, scope, and closures

A **binding** connects a name to something. Its **scope** is the region of code where that connection is
visible. Under lexical scope, the program's written nesting determines the binding of each variable use.

The companion defines `base` as `10`, then defines `addBase`, then calls `addBase` inside another function
whose parameter is also named `base` and holds `100`. The answer is `15`: the caller's parameter does not
change the meaning of a variable inside a previously defined function.

A variable is **free** in an expression if that expression does not bind it. In the mathematical expression
`λx. x + base`, `x` is bound and `base` is free, assuming `+` is a primitive. A **closure** pairs a function's
code with the environment needed for its free variables. This lets a function keep working after the
function call that created it has returned.

Renaming a bound variable consistently, without capturing another variable, preserves meaning. This is
**alpha equivalence**: `λx.x` and `λy.y` describe the same behavior. But replacing `x` with `y` in `λy.x`
must not produce `λy.y`: that would accidentally capture the formerly free `y`. First rename the binder
to a fresh name, then substitute. This is **capture-avoiding substitution**.

**Exercise.** List the free variables of `λx. λy. add(x, z)`, treating `add` as a primitive. Substitute `y`
for the free `x` in `λy.x`, showing the necessary renaming. Explain which environment a dynamically scoped
language would consult instead when a function accesses a name supplied by its caller.

**Read afterward:** `Core::free_locals` in [`core.rs`](../crates/lamlet/src/core.rs) and
`lambda_application_and_closure_capture` in [`semantics.rs`](../crates/lamlet/tests/semantics.rs).

## 4. Lambda calculus and evaluation strategies

The lambda calculus studies functions through a very small language: variables, function expressions, and
applications. Its notation `λx.e` means a function taking `x` and returning `e`. Lamlet's `{^ {x} e}` is a
related spelling, extended with multiple arguments and other features.

The central computation rule is **beta reduction**: applying a function substitutes its argument into its
body, avoiding capture. An implementation can use closures and environments instead of physically replacing
text. The rule specifies behavior; it does not require a particular implementation technique.

Under **call by value**, evaluate the argument to a value before entering the function body:

```text
(λx. x + 1) (2 + 3) → (λx. x + 1) 5 → 5 + 1 → 6
```

The arrow describes one computation step. This is **small-step semantics**. Session 1's `⇓` relates a whole
terminating evaluation to its answer and is called **big-step semantics**.

**Call by name** delays the argument and evaluates it when used, potentially repeatedly. **Call by need**
also delays it but shares the evaluation result. Neither means the same thing as call by value. Evaluation
strategy matters even without mutable state: an unused argument might fail or run forever.

Lamlet is call by value and evaluates the callee, then arguments left to right. A zero-argument lambda,
called a **thunk**, delays its body. `!if` expands into a call to `K::caseBool` with two thunks, so only the
chosen body executes. An ordinary function receiving already computed branch values cannot undo their
evaluation. An ordinary function receiving thunks can provide conditional behavior.

**Exercise.** Explain why `{!if true 7 {error "boom"}}` returns `7`, while
`{choose true 7 {error "boom"}}` fails. Both are well typed. Would call by name also skip an unused argument
to a function that always returns `7`?

**Read afterward:** [specification §3](spec.md#3-core-calculus), plus `!if` and `choose` in
[`std.lam`](../crates/lamlet/std/std.lam).

## 5. Type systems and soundness

A static type system establishes properties of programs without evaluating all their possible executions.
It can reject an attempt to add a Boolean to an integer while still allowing functions whose inputs are
not yet known.

For a small language with integer-only addition, a typing rule is:

```text
Γ ⊢ e₁ : Int     Γ ⊢ e₂ : Int
──────────────────────────────
      Γ ⊢ add(e₁, e₂) : Int
```

Read the premises above the line as requirements, and the conclusion below as what follows. A **typing
derivation** is a tree of such rules. Lamlet's actual `+` also supports `Float` through numeric constraints;
the simplified rule is for learning how derivations work.

For one-argument functions, the application rule is:

```text
Γ ⊢ f : A → B     Γ ⊢ argument : A
──────────────────────────────────
        Γ ⊢ app(f, argument) : B
```

Here `A → B` is mathematical notation for a function type. Lamlet prints a one-argument function type as
`Fn([A], B)`.

Two common ingredients of a type safety proof are **progress** and **preservation**. Progress says a closed,
well-typed expression is a value or can take an evaluation step. Preservation says a step preserves its type.
Together they rule out getting stuck in states the semantics does not account for. A real language's theorem
must explicitly account for errors, primitives, and effects. It does not automatically establish termination
or the correctness of an implementation. This course does not claim such a proof exists for Lamlet.

**Exercise.** Derive the type of `add(1, add(2, 3))` in the integer-only language. Explain why typing alone
does not establish that an integer division's denominator is nonzero.

**Read afterward:** [specification §4](spec.md#4-types) and `type_checking_happens_before_execution` in
[`semantics.rs`](../crates/lamlet/tests/semantics.rs).

## 6. Type inference and polymorphism

**Type checking** verifies a type. **Type inference** discovers one, often by introducing unknown types and
solving constraints. **Unification** finds a consistent substitution for those unknowns.

For `λx.x`, assign the parameter an unknown type `α`. The body has that same type, so the function has type
`α → α`. When a suitable binding generalizes it, the type scheme becomes `∀α. α → α`: for every type `α`,
this function can accept and return that type. The companion's `identity` is consequently usable at both
`Int` and `Bool`.

Hindley–Milner inference distinguishes **generalization** at a let binding from **instantiation** when that
binding is used. In the basic system, generalize type variables that are free in the inferred type but not
free in the surrounding type environment. At each use, replace the quantified variables with fresh unknowns.
Variables fixed by the environment must remain connected to it.

A lambda parameter is not automatically a polymorphic type scheme. Giving one parameter both
`Int → Int` and `Bool → Bool` uses needs more than ordinary Hindley–Milner parameter typing. Let-bound
polymorphism does not make every function parameter polymorphic.

For `λf.λx.f x`, begin with types `α` for `f` and `β` for `x`. Application requires `α = β → γ`. The result
is `(β → γ) → β → γ`. An **occurs check** rejects equations such as `α = α → β`, which would require an
infinite type in this type language. Lamlet adds numeric, equality, field, variant, and capability constraints
to this foundation; implementing those is more work than basic unification.

**Exercise.** Infer the type of `λf.λx.f (f x)`. Then explain why typing `λx.x x` generates an occurs-check
failure. These expressions use mathematical notation rather than Lamlet's multi-argument calling convention.

**Read afterward:** `Scheme`, `TypeCtx::instantiate`, and `TypeCtx::unify` in
[`types.rs`](../crates/lamlet/src/types.rs).

## 7. Products, sums, and recursive data

A **product type** combines values: a record containing a name and an age has both fields. A **sum type**
chooses an alternative: an option has either no value or one value. These constructions model different
information. Making every field optional often loses relationships that a sum can express directly.

In mathematical notation, an option is `Option(A) = Unit + A`. `Unit` has one value. A list satisfies the
recursive description `List(A) = Unit + (A × List(A))`: either empty or an element together with another
list. These are mathematical descriptions, not Lamlet type-declaration syntax. Lamlet has built-in lists
but does not support arbitrary user-defined recursive types.

Lamlet uses **structural** records and variants. Their structure determines compatibility; two records do
not become different types just because two constructors have different names. A **nominal** system can
make separately declared types distinct even when their fields have the same structure.

Data has ways to be constructed and ways to be consumed. Record access, list cases, and variant matching
are examples of consumers, often called **eliminators**. In Lamlet, `match` receives a record of handlers,
with one handler per variant alternative. An unhandled alternative is a typing problem, rather than an
implicit runtime fallthrough.

Lamlet record literals have closed shapes. A field accessor can still work on several record shapes
through a generalized field constraint. That is a form of polymorphism, not an automatic conversion between
all larger and smaller records.

**Exercise.** Model a request as either pending, successful with a String, or failed with an error String.
Explain why two independent optional fields, `result` and `error`, permit unwanted states. How many values
do `Bool × Bool` and `Bool + Bool` have, and what distinguishes their values?

**Read afterward:** the companion's `describe`, [rationale D3](rationale.md#d3-structural-records-and-labeled-variants),
and `Case` / `Cons` in [`types.rs`](../crates/lamlet/src/types.rs).

## 8. Recursion, tail calls, and termination

Recursion lets a definition refer to itself. This needs a binding rule; it is not a consequence of choosing
a variable name. Lamlet introduces a self binding explicitly with `{^ self {parameters} body}`. A plain
`[# name expression]` does not make `name` visible in its own initializer.

In a **tail call**, the current function has no work left after the called function returns. In
`factorial(n) = n * factorial(n - 1)`, multiplication remains, so the recursive call is not a tail call.
An accumulator can carry that work into the next invocation instead. Lamlet's VM reuses frames for tail
calls. This bounds frame growth, not total allocation or execution time.

Proving termination usually requires a measure that decreases in a well-founded order. For the companion's
`sumDown`, a nonnegative integer `n` decreases toward zero. Negative inputs do not satisfy that argument;
integer overflow and resource limits also need consideration.

The simply typed lambda calculus without general recursion has strong termination properties. Adding
unrestricted recursion changes what can be proved. For a sufficiently expressive general-purpose language,
there is no algorithm that decides termination for every program. A runtime budget solves a different
problem: it bounds how much work the host permits. Exhausting a budget does not prove that the program
would run forever.

Lamlet also gives a recursive function one monomorphic type within its body. Supporting polymorphic
recursion generally requires additional type-system machinery or annotations.

**Exercise.** Turn a recursive list sum into an accumulator-based sum in pseudocode. Mark which calls are
in tail position. Give one terminating computation that could nevertheless exceed an instruction budget.

**Read afterward:** `recursive_functions` / `termination_and_budgets` in
[`semantics.rs`](../crates/lamlet/tests/semantics.rs), then `SelfClosure` in
[`compile.rs`](../crates/lamlet/src/compile.rs).

## 9. Effects, state, and host boundaries

An **effect** is behavior beyond returning a value: modifying state, reading external input, writing output,
raising an error, or performing a nondeterministic action. The exact classification depends on the language
and its analysis. Immutability alone does not rule out every effect.

An immutable record update produces a new record value. Mutation changes a location in a store, potentially
observable through aliases. To describe mutation formally, evaluation needs both an environment and a
**store** mapping locations to current contents. Two names can then refer to the same changing location.
Lamlet has immutable language values, so its ordinary record updates do not introduce this store behavior.

Making inputs explicit can support deterministic replay: evaluate the same program with the same inputs
and obtain the same result. Native functions form a trust boundary. A declaration does not prove that the
host implementation obeys its contract. Determinism depends on those contracts and implementations too.

An **effect system** augments types with information about behavior or permissions. Lamlet propagates
`Static` and `Runtime` native capabilities through function types and uses them to enforce stage rules.
These capabilities describe allowed execution stages; they are not a complete analysis of all possible I/O,
allocation, failure, and termination behavior.

Effects also interact with polymorphism. In languages with mutable references, freely generalizing every
binding can be unsound: one shared cell cannot safely be treated independently as a cell of integers and
a cell of booleans. Restrictions on generalization address this. This is a reason that “add mutable state”
can require changes outside the evaluator.

**Exercise.** Sketch `update : State × Event → State` for a counter with increment and reset events. Explain
how replay differs from calling a hidden global counter. What responsibility remains with a host native
declared to be pure?

**Read afterward:** [`host.md`](host.md), `Availability` in [`syntax.rs`](../crates/lamlet/src/syntax.rs), and
`native_runtime_context_and_capabilities` in [`semantics.rs`](../crates/lamlet/tests/semantics.rs).

## 10. Macros, hygiene, and staging

A **macro** transforms syntax into syntax. A function consumes values and produces values. They operate
on different representations, so neither is an interchangeable replacement for the other.

Lamlet's macro call `{!if c a b}` expands to `{K::caseBool c {^ {} a} {^ {} b}}`. The branch expressions
become bodies of lambdas before ordinary argument evaluation occurs. This explains how the surface control
form works without making every function lazy.

**Hygiene** protects bindings during expansion. Introduced helper names should refer to the definitions
intended by the macro author, and introduced binders should not accidentally capture names in the user's
input. This is the binding problem from session 3 applied to generated syntax.

**Staging** separates when computations execute. Lamlet's `[% e]` checks and evaluates `e` during checking,
then embeds the resulting value. It is evaluation, not a syntax rewrite. Static code cannot depend on an
ordinary runtime parameter that has not been supplied yet.

In Lamlet, `[# a {+ 1 2}]` is a runtime binding, while `[# a [% {+ 1 2}]]` makes the computed result
statically available. The distinction is defined by the language, even if an optimizer could precompute
both additions. Optimization must not silently change which programs the staging rules accept.

Types can also be values without giving the language unrestricted dependent types. Lamlet computes ground
type descriptors; it does not support arbitrary types indexed by runtime values or polymorphic computed
type descriptors containing free type variables.

**Exercise.** Explain why a function parameter cannot be used inside `[% ...]` merely because one caller
passes a literal. Design a macro with an introduced temporary variable and explain a possible capture bug
if expansion were unhygienic. Keep this as pseudocode until you have read Lamlet's macro pattern rules.

**Read afterward:** [rationale D6](rationale.md#d6-explicit-syntactic-staging-types-are-ordinary-data),
[rationale D9](rationale.md#d9-pattern-macros-only), and `macro_hygiene` in
[`semantics.rs`](../crates/lamlet/tests/semantics.rs).

## 11. From semantics to an implementation

A language's rules do not require one particular execution strategy. An implementation might interpret an
AST, compile to bytecode for a virtual machine, generate machine code, or combine techniques. “Compiled”
and “interpreted” describe implementation choices; they are not an exhaustive division of language designs.

Lamlet checks and elaborates source into `Core`, compiles that representation to bytecode, and executes
the bytecode in a VM. Static evaluation also uses the VM during checking, so its pipeline has interactions
that a simple straight-line diagram omits.

**Closure conversion** makes captured variables explicit. For the companion's `makeAdder`, the returned
closure retains `amount`. A bytecode implementation can use local slots, capture slots, and a code pointer
rather than searching a dictionary of source-level variable names on every access.

An explicit VM frame records where to resume a call, its locals, and related execution state. A tail call
can replace the current frame. An iterative VM can manage these frames without using one Rust call frame
per Lamlet call.

Memory management is another implementation choice. Lamlet uses reference-counted values. Other runtimes
may use tracing garbage collection, ownership rules, or regions. A source-level type rule does not by itself
choose a memory manager, and reference counting generally needs a plan for cycles.

**Exercise.** Trace `makeAdder(5)` returning a function, then calling that function with `7`. Draw the code
and captured values. Explain why keeping a pointer to a destroyed stack local would be incorrect.

**Read afterward:** [`implementation.md`](implementation.md), then
[`compile.rs`](../crates/lamlet/src/compile.rs), [`bytecode.rs`](../crates/lamlet/src/bytecode.rs), and
[`vm.rs`](../crates/lamlet/src/vm.rs), following one closure call rather than reading every line.

## 12. Equivalence, evidence, and deliberate design

Two expressions are **observationally equivalent** when the language's allowed observations cannot
distinguish them in the relevant program contexts. First specify those observations: returned values,
errors, effects, and possibly resource behavior. Matching answers on one input is much weaker.

Replacing `{const 7 {error "boom"}}` with `7` would change Lamlet's behavior. The original evaluates the
argument and fails; the replacement returns normally. An optimization justified for a total mathematical
function may fail in a language with errors or nontermination.

Numeric laws also require assumptions. Lamlet's `Int` overflows with an error. Thus
`maximum + (1 + -1)` succeeds, while `(maximum + 1) + -1` fails. Floating-point addition has its own
rounding behavior. “Addition is associative” is not a valid unconditional optimization rule here.

Tests check examples and can expose counterexamples. Property-based tests generate many cases of a stated
property. A proof establishes a claim for all cases covered by its assumptions, through a mathematical
argument. A proof about an idealized core does not automatically verify the parser, native implementations,
or compiler. Each connection needs justification.

For a language change, write down the problem, proposed syntax, static rules, dynamic rules, interaction
with existing features, and compatibility cost. Ask what invariant the change preserves. Lamlet's rationale
contains decisions with costs: exact arity simplifies calls but removes implicit partial application;
explicit staging improves predictability but requires annotations; structural data improves composition
but does not provide nominal abstraction by itself.

**Exercise.** Propose implicit partial application for Lamlet. Specify what `{+ 1}` would mean, how its type
would be inferred, and what happens to too-many-argument calls. Identify a currently rejected program that
would become accepted. Explain the tradeoff before writing implementation code.

**Read afterward:** [`rationale.md`](rationale.md) alongside the corresponding semantic tests. Treat the
document as a set of decisions to understand and evaluate, rather than universal laws of language design.

## Capstone: implement a language small enough to explain

Build a separate teaching interpreter for integers, booleans, variables, one-argument functions,
applications, let bindings, and conditionals. Do not begin by reproducing all of Lamlet.

1. Define its AST and a grammar. Writing AST values directly is sufficient for the first evaluator.
2. Write lexical, call-by-value evaluation rules, including the behavior of errors.
3. Implement an evaluator using environments and closures.
4. Add a simply typed checker with annotations on function parameters.
5. State progress and preservation for a small pure subset; work through the application case with guidance.
6. Replace parameter annotations with unification, then add let generalization and instantiation.
7. Add one data feature, such as options, and update both typing and evaluation.
8. Compare your rules with Lamlet's implementation. Explain three differences and their consequences.

Keep general recursion out of the initial proof exercise; add it as a later extension. A bytecode compiler
is optional. The central deliverable is a specification and implementation that agree, with tests chosen
to distinguish plausible alternative semantics.

## Exercise checkpoints

Try the exercises before reading these. They are checkpoints, not complete derivations.

1. Unclosed brace: syntax; missing name: resolution/name; adding a Bool: type; integer division by zero:
   runtime in an ordinary evaluated region. Explicit staging can move a computation's failure into checking.
2. The outer call's arguments are `1` and the call `(* 2 3)`.
   `xs.reverse.length` expands to `{length {reverse xs}}`. Definitions require a scope/environment.
3. Only `z` is free when `add` is primitive. Rename `λy.x` to `λfresh.x`, then substitute to get `λfresh.y`.
   Dynamic scope consults the active calling environment rather than preserving lexical binding in the same way.
4. The macro puts branches inside thunks; `choose` receives branch values after evaluation. Call by name
   can skip an argument the body never uses.
5. Each literal has type Int; the inner addition has type Int; apply the addition rule again for the root.
   Knowing a denominator's type does not establish its value.
6. `(A → A) → A → A`. Self-application requires one unknown to equal a function type containing itself.
7. Use three tagged alternatives with appropriate payloads. Independent options permit both result and error
   to be present, and can also conflate pending with other empty states. Both Bool constructions have four
   values; products contain two booleans, while sums contain one boolean and a choice of tag.
8. `sum([], acc) = acc`; `sum(x::xs, acc) = sum(xs, acc + x)`. The recursive call is in tail position.
   A finite but sufficiently long list traversal can exceed a budget.
9. Increment maps a count to its successor; reset maps it to zero. Replay supplies the state and events
   explicitly. A native must actually honor its declared behavior; its signature cannot enforce its Rust body.
10. A definition is checked before arbitrary future runtime arguments exist. A macro-introduced binder
    named like a free variable in an operand could accidentally capture that occurrence without hygiene.
11. The returned function has code for adding its argument to a captured `5`. Its environment must remain
    valid independently of the completed `makeAdder` call.
12. A possible rule would make underapplication construct a closure of the remaining arguments, with a
    corresponding function type. Too-many-argument behavior is a separate design decision. This changes
    exact arity and diagnostics; usefulness must be weighed against that cost.

After session 1, a useful first discussion is your classification of the four errors and your explanation
of the difference between a type judgment and an evaluation judgment. Later sessions build on that distinction.
