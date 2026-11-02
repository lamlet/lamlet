//! The `K` package: Lamlet's irreducible primitives, defined through one declarative table.
//!
//! Every entry is an ordinary value with an ordinary (constrained) type scheme. The checker has no
//! per-primitive rules, so a primitive behaves identically when aliased, wrapped in a lambda, or passed to a
//! higher-order function. `docs/spec.md` §8 justifies each entry.
//!
//! # Adding a primitive
//!
//! Add one [`Intrinsic`] to [`INTRINSICS`] with its scheme in the syntax accepted by [`parse_scheme`], for
//! example `forall a:Eq. Fn([a], Int)`, and justify it in the spec: it must not be expressible in Lamlet with
//! reasonable performance. Host-dependent functions with concrete signatures are native stubs instead.

use crate::types::*;
use crate::value::{display, format_float, num_cmp, List, Value};
use std::collections::HashMap;
use std::rc::Rc;

pub enum Typing {
    /// A type scheme in the syntax accepted by [`parse_scheme`].
    Scheme(&'static str),
    /// `K::match`: a scheme whose case constraint the scheme syntax cannot write; built by [`match_scheme`].
    Match,
}

pub enum Outcome {
    Value(Value),
    /// Call this function with these arguments in place of the primitive (an eliminator's selected handler).
    Call(Value, Vec<Value>),
}

type R = Result<Outcome, String>;

pub struct Intrinsic {
    pub name: &'static str,
    pub typing: Typing,
    pub arity: usize,
    pub run: fn(&[Value]) -> R,
}

/// A non-callable `K` constant.
pub struct Constant {
    pub name: &'static str,
    pub scheme: &'static str,
    pub value: fn() -> Value,
}

/// A handle to an entry of [`INTRINSICS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Builtin(u16);

impl Builtin {
    pub fn info(self) -> &'static Intrinsic {
        &INTRINSICS[self.0 as usize]
    }
    pub fn name(self) -> &'static str {
        self.info().name
    }
    pub fn all() -> impl Iterator<Item = Builtin> {
        (0..INTRINSICS.len()).map(|i| Builtin(i as u16))
    }
    pub fn by_name(name: &str) -> Option<Builtin> {
        INTRINSICS.iter().position(|i| i.name == name).map(|i| Builtin(i as u16))
    }
}

fn v(x: Value) -> R {
    Ok(Outcome::Value(x))
}

fn ty_value(t: TypeDesc) -> Value {
    Value::Type(Rc::new(t))
}

macro_rules! intrinsic {
    ($name:expr, $scheme:expr, $arity:expr, $run:expr) => {
        Intrinsic { name: $name, typing: Typing::Scheme($scheme), arity: $arity, run: $run }
    };
}

fn float1(a: &[Value], f: fn(f64) -> Result<f64, String>) -> R {
    v(check_float(f(float_of(&a[0])?)?)?)
}

pub static CONSTANTS: &[Constant] = &[
    Constant { name: "nil", scheme: "List(a)", value: || Value::List(List::nil()) },
    Constant { name: "unit", scheme: "Unit", value: || Value::Unit },
    Constant { name: "intType", scheme: "Type", value: || ty_value(TypeDesc::Int) },
    Constant { name: "floatType", scheme: "Type", value: || ty_value(TypeDesc::Float) },
    Constant { name: "boolType", scheme: "Type", value: || ty_value(TypeDesc::Bool) },
    Constant { name: "stringType", scheme: "Type", value: || ty_value(TypeDesc::String) },
    Constant { name: "unitType", scheme: "Type", value: || ty_value(TypeDesc::Unit) },
    Constant { name: "typeType", scheme: "Type", value: || ty_value(TypeDesc::Type) },
];

