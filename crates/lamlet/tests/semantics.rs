//! Semantic tests for the language (docs/spec.md). Each test states one rule of the specification and
//! checks it, including the failure cases. Fixture documents in `tests/fixtures/` complement these.

use lamlet::source::DiagKind;
use lamlet::syntax::Availability;
use lamlet::types::TypeDesc;
use lamlet::value::{display, Value};
use lamlet::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local!(static LOG: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) });

fn registry() -> NativeRegistry {
    let mut r = NativeRegistry::new();
    r.register_fn("example.math/add", vec![TypeDesc::Int, TypeDesc::Int], TypeDesc::Int, Availability::Pure, |_, a| match (&a[0], &a[1]) {
        (Value::Int(x), Value::Int(y)) => x.checked_add(*y).map(Value::Int).ok_or_else(|| "Int overflow".into()),
        _ => Err("bad args".into()),
    })
    .unwrap();
    r.register_fn("example.host/clock", vec![], TypeDesc::Float, Availability::Runtime, |ctx, _| ctx.input("clock")).unwrap();
    r.register_fn("example.test/read", vec![], TypeDesc::Int, Availability::Runtime, |_, _| Ok(Value::Int(3))).unwrap();
    // Records the order in which it is called, then returns its argument.
    r.register_fn("example.test/log", vec![TypeDesc::Int], TypeDesc::Int, Availability::Runtime, |_, a| {
        if let Value::Int(n) = a[0] {
            LOG.with(|l| l.borrow_mut().push(n));
        }
        Ok(a[0].clone())
    })
    .unwrap();
    r.register_fn("example.test/bad", vec![], TypeDesc::Int, Availability::Pure, |_, _| Ok(Value::str("not an int"))).unwrap();
    r.register_fn("example.test/prepared", vec![], TypeDesc::String, Availability::Static, |_, _| Ok(Value::str("prepared"))).unwrap();
    r.register_type("example.ui/widget", "1").unwrap();
    r.register_fn("example.ui/make-widget", vec![TypeDesc::String], TypeDesc::Opaque(Rc::from("example.ui/widget")), Availability::Runtime, |_, a| {
        Ok(host::opaque("example.ui/widget", a[0].clone(), None))
    })
    .unwrap();
    r.register_fn("example.ui/widget-label", vec![TypeDesc::Opaque(Rc::from("example.ui/widget"))], TypeDesc::String, Availability::Runtime, |_, a| {
        match &a[0] {
            Value::Opaque(o) => Ok(o.data.downcast_ref::<Value>().unwrap().clone()),
            _ => Err("bad".into()),
        }
    })
    .unwrap();
    let mut secret = NativeEntry {
        api_id: "example.host/secret".into(),
        version: "1".into(),
        params: vec![],
        ret: TypeDesc::String,
        availability: Availability::Runtime,
        capabilities: vec!["secrets".into()],
        callback: None,
        doc: None,
    };
    secret.callback = Some(Rc::new(|_, _| Ok(Value::str("s3cret"))));
    r.register(secret).unwrap();
    r
}

fn resolver() -> MemoryResolver {
    let mut r = MemoryResolver::new();
    r.add("package-name", "$[Import* std] $[# content \"hello\"] $[# transform {^ {s} {++ s \"!\"}}] $[# _private 1]", Exports::All);
    r.add("first-package", "$[# map {^ {x} x}]", Exports::All);
    r.add("second-package", "$[Import std] $[# map {^ {x} {std::+ x 1}}]", Exports::All);
    r.add("graphics-color", "$[Import* std] $[# rgb {^ {r g b} {+ r {+ g b}}}]", Exports::All);
    r.add("graphics", "$[Import graphics-color] $[ImportAs graphics-color color] $[# draw 1]", Exports::Names(vec!["draw".into(), "color".into()]));
    r.add("graphics-private", "$[ImportAs graphics-color color] $[# draw 1]", Exports::Names(vec!["draw".into()]));
    r.add("cyc-a", "$[Import cyc-b] $[# a 1]", Exports::All);
    r.add("cyc-b", "$[Import cyc-a] $[# b 1]", Exports::All);
    r.add("reexport", "$[Import std] $[# plus std::+]", Exports::Names(vec!["plus".into()]));
    r.add(
        "live",
        "$[Import* std] $[Native read [% {Fn {'} Int}] \"example.test/read\" Runtime] $[# value {read}] $[# twice {^ {} {* 2 value}}]",
        Exports::All,
    );
    r.add("diamond-left", "$[Import live] $[# left live::value]", Exports::All);
    r.add("diamond-right", "$[Import live] $[# right live::value]", Exports::All);
    r
}

fn check(src: &str) -> Result<Document, Failure> {
    check_document("<test>", src, resolver().into_rc(), Rc::new(registry()), &Options::default())
}

/// Evaluate and display each expression region.
fn eval_ctx(src: &str, ctx: HostContext) -> Result<Vec<String>, (DiagKind, String)> {
    let doc = check(src).map_err(|f| (f.diagnostics[0].kind, f.render()))?;
    let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).map_err(|d| (d.kind, d.message))?;
    let out = inst.evaluate(ctx, Limits::default()).map_err(|e| (e.kind, e.to_diagnostic().render(&doc.sources)))?;
    Ok(out.iter().map(|(_, v)| display(v)).collect())
}

fn eval(src: &str) -> Vec<String> {
    match eval_ctx(src, HostContext::default()) {
        Ok(v) => v,
        Err((_, m)) => panic!("unexpected failure:\n{}", m),
    }
}

fn one(src: &str) -> String {
    eval(src).last().cloned().expect("no result")
}

fn fails(src: &str) -> (DiagKind, String) {
    match eval_ctx(src, HostContext::default()) {
        Ok(v) => panic!("expected failure, got {:?}", v),
        Err(e) => e,
    }
}

fn fails_with(src: &str, kind: DiagKind, needle: &str) {
    let (k, m) = fails(src);
    assert_eq!(k, kind, "wrong kind for {}: {}", src, m);
    assert!(m.contains(needle), "message for {} lacks {:?}:\n{}", src, needle, m);
}

/// The order in which `log` was called while evaluating `src`.
fn call_order(src: &str) -> Result<Vec<i64>, (DiagKind, String)> {
    LOG.with(|l| l.borrow_mut().clear());
    eval_ctx(src, HostContext::default())?;
    Ok(LOG.with(|l| l.borrow().clone()))
}

const STD: &str = "$[Import* std]\n";
const LOG_NATIVE: &str = "$[Native log [% {Fn {' Int} Int}] \"example.test/log\" Runtime]\n";

fn s(body: &str) -> String {
    format!("{}{}", STD, body)
}

fn logged(body: &str) -> String {
    format!("{}{}{}", STD, LOG_NATIVE, body)
}

// ---------------------------------------------------------------------------------------------------------
// §3 Core calculus: application, closures, scope

#[test]
fn readme_example() {
    assert_eq!(one(&s("$[# square {^ {x} {* x x}}]\n$[f \"The square of 6 is ${square 6}.\"]")), "\"The square of 6 is 36.\"");
}

#[test]
fn lambda_application_and_closure_capture() {
    // A closure keeps its lexical binding; a caller's same-named parameter does not affect it.
    assert_eq!(one(&s("$[# b 3]\n$[# f {^ {a} {+ a b}}]\n${{^ {b} {f 2}} 1}")), "5");
    // Captured values are fixed when the closure is created.
    assert_eq!(one(&s("$[# make {^ {n} {^ {x} {+ x n}}}] $[# add5 {make 5}] $[# add7 {make 7}] ${+ {add5 0} {add7 0}}")), "12");
    // Explicit currying is nested lambdas; there is no implicit partial application.
    assert_eq!(one(&s("${{{^ {a} {^ {b} {- a b}}} 10} 3}")), "7");
    // A body's declarations bind the rest of the body, in order.
    assert_eq!(one(&s("${{^ {}\n [# x 1]\n [# y {+ x 1}]\n {+ x y}\n}}")), "3");
}

