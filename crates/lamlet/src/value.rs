//! Immutable runtime values.
//!
//! Values are shared through reference counting. The language has no mutable references, and a recursive
//! lambda refers to itself through its call frame rather than by capturing itself, so value graphs are acyclic
//! and reference counting releases everything without a cycle collector.

use crate::bytecode::Code;
use crate::types::{MuDef, TypeDesc};
use std::any::Any;
use std::fmt;
use std::rc::Rc;

#[derive(Clone)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    /// Finite, with zero normalized to positive zero.
    Float(f64),
    Str(Rc<str>),
    List(List),
    /// Fields sorted by label.
    Record(Rc<Vec<(Rc<str>, Value)>>),
    /// A variant value: its alternative's label and its payload.
    Variant(Rc<(Rc<str>, Value)>),
    Type(Rc<TypeDesc>),
    Closure(Rc<Closure>),
    Builtin(crate::builtins::Builtin),
    /// A label function: `.l`, `.l=`, `#l`, or a record constructor.
    Label(Rc<LabelFn>),
    Native(Rc<NativeFn>),
    Opaque(Rc<OpaqueValue>),
}

/// Functions determined by labels in the source syntax, or by a `[Type ...]` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelFn {
    /// `.l`: read field `l`.
    Get(Rc<str>),
    /// `.l=`: a copy of a record with field `l` replaced.
    Set(Rc<str>),
    /// `#l`: construct alternative `l`.
    Tag(Rc<str>),
    /// `{. l1 e1 ...}`: build a record from arguments given in this (source) label order.
    Record(Vec<Rc<str>>),
    /// `{' e1 ... en}`: build a list of its n arguments (flat, so long literals do not nest).
    List(usize),
    /// A recursive type's roll function: from its structure to the type. The identity on values.
    Roll(Rc<MuDef>),
    /// A recursive type's unroll function: from the type to its structure. The identity on values.
    Unroll(Rc<MuDef>),
    /// A recursive type's constructor: applies it to Type arguments, giving a Type.
    TypeApply(Rc<MuDef>),
}

impl LabelFn {
    pub fn arity(&self) -> usize {
        match self {
            LabelFn::Get(_) | LabelFn::Tag(_) | LabelFn::Roll(_) | LabelFn::Unroll(_) => 1,
            LabelFn::TypeApply(def) => def.params.len(),
            LabelFn::Set(_) => 2,
            LabelFn::Record(ls) => ls.len(),
            LabelFn::List(n) => *n,
        }
    }
}

pub struct Closure {
    pub code: Rc<Code>,
    pub captures: Vec<Value>,
}

/// A native stub value. The implementation is looked up in the executing VM's linked registry.
pub struct NativeFn {
    pub name: String,
    pub api_id: String,
    pub params: Vec<TypeDesc>,
    pub ret: TypeDesc,
    pub availability: crate::syntax::Availability,
}

/// A native-owned value of a registered opaque type. Dropping the last reference runs `release`.
pub struct OpaqueValue {
    pub type_id: Rc<str>,
    pub data: Box<dyn Any>,
    pub release: Option<Release>,
}

/// A hook run when the last reference to an opaque value is dropped.
pub type Release = Box<dyn Fn(&mut dyn Any)>;

impl Drop for OpaqueValue {
    fn drop(&mut self) {
        if let Some(r) = self.release.take() {
            r(self.data.as_mut());
        }
    }
}

/// Immutable singly linked list.
#[derive(Clone, Default)]
pub struct List(pub Option<Rc<Cons>>);

pub struct Cons {
    pub head: Value,
    pub tail: List,
}

impl Drop for Cons {
    fn drop(&mut self) {
        // Drop long tails iteratively to avoid host stack overflow.
        let mut next = self.tail.0.take();
        while let Some(rc) = next {
            match Rc::try_unwrap(rc) {
                Ok(mut cons) => next = cons.tail.0.take(),
                Err(_) => break,
            }
        }
    }
}

impl List {
    pub fn nil() -> List {
        List(None)
    }
    pub fn cons(head: Value, tail: List) -> List {
        List(Some(Rc::new(Cons { head, tail })))
    }
    pub fn from_vec(items: Vec<Value>) -> List {
        let mut l = List::nil();
        for v in items.into_iter().rev() {
            l = List::cons(v, l);
        }
        l
    }
    pub fn iter(&self) -> ListIter {
        ListIter(self.0.clone())
    }
    pub fn to_vec(&self) -> Vec<Value> {
        self.iter().collect()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }
}

pub struct ListIter(Option<Rc<Cons>>);