pub static INTRINSICS: &[Intrinsic] = &[
    // Numbers. Num dispatches on the runtime representation of the operands (Int or Float).
    intrinsic!("add", "forall n:Num. Fn([n, n], n)", 2, |a| arith(&a[0], &a[1], "addition", i64::checked_add, |x, y| x + y)),
    intrinsic!("subtract", "forall n:Num. Fn([n, n], n)", 2, |a| arith(&a[0], &a[1], "subtraction", i64::checked_sub, |x, y| x - y)),
    intrinsic!("multiply", "forall n:Num. Fn([n, n], n)", 2, |a| arith(&a[0], &a[1], "multiplication", i64::checked_mul, |x, y| x * y)),
    intrinsic!("negate", "forall n:Num. Fn([n], n)", 1, |a| v(match &a[0] {
        Value::Int(n) => Value::Int(n.checked_neg().ok_or("Int overflow in negation")?),
        Value::Float(f) => check_float(-f)?,
        o => return Err(format!("cannot negate {}", o.type_name())),
    })),
    intrinsic!("divideFloat", "Fn([Float, Float], Float)", 2, |a| {
        let (x, y) = (float_of(&a[0])?, float_of(&a[1])?);
        if y == 0.0 {
            return Err("division by zero".into());
        }
        v(check_float(x / y)?)
    }),
    intrinsic!("quotientEuclidean", "Fn([Int, Int], Int)", 2, |a| {
        let (q, _) = euclid(int_of(&a[0])?, int_of(&a[1])?)?;
        v(Value::Int(i64::try_from(q).map_err(|_| "Int overflow in div")?))
    }),
    intrinsic!("remainderEuclidean", "Fn([Int, Int], Int)", 2, |a| {
        let (_, r) = euclid(int_of(&a[0])?, int_of(&a[1])?)?;
        v(Value::Int(r as i64))
    }),
    intrinsic!("toFloat", "Fn([Int], Float)", 1, |a| v(check_float(int_of(&a[0])? as f64)?)),
    intrinsic!("floor", "Fn([Float], Int)", 1, |a| v(Value::Int(float_to_int(float_of(&a[0])?.floor(), "floor")?))),
    intrinsic!("ceil", "Fn([Float], Int)", 1, |a| v(Value::Int(float_to_int(float_of(&a[0])?.ceil(), "ceil")?))),
    intrinsic!("round", "Fn([Float], Int)", 1, |a| v(Value::Int(float_to_int(float_of(&a[0])?.round(), "round")?))),
    intrinsic!("truncate", "Fn([Float], Int)", 1, |a| v(Value::Int(float_to_int(float_of(&a[0])?.trunc(), "truncate")?))),
    intrinsic!("sqrt", "Fn([Float], Float)", 1, |a| float1(a, |x| if x < 0.0 { Err("sqrt of a negative number".into()) } else { Ok(x.sqrt()) })),
    intrinsic!("sin", "Fn([Float], Float)", 1, |a| float1(a, |x| Ok(det::sin(x)))),
    intrinsic!("cos", "Fn([Float], Float)", 1, |a| float1(a, |x| Ok(det::cos(x)))),
    intrinsic!("exp", "Fn([Float], Float)", 1, |a| float1(a, det::exp)),
    intrinsic!("ln", "Fn([Float], Float)", 1, |a| float1(a, |x| if x <= 0.0 { Err("ln of a nonpositive number".into()) } else { Ok(det::ln(x)) })),
    intrinsic!("fixed", "Fn([Float, Int], String)", 2, |a| {
        let (x, digits) = (float_of(&a[0])?, int_of(&a[1])?);
        if !(0..=MAX_FIXED_DIGITS).contains(&digits) {
            return Err(format!("fixed needs 0 to {} decimal places, not {}", MAX_FIXED_DIGITS, digits));
        }
        v(Value::Str(Rc::from(fixed(x, digits as usize))))
    }),
    // Equality, order, hashing: structural over every representation.
    intrinsic!("equal", "forall a:Eq. Fn([a, a], Bool)", 2, |a| v(Value::Bool(a[0].equal(&a[1])?))),
    intrinsic!("less", "forall a:Ord. Fn([a, a], Bool)", 2, |a| v(Value::Bool(match (&a[0], &a[1]) {
        (Value::Str(x), Value::Str(y)) => x < y,
        (x, y) => num_cmp(x, y)? == std::cmp::Ordering::Less,
    }))),
    intrinsic!("hash", "forall a:Eq. Fn([a], Int)", 1, |a| v(Value::Int(hash_value(&a[0])?))),
    // Eliminators: each takes handler functions and calls exactly one.
    intrinsic!("caseBool", "Fn([Bool, Fn([], a) !e, Fn([], a) !e], a) !e", 3, |a| match &a[0] {
        Value::Bool(true) => Ok(Outcome::Call(a[1].clone(), vec![])),
        Value::Bool(false) => Ok(Outcome::Call(a[2].clone(), vec![])),
        o => Err(format!("expected Bool, found {}", o.type_name())),
    }),
    intrinsic!("cons", "Fn([a, List(a)], List(a))", 2, |a| v(Value::List(List::cons(a[0].clone(), list_of(&a[1])?)))),
    intrinsic!("caseList", "Fn([List(a), Fn([], r) !e, Fn([a, List(a)], r) !e], r) !e", 3, |a| match &list_of(&a[0])?.0 {
        None => Ok(Outcome::Call(a[1].clone(), vec![])),
        Some(c) => Ok(Outcome::Call(a[2].clone(), vec![c.head.clone(), Value::List(c.tail.clone())])),
    }),
    Intrinsic {
        name: "match",
        typing: Typing::Match,
        arity: 2,
        run: |a| match (&a[0], &a[1]) {
            (Value::Variant(x), Value::Record(_)) => match a[1].field(&x.0) {
                Some(h) => Ok(Outcome::Call(h.clone(), vec![x.1.clone()])),
                None => Err(format!("no handler for alternative #{}", x.0)),
            },
            (x, h) => Err(format!("K::match expects a variant and a handler record, found {} and {}", x.type_name(), h.type_name())),
        },
    },
    // Strings.
    intrinsic!("stringAppend", "Fn([String, String], String)", 2, |a| {
        let (x, y) = (str_of(&a[0])?, str_of(&a[1])?);
        let mut s = String::with_capacity(x.len() + y.len());
        s.push_str(&x);
        s.push_str(&y);
        v(Value::Str(Rc::from(s)))
    }),
    intrinsic!("stringScalars", "Fn([String], List(String))", 1, |a| {
        let s = str_of(&a[0])?;
        v(Value::List(List::from_vec(s.chars().map(|c| Value::Str(Rc::from(c.to_string()))).collect())))
    }),
    intrinsic!("scalarString", "forall s:Scalar. Fn([s], String)", 1, |a| v(Value::Str(Rc::from(scalar_string(&a[0])?)))),
    intrinsic!("latexEscape", "Fn([String], String)", 1, |a| v(Value::Str(Rc::from(latex_escape(&str_of(&a[0])?))))),
    intrinsic!("debugString", "Fn([a], String)", 1, |a| v(Value::Str(Rc::from(display(&a[0]))))),
    intrinsic!("fail", "Fn([String], a)", 1, |a| Err(str_of(&a[0])?.to_string())),
    // Type descriptors: ordinary pure functions over Type data.
    intrinsic!("listType", "Fn([Type], Type)", 1, |a| v(ty_value(TypeDesc::List(Rc::new(type_of(&a[0])?))))),
    intrinsic!("functionType", "Fn([List(Type), Type], Type)", 2, |a| {
        let args = list_of(&a[0])?.iter().map(|t| type_of(&t)).collect::<Result<Vec<_>, _>>()?;
        v(ty_value(TypeDesc::Func(args, Rc::new(type_of(&a[1])?))))
    }),
    intrinsic!("recordType", "Fn([List({name: String, type: Type})], Type)", 1, |a| v(ty_value(TypeDesc::Record(labeled(&a[0], "field")?)))),
    intrinsic!("variantType", "Fn([List({name: String, type: Type})], Type)", 1, |a| v(ty_value(TypeDesc::Variant(labeled(&a[0], "alternative")?)))),
];