#[test]
fn lexical_shadowing() {
    assert_eq!(one(&s("$[# x 1] ${{^ {x} {+ x 10}} 5} ${{^ {} [# y {+ x 1}] y}}")), "2");
    assert_eq!(one(&s("${{^ {x} {{^ {x} {* x 10}} {+ x 1}}} 1}")), "20");
    // Shadowing a std name inside a scope is allowed and local.
    assert_eq!(eval(&s("${{^ {+} {+ 2 3}} *} ${+ 2 3}")), vec!["6", "5"]);
    // Local declarations generalize (let-polymorphism).
    assert_eq!(one(&s("${{^ {} [# ident {^ {v} v}] {!if {ident true} {ident 1} 2}}}")), "1");
}

#[test]
fn definition_scope_and_ordering() {
    fails_with(&s("${{^ {} [# y 1] [# y 2] y}}"), DiagKind::Name, "already bound");
    fails_with(&s("${{^ {} [# z z] z}}"), DiagKind::Name, "unknown name `z`");
    fails_with(&s("${{^ {} [# y 1] y}} $y"), DiagKind::Name, "unknown name `y`");
    fails_with(&s("$later $[# later 1]"), DiagKind::Name, "unknown name `later`");
    fails_with(&s("$[# x 1] $[# x 2]"), DiagKind::Name, "already bound");
    fails_with(&s("$[# + 1]"), DiagKind::Name, "already bound");
    // A non-recursive binding cannot see itself: recursion needs a named lambda.
    fails_with(&s("$[# f {^ {n} {f n}}]"), DiagKind::Name, "unknown name `f`");
}

#[test]
fn exact_arity() {
    fails_with(&s("${{^ {a b} a} 1}"), DiagKind::Type, "expects 2 arguments, but 1 were given");
    fails_with(&s("${{^ {} 1} 2}"), DiagKind::Type, "expects 0 arguments");
    fails_with(&s("$[# AGE 21] ${AGE}"), DiagKind::Type, "not callable");
    // A function passed where a different arity is expected is a type error, not a runtime surprise.
    fails_with(&s("${map {' 1} +}"), DiagKind::Type, "arity mismatch");
    // Zero-argument functions are distinct from their results.
    assert_eq!(eval(&s("$[# k {^ {} 42}] ${k} ${{const k 0}}")), vec!["42", "42"]);
}

#[test]
fn evaluation_order_is_left_to_right_and_exactly_once() {
    // Each argument, left to right.
    assert_eq!(call_order(&logged("${{^ {a b c} 0} {log 1} {log 2} {log 3}}")).unwrap(), vec![1, 2, 3]);
    // The callee is evaluated before the arguments.
    assert_eq!(call_order(&logged("${{{^ {} {log 1} {^ {y} y}}} {log 2}}")).unwrap_err().0, DiagKind::Syntax);
    assert_eq!(call_order(&logged("${{{^ {x} {^ {y} y}} {log 1}} {log 2}}")).unwrap(), vec![1, 2]);
    // Record fields evaluate in source order, whatever their canonical order.
    assert_eq!(call_order(&logged("${. b {log 1} a {log 2}}")).unwrap(), vec![1, 2]);
    // List literal elements evaluate left to right.
    assert_eq!(call_order(&logged("${' {log 3} {log 1} {log 2}}")).unwrap(), vec![3, 1, 2]);
    // A bound value is evaluated once, however often it is used.
    assert_eq!(call_order(&logged("${{^ {} [# x {log 7}] {+ x x}}}")).unwrap(), vec![7]);
    // An error stops the evaluation: later arguments never run.
    let (k, _) = call_order(&logged("${+ {error \"stop\"} {log 9}}")).unwrap_err();
    assert_eq!(k, DiagKind::Runtime);
    assert!(LOG.with(|l| l.borrow().is_empty()));
}

#[test]
fn conditional_laziness() {
    // Only the selected branch runs; the other is still checked.
    assert_eq!(one(&s("${!if true 7 {error \"unused\"}}")), "7");
    fails_with(&s("${!if false 7 {error \"chosen\"}}"), DiagKind::Runtime, "chosen");
    fails_with(&s("${!if true 7 \"wrong\"}"), DiagKind::Type, "same type");
    fails_with(&s("${!if 1 2 3}"), DiagKind::Type, "Bool");
    assert_eq!(call_order(&logged("${!if true {log 1} {log 2}}")).unwrap(), vec![1]);
    // `!and`/`!or` short-circuit left to right.
    assert_eq!(call_order(&logged("${!and {= {log 1} 1} {= {log 2} 0} {= {log 3} 3}}")).unwrap(), vec![1, 2]);
    assert_eq!(eval(&s("${!and} ${!or} ${!and false {error \"no\"}} ${!or true {error \"no\"}}")), vec!["true", "false", "false", "true"]);
    // `choose` is an ordinary eager function: both operands evaluate.
    assert_eq!(call_order(&logged("${choose true {log 1} {log 2}}")).unwrap(), vec![1, 2]);
    // The first-class eliminator takes thunks and runs one.
    assert_eq!(call_order(&logged("${caseBool false {^ {} {log 1}} {^ {} {log 2}}}")).unwrap(), vec![2]);
}

#[test]
fn recursive_functions() {
    assert_eq!(one(&s("$[# fact {^ f {n} {!if {= n 0} 1 {* n {f {- n 1}}}}}] ${fact 20}")), "2432902008176640000");
    // Tail recursion runs in constant VM frames.
    assert_eq!(one(&s("$[# count {^ loop {n acc} {!if {= n 0} acc {loop {- n 1} {+ acc 1}}}}] ${count 1000000 0}")), "1000000");
    // Non-tail recursion over long lists runs on VM frames, not the host stack.
    assert_eq!(one(&s("${length {map {range 0 50000} {^ {x} x}}}")), "50000");
    // The list library is tail-recursive: lists longer than the frame limit are fine.
    assert_eq!(one(&s("${length {filter {map {range 0 300000} {^ {x} {* x 2}}} {^ {x} {> x 10}}}}")), "299994");
    assert_eq!(one(&s("${fold {take {foldr {range 0 300000} {'} cons} 3} 0 +}")), "3");
    // A recursive lambda's name is visible to inner closures and can be returned.
    assert_eq!(one(&s("$[# countTo {^ next {n limit} {^ {} {!if {>= n limit} n {{next {+ n 1} limit}}}}}] ${{countTo 1 4}}")), "4");
    // Recursion is monomorphic in its own body.
    fails_with(&s("$[# f {^ f {x} {!if true x {f 1}}}] ${f \"s\"}"), DiagKind::Type, "mismatch");
    // The name may not double as a parameter.
    fails_with(&s("${^ f {f} 1}"), DiagKind::Syntax, "both the lambda's name and a parameter");
}

#[test]
fn termination_and_budgets() {
    let (k, m) = eval_ctx(&s("$[# loop {^ loop {n} {+ 1 {loop n}}}] ${loop 1}"), HostContext::default()).unwrap_err();
    assert_eq!(k, DiagKind::Limit, "{}", m);
    assert!(m.contains("call depth"), "{}", m);
    let (k, m) = eval_ctx(&s("$[# spin {^ spin {n} {spin n}}] ${spin 1}"), HostContext::default()).unwrap_err();
    assert_eq!(k, DiagKind::Limit);
    assert!(m.contains("instruction budget"), "{}", m);
    // Static evaluation is budgeted the same way.
    fails_with(&s("$[# spin {^ spin {n} {spin n}}] $[% {spin 1}]"), DiagKind::Limit, "budget");
    // Macro expansion and expression nesting are bounded (diagnostics, not host stack overflows).
    fails_with(&s("$[Macro !loop {{x} {!loop x}}] ${!loop 1}"), DiagKind::Limit, "expansion depth");
    let deep = format!("${}1{}", "{id ".repeat(150), "}".repeat(150));
    fails_with(&s(&deep), DiagKind::Limit, "nesting limit");
    // String size budget.
    let src = s("$[# grow {^ g {s n} {!if {= n 0} s {g {++ s s} {- n 1}}}}] ${length {chars {grow \"ab\" 30}}}");
    fails_with(&src, DiagKind::Limit, "String size limit");
}

// ---------------------------------------------------------------------------------------------------------
// §4 Types

#[test]
fn type_checking_happens_before_execution() {
    // A type error in code that would never run is still an error, and nothing runs.
    assert_eq!(call_order(&logged("${log 1} ${!if false {+ 1 \"x\"} 2}")).unwrap_err().0, DiagKind::Type);
    assert!(LOG.with(|l| l.borrow().is_empty()));
    fails_with(&s("${+ 1 2.0}"), DiagKind::Type, "mismatch");
    fails_with(&s("${' 1 \"a\"}"), DiagKind::Type, "argument 2");
    fails_with(&s("${= {^ {x} x} {^ {x} x}}"), DiagKind::Type, "equality");
    fails_with(&s("${< true false}"), DiagKind::Type, "ordering");
    fails_with(&s("${str {' 1}}"), DiagKind::Type, "scalar");
}