impl Iterator for ListIter {
    type Item = Value;
    fn next(&mut self) -> Option<Value> {
        let c = self.0.take()?;
        self.0 = c.tail.0.clone();
        Some(c.head.clone())
    }
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Rc::from(s))
    }

    /// A record from fields in any order (they are sorted by label).
    pub fn record(mut fields: Vec<(Rc<str>, Value)>) -> Value {
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        Value::Record(Rc::new(fields))
    }

    pub fn variant(label: &str, payload: Value) -> Value {
        Value::Variant(Rc::new((Rc::from(label), payload)))
    }

    /// The value of a record field, if this is a record that has it.
    pub fn field(&self, label: &str) -> Option<&Value> {
        match self {
            Value::Record(fs) => fs.iter().find(|f| &*f.0 == label).map(|f| &f.1),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Unit => "Unit",
            Value::Bool(_) => "Bool",
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Str(_) => "String",
            Value::List(_) => "List",
            Value::Record(_) => "record",
            Value::Variant(_) => "variant",
            Value::Type(_) => "Type",
            Value::Closure(_) | Value::Builtin(_) | Value::Label(_) | Value::Native(_) => "function",
            Value::Opaque(_) => "native value",
        }
    }

    /// Whether this is plain data at its top (not a function or a native value), so equality can compare it.
    pub fn is_data(&self) -> bool {
        !self.is_callable() && !matches!(self, Value::Opaque(_))
    }

    pub fn is_callable(&self) -> bool {
        matches!(self, Value::Closure(_) | Value::Builtin(_) | Value::Label(_) | Value::Native(_))
    }

    /// Structural equality on comparable values (checked before evaluation by the type checker).
    pub fn equal(&self, other: &Value) -> Result<bool, String> {
        let mut work: Vec<(Value, Value)> = vec![(self.clone(), other.clone())];
        while let Some((a, b)) = work.pop() {
            let same = match (&a, &b) {
                (Value::Unit, Value::Unit) => true,
                (Value::Bool(x), Value::Bool(y)) => x == y,
                (Value::Int(x), Value::Int(y)) => x == y,
                (Value::Float(x), Value::Float(y)) => x == y,
                (Value::Str(x), Value::Str(y)) => x == y,
                (Value::Type(x), Value::Type(y)) => x == y,
                (Value::List(x), Value::List(y)) => {
                    let (mut i, mut j) = (x.iter(), y.iter());
                    loop {
                        match (i.next(), j.next()) {
                            (None, None) => break true,
                            (Some(p), Some(q)) => work.push((p, q)),
                            _ => return Ok(false),
                        }
                    }
                }
                (Value::Record(x), Value::Record(y)) => {
                    if x.len() != y.len() || x.iter().zip(y.iter()).any(|(p, q)| p.0 != q.0) {
                        false
                    } else {
                        for (p, q) in x.iter().zip(y.iter()) {
                            work.push((p.1.clone(), q.1.clone()));
                        }
                        true
                    }
                }
                (Value::Variant(x), Value::Variant(y)) => {
                    if x.0 != y.0 {
                        false
                    } else {
                        work.push((x.1.clone(), y.1.clone()));
                        true
                    }
                }
                // Values of one static type always share a representation; two `Data` values need not.
                (x, y) if x.is_data() && y.is_data() => false,
                _ => return Err(format!("equality is not supported between {} and {}", a.type_name(), b.type_name())),
            };
            if !same {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub fn num_cmp(a: &Value, b: &Value) -> Result<std::cmp::Ordering, String> {
    use std::cmp::Ordering;
    Ok(match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        _ => return Err(format!("cannot compare {} with {}", a.type_name(), b.type_name())),
    })
}

/// Format a Float deterministically: shortest round-trip decimal, always with a fraction or exponent.
pub fn format_float(f: f64) -> String {
    let f = if f == 0.0 { 0.0 } else { f };
    format!("{:?}", f)
}

/// Render a value for display (also the implementation of `K::debugString`).
pub fn display(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v);
    out
}

fn write_value(out: &mut String, v: &Value) {
    match v {
        Value::Unit => out.push_str("()"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Float(f) => out.push_str(&format_float(*f)),
        Value::Str(s) => out.push_str(&format!("{:?}", &**s)),
        Value::List(l) => {
            out.push('[');
            for (i, x) in l.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(out, &x);
            }
            out.push(']');
        }
        Value::Record(fs) => {
            out.push('{');
            for (i, (n, x)) in fs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(n);
                out.push_str(": ");
                write_value(out, x);
            }
            out.push('}');
        }
        Value::Variant(x) => {
            out.push('#');
            out.push_str(&x.0);
            if !matches!(x.1, Value::Unit) {
                out.push('(');
                write_value(out, &x.1);
                out.push(')');
            }
        }
        Value::Type(t) => out.push_str(&t.to_string()),
        Value::Closure(c) => out.push_str(&format!("<function {}>", c.code.name)),
        Value::Builtin(b) => out.push_str(&format!("<primitive K.{}>", b.name())),
        Value::Label(l) => match &**l {
            LabelFn::Get(n) => out.push_str(&format!("<getter .{}>", n)),
            LabelFn::Set(n) => out.push_str(&format!("<setter .{}=>", n)),
            LabelFn::Tag(n) => out.push_str(&format!("<constructor #{}>", n)),
            LabelFn::Record(_) => out.push_str("<record constructor>"),
            LabelFn::List(_) => out.push_str("<list constructor>"),
            LabelFn::Roll(def) => out.push_str(&format!("<roll {}>", def.name)),
            LabelFn::Unroll(def) => out.push_str(&format!("<unroll {}>", def.name)),
            LabelFn::TypeApply(def) => out.push_str(&format!("<type constructor {}>", def.name)),
        },
        Value::Native(n) => out.push_str(&format!("<native {}>", n.api_id)),
        Value::Opaque(o) => out.push_str(&format!("<native value {}>", o.type_id)),
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", display(self))
    }
}

/// Approximate allocation size for budget accounting.
pub fn alloc_size(v: &Value) -> usize {
    match v {
        Value::Str(s) => 16 + s.len(),
        Value::List(_) => 48,
        Value::Record(r) => 24 + r.len() * 32,
        Value::Variant(_) => 40,
        Value::Closure(c) => 32 + c.captures.len() * 16,
        _ => 16,
    }
}