fn labeled(list: &Value, what: &str) -> Result<Vec<(Rc<str>, TypeDesc)>, String> {
    let mut items = Vec::new();
    for m in list_of(list)?.iter() {
        let name = m.field("name").ok_or("expected {name, type} records")?;
        let ty = m.field("type").ok_or("expected {name, type} records")?;
        items.push((str_of(name)?, type_of(ty)?));
    }
    normalize_labeled(items, what)
}

/// The scheme of `K::match`: `∀ v h r e. cases(h, r, e) on v ⇒ Fn([v, h], r) ! e`.
pub fn match_scheme(t: &mut TypeCtx) -> Ty {
    let r = t.fresh(GENERIC_LEVEL);
    let e = t.fresh_eff(GENERIC_LEVEL, 0);
    let v = t.fresh(GENERIC_LEVEL);
    let h = t.fresh(GENERIC_LEVEL);
    let (Ty::Var(vi), Ty::Var(hi)) = (&v, &h) else { unreachable!() };
    let (vi, hi) = (*vi, *hi);
    t.add_cons(vi, Cons { cases: vec![Case { other: h.clone(), ret: r.clone(), eff: e }], ..Cons::default() }).unwrap();
    t.add_cons(hi, Cons { handles: vec![Case { other: v.clone(), ret: r.clone(), eff: e }], ..Cons::default() }).unwrap();
    Ty::Fn(Box::new(FnTy { params: vec![v, h], ret: r, eff: e }))
}