#[test]
fn ascription() {
    assert_eq!(one(&s("$[# addChecked {: {^ {a b} {+ a b}} [% {Fn {' Int Int} Int}]}]\n${addChecked 2 3}")), "5");
    fails_with(&s("${{: {^ {a b} {+ a b}} [% {Fn {' Int Int} Int}]} 2.0 3.0}"), DiagKind::Type, "argument 1");
    fails_with(&s("${: \"wrong\" Int}"), DiagKind::Type, "ascribed type Int");
    // The Type operand must be statically known: ascription never runs code to find a type.
    fails_with(&s("${: 1 {List Int}}"), DiagKind::Stage, "statically known");
    // Ascription cannot erase a capability requirement.
    fails_with(
        &s("$[Native clock [% {Fn {'} Float}] \"example.host/clock\" Runtime] $[# c {: clock [% {Fn {'} Float}]}] $[% {c}]"),
        DiagKind::Stage,
        "Runtime",
    );
}

#[test]
fn records_are_structural() {
    assert_eq!(eval(&s("$[# r {. count 3 label \"items\"}] ${.count r} ${.count {.count= r 4}} $r")), vec![
        "3",
        "4",
        "{count: 3, label: \"items\"}"
    ]);
    // Field access on a not-yet-known type is a row constraint, so generic accessors work on any record.
    assert_eq!(eval(&s("$[# getN {^ {r} {.n r}}] ${getN {. n 1}} ${getN {. n \"s\" m 2}}")), vec!["1", "\"s\""]);
    fails_with(&s("${.missing {. a 1}}"), DiagKind::Type, "has no field `missing`");
    fails_with(&s("${.a= {. a 1} \"x\"}"), DiagKind::Type, "mismatch");
    fails_with(&s("${. a 1 a 2}"), DiagKind::Syntax, "duplicate field");
    fails_with(&s("$[# C [% {Struct {' {Field \"a\" Int}}}]] ${: {. a 1 b 2} C}"), DiagKind::Type, "ascribed type");
    // Equal field maps are equal types and equal values, regardless of order.
    assert_eq!(one(&s("${= {. x 1 y 2} {. y 2 x 1}}")), "true");
}

#[test]
fn variants_and_match() {
    let shape = "$[# area {^ {s} {match s {. circle {^ {r} {* r r}} square {^ {w} {* w w}}}}}]\n";
    assert_eq!(eval(&s(&format!("{}${{area {{#circle 2}}}} ${{area {{#square 3}}}}", shape))), vec!["4", "9"]);
    // The handler record determines the variant: an unknown alternative is a type error.
    fails_with(&s(&format!("{}${{area {{#triangle 1}}}}", shape)), DiagKind::Type, "");
    // Handlers must agree on the result type.
    fails_with(&s("${match {#a 1} {. a {^ {x} x} b {^ {y} \"s\"}}}"), DiagKind::Type, "mismatch");
    // A top-level construction with no other information defaults to the closed variant of what it saw.
    assert_eq!(one(&s("${#hello 1}")), "#hello(1)");
    // Options are ordinary variants.
    assert_eq!(eval(&s("${withDefault {some 4} 0} ${withDefault none 9}")), vec!["4", "9"]);
    fails_with(&s("${withDefault {#maybe 1} 0}"), DiagKind::Type, "");
}

#[test]
fn type_values_are_normalized_data() {
    assert_eq!(
        one(&s("${!assert [% {Struct {' {Field \"a\" Int} {Field \"b\" String}}}] [% {Struct {' {Field \"b\" String} {Field \"a\" Int}}}]}")),
        "{a: Int, b: String}"
    );
    assert_eq!(eval(&s("$[% {Variant {'}}] $[% {Fn {' Int String} Int}] $[% {List {Option {Pair Int Bool}}}]")), vec![
        "Never",
        "Fn([Int, String], Int)",
        "List(<none: Unit, some: {first: Int, second: Bool}>)"
    ]);
    fails_with(&s("$[% {Struct {' {Field \"a\" Int} {Field \"a\" String}}}]"), DiagKind::Static, "duplicate field");
    fails_with(&s("$[% {Struct {' {Field \"not a label\" Int}}}]"), DiagKind::Static, "not a valid field label");
    // Type values are ordinary data at runtime too; only checking positions need static ones.
    assert_eq!(one(&s("${= {List Int} {List Int}}")), "true");
    // A computed Type checks a value of that type.
    assert_eq!(one(&s("$[# Opt [% {Option Int}]] ${withDefault {: {some 3} Opt} 0}")), "3");
}

// ---------------------------------------------------------------------------------------------------------
// Constructibility: primitives behave the same when aliased, wrapped, or passed (task §1.2, spec §8)

#[test]
fn primitive_aliases_keep_their_type() {
    // An alias of a constrained primitive keeps its constraint and generalizes.
    assert_eq!(eval(&s("$[# plus K::add] ${plus 1 2} ${plus 1.5 1.0}")), vec!["3", "2.5"]);
    fails_with(&s("$[# plus K::add] ${plus \"a\" \"b\"}"), DiagKind::Type, "numeric");
    // Aliases through std, packages, and aliases of aliases are the same function.
    assert_eq!(one("$[ImportAs std s] $[# p s::+] $[# q p] ${q {p 1 2} 3}"), "6");
    // Label functions are values too.
    assert_eq!(eval(&s("$[# getX .x] $[# setX .x=] $[# mk #mk] ${getX {setX {. x 1} 2}} ${mk 1}")), vec!["2", "#mk(1)"]);
}

#[test]
fn wrappers_around_primitives_behave_like_the_primitive() {
    // Every primitive has an ordinary type, so a lambda wrapper has the same constrained type as the primitive.
    assert_eq!(one(&s("$[# myAdd {^ {a b} {K::add a b}}] ${myAdd 1.5 {myAdd 1.0 0.5}}")), "3.0");
    fails_with(&s("$[# myAdd {^ {a b} {K::add a b}}] ${myAdd true false}"), DiagKind::Type, "numeric");
    assert_eq!(one(&s("$[# myMatch {^ {v h} {K::match v h}}] ${myMatch {#on 2} {. on {^ {n} {+ n 1}} off {^ {u} 0}}}")), "3");
    fails_with(&s("$[# myMatch {^ {v h} {K::match v h}}] ${myMatch 3 {. a id}}"), DiagKind::Type, "");
    assert_eq!(one(&s("$[# myEq {^ {a b} {K::equal a b}}] ${myEq {' 1} {' 1}}")), "true");
    fails_with(&s("$[# myEq {^ {a b} {K::equal a b}}] ${myEq id id}"), DiagKind::Type, "equality");
    assert_eq!(one(&s("$[# myGet {^ {r} {.label r}}] ${myGet {. label \"x\" other 1}}")), "\"x\"");
    // Effects survive wrapping: a wrapped Runtime native is still Runtime.
    fails_with(
        &s("$[Native read [% {Fn {'} Int}] \"example.test/read\" Runtime] $[# wrapped {^ {} {read}}] $[% {wrapped}]"),
        DiagKind::Stage,
        "Runtime",
    );
}

#[test]
fn higher_order_use_of_primitives() {
    assert_eq!(eval(&s("${fold {' 1 2 3} 0 +} ${map {' 1 4 9} K::toFloat} ${map {' {. x 1} {. x 2}} .x} ${map {' 1 2} #n}")), vec![
        "6",
        "[1.0, 4.0, 9.0]",
        "[1, 2]",
        "[#n(1), #n(2)]"
    ]);
    assert_eq!(one(&s("${length {unique {map {' 1 2 1} hash}}}")), "2");
    assert_eq!(eval(&s("$[# twice {^ {f x} {f {f x}}}] ${twice {^ {n} {+ n 1}} 1} ${twice {^ {t} {++ t t}} \"a\"}")), vec!["3", "\"aaaa\""]);
    assert_eq!(one(&s("${map {' 1 2} {^ {x} {map {' x} {^ {y} {pair x y}}}}}")), "[[{first: 1, second: 1}], [{first: 2, second: 2}]]");
}

#[test]
fn case_bool_inlining_is_by_identity_not_name() {
    // The std macro expands to `K.caseBool` and compiles to a jump.
    let doc = check(&s("${!if true 1 2}")).unwrap();
    let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
    inst.evaluate(HostContext::default(), Limits::default()).unwrap();
    assert!(inst.stats().instructions < 10, "{} instructions", inst.stats().instructions);
    // An alias is the same primitive and behaves identically; a user function named `caseBool` is not
    // inlined and keeps its own meaning.
    assert_eq!(one(&s("$[# cb K::caseBool] ${cb true {^ {} 1} {^ {} {error \"no\"}}}")), "1");
    assert_eq!(call_order(&logged("${{^ {caseBool} {caseBool false {^ {} {log 1}} {^ {} {log 2}}}} {^ {c a b} {a}}}")).unwrap(), vec![1]);
}

#[test]
fn method_call_sugar_is_lexical_application() {
    // `{x.f a}` is exactly `{f x a}`: the same result, type errors, and evaluation order.
    assert_eq!(eval(&s("${\"Hello!\".substr 0 3} ${substr \"Hello!\" 0 3}")), vec!["\"Hel\"", "\"Hel\""]);
    fails_with(&s("${1.substr 0 3}"), DiagKind::Type, "mismatch");
    assert_eq!(call_order(&logged("${{log 1}.max {log 2}}")).unwrap(), vec![1, 2]);
    fails_with(&s("${1.+ 2}"), DiagKind::Syntax, "method name after `.` must be a word");
    // `f` is resolved where it is written, by lexical scope, never by the receiver's type: a local `size`
    // shadows nothing about strings, it simply is the function called.
    assert_eq!(one(&s("${{^ {size} {\"abc\".size}} {^ {t} 99}}")), "99");
    // Wrapping in a lambda keeps the meaning (there is no type-directed dispatch to lose).
    assert_eq!(one(&s("$[# firstTwo {^ {v} {v.take 2}}] ${firstTwo {' 7 8 9}}")), "[7, 8]");
    // Module paths use `::`, so a dot can never be mistaken for a namespace.
    assert_eq!(one("$[Import std] ${\"ab\".std::chars}"), "[\"a\", \"b\"]");
    fails_with(&s("$[# p {. name 1}] ${p.name}"), DiagKind::Name, "to read a field, write `{.name x}`");
}

// ---------------------------------------------------------------------------------------------------------
// §5 Stages

#[test]
fn static_evaluation() {
    let src = s("$[# increment [% {^ {x} {+ x 1}}]]\n$[# prepared [% {increment 4}]]\n${!assert prepared 5}\n${increment 10}");
    assert_eq!(eval(&src), vec!["5", "11"]);
    let doc = check(&src).unwrap();
    assert!(matches!(doc.static_binding("prepared"), Some(Value::Int(5))));
    // Static results are embedded: the runtime does not recompute them.
    let src = s("$[# n [% {length {range 0 1000}}]] ${+ n 1}");
    let doc = check(&src).unwrap();
    let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
    inst.evaluate(HostContext::default(), Limits::default()).unwrap();
    assert!(inst.stats().instructions < 10, "{} instructions", inst.stats().instructions);
}

#[test]
fn static_availability_is_syntactic() {
    // Literals, lambdas over static values, static names, [% ...], and literal forms of static parts are static.
    assert_eq!(eval(&s("$[# a 3] $[# f {^ {x} {+ x a}}] $[# l {' 1 2}] $[# r {. x a}] $[# t [f \"$a\"]] $[% {f {+ {length l} {.x r}}}] $[% t]")), vec![
        "8", "\"3\""
    ]);
    // A computation is runtime unless explicitly staged, even if it could be folded.
    fails_with(&s("$[# b {+ 1 2}] $[% b]"), DiagKind::Stage, "runtime binding `b`");
    assert_eq!(one(&s("$[# b [% {+ 1 2}]] $[% b]")), "3");
    fails_with(&s("$[# v {#tag 1}] $[% v]"), DiagKind::Stage, "runtime binding `v`");
}

#[test]
fn stage_violations() {
    // Runtime arguments are invisible to static code.
    fails_with(&s("$[# f {^ {x} [% {+ x 1}]}]"), DiagKind::Stage, "runtime binding `x`");
    // Runtime natives cannot run statically, directly or through closures and library functions.
    let clock = "$[Native clock [% {Fn {'} Float}] \"example.host/clock\" Runtime]\n";
    fails_with(&s(&format!("{}$[% {{clock}}]", clock)), DiagKind::Stage, "Runtime");
    fails_with(&s(&format!("{}$[% {{map {{' 1 2}} {{^ {{x}} {{clock}}}}}}]", clock)), DiagKind::Stage, "Runtime");
    fails_with(&s(&format!("{}$[# later [% {{^ {{}} {{clock}}}}]] $[% {{later}}]", clock)), DiagKind::Stage, "Runtime");
    // ...but a static closure may hold a Runtime native and be called at runtime.
    let ctx = HostContext::default().with_input("clock", Value::Float(1.0));
    assert_eq!(eval_ctx(&s(&format!("{}$[# later [% {{^ {{}} {{clock}}}}]] ${{later}}", clock)), ctx).unwrap(), vec!["1.0"]);
    // Static natives cannot run at runtime.
    fails_with(&s("$[Native p [% {Fn {'} String}] \"example.test/prepared\" Static] ${p}"), DiagKind::Stage, "Static capabilities");
    assert_eq!(one(&s("$[Native p [% {Fn {'} String}] \"example.test/prepared\" Static] $[% {p}]")), "\"prepared\"");
    // Package bindings initialized at runtime are not static.
    fails_with("$[Import live] $[% live::value]", DiagKind::Stage, "runtime-initialized");
}

#[test]
fn assert_macro() {
    assert_eq!(one(&s("${!assert false false}")), "false");
    fails_with(&s("${!assert 1 2}"), DiagKind::Static, "assertion failed: 1 is not equal to 2");
    fails_with(&s("${!assert 1 \"1\"}"), DiagKind::Type, "type mismatch");
    fails_with(&s("$[Native foreignRead [% {Fn {'} Int}] \"example.test/read\" Runtime]\n${!assert {foreignRead} 3}"), DiagKind::Stage, "Runtime");
    fails_with(&s("$[# x {+ 1 2}] ${!assert x 3}"), DiagKind::Stage, "runtime binding `x`");
}

#[test]
fn stl_standalone_example() {
    let src = s("$[# next\n    [% {: {^ {x} {+ x 1}}\n          [% {Fn {' Int} Int}]}]\n]\n$[# nums [% {map {' 1 2 3} next}]]\n$[# checkedNums {!assert nums {' 2 3 4}}]\n$[# total [% {fold checkedNums 0 +}]]\n$[# checkedTotal {!assert total 9}]\n$[f \"The total is $checkedTotal.\"]");
    assert_eq!(eval(&src), vec!["\"The total is 9.\""]);
}

// ---------------------------------------------------------------------------------------------------------
// Standard library

#[test]
fn arithmetic() {
    assert_eq!(eval(&s("${- 7 10} ${neg 7} ${% -8 3} ${% 8 -3} ${div 7 2} ${div -7 2} ${/ 1.0 4.0} ${float 3} ${abs -4} ${abs -2.5}")), vec![
        "-3", "-7", "1", "2", "3", "-4", "0.25", "3.0", "4", "2.5"
    ]);
    fails_with(&s("${% 4 0}"), DiagKind::Runtime, "division by zero");
    fails_with(&s("${+ 9223372036854775807 1}"), DiagKind::Runtime, "overflow");
    fails_with(&s("${/ 1.0 0.0}"), DiagKind::Runtime, "division by zero");
    assert_eq!(one(&s("${% -9223372036854775808 -1}")), "0");
    fails_with(&s("${div -9223372036854775808 -1}"), DiagKind::Runtime, "overflow");
    // A Float is always a Float, including the results of folds over empty lists.
    assert_eq!(eval(&s("${str {: 0.0 Float}} ${str {fold {'} 0.0 +}}")), vec!["\"0.0\"", "\"0.0\""]);
}