/// The generic type of a label function.
pub fn label_scheme(t: &mut TypeCtx, l: &crate::value::LabelFn) -> Ty {
    use crate::value::LabelFn;
    let g = GENERIC_LEVEL;
    let a = t.fresh(g);
    let eff = t.fresh_eff(g, 0);
    match l {
        LabelFn::Get(n) => {
            let r = t.fresh_with(g, Cons { fields: vec![(n.clone(), a.clone())], ..Cons::default() });
            Ty::Fn(Box::new(FnTy { params: vec![r], ret: a, eff }))
        }
        LabelFn::Set(n) => {
            let r = t.fresh_with(g, Cons { fields: vec![(n.clone(), a.clone())], ..Cons::default() });
            Ty::Fn(Box::new(FnTy { params: vec![r.clone(), a], ret: r, eff }))
        }
        LabelFn::Tag(n) => {
            let vt = t.fresh_with(g, Cons { alts: vec![(n.clone(), a.clone())], ..Cons::default() });
            Ty::Fn(Box::new(FnTy { params: vec![a], ret: vt, eff }))
        }
        LabelFn::Record(ls) => {
            let params: Vec<Ty> = ls.iter().map(|_| t.fresh(g)).collect();
            let mut fields: Vec<(Rc<str>, Ty)> = ls.iter().cloned().zip(params.iter().cloned()).collect();
            fields.sort_by(|x, y| x.0.cmp(&y.0));
            Ty::Fn(Box::new(FnTy { params, ret: Ty::Record(fields), eff }))
        }
        LabelFn::List(n) => Ty::Fn(Box::new(FnTy { params: vec![a.clone(); *n], ret: list(a), eff })),
    }
}

/// Run a label function.
pub fn call_label(l: &crate::value::LabelFn, args: &[Value]) -> Result<Value, String> {
    use crate::value::LabelFn;
    match l {
        LabelFn::Get(n) => args[0].field(n).cloned().ok_or_else(|| format!("{} has no field `{}`", args[0].type_name(), n)),
        LabelFn::Set(n) => match &args[0] {
            Value::Record(r) => {
                let mut fs = (**r).clone();
                match fs.iter_mut().find(|f| f.0 == *n) {
                    Some(f) => f.1 = args[1].clone(),
                    None => return Err(format!("record has no field `{}`", n)),
                }
                Ok(Value::Record(Rc::new(fs)))
            }
            o => Err(format!("expected a record, found {}", o.type_name())),
        },
        LabelFn::Tag(n) => Ok(Value::Variant(Rc::new((n.clone(), args[0].clone())))),
        LabelFn::Record(ls) => Ok(Value::record(ls.iter().cloned().zip(args.iter().cloned()).collect())),
        LabelFn::List(_) => Ok(Value::List(List::from_vec(args.to_vec()))),
    }
}

/// Execute an intrinsic.
pub fn call(b: Builtin, args: &[Value]) -> R {
    let info = b.info();
    if args.len() != info.arity {
        return Err(format!("K.{} expects {} arguments, got {}", info.name, info.arity, args.len()));
    }
    (info.run)(args)
}

/// The type of a `K` export (intrinsics and constants alike).
pub fn intrinsic_type(b: Builtin, t: &mut TypeCtx) -> Ty {
    match b.info().typing {
        Typing::Scheme(src) => parse_scheme(src, t).unwrap_or_else(|e| panic!("intrinsic K.{}: {}", b.name(), e)),
        Typing::Match => match_scheme(t),
    }
}

// ---------------------------------------------------------------------------------------------------------
// Scheme syntax
//
//   scheme  := ['forall' binder* '.'] type
//   binder  := var [':' Constraint ('+' Constraint)*]       Constraint: Num | Eq | Ord | Scalar
//   type    := Int | Float | Bool | String | Unit | Type | var
//            | List(type) | {label: type, ...}
//            | Fn([type, ...], type) ['!' effect]          effect: Static | Runtime | var
//
// Lowercase identifiers are type variables (binders only add constraints); all variables are quantified.
// Effect variables with the same name are shared within one scheme.

/// Parse a scheme into a type whose variables are all quantified (level [`GENERIC_LEVEL`]).
pub fn parse_scheme(src: &str, t: &mut TypeCtx) -> Result<Ty, String> {
    let toks = scheme_tokens(src);
    let mut p = SchemeParser { toks, pos: 0, vars: HashMap::new(), effs: HashMap::new(), src };
    if p.peek() == Some("forall") {
        p.pos += 1;
        while p.peek() != Some(".") {
            let name = p.next()?;
            let mut cons = Cons::default();
            if p.peek() == Some(":") {
                p.pos += 1;
                loop {
                    match p.next()?.as_str() {
                        "Num" => cons.num = true,
                        "Eq" => cons.eq = true,
                        "Ord" => cons.ord = true,
                        "Scalar" => cons.scalar = true,
                        other => return Err(format!("unknown constraint `{}` in scheme `{}`", other, src)),
                    }
                    if p.peek() != Some("+") {
                        break;
                    }
                    p.pos += 1;
                }
            }
            let var = t.fresh_with(GENERIC_LEVEL, cons);
            p.vars.insert(name, var);
        }
        p.pos += 1;
    }
    let ty = p.ty(t)?;
    if p.pos != p.toks.len() {
        return Err(format!("trailing tokens in scheme `{}`", src));
    }
    Ok(ty)
}