#[test]
fn deterministic_math() {
    assert_eq!(eval(&s("${sin 0.0} ${cos 0.0} ${exp 0.0} ${ln 1.0} ${sqrt 16.0} ${pow 2.0 10.0} ${round {* 1000.0 {sin {/ pi 6.0}}}}")), vec![
        "0.0", "1.0", "1.0", "0.0", "4.0", "1024.0", "500"
    ]);
    fails_with(&s("${ln 0.0}"), DiagKind::Runtime, "nonpositive");
    fails_with(&s("${sqrt -1.0}"), DiagKind::Runtime, "negative");
    fails_with(&s("${exp 1000.0}"), DiagKind::Runtime, "not finite");
}

#[test]
fn lists() {
    assert_eq!(one(&s("${' {+ 1 2} 4}")), "[3, 4]");
    assert_eq!(one(&s("${fold {' 1 2 3} 0 -}")), "-6");
    assert_eq!(one(&s("${union {' 2 1 2} {' 1 3}}")), "[2, 1, 3]");
    assert_eq!(
        eval(&s("${filter {' 1 2 3 4} {^ {x} {= {% x 2} 0}}} ${length {' 1 2 3}} ${reverse {' 1 2 3}} ${append {' 1} {' 2}} ${flatten {' {' 1} {' 2 3}}} ${contains {' 1 2} 2} ${head {' 5 6}} ${tail {' 5 6}} ${isEmpty {'}} ${foldr {' 1 2 3} 0 -} ${unique {' 3 3 1}}")),
        vec!["[2, 4]", "3", "[3, 2, 1]", "[1, 2]", "[1, 2, 3]", "true", "5", "[6]", "true", "2", "[3, 1]"]
    );
    fails_with(&s("${head {'}}"), DiagKind::Runtime, "head of an empty list");
    // Long literals are flat (no nesting limit).
    let long = format!("${{length {{' {}}}}}", (0..2000).map(|i| i.to_string()).collect::<Vec<_>>().join(" "));
    assert_eq!(one(&s(&long)), "2000");
}

#[test]
fn comparisons() {
    assert_eq!(
        eval(&s("${= 1 1} ${< 1 2} ${< 2 1} ${<= 1 1} ${> 3 2} ${>= 2 3} ${!= 1 2} ${min 3 1} ${max 3 1} ${< \"a\" \"b\"} ${= {' 1 2} {' 1 2}} ${clamp 5 0 3}")),
        vec!["true", "true", "false", "true", "true", "false", "true", "1", "3", "true", "true", "3"]
    );
}

#[test]
fn strings_and_formatting() {
    assert_eq!(
        eval(&s("${format \"Price: ${}\" {' {str 50}}} ${format \"{{}} {} {}\" {' {str 1.5} {str true}}} ${str 0.1} ${str -0.0} ${concat {' \"a\" \"b\" \"c\"}} ${concat {'}} ${join \"-\" {' \"x\" \"y\"}}")),
        vec!["\"Price: $50\"", "\"{} 1.5 true\"", "\"0.1\"", "\"0.0\"", "\"abc\"", "\"\"", "\"x-y\""]
    );
    fails_with(&s("${format \"{}\" {'}}"), DiagKind::Runtime, "too few arguments");
    fails_with(&s("${format \"{\" {' \"1\"}}"), DiagKind::Runtime, "unmatched {");
    // Template holes are typed one by one, at compile time.
    assert_eq!(one(&s("$[# n 3] $[# who \"Ann\"] $[# many \"many\"] $[# single \"one\"] $[f \"$who has $n items: ${!if {> n 1} many single}\"]")), "\"Ann has 3 items: many\"");
    fails_with(&s("$[f \"${str \"x\"}\"]"), DiagKind::Syntax, "cannot contain a String literal");
    fails_with(&s("$[# xs {' 1}] $[f \"$xs\"]"), DiagKind::Type, "template hole");
    // Evaluated Strings are data, never code.
    assert_eq!(one(&s("${++ \"$\" \"{+ 1 2}\"}")), "\"${+ 1 2}\"");
}

#[test]
fn latex_templates() {
    assert_eq!(eval(&s("$[# a [latex \"x^2\"]] $[latex \"\\sqrt{$a + ${+ 1 2}}\"] ${latexSource [latex \"$$\\alpha\"]}")), vec![
        "{latex: \"\\\\sqrt{x^2 + 3}\"}",
        "\"$\\\\alpha\""
    ]);
    // Strings inserted into LaTeX are escaped; Latex values are inserted as source.
    assert_eq!(one(&s("$[# label \"a_b\"] ${latexSource [latex \"$label\"]}")), "\"a\\\\_b\"");
    fails_with(&s("$[# xs {' 1}] $[latex \"$xs\"]"), DiagKind::Type, "template hole");
}

#[test]
fn hash_function() {
    assert_eq!(eval(&s("${= {hash \"abc\"} {hash \"abc\"}} ${= {hash {' 1 2}} {hash {' 2 1}}} ${= {hash {. a 1 b 2}} {hash {. b 2 a 1}}}")), vec![
        "true", "false", "true"
    ]);
    // Stable across runs and stages: pinned by the encoding in the contract.
    assert_eq!(one(&s("${hash 1}")), one(&s("$[% {hash 1}]")));
    assert_eq!(eval(&s("${hash {#a {. x 1}}} ${hash Int}")).len(), 2);
    fails_with(&s("${hash {^ {x} x}}"), DiagKind::Type, "equality");
}

#[test]
fn time_function() {
    let ctx = || HostContext::default().with_time(1_700_000_000);
    assert_eq!(eval_ctx(&s("${time} ${= {time} {time}} ${+ {time} 60}"), ctx()).unwrap(), vec!["1700000000", "true", "1700000060"]);
    let (k, m) = eval_ctx(&s("${time}"), HostContext::default()).unwrap_err();
    assert_eq!(k, DiagKind::Native);
    assert!(m.contains("std.time"), "{}", m);
    fails_with(&s("$[% {time}]"), DiagKind::Stage, "Runtime");
    fails_with(&s("$[# t {time}] ${!assert t 1}"), DiagKind::Stage, "runtime binding");
    assert_eq!(eval_ctx(&s("$[# later [% {^ {} {time}}]] ${later}"), ctx()).unwrap(), vec!["1700000000"]);
    let doc = check(&s("${+ 1 2}")).unwrap();
    assert!(Instance::new(doc.program.clone(), Rc::new(NativeRegistry::empty())).is_err());
}

// ---------------------------------------------------------------------------------------------------------
// Document source convention

#[test]
fn document_convention() {
    let src = "This paragraph explains the program and is a comment to Lamlet.\n\n$[Import* std]\n$[# AGE 21]\n\nHi! I am Jerrimy. I am $AGE years old.\nI am${!if {>= AGE 21} \" eligible to drink.\" \" not eligible to drink.\"}\n";
    assert_eq!(eval(src), vec!["21", "\" eligible to drink.\""]);
    let stat = "$[Import* std]\n$[# AGE [% 21]]\nI am $[% AGE] years old.\nI am$[% {!if {>= AGE 21} \" eligible.\" \" not eligible.\"}]";
    let doc = check(stat).unwrap();
    let prepared = doc.prepared();
    let values: Vec<String> = prepared
        .iter()
        .filter_map(|p| match p {
            PreparedSpan::Value { value, .. } => Some(display(value)),
            _ => None,
        })
        .collect();
    assert_eq!(values, vec!["21", "\" eligible.\""]);
    assert!(prepared.iter().any(|p| matches!(p, PreparedSpan::Text { text, .. } if text.contains("years old."))));
}

#[test]
fn escapes_and_comments() {
    let src = "$[Import* std]\nThe example $${+ 1 2} is literal. $$[Import std] too. Unprefixed {+ 1 2} and [# x 3]. Costs $50 or $ {q}.\n```\n${+ 1 2}\n```";
    assert_eq!(eval(src), vec!["3"]);
    let doc = check(src).unwrap();
    match &doc.prepared()[1] {
        PreparedSpan::Text { text, .. } => assert!(text.contains("The example ${+ 1 2} is literal. $[Import std] too.")),
        _ => panic!(),
    }
}

#[test]
fn source_errors() {
    // Former comment spellings must not silently discard code.
    for old_comment in ["${+ 1 ~{ignored} 2}", "${+ 1 ~[ignored] 2}", "${+ 1 2 ; old comment}"] {
        assert!(check(&s(old_comment)).is_err(), "legacy comment was accepted: {}", old_comment);
    }
    fails_with(&s("$unknown"), DiagKind::Name, "unknown name `unknown`");
    fails_with(&s("${}"), DiagKind::Syntax, "empty application");
    fails_with(&s("${+ 1 2"), DiagKind::Syntax, "unterminated");
    fails_with(&s("$[Frob 1]"), DiagKind::Syntax, "unknown bracket form");
    fails_with(&s("${f [# x 1]}"), DiagKind::Syntax, "declaration");
    fails_with(&s("${{^ {x} 1 2} 3}"), DiagKind::Syntax, "exactly one result");
    fails_with(&s("${^ {x x} x}"), DiagKind::Name, "duplicate parameter");
    fails_with(&s("${+ 1 $x}"), DiagKind::Syntax, "`$` does not open");
    fails_with(&s("$[# ' 1]"), DiagKind::Syntax, "binding name");
    fails_with(&s("${. a}"), DiagKind::Syntax, "a value after every label");
    fails_with(&s("${if true 1 2}"), DiagKind::Name, "`!if` is a macro");
}

#[test]
fn source_positions_survive_elaboration() {
    let f = match check("line one\n$[Import* std]\n  ${+ 1 \"x\"}") {
        Err(f) => f,
        Ok(_) => panic!(),
    };
    let loc = f.sources.location(f.diagnostics[0].span.unwrap());
    assert_eq!((loc.line, loc.column), (3, 9));
    // Errors inside an expansion point at the template and note the use site.
    let f = match check(&s("$[Macro !bad {{x} {+ x \"s\"}}]\n${!bad 1}")) {
        Err(f) => f,
        Ok(_) => panic!(),
    };
    let text = f.render();
    assert!(text.contains("<test>:2:24") && text.contains("in the expansion of `!bad` (at <test>:3:2)"), "{}", text);
    // Runtime errors carry the call stack with source positions.
    let (_, m) = fails(&s("$[# inner {^ {} [# x {error \"boom\"}] x}]\n$[# outer {^ {} [# y {inner}] y}]\n${outer}"));
    assert!(m.contains("boom") && m.contains("in inner") && m.contains("in outer"), "{}", m);
    // Package diagnostics locate the package source and the import.
    let mut r = resolver();
    r.add("broken", "$[Import* std] $[# bad {+ 1 \"x\"}]", Exports::All);
    let f = match check_document("<t>", "$[Import broken]", r.into_rc(), Rc::new(registry()), &Options::default()) {
        Err(f) => f,
        Ok(_) => panic!(),
    };
    let text = f.render();
    assert!(text.contains("<broken>:1:") && text.contains("while checking package `broken`"), "{}", text);
}

// ---------------------------------------------------------------------------------------------------------
// Packages

#[test]
fn import_forms() {
    assert_eq!(one("$[Import std]\n${std::+ 1 2}"), "3");
    assert_eq!(one("$[Import* std]\n${+ 1 2}"), "3");
    assert_eq!(one("$[ImportAs std s]\n${s::+ 1 2}"), "3");
    fails_with("$[ImportAs std s]\n${std::+ 1 2}", DiagKind::Name, "unknown name `std`");
    fails_with("$[Import std]\n${+ 1 2}", DiagKind::Name, "unknown name `+`");
    assert_eq!(one("$[Import package-name]\n${package-name::transform package-name::content}"), "\"hello!\"");
    assert_eq!(one("$[Import* package-name]\n${transform content}"), "\"hello!\"");
    assert_eq!(one("$[ImportAs package-name pn]\n${pn::transform pn::content}"), "\"hello!\"");
    assert_eq!(one("$[ImportAs package-name pn]\nText $pn::content. more"), "\"hello\"");
    fails_with("$[ImportAs package-name pn]\n${pn::content}", DiagKind::Type, "not callable");
}

#[test]
fn package_visibility_identity_and_conflicts() {
    fails_with("$[Import package-name]\n$package-name::_private", DiagKind::Name, "no public export");
    fails_with("$[Import* first-package]\n$[Import* second-package]", DiagKind::Name, "import conflict");
    assert_eq!(one("$[Import first-package] $[Import second-package] ${second-package::map {first-package::map 1}}"), "2");
    // Repeated imports of the same binding are idempotent; aliases preserve identity.
    assert_eq!(one("$[Import* std] $[Import* std] $[ImportAs std s] $[ImportAs std t] ${s::+ {t::+ 1 1} 1}"), "3");
    fails_with("$[Import* std] $[# map 1]", DiagKind::Name, "already bound");
    assert_eq!(one("$[Import* std] ${{^ {} [# map 1] map}}"), "1");
    fails_with("$[Import nope]", DiagKind::Package, "cannot be resolved");
    fails_with("$[Import cyc-a]", DiagKind::Package, "import cycle: cyc-a -> cyc-b -> cyc-a");
    fails_with("$[# std 1] $[Import std]", DiagKind::Name, "import conflict");
}

#[test]
fn submodules_and_reexports() {
    assert_eq!(one("$[Import graphics] ${graphics::color::rgb 1 2 3}"), "6");
    assert_eq!(one("$[Import* graphics] ${color::rgb 1 2 3}"), "6");
    assert_eq!(one("$[ImportAs graphics g] ${g::color::rgb 1 2 3}"), "6");
    fails_with("$[Import graphics-private] ${graphics-private::color::rgb 1 2 3}", DiagKind::Name, "no public export `color`");
    assert_eq!(one("$[Import reexport] ${reexport::plus 1 2}"), "3");
}

#[test]
fn package_runtime_initialization_once() {
    let doc = check("$[Import* std] $[Import diamond-left] $[Import diamond-right] $[Import* live] ${+ diamond-left::left {+ diamond-right::right {twice}}}").unwrap();
    let names: Vec<&str> = doc.program.package_inits.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["live", "diamond-left", "diamond-right"]);
    let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
    let out = inst.evaluate(HostContext::default(), Limits::default()).unwrap();
    assert!(matches!(out[0].1, Value::Int(12)));
}

#[test]
fn core_package_k() {
    assert_eq!(one("${K::add 1 2}"), "3");
    assert_eq!(one("${K::caseBool true {^ {} 7} {^ {} 8}}"), "7");
    fails_with(&s("${std::K::add 1 2}"), DiagKind::Name, "no public export `K`");
    assert_eq!(one("$[Import* K] ${add 1 {multiply 2 3}}"), "7");
    assert_eq!(one("$[ImportAs K P] $[Import K] ${P::add {K::add 1 2} 3}"), "6");
    // The default import is an enclosing scope: a module may shadow K, and `core` is an ordinary user name.
    assert_eq!(eval("$[# K 3] $K ${{^ {} [# K \"local\"] K}}"), vec!["3", "\"local\""]);
    let mut r = resolver();
    r.add("core", "$[# answer 42]", Exports::All);
    r.add("K", "$[# add 1]", Exports::All);
    let doc = check_document("<t>", "$[Import core] $core::answer ${K::add 1 2}", r.into_rc(), Rc::new(registry()), &Options::default()).unwrap();
    let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
    let out = inst.evaluate(HostContext::default(), Limits::default()).unwrap();
    assert_eq!(format!("{:?} {:?}", out[0].1, out[1].1), "42 3");
    assert!(doc.dependencies.iter().any(|(n, v, _)| n == "K" && v == LANGUAGE_VERSION));
}

// ---------------------------------------------------------------------------------------------------------
// Macros