fn scheme_tokens(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in src.chars() {
        if c.is_alphanumeric() || c == '_' {
            cur.push(c);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

struct SchemeParser<'a> {
    toks: Vec<String>,
    pos: usize,
    vars: HashMap<String, Ty>,
    effs: HashMap<String, u32>,
    src: &'a str,
}

impl<'a> SchemeParser<'a> {
    fn peek(&self) -> Option<&str> {
        self.toks.get(self.pos).map(|s| s.as_str())
    }
    fn next(&mut self) -> Result<String, String> {
        let t = self.toks.get(self.pos).cloned().ok_or_else(|| format!("unexpected end of scheme `{}`", self.src))?;
        self.pos += 1;
        Ok(t)
    }
    fn expect(&mut self, s: &str) -> Result<(), String> {
        let t = self.next()?;
        if t == s {
            Ok(())
        } else {
            Err(format!("expected `{}`, found `{}` in scheme `{}`", s, t, self.src))
        }
    }
    fn ty(&mut self, t: &mut TypeCtx) -> Result<Ty, String> {
        let name = self.next()?;
        Ok(match name.as_str() {
            "Int" => INT,
            "Float" => FLOAT,
            "Bool" => BOOL,
            "String" => STRING,
            "Unit" => UNIT,
            "Type" => TYPE,
            "List" => {
                self.expect("(")?;
                let e = self.ty(t)?;
                self.expect(")")?;
                list(e)
            }
            "{" => {
                let mut fields = Vec::new();
                while self.peek() != Some("}") {
                    if !fields.is_empty() {
                        self.expect(",")?;
                    }
                    let label = self.next()?;
                    self.expect(":")?;
                    fields.push((Rc::from(label.as_str()), self.ty(t)?));
                }
                self.pos += 1;
                fields.sort_by(|a: &(Rc<str>, Ty), b| a.0.cmp(&b.0));
                Ty::Record(fields)
            }
            "Fn" => {
                self.expect("(")?;
                self.expect("[")?;
                let mut params = Vec::new();
                while self.peek() != Some("]") {
                    if !params.is_empty() {
                        self.expect(",")?;
                    }
                    params.push(self.ty(t)?);
                }
                self.pos += 1;
                self.expect(",")?;
                let ret = self.ty(t)?;
                self.expect(")")?;
                let eff = if self.peek() == Some("!") {
                    self.pos += 1;
                    match self.next()?.as_str() {
                        "Static" => t.fresh_eff(GENERIC_LEVEL, EFF_STATIC),
                        "Runtime" => t.fresh_eff(GENERIC_LEVEL, EFF_RUNTIME),
                        e => *self.effs.entry(e.to_string()).or_insert_with(|| t.fresh_eff(GENERIC_LEVEL, 0)),
                    }
                } else {
                    t.fresh_eff(GENERIC_LEVEL, 0)
                };
                Ty::Fn(Box::new(FnTy { params, ret, eff }))
            }
            v if v.starts_with(|c: char| c.is_lowercase()) => self.vars.entry(v.to_string()).or_insert_with(|| t.fresh(GENERIC_LEVEL)).clone(),
            other => return Err(format!("unknown type `{}` in scheme `{}`", other, self.src)),
        })
    }
}

// ---------------------------------------------------------------------------------------------------------
// Runtime helpers

pub fn check_float(f: f64) -> Result<Value, String> {
    if f.is_finite() {
        Ok(Value::Float(if f == 0.0 { 0.0 } else { f }))
    } else {
        Err("Float result is not finite".into())
    }
}

fn arith(a: &Value, b: &Value, what: &str, iop: fn(i64, i64) -> Option<i64>, fop: fn(f64, f64) -> f64) -> R {
    v(match (a, b) {
        (Value::Int(x), Value::Int(y)) => Value::Int(iop(*x, *y).ok_or_else(|| format!("Int overflow in {}", what))?),
        (Value::Float(x), Value::Float(y)) => check_float(fop(*x, *y))?,
        _ => return Err(format!("numeric primitive applied to {} and {}", a.type_name(), b.type_name())),
    })
}

fn int_of(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int(n) => Ok(*n),
        _ => Err(format!("expected Int, found {}", v.type_name())),
    }
}

fn float_of(v: &Value) -> Result<f64, String> {
    match v {
        Value::Float(f) => Ok(*f),
        _ => Err(format!("expected Float, found {}", v.type_name())),
    }
}

fn str_of(v: &Value) -> Result<Rc<str>, String> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        _ => Err(format!("expected String, found {}", v.type_name())),
    }
}

fn type_of(v: &Value) -> Result<TypeDesc, String> {
    match v {
        Value::Type(t) => Ok((**t).clone()),
        _ => Err(format!("expected Type, found {}", v.type_name())),
    }
}

fn list_of(v: &Value) -> Result<List, String> {
    match v {
        Value::List(l) => Ok(l.clone()),
        _ => Err(format!("expected List, found {}", v.type_name())),
    }
}

fn euclid(a: i64, b: i64) -> Result<(i128, i128), String> {
    if b == 0 {
        return Err("division by zero".into());
    }
    let (a, b) = (a as i128, b as i128);
    let r = a.rem_euclid(b.abs());
    Ok(((a - r) / b, r))
}

pub const MAX_FIXED_DIGITS: i64 = 20;

/// A Float as an Int, or an error if it is outside the Int range.
fn float_to_int(x: f64, what: &str) -> Result<i64, String> {
    if x.is_finite() && (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&x) {
        Ok(x as i64)
    } else {
        Err(format!("{} result {} is outside the Int range", what, format_float(x)))
    }
}

/// Decimal text with exactly `digits` places, correctly rounded from the exact binary value (ties to even);
/// a result that rounds to zero is never negative.
pub fn fixed(x: f64, digits: usize) -> String {
    let s = format!("{:.*}", digits, x);
    match s.strip_prefix('-') {
        Some(rest) if rest.chars().all(|c| c == '0' || c == '.') => rest.to_string(),
        _ => s,
    }
}

/// Escape text for insertion into a LaTeX formula. Characters with a math-safe escape use it; `\`, `^`, and
/// `~` (which have none) become `\text{...}` symbols.
pub fn latex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '{' | '}' | '$' | '&' | '#' | '%' | '_' => {
                out.push('\\');
                out.push(c);
            }
            '\\' => out.push_str("\\text{\\textbackslash}"),
            '^' => out.push_str("\\text{\\textasciicircum}"),
            '~' => out.push_str("\\text{\\textasciitilde}"),
            _ => out.push(c),
        }
    }
    out
}

pub fn scalar_string(x: &Value) -> Result<String, String> {
    Ok(match x {
        Value::Str(s) => s.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => format_float(*f),
        Value::Bool(b) => b.to_string(),
        Value::Unit => String::new(),
        other => return Err(format!("{} is not a formattable scalar", other.type_name())),
    })
}

/// Deterministic transcendental functions: only IEEE-754 `+ - * /` (correctly rounded, so identical on every
/// platform) and exact scaling by powers of two. Accuracy is within a few ulps over the useful range;
/// `sin`/`cos` reduce the argument with a three-part π/2 and are accurate for |x| below about 1e9.
pub mod det {
    const PIO2_1: f64 = 1.570_796_326_734_125_6;
    const PIO2_2: f64 = 6.077_100_506_303_966e-11;
    const PIO2_3: f64 = 2.022_266_248_711_166_5e-21;
    const LN2_HI: f64 = 6.931_471_803_691_238e-1;
    const LN2_LO: f64 = 1.908_214_929_270_587_7e-10;
    /// n! for n = 0..=17 (all exactly representable).
    const FACT: [f64; 18] = [
        1.0, 1.0, 2.0, 6.0, 24.0, 120.0, 720.0, 5040.0, 40320.0, 362880.0, 3628800.0, 39916800.0, 479001600.0, 6227020800.0,
        87178291200.0, 1307674368000.0, 20922789888000.0, 355687428096000.0,
    ];

    /// x·2^k, exactly (barring underflow), in steps that stay in range.
    fn scale(mut x: f64, mut k: i32) -> f64 {
        while k > 1000 {
            x *= f64::from_bits(((1000 + 1023) as u64) << 52);
            k -= 1000;
        }
        while k < -1000 {
            x *= f64::from_bits(((-1000 + 1023) as u64) << 52);
            k += 1000;
        }
        x * f64::from_bits(((k + 1023) as u64) << 52)
    }