#[test]
fn user_macros() {
    let src = s("$[Macro !unless {{c then else} {!if c else then}}]\n$[# unless 5]\n${!unless false \"ran\" {error \"skipped\"}} $unless");
    assert_eq!(eval(&src), vec!["\"ran\"", "5"]);
    let src = s("$[Macro !sum {{} 0} {{x rest ...} {+ x {!sum rest ...}}}]\n$[Macro !pairs {{{k v} ...} {' {pair k v} ...}}]\n$[Macro !tag {{\"int\" x} {+ x 0}} {{_ x} x}]\n${!sum 1 2 3} ${!pairs {1 \"a\"} {2 \"b\"}} ${!tag \"int\" 4} ${!tag \"str\" \"s\"}");
    assert_eq!(eval(&src), vec!["6", "[{first: 1, second: \"a\"}, {first: 2, second: \"b\"}]", "4", "\"s\""]);
    fails_with(&s("$[Macro !two {{a b} a}] ${!two 1}"), DiagKind::Syntax, "no clause of `!two` matches");
    fails_with(&s("$[Macro !bad {{x} {+ x \"s\"}}] ${!bad 1}"), DiagKind::Type, "in the expansion of `!bad`");
    assert_eq!(one(&s("${{^ {} [Macro !twice {{e} {+ e e}}] {!twice 21}}}")), "42");
    fails_with(&s("${{^ {} [Macro !twice {{e} {+ e e}}] 1}} ${!twice 1}"), DiagKind::Name, "unknown macro");
    fails_with(&s("$[Macro !m {{x} x}] $[Macro !m {{x} x}]"), DiagKind::Name, "already bound");
    // A macro is not a value.
    fails_with(&s("${map {' 1} !if}"), DiagKind::Syntax, "a macro must be applied");
    // Labels in templates are syntax, not names: hygiene does not rename them.
    assert_eq!(one(&s("$[Macro !point {{a b} {. x a y b}}] ${.y {!point 1 2}}")), "2");
}

#[test]
fn macro_hygiene() {
    let src = s("$[Macro !swapSum {{a b} {{^ {tmp} {+ tmp a}} b}}]\n$[# tmp 100]\n${!swapSum tmp 1}");
    assert_eq!(one(&src), "101");
    let src = s("$[Macro !plusOne {{x} {+ x 1}}]\n${{^ {+} {!plusOne 1}} 0} ${{^ {K} {!if true 1 2}} 0}");
    assert_eq!(eval(&src), vec!["2", "1"]);
    let mut r = resolver();
    r.add("lib", "$[Import* std] $[# _secret 41] $[Macro !answer {{} {+ _secret 1}}]", Exports::All);
    let doc = check_document("<t>", "$[Import* lib] ${!answer}", r.into_rc(), Rc::new(registry()), &Options::default()).unwrap();
    let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
    assert!(matches!(inst.evaluate(HostContext::default(), Limits::default()).unwrap()[0].1, Value::Int(42)));
    assert_eq!(one(&s("$[Macro !first {{a b} a}] ${{^ {a b} {!first b a}} 1 2}")), "2");
    // A recursive lambda introduced by a template cannot capture a user operand of the same name.
    assert_eq!(one(&s("$[Macro !rep {{e} {{^ go {n} {!if {= n 0} e {go {- n 1}}}} 3}}] $[# go 9] ${!rep go}")), "9");
}

#[test]
fn package_macros() {
    let mut r = resolver();
    r.add("ctl", "$[Import std] $[Macro !_helper {{x} x}] $[Macro !unless {{c a b} {!std::if c b a}}]", Exports::All);
    r.add("my-lib", "$[Import ctl] $[ImportAs ctl tools] $[# n 1]", Exports::Names(vec!["tools".into(), "n".into()]));
    let run = |src: &str| {
        let doc = check_document("<t>", src, r.clone().into_rc(), Rc::new(registry()), &Options::default()).map_err(|f| f.render())?;
        let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
        Ok::<_, String>(format!("{:?}", inst.evaluate(HostContext::default(), Limits::default()).unwrap()[0].1))
    };
    assert_eq!(run("$[Import ctl] ${!ctl::unless false 1 2}").unwrap(), "1");
    assert_eq!(run("$[Import* ctl] ${!unless true 1 2}").unwrap(), "2");
    assert_eq!(run("$[ImportAs ctl c] ${!c::unless false 1 2}").unwrap(), "1");
    assert_eq!(run("$[Import my-lib] ${!my-lib::tools::unless false 1 2}").unwrap(), "1");
    assert!(run("$[Import ctl] ${!ctl::_helper 1}").unwrap_err().contains("unknown macro `!ctl::_helper`"));
}

// ---------------------------------------------------------------------------------------------------------
// §9 Natives

#[test]
fn native_contracts() {
    let add = "$[Native addNative [% {Fn {' Int Int} Int}] \"example.math/add\" Pure]";
    assert_eq!(one(&s(&format!("{}\n${{!assert {{addNative 2 3}} 5}}\n${{addNative 1 1}}", add))), "2");
    fails_with(&s(&format!("{} ${{addNative 9223372036854775807 1}}", add)), DiagKind::Native, "overflow");
    fails_with(&s("$[Native addNative [% {Fn {' Int Int} Int}] \"example.math/add\" Runtime]"), DiagKind::Native, "cannot change it");
    fails_with(&s("$[Native addNative [% {Fn {' Int} Int}] \"example.math/add\" Pure]"), DiagKind::Native, "registered as");
    fails_with(&s("$[Native b [% {Fn {'} Int}] \"example.test/bad\" Pure] ${b}"), DiagKind::Native, "does not conform");
    fails_with(&s("$[Native addNative Int \"example.math/add\" Pure]"), DiagKind::Native, "function type");
    // Interface-only checking accepts an unregistered stub; instantiation requires it.
    let doc = check(&s("$[Native later [% {Fn {'} Int}] \"example.missing/x\" Runtime] ${later}")).unwrap();
    assert!(Instance::new(doc.program.clone(), Rc::new(registry())).is_err());
    let linked = check_document(
        "<t>",
        &s("$[Native later [% {Fn {'} Int}] \"example.missing/x\" Runtime]"),
        resolver().into_rc(),
        Rc::new(registry()),
        &Options { require_linked: true, ..Options::default() },
    );
    assert!(linked.is_err());
    let iface = Rc::new(registry().interface_only());
    let doc = check_document("<t>", &s(&format!("{} ${{addNative 1 2}}", add)), resolver().into_rc(), iface.clone(), &Options::default()).unwrap();
    assert!(Instance::new(doc.program.clone(), iface).is_err());
    assert!(Instance::new(doc.program.clone(), Rc::new(registry())).is_ok());
    // A wrapper keeps the native's contract.
    fails_with(&s(&format!("{} $[# w {{^ {{a}} {{addNative a 1}}}}] ${{w 1.5}}", add)), DiagKind::Type, "mismatch");
}

#[test]
fn native_runtime_context_and_capabilities() {
    let src = s("$[Native clock [% {Fn {'} Float}] \"example.host/clock\" Runtime]\n${+ {clock} 1.0}");
    let ctx = HostContext::default().with_input("clock", Value::Float(2.5));
    assert_eq!(eval_ctx(&src, ctx).unwrap(), vec!["3.5"]);
    let (k, m) = eval_ctx(&src, HostContext::default()).unwrap_err();
    assert_eq!(k, DiagKind::Native);
    assert!(m.contains("not provided"));
    let src = s("$[Native secret [% {Fn {'} String}] \"example.host/secret\" Runtime] ${secret}");
    fails_with(&src, DiagKind::Native, "capability");
    assert_eq!(eval_ctx(&src, HostContext::default().grant("secrets")).unwrap(), vec!["\"s3cret\""]);
}

#[test]
fn opaque_native_types() {
    let src = s("$[NativeType Widget \"example.ui/widget\"]\n$[Native makeWidget [% {Fn {' String} Widget}] \"example.ui/make-widget\" Runtime]\n$[Native widgetLabel [% {Fn {' Widget} String}] \"example.ui/widget-label\" Runtime]\n${widgetLabel {makeWidget \"OK\"}}");
    assert_eq!(eval(&src), vec!["\"OK\""]);
    fails_with(
        &s("$[NativeType Widget \"example.ui/widget\"] $[Native mk [% {Fn {' String} Widget}] \"example.ui/make-widget\" Runtime] ${= {mk \"a\"} {mk \"a\"}}"),
        DiagKind::Type,
        "equality",
    );
}

#[test]
fn opaque_handles_are_released() {
    thread_local!(static LIVE: Cell<i64> = const { Cell::new(0) });
    let mut r = registry();
    r.register_type("example.res/handle", "1").unwrap();
    r.register_fn("example.res/open", vec![], TypeDesc::Opaque(Rc::from("example.res/handle")), Availability::Runtime, |_, _| {
        LIVE.with(|l| l.set(l.get() + 1));
        Ok(host::opaque("example.res/handle", (), Some(Box::new(|_| LIVE.with(|l| l.set(l.get() - 1))))))
    })
    .unwrap();
    let r = Rc::new(r);
    let src = s("$[NativeType H \"example.res/handle\"] $[Native open [% {Fn {'} H}] \"example.res/open\" Runtime] $[# h {open}] ${length {' h h {open}}}");
    let doc = check_document("<t>", &src, resolver().into_rc(), r.clone(), &Options::default()).unwrap();
    let mut inst = Instance::new(doc.program.clone(), r).unwrap();
    for _ in 0..3 {
        inst.evaluate(HostContext::default(), Limits::default()).unwrap();
    }
    inst.reset();
    drop(inst);
    assert_eq!(LIVE.with(|l| l.get()), 0);
}