    fn reduce(x: f64) -> (f64, i64) {
        let k = (x * (2.0 / std::f64::consts::PI)).round();
        let r = ((x - k * PIO2_1) - k * PIO2_2) - k * PIO2_3;
        (r, k as i64)
    }

    /// sin on [-π/4, π/4]: Taylor series to x^19 (truncation error below 1e-19).
    fn sin_kernel(r: f64) -> f64 {
        let r2 = r * r;
        let mut term = r;
        let mut sum = r;
        let mut n = 1.0;
        for _ in 0..9 {
            term = -term * r2 / ((n + 1.0) * (n + 2.0));
            sum += term;
            n += 2.0;
        }
        sum
    }

    fn cos_kernel(r: f64) -> f64 {
        let r2 = r * r;
        let mut term = 1.0;
        let mut sum = 1.0;
        let mut n = 0.0;
        for _ in 0..10 {
            term = -term * r2 / ((n + 1.0) * (n + 2.0));
            sum += term;
            n += 2.0;
        }
        sum
    }

    pub fn sin(x: f64) -> f64 {
        let (r, k) = reduce(x);
        match k.rem_euclid(4) {
            0 => sin_kernel(r),
            1 => cos_kernel(r),
            2 => -sin_kernel(r),
            _ => -cos_kernel(r),
        }
    }

    pub fn cos(x: f64) -> f64 {
        let (r, k) = reduce(x);
        match k.rem_euclid(4) {
            0 => cos_kernel(r),
            1 => -sin_kernel(r),
            2 => -cos_kernel(r),
            _ => sin_kernel(r),
        }
    }

    pub fn exp(x: f64) -> Result<f64, String> {
        if x > 709.78 {
            return Err("exp result is not finite".into());
        }
        if x < -745.2 {
            return Ok(0.0);
        }
        let k = (x / std::f64::consts::LN_2).round();
        let r = (x - k * LN2_HI) - k * LN2_LO;
        // e^r = 1 + (r + r²·q), q = Σ r^(n-2)/n! by Horner; adding the large terms last limits rounding.
        let mut q = 0.0;
        for n in (2..=17).rev() {
            q = q * r + 1.0 / FACT[n];
        }
        Ok(scale(1.0 + (r + r * r * q), k as i32))
    }

    pub fn ln(x: f64) -> f64 {
        // x = m·2^e with m in [√½, √2).
        let (mut x, mut e) = (x, 0i32);
        if x < f64::MIN_POSITIVE {
            x *= f64::from_bits(((54 + 1023) as u64) << 52);
            e -= 54;
        }
        let bits = x.to_bits();
        e += ((bits >> 52) & 0x7ff) as i32 - 1023;
        let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | (1023u64 << 52));
        if m > std::f64::consts::SQRT_2 {
            m *= 0.5;
            e += 1;
        }
        // ln m = 2·atanh(s), s = (m-1)/(m+1), |s| < 0.1716.
        let s = (m - 1.0) / (m + 1.0);
        let s2 = s * s;
        let mut term = s;
        let mut sum = 0.0;
        let mut n = 1.0;
        for _ in 0..14 {
            sum += term / n;
            term *= s2;
            n += 2.0;
        }
        let e = e as f64;
        e * LN2_HI + (e * LN2_LO + 2.0 * sum)
    }
}

/// The `hash` contract: FNV-1a (64-bit) over a canonical byte encoding, returned as the two's-complement Int.
/// Equal values (in the sense of `K::equal`) have equal hashes on every platform and in every run.
pub fn hash_value(x: &Value) -> Result<i64, String> {
    let mut h = Fnv(0xcbf29ce484222325);
    encode_hash(x, &mut h)?;
    Ok(h.0 as i64)
}

struct Fnv(u64);