// ---------------------------------------------------------------------------------------------------------
// Host API

#[test]
fn instances_share_programs_and_recover() {
    let src = s("$[Native clock [% {Fn {'} Float}] \"example.host/clock\" Runtime]\n$[# double {^ {x} {* 2.0 x}}]\n${double {clock}}");
    let doc = check(&src).unwrap();
    let reg = Rc::new(registry());
    let mut a = Instance::new(doc.program.clone(), reg.clone()).unwrap();
    let mut b = Instance::new(doc.program.clone(), reg).unwrap();
    let ctx = |t: f64| HostContext::default().with_input("clock", Value::Float(t));
    assert!(a.evaluate(HostContext::default(), Limits::default()).is_err());
    for t in [3.0, 1.0, 2.0] {
        let ra = a.evaluate(ctx(t), Limits::default()).unwrap();
        let rb = b.evaluate(ctx(t), Limits::default()).unwrap();
        assert_eq!(format!("{:?}", ra[0].1), format!("{:?}", rb[0].1));
        assert_eq!(format!("{:?}", ra[0].1), format!("{:?}", Value::Float(2.0 * t)));
    }
    let double = doc.static_binding("double").unwrap().clone();
    let r = a.call(&double, vec![Value::Float(4.0)], HostContext::default(), Limits::default()).unwrap();
    assert!(matches!(r, Value::Float(f) if f == 8.0));
    let tight = Limits { max_instructions: 3, ..Limits::default() };
    assert!(a.evaluate(ctx(1.0), tight).is_err());
    assert!(a.evaluate(ctx(1.0), Limits::default()).is_ok());
}

#[test]
fn fingerprints_track_dependencies() {
    let a = check(&s("${+ 1 2}")).unwrap();
    let b = check(&s("${+ 1 3}")).unwrap();
    let c = check(&s("${+ 1 2}")).unwrap();
    assert_ne!(a.program.fingerprint, b.program.fingerprint);
    assert_eq!(a.program.fingerprint, c.program.fingerprint);
    let mut r2 = registry();
    r2.register_fn("example.extra/x", vec![], TypeDesc::Int, Availability::Pure, |_, _| Ok(Value::Int(1))).unwrap();
    let d = check_document("<test>", &s("${+ 1 2}"), resolver().into_rc(), Rc::new(r2), &Options::default()).unwrap();
    assert_ne!(a.program.fingerprint, d.program.fingerprint);
}

#[test]
fn prose_only_and_empty_documents() {
    for src in ["", "Just prose, with $$ and $5 and {braces}.", "$$"] {
        let doc = check(src).unwrap();
        assert!(doc.regions.is_empty());
        let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
        assert!(inst.evaluate(HostContext::default(), Limits::default()).unwrap().is_empty());
    }
    let doc = check("a $$ b").unwrap();
    match &doc.prepared()[0] {
        PreparedSpan::Text { text, span } => {
            assert_eq!(text, "a $ b");
            assert_eq!((span.start, span.end), (0, 6));
        }
        _ => panic!(),
    }
}

#[test]
fn region_ids_are_stable_and_distinct() {
    let src = s("${+ 1 2} ${+ 1 2}");
    let a = check(&src).unwrap();
    let b = check(&src).unwrap();
    let ids: Vec<&String> = a.regions.iter().map(|r| &r.stable_id).collect();
    assert_eq!(ids, b.regions.iter().map(|r| &r.stable_id).collect::<Vec<_>>());
    assert_ne!(ids[1], ids[2]);
}

#[test]
fn cancellation_and_reset() {
    let doc = check(&s("$[# spin {^ spin {n} {spin n}}] ${spin 0}")).unwrap();
    let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
    let flag = inst.cancel_flag();
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    let e = inst.evaluate(HostContext::default(), Limits::default()).unwrap_err();
    assert!(e.message.contains("cancelled"), "{}", e.message);
    let doc = check("$[Import live] ${live::twice}").unwrap();
    let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
    for _ in 0..2 {
        assert!(matches!(inst.evaluate(HostContext::default(), Limits::default()).unwrap()[0].1, Value::Int(6)));
        inst.reset();
    }
}

#[test]
fn host_calls_are_checked_at_runtime() {
    let doc = check(&s("$[# inc {^ {x} {+ x 1}}] $[# n 3]")).unwrap();
    let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
    let inc = doc.static_binding("inc").unwrap().clone();
    let n = doc.static_binding("n").unwrap().clone();
    assert!(inst.call(&inc, vec![], HostContext::default(), Limits::default()).unwrap_err().message.contains("expects 1 arguments"));
    assert!(inst.call(&n, vec![], HostContext::default(), Limits::default()).unwrap_err().message.contains("not callable"));
    assert!(inst.call(&inc, vec![Value::str("x")], HostContext::default(), Limits::default()).is_err());
    assert!(matches!(inst.call(&inc, vec![Value::Int(1)], HostContext::default(), Limits::default()).unwrap(), Value::Int(2)));
}

// ---------------------------------------------------------------------------------------------------------
// Doc comments

#[test]
fn doc_comments() {
    let mut r = resolver();
    r.add("documented", "$[Import std]\nAdds one.\n$[# inc {^ {x} {std::+ x 1}}]\nSkips.\n$[Macro !skip {{x} 0}]", Exports::All);
    let src = "$[Import* std]\n$[Import* documented]\n\nSquares a number;\nused by the report below.\n$[# square {^ {x} {* x x}}]\n\n$;[Doubles its input.]\n$[# double {^ {x} {* 2 x}}]\n\nNot a doc: a blank line follows.\n\n$[# plain 1]\nTotal: $plain more text\n$[# cont 2]\n${{^ {x} ;[Local doc.] [# d {* 2 x}] d} 1}\nA native.\n$[Native addN [% {Fn {' Int Int} Int}] \"example.math/add\" Pure]";
    let doc = check_document("<t>", src, r.into_rc(), Rc::new(registry()), &Options::default()).unwrap_or_else(|f| panic!("{}", f.render()));
    assert_eq!(doc.docs.get("square").map(String::as_str), Some("Squares a number;\nused by the report below."));
    assert_eq!(doc.docs.get("double").map(String::as_str), Some("Doubles its input."));
    assert_eq!(doc.docs.get("inc").map(String::as_str), Some("Adds one."));
    assert_eq!(doc.docs.get("!skip").map(String::as_str), Some("Skips."));
    assert_eq!(doc.docs.get("addN").map(String::as_str), Some("A native."));
    assert!(!doc.docs.contains_key("plain") && !doc.docs.contains_key("cont"));
    let sq = doc.regions.iter().find(|r| r.declares.as_deref() == Some("square")).unwrap();
    assert!(sq.is_declaration && sq.doc.as_deref().unwrap().starts_with("Squares"));
    assert!(doc.prepared().iter().any(|p| matches!(p, PreparedSpan::Comment { text, .. } if text == "Doubles its input.")));
    let mut inst = Instance::new(doc.program.clone(), Rc::new(registry())).unwrap();
    let out = inst.evaluate(HostContext::default(), Limits::default()).unwrap();
    assert_eq!(format!("{:?}", out.iter().map(|x| &x.1).collect::<Vec<_>>()), "[1, 2]");
}

#[test]
fn every_std_export_is_documented() {
    let doc = check("$[Import* std]").unwrap();
    let undocumented: Vec<&str> = doc.static_bindings.iter().filter(|b| b.name != "std" && b.doc.is_none()).map(|b| b.name.as_str()).collect();
    assert!(doc.static_bindings.len() > 80, "std should export its library");
    assert!(undocumented.is_empty(), "std exports without docs: {:?}", undocumented);
    for m in ["!if", "!and", "!or", "!assert"] {
        assert!(doc.docs.contains_key(m), "macro {} is undocumented", m);
    }
    assert!(!doc.docs.contains_key("_render"));
    assert!(check("$[Import std]").unwrap().docs.is_empty(), "a qualified import binds only the namespace");
}