impl Fnv {
    fn bytes(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn tag(&mut self, t: u8) {
        self.bytes(&[t]);
    }
    fn len(&mut self, n: usize) {
        self.bytes(&(n as u64).to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.len(s.len());
        self.bytes(s.as_bytes());
    }
}

fn encode_hash(x: &Value, h: &mut Fnv) -> Result<(), String> {
    match x {
        Value::Unit => h.tag(0),
        Value::Bool(b) => {
            h.tag(1);
            h.bytes(&[*b as u8]);
        }
        Value::Int(n) => {
            h.tag(2);
            h.bytes(&n.to_le_bytes());
        }
        Value::Float(f) => {
            h.tag(3);
            h.bytes(&(if *f == 0.0 { 0.0f64 } else { *f }).to_bits().to_le_bytes());
        }
        Value::Str(s) => {
            h.tag(4);
            h.str(s);
        }
        Value::List(l) => {
            h.tag(5);
            let items = l.to_vec();
            h.len(items.len());
            for i in &items {
                encode_hash(i, h)?;
            }
        }
        Value::Record(fs) => {
            h.tag(6);
            h.len(fs.len());
            for (n, x) in fs.iter() {
                h.str(n);
                encode_hash(x, h)?;
            }
        }
        Value::Variant(x) => {
            h.tag(7);
            h.str(&x.0);
            encode_hash(&x.1, h)?;
        }
        Value::Type(t) => {
            h.tag(8);
            h.str(&t.to_string());
        }
        other => return Err(format!("{} values cannot be hashed", other.type_name())),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(name: &str, args: Vec<Value>) -> Result<Value, String> {
        match call(Builtin::by_name(name).unwrap(), &args)? {
            Outcome::Value(v) => Ok(v),
            _ => panic!("not a value"),
        }
    }

    #[test]
    fn every_scheme_parses() {
        let mut t = TypeCtx::default();
        for b in Builtin::all() {
            intrinsic_type(b, &mut t);
        }
        for c in CONSTANTS {
            parse_scheme(c.scheme, &mut t).unwrap();
        }
        assert!(parse_scheme("Fn([Int], Wat)", &mut t).is_err());
        assert!(parse_scheme("forall a:Bogus. a", &mut t).is_err());
    }

    #[test]
    fn euclidean() {
        let r = |a, b| run("remainderEuclidean", vec![Value::Int(a), Value::Int(b)]).map(|v| int_of(&v).unwrap());
        assert_eq!(r(-8, 3), Ok(1));
        assert_eq!(r(8, -3), Ok(2));
        assert_eq!(r(i64::MIN, -1), Ok(0));
        assert!(r(4, 0).is_err());
        assert!(run("quotientEuclidean", vec![Value::Int(i64::MIN), Value::Int(-1)]).is_err());
    }

    #[test]
    fn overflow() {
        assert!(run("add", vec![Value::Int(i64::MAX), Value::Int(1)]).is_err());
        assert!(run("multiply", vec![Value::Float(1e308), Value::Float(10.0)]).is_err());
        assert!(run("add", vec![Value::Int(1), Value::Float(1.0)]).is_err());
    }

    #[test]
    fn deterministic_math_is_accurate() {
        let close = |a: f64, b: f64| (a - b).abs() <= 4.0 * f64::EPSILON * b.abs().max(1.0);
        for i in -2000..2000 {
            let x = i as f64 * 0.0137;
            assert!(close(det::sin(x), x.sin()), "sin {}", x);
            assert!(close(det::cos(x), x.cos()), "cos {}", x);
            assert!(close(det::exp(x / 10.0).unwrap(), (x / 10.0).exp()), "exp {}", x);
            let y = (i as f64).abs() * 0.731 + 1e-3;
            assert!((det::ln(y) - y.ln()).abs() <= 4.0 * f64::EPSILON * y.ln().abs().max(1.0), "ln {}", y);
        }
        assert_eq!(det::ln(1.0), 0.0);
        assert_eq!(det::sin(0.0), 0.0);
        assert!(close(det::ln(1e-310), (1e-310f64).ln()));
        // Pinned bits: these must never change across platforms or releases.
        assert_eq!(det::sin(1.0).to_bits(), 0x3feaed548f090cee);
        assert_eq!(det::exp(1.0).unwrap().to_bits(), 0x4005bf0a8b14576a); // e, 1 ulp above the correctly rounded value
    }

    #[test]
    fn hash_is_stable_and_respects_equality() {
        let h = |v: Value| hash_value(&v).unwrap();
        // Pinned value: the encoding is part of the contract.
        assert_eq!(h(Value::Unit), 0xaf63bd4c8601b7df_u64 as i64);
        assert_eq!(h(Value::Float(-0.0)), h(Value::Float(0.0)));
        assert_ne!(h(Value::str("ab")), h(Value::str("ba")));
        let l = |xs: &[i64]| Value::List(List::from_vec(xs.iter().map(|x| Value::Int(*x)).collect()));
        assert_eq!(h(l(&[1, 2])), h(l(&[1, 2])));
        assert_ne!(h(l(&[1, 2])), h(l(&[2, 1])));
        assert_ne!(h(Value::variant("a", Value::Unit)), h(Value::variant("b", Value::Unit)));
        assert!(hash_value(&Value::Builtin(Builtin::by_name("add").unwrap())).is_err());
    }
}
