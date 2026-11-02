//! Normalized Type descriptors (the values of `Type`) and the inference types used by the checker.
//!
//! `TypeDesc` is a finite, ground, canonically ordered description: record fields and variant alternatives
//! are sorted by label. `Ty` adds inference variables, constrained variables, and capability (effect)
//! variables on function types. See `docs/spec.md` §4.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeDesc {
    Unit,
    Bool,
    Int,
    Float,
    String,
    Type,
    List(Rc<TypeDesc>),
    /// Argument types, then the return type.
    Func(Vec<TypeDesc>, Rc<TypeDesc>),
    /// Fields sorted by label; labels unique and nonempty.
    Record(Vec<(Rc<str>, TypeDesc)>),
    /// Alternatives sorted by label; labels unique and nonempty. The empty variant is `Never`.
    Variant(Vec<(Rc<str>, TypeDesc)>),
    Opaque(Rc<str>),
}

impl TypeDesc {
    pub fn never() -> TypeDesc {
        TypeDesc::Variant(vec![])
    }

    /// Whether values of this type support generic equality.
    pub fn comparable(&self) -> bool {
        match self {
            TypeDesc::Func(..) | TypeDesc::Opaque(_) => false,
            TypeDesc::List(t) => t.comparable(),
            TypeDesc::Record(fs) | TypeDesc::Variant(fs) => fs.iter().all(|(_, t)| t.comparable()),
            _ => true,
        }
    }
}

fn write_labeled(f: &mut fmt::Formatter<'_>, open: &str, close: &str, items: &[(Rc<str>, TypeDesc)]) -> fmt::Result {
    write!(f, "{}", open)?;
    for (i, (n, t)) in items.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{}: {}", n, t)?;
    }
    write!(f, "{}", close)
}

impl fmt::Display for TypeDesc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeDesc::Unit => write!(f, "Unit"),
            TypeDesc::Bool => write!(f, "Bool"),
            TypeDesc::Int => write!(f, "Int"),
            TypeDesc::Float => write!(f, "Float"),
            TypeDesc::String => write!(f, "String"),
            TypeDesc::Type => write!(f, "Type"),
            TypeDesc::List(t) => write!(f, "List({})", t),
            TypeDesc::Func(args, r) => {
                write!(f, "Fn([")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", a)?;
                }
                write!(f, "], {})", r)
            }
            TypeDesc::Record(fs) => write_labeled(f, "{", "}", fs),
            TypeDesc::Variant(fs) if fs.is_empty() => write!(f, "Never"),
            TypeDesc::Variant(fs) => write_labeled(f, "<", ">", fs),
            TypeDesc::Opaque(id) => write!(f, "Native({:?})", id),
        }
    }
}

/// Validate and sort labeled members (record fields or variant alternatives).
pub fn normalize_labeled(mut items: Vec<(Rc<str>, TypeDesc)>, what: &str) -> Result<Vec<(Rc<str>, TypeDesc)>, String> {
    items.sort_by(|a, b| a.0.cmp(&b.0));
    for (n, _) in &items {
        if !is_label(n) {
            return Err(format!("{:?} is not a valid {} label (a word identifier)", &**n, what));
        }
    }
    for w in items.windows(2) {
        if w[0].0 == w[1].0 {
            return Err(format!("duplicate {} label {:?}", what, &*w[0].0));
        }
    }
    Ok(items)
}

/// A label is a word identifier (so every Type value's labels can be written as `.l`/`#l` syntax).
pub fn is_label(s: &str) -> bool {
    let mut cs = s.chars();
    match cs.next() {
        Some(c) if crate::source::is_word_start(c) => {}
        _ => return false,
    }
    let rest: Vec<char> = cs.collect();
    rest.iter().enumerate().all(|(i, c)| crate::source::is_word_continue(*c) || (*c == '-' && rest.get(i + 1).is_some_and(|n| crate::source::is_word_continue(*n))))
}

// ---------------------------------------------------------------------------------------------------------
// Inference types

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prim {
    Unit,
    Bool,
    Int,
    Float,
    String,
    Type,
}

#[derive(Clone, Debug)]
pub struct FnTy {
    pub params: Vec<Ty>,
    pub ret: Ty,
    /// Effect variable holding the latent capability requirements of calling the function.
    pub eff: u32,
}

#[derive(Clone, Debug)]
pub enum Ty {
    Var(u32),
    Prim(Prim),
    List(Box<Ty>),
    Fn(Box<FnTy>),
    /// Fields sorted by label.
    Record(Vec<(Rc<str>, Ty)>),
    /// Alternatives sorted by label.
    Variant(Vec<(Rc<str>, Ty)>),
    Opaque(Rc<str>),
}

pub const INT: Ty = Ty::Prim(Prim::Int);
pub const FLOAT: Ty = Ty::Prim(Prim::Float);
pub const BOOL: Ty = Ty::Prim(Prim::Bool);
pub const STRING: Ty = Ty::Prim(Prim::String);
pub const UNIT: Ty = Ty::Prim(Prim::Unit);
pub const TYPE: Ty = Ty::Prim(Prim::Type);

pub fn list(t: Ty) -> Ty {
    Ty::List(Box::new(t))
}

/// Capability atoms tracked by effect variables.
pub const EFF_STATIC: u8 = 1;
pub const EFF_RUNTIME: u8 = 2;

/// A case-analysis relation between a variant type and a handler record (see `K::match`).
#[derive(Clone, Debug)]
pub struct Case {
    /// The other side of the relation: the handler record (on a variant) or the variant (on a handler record).
    pub other: Ty,
    /// The result type of every handler.
    pub ret: Ty,
    /// The capability variable shared by every handler.
    pub eff: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Cons {
    pub num: bool,
    pub eq: bool,
    pub ord: bool,
    pub scalar: bool,
    /// Record fields the type must have (from `.l`/`.l=`).
    pub fields: Vec<(Rc<str>, Ty)>,
    /// Variant alternatives the type must have (from `#l`).
    pub alts: Vec<(Rc<str>, Ty)>,
    /// The type is a variant eliminated by these handler records.
    pub cases: Vec<Case>,
    /// The type is a handler record eliminating these variants.
    pub handles: Vec<Case>,
}

impl Cons {
    fn is_empty(&self) -> bool {
        !self.num && !self.eq && !self.ord && !self.scalar && self.fields.is_empty() && self.alts.is_empty() && self.cases.is_empty() && self.handles.is_empty()
    }
}

#[derive(Clone, Debug)]
struct VarSlot {
    bound: Option<Ty>,
    level: u32,
    cons: Cons,
}

#[derive(Clone, Debug)]
struct EvarSlot {
    parent: u32,
    atoms: u8,
    level: u32,
}

/// A generalized type: variables with level greater than `level` are quantified.
#[derive(Clone, Debug)]
pub struct Scheme {
    pub ty: Ty,
    pub level: u32,
}

impl Scheme {
    pub fn mono(ty: Ty) -> Scheme {
        Scheme { ty, level: u32::MAX }
    }
}

/// Level used for the variables of builtin schemes; they are always quantified.
pub const GENERIC_LEVEL: u32 = 1_000_000;

#[derive(Default)]
pub struct TypeCtx {
    vars: Vec<VarSlot>,
    evars: Vec<EvarSlot>,
}

pub type TResult<T> = Result<T, String>;

fn sorted<T>(mut v: Vec<(Rc<str>, T)>) -> Vec<(Rc<str>, T)> {
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

impl TypeCtx {
    pub fn fresh(&mut self, level: u32) -> Ty {
        self.fresh_with(level, Cons::default())
    }
    pub fn fresh_with(&mut self, level: u32, cons: Cons) -> Ty {
        self.vars.push(VarSlot { bound: None, level, cons });
        Ty::Var((self.vars.len() - 1) as u32)
    }
    pub fn fresh_eff(&mut self, level: u32, atoms: u8) -> u32 {
        let id = self.evars.len() as u32;
        self.evars.push(EvarSlot { parent: id, atoms, level });
        id
    }

    pub fn eff_root(&self, mut e: u32) -> u32 {
        while self.evars[e as usize].parent != e {
            e = self.evars[e as usize].parent;
        }
        e
    }
    pub fn eff_atoms(&self, e: u32) -> u8 {
        self.evars[self.eff_root(e) as usize].atoms
    }
    pub fn eff_add(&mut self, e: u32, atoms: u8) {
        let r = self.eff_root(e);
        self.evars[r as usize].atoms |= atoms;
    }
    /// Merge two effect variables (conservative join of their requirements).
    pub fn eff_merge(&mut self, a: u32, b: u32) {
        let ra = self.eff_root(a);
        let rb = self.eff_root(b);
        if ra == rb {
            return;
        }
        let atoms = self.evars[ra as usize].atoms | self.evars[rb as usize].atoms;
        let level = self.evars[ra as usize].level.min(self.evars[rb as usize].level);
        self.evars[rb as usize].parent = ra;
        self.evars[ra as usize].atoms = atoms;
        self.evars[ra as usize].level = level;
    }

    pub fn var_cons(&self, v: u32) -> &Cons {
        &self.vars[v as usize].cons
    }

    /// Follow variable bindings at the top of a type.
    pub fn resolve(&self, ty: &Ty) -> Ty {
        let mut t = ty.clone();
        while let Ty::Var(v) = t {
            match &self.vars[v as usize].bound {
                Some(b) => t = b.clone(),
                None => return t,
            }
        }
        t
    }

    /// Convert a ground descriptor to an inference type (function types get fresh effect variables).
    pub fn from_desc(&mut self, d: &TypeDesc, level: u32) -> Ty {
        match d {
            TypeDesc::Unit => UNIT,
            TypeDesc::Bool => BOOL,
            TypeDesc::Int => INT,
            TypeDesc::Float => FLOAT,
            TypeDesc::String => STRING,
            TypeDesc::Type => TYPE,
            TypeDesc::List(t) => list(self.from_desc(t, level)),
            TypeDesc::Func(args, r) => {
                let params = args.iter().map(|a| self.from_desc(a, level)).collect();
                let ret = self.from_desc(r, level);
                let eff = self.fresh_eff(level, 0);
                Ty::Fn(Box::new(FnTy { params, ret, eff }))
            }
            TypeDesc::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), self.from_desc(t, level))).collect()),
            TypeDesc::Variant(fs) => Ty::Variant(fs.iter().map(|(n, t)| (n.clone(), self.from_desc(t, level))).collect()),
            TypeDesc::Opaque(id) => Ty::Opaque(id.clone()),
        }
    }

    /// Convert to a ground descriptor (ignoring effects). Fails if variables remain.
    pub fn to_desc(&self, ty: &Ty) -> Option<TypeDesc> {
        Some(match self.resolve(ty) {
            Ty::Var(_) => return None,
            Ty::Prim(p) => match p {
                Prim::Unit => TypeDesc::Unit,
                Prim::Bool => TypeDesc::Bool,
                Prim::Int => TypeDesc::Int,
                Prim::Float => TypeDesc::Float,
                Prim::String => TypeDesc::String,
                Prim::Type => TypeDesc::Type,
            },
            Ty::List(t) => TypeDesc::List(Rc::new(self.to_desc(&t)?)),
            Ty::Fn(f) => TypeDesc::Func(f.params.iter().map(|p| self.to_desc(p)).collect::<Option<_>>()?, Rc::new(self.to_desc(&f.ret)?)),
            Ty::Record(fs) => TypeDesc::Record(fs.iter().map(|(n, t)| Some((n.clone(), self.to_desc(t)?))).collect::<Option<_>>()?),
            Ty::Variant(fs) => TypeDesc::Variant(fs.iter().map(|(n, t)| Some((n.clone(), self.to_desc(t)?))).collect::<Option<_>>()?),
            Ty::Opaque(id) => TypeDesc::Opaque(id),
        })
    }

    // -----------------------------------------------------------------------------------------------------
    // Instantiation

    pub fn instantiate(&mut self, s: &Scheme, level: u32) -> Ty {
        let mut m = InstMaps::default();
        self.inst(&s.ty, s.level, level, &mut m)
    }

    fn inst_eff(&mut self, e: u32, gen: u32, level: u32, m: &mut InstMaps) -> u32 {
        let r = self.eff_root(e);
        if self.evars[r as usize].level <= gen {
            return r;
        }
        if let Some(x) = m.effs.get(&r) {
            return *x;
        }
        let atoms = self.evars[r as usize].atoms;
        let x = self.fresh_eff(level, atoms);
        m.effs.insert(r, x);
        x
    }

    fn inst_case(&mut self, c: &Case, gen: u32, level: u32, m: &mut InstMaps) -> Case {
        Case { other: self.inst(&c.other, gen, level, m), ret: self.inst(&c.ret, gen, level, m), eff: self.inst_eff(c.eff, gen, level, m) }
    }

    fn inst(&mut self, ty: &Ty, gen: u32, level: u32, m: &mut InstMaps) -> Ty {
        match self.resolve(ty) {
            Ty::Var(v) => {
                if self.vars[v as usize].level <= gen {
                    return Ty::Var(v);
                }
                if let Some(t) = m.tys.get(&v) {
                    return t.clone();
                }
                let fresh = self.fresh(level);
                m.tys.insert(v, fresh.clone());
                // Instantiate the constraints after recording the variable: constraints may refer back to it.
                let cons = self.vars[v as usize].cons.clone();
                let new_cons = Cons {
                    num: cons.num,
                    eq: cons.eq,
                    ord: cons.ord,
                    scalar: cons.scalar,
                    fields: cons.fields.iter().map(|(n, t)| (n.clone(), self.inst(t, gen, level, m))).collect(),
                    alts: cons.alts.iter().map(|(n, t)| (n.clone(), self.inst(t, gen, level, m))).collect(),
                    cases: cons.cases.iter().map(|c| self.inst_case(c, gen, level, m)).collect(),
                    handles: cons.handles.iter().map(|c| self.inst_case(c, gen, level, m)).collect(),
                };
                if let Ty::Var(f) = fresh {
                    self.vars[f as usize].cons = new_cons;
                }
                fresh
            }
            Ty::Prim(p) => Ty::Prim(p),
            Ty::List(t) => Ty::List(Box::new(self.inst(&t, gen, level, m))),
            Ty::Fn(f) => {
                let params = f.params.iter().map(|p| self.inst(p, gen, level, m)).collect();
                let ret = self.inst(&f.ret, gen, level, m);
                let eff = self.inst_eff(f.eff, gen, level, m);
                Ty::Fn(Box::new(FnTy { params, ret, eff }))
            }
            Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), self.inst(t, gen, level, m))).collect()),
            Ty::Variant(fs) => Ty::Variant(fs.iter().map(|(n, t)| (n.clone(), self.inst(t, gen, level, m))).collect()),
            Ty::Opaque(id) => Ty::Opaque(id),
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // Unification

    pub fn unify(&mut self, a: &Ty, b: &Ty) -> TResult<()> {
        let a = self.resolve(a);
        let b = self.resolve(b);
        match (&a, &b) {
            (Ty::Var(x), Ty::Var(y)) if x == y => Ok(()),
            (Ty::Var(x), Ty::Var(y)) => {
                let (x, y) = (*x, *y);
                let cx = std::mem::take(&mut self.vars[x as usize].cons);
                let level = self.vars[x as usize].level.min(self.vars[y as usize].level);
                self.vars[x as usize].bound = Some(Ty::Var(y));
                self.vars[y as usize].level = level;
                self.add_cons(y, cx)
            }
            (Ty::Var(x), _) => self.bind(*x, &b),
            (_, Ty::Var(y)) => self.bind(*y, &a),
            (Ty::Prim(p), Ty::Prim(q)) if p == q => Ok(()),
            (Ty::List(x), Ty::List(y)) => self.unify(x, y),
            (Ty::Fn(f), Ty::Fn(g)) => {
                if f.params.len() != g.params.len() {
                    return Err(format!(
                        "arity mismatch: a {}-argument function versus a {}-argument function ({})",
                        f.params.len(),
                        g.params.len(),
                        self.mismatch(&a, &b)
                    ));
                }
                for (p, q) in f.params.iter().zip(&g.params) {
                    self.unify(p, q)?;
                }
                self.unify(&f.ret, &g.ret)?;
                self.eff_merge(f.eff, g.eff);
                Ok(())
            }
            (Ty::Record(f1), Ty::Record(f2)) | (Ty::Variant(f1), Ty::Variant(f2))
                if f1.len() == f2.len() && f1.iter().zip(f2).all(|(x, y)| x.0 == y.0) =>
            {
                for ((_, t1), (_, t2)) in f1.iter().zip(f2) {
                    self.unify(t1, t2)?;
                }
                Ok(())
            }
            (Ty::Opaque(x), Ty::Opaque(y)) if x == y => Ok(()),
            _ => Err(self.mismatch(&a, &b)),
        }
    }

    fn mismatch(&self, a: &Ty, b: &Ty) -> String {
        let names = &mut Names::default();
        format!("type mismatch: `{}` versus `{}`", self.show_with(a, names), self.show_with(b, names))
    }

    fn bind(&mut self, v: u32, t: &Ty) -> TResult<()> {
        let level = self.vars[v as usize].level;
        if self.occurs_adjust(v, t, level, &mut HashSet::new()) {
            return Err(format!("recursive type: a type variable occurs in `{}`", self.show(t)));
        }
        let cons = std::mem::take(&mut self.vars[v as usize].cons);
        self.vars[v as usize].bound = Some(t.clone());
        self.check_cons(t, cons, level)
    }

    /// Occurs check plus level adjustment of variables in `t` (including those its constraints mention).
    fn occurs_adjust(&mut self, v: u32, t: &Ty, level: u32, seen: &mut HashSet<u32>) -> bool {
        match self.resolve(t) {
            Ty::Var(w) => {
                if w == v {
                    return true;
                }
                if !seen.insert(w) {
                    return false;
                }
                let slot = &mut self.vars[w as usize];
                if slot.level > level {
                    slot.level = level;
                }
                // Field and alternative types are part of the type; a recursive occurrence is an error.
                let inner: Vec<Ty> = slot.cons.fields.iter().chain(&slot.cons.alts).map(|f| f.1.clone()).collect();
                // Case relations only need level adjustment (they refer back to this variable by design).
                let related: Vec<(Ty, Ty, u32)> = slot.cons.cases.iter().chain(&slot.cons.handles).map(|c| (c.other.clone(), c.ret.clone(), c.eff)).collect();
                if inner.iter().any(|f| self.occurs_adjust(v, f, level, seen)) {
                    return true;
                }
                for (o, r, e) in related {
                    self.adjust_eff(e, level);
                    self.adjust_levels(&o, level, seen);
                    self.adjust_levels(&r, level, seen);
                }
                false
            }
            Ty::Prim(_) | Ty::Opaque(_) => false,
            Ty::List(x) => self.occurs_adjust(v, &x, level, seen),
            Ty::Fn(f) => {
                self.adjust_eff(f.eff, level);
                f.params.iter().any(|p| self.occurs_adjust(v, p, level, seen)) || self.occurs_adjust(v, &f.ret, level, seen)
            }
            Ty::Record(fs) | Ty::Variant(fs) => fs.iter().any(|(_, t)| self.occurs_adjust(v, t, level, seen)),
        }
    }

    /// Lower the levels of the variables in `t` to at most `level`, without an occurs check.
    fn adjust_levels(&mut self, t: &Ty, level: u32, seen: &mut HashSet<u32>) {
        // No real variable has this id, so the traversal only adjusts levels.
        const NO_VARIABLE: u32 = u32::MAX;
        let _occurs = self.occurs_adjust(NO_VARIABLE, t, level, seen);
    }

    fn adjust_eff(&mut self, e: u32, level: u32) {
        let r = self.eff_root(e);
        if self.evars[r as usize].level > level {
            self.evars[r as usize].level = level;
        }
    }

    /// Add constraints to an unbound variable (merging with existing ones).
    pub fn add_cons(&mut self, v: u32, c: Cons) -> TResult<()> {
        if c.is_empty() {
            return Ok(());
        }
        let mut pending = Vec::new();
        {
            let slot = &mut self.vars[v as usize];
            slot.cons.num |= c.num;
            slot.cons.eq |= c.eq;
            slot.cons.ord |= c.ord;
            slot.cons.scalar |= c.scalar;
            for (n, t) in c.fields {
                match slot.cons.fields.iter().find(|f| f.0 == n) {
                    Some((_, existing)) => pending.push((existing.clone(), t)),
                    None => slot.cons.fields.push((n, t)),
                }
            }
            for (n, t) in c.alts {
                match slot.cons.alts.iter().find(|f| f.0 == n) {
                    Some((_, existing)) => pending.push((existing.clone(), t)),
                    None => slot.cons.alts.push((n, t)),
                }
            }
            slot.cons.cases.extend(c.cases);
            slot.cons.handles.extend(c.handles);
            if !slot.cons.fields.is_empty() && !slot.cons.alts.is_empty() {
                return Err("a type cannot be both a record (it has fields) and a variant (it has alternatives)".into());
            }
        }
        for (a, b) in pending {
            self.unify(&a, &b)?;
        }
        Ok(())
    }

    /// Check constraints against a type (adding them if it is a variable).
    pub fn check_cons(&mut self, t: &Ty, c: Cons, level: u32) -> TResult<()> {
        if c.is_empty() {
            return Ok(());
        }
        let t = self.resolve(t);
        if let Ty::Var(v) = t {
            return self.add_cons(v, c);
        }
        let shown = || self.show(&t);
        if c.num && !matches!(t, Ty::Prim(Prim::Int) | Ty::Prim(Prim::Float)) {
            return Err(format!("`{}` is not a numeric type (Int or Float)", shown()));
        }
        if c.ord && !matches!(t, Ty::Prim(Prim::Int) | Ty::Prim(Prim::Float) | Ty::Prim(Prim::String)) {
            return Err(format!("`{}` has no ordering (only Int, Float, and String are ordered)", shown()));
        }
        if c.scalar && !matches!(t, Ty::Prim(Prim::Int) | Ty::Prim(Prim::Float) | Ty::Prim(Prim::String) | Ty::Prim(Prim::Bool) | Ty::Prim(Prim::Unit)) {
            return Err(format!("`{}` is not a formattable scalar (String, Int, Float, Bool, or Unit)", shown()));
        }
        if c.eq {
            self.require_eq(&t)?;
        }
        for (name, fty) in c.fields {
            match &t {
                Ty::Record(fs) => match fs.iter().find(|f| f.0 == name) {
                    Some((_, actual)) => {
                        let actual = actual.clone();
                        self.unify(&actual, &fty).map_err(|m| format!("field `{}`: {}", name, m))?
                    }
                    None => return Err(format!("record type `{}` has no field `{}`", self.show(&t), name)),
                },
                _ => return Err(format!("`{}` is not a record type, so it has no field `{}`", self.show(&t), name)),
            }
        }
        for (name, aty) in c.alts {
            match &t {
                Ty::Variant(fs) => match fs.iter().find(|f| f.0 == name) {
                    Some((_, actual)) => {
                        let actual = actual.clone();
                        self.unify(&actual, &aty).map_err(|m| format!("alternative `#{}`: {}", name, m))?
                    }
                    None => return Err(format!("variant type `{}` has no alternative `#{}`", self.show(&t), name)),
                },
                _ => return Err(format!("`{}` is not a variant type, so it has no alternative `#{}`", self.show(&t), name)),
            }
        }
        for case in c.cases {
            match &t {
                Ty::Variant(alts) => {
                    let handlers = Ty::Record(
                        alts.iter()
                            .map(|(n, p)| (n.clone(), Ty::Fn(Box::new(FnTy { params: vec![p.clone()], ret: case.ret.clone(), eff: case.eff }))))
                            .collect(),
                    );
                    self.unify(&case.other, &handlers).map_err(|m| format!("the handlers do not cover exactly the alternatives of `{}`: {}", self.show(&t), m))?;
                }
                _ => return Err(format!("`{}` is not a variant type and cannot be matched", self.show(&t))),
            }
        }
        for case in c.handles {
            match &t {
                Ty::Record(fields) => {
                    let mut alts = Vec::new();
                    for (n, h) in fields.clone() {
                        let p = self.fresh(level);
                        let want = Ty::Fn(Box::new(FnTy { params: vec![p.clone()], ret: case.ret.clone(), eff: case.eff }));
                        self.unify(&h, &want).map_err(|m| format!("handler `{}` must be a one-argument function: {}", n, m))?;
                        alts.push((n, p));
                    }
                    self.unify(&case.other, &Ty::Variant(alts)).map_err(|m| format!("the handlers do not cover exactly the matched alternatives: {}", m))?;
                }
                _ => return Err(format!("the handlers of a match must be a record of functions, not `{}`", self.show(&t))),
            }
        }
        Ok(())
    }

    pub fn require_eq(&mut self, t: &Ty) -> TResult<()> {
        match self.resolve(t) {
            Ty::Var(v) => self.add_cons(v, Cons { eq: true, ..Cons::default() }),
            Ty::Prim(_) => Ok(()),
            Ty::List(x) => self.require_eq(&x),
            Ty::Record(fs) | Ty::Variant(fs) => fs.iter().try_for_each(|(_, t)| self.require_eq(t)),
            other => Err(format!("`{}` does not support equality", self.show(&other))),
        }
    }

    /// Resolve the remaining ambiguity of a top-level type: numeric variables become Int, and a variable known
    /// only through its variant alternatives becomes the closed variant of exactly those alternatives.
    pub fn default_type(&mut self, t: &Ty) {
        self.default_rec(t, &mut HashSet::new())
    }

    fn default_rec(&mut self, t: &Ty, seen: &mut HashSet<u32>) {
        match self.resolve(t) {
            Ty::Var(v) => {
                if !seen.insert(v) {
                    return;
                }
                let c = self.vars[v as usize].cons.clone();
                if c.num {
                    let _ = self.unify(&Ty::Var(v), &INT);
                } else if !c.alts.is_empty() && c.cases.is_empty() && c.handles.is_empty() {
                    let closed = Ty::Variant(sorted(c.alts.clone()));
                    let _ = self.unify(&Ty::Var(v), &closed);
                    self.default_rec(&closed, seen);
                } else {
                    for (_, f) in c.fields.iter().chain(&c.alts) {
                        self.default_rec(f, seen);
                    }
                }
            }
            Ty::Prim(_) | Ty::Opaque(_) => {}
            Ty::List(x) => self.default_rec(&x, seen),
            Ty::Fn(f) => {
                for p in &f.params {
                    self.default_rec(p, seen);
                }
                self.default_rec(&f.ret, seen);
            }
            Ty::Record(fs) | Ty::Variant(fs) => fs.iter().for_each(|(_, t)| self.default_rec(t, seen)),
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // Display

    pub fn show(&self, t: &Ty) -> String {
        self.show_with(t, &mut Names::default())
    }

    pub fn show_with(&self, t: &Ty, names: &mut Names) -> String {
        match self.resolve(t) {
            Ty::Var(v) => {
                let n = names.name(v);
                if names.shown.contains(&v) {
                    return n;
                }
                names.shown.push(v);
                let c = self.vars[v as usize].cons.clone();
                let mut tags = Vec::new();
                for (flag, name) in [(c.num, "Num"), (c.ord, "Ord"), (c.eq, "Eq"), (c.scalar, "Scalar")] {
                    if flag {
                        tags.push(name.to_string());
                    }
                }
                for (f, ft) in &c.fields {
                    tags.push(format!("has .{}: {}", f, self.show_with(ft, names)));
                }
                for (f, ft) in &c.alts {
                    tags.push(format!("has #{}: {}", f, self.show_with(ft, names)));
                }
                if !c.cases.is_empty() || !c.handles.is_empty() {
                    tags.push("matched".into());
                }
                if tags.is_empty() {
                    n
                } else {
                    format!("{}:{}", n, tags.join("+"))
                }
            }
            Ty::Prim(p) => format!("{:?}", p),
            Ty::List(x) => format!("List({})", self.show_with(&x, names)),
            Ty::Fn(f) => {
                let ps: Vec<String> = f.params.iter().map(|p| self.show_with(p, names)).collect();
                let atoms = self.eff_atoms(f.eff);
                let eff = match (atoms & EFF_STATIC != 0, atoms & EFF_RUNTIME != 0) {
                    (false, false) => String::new(),
                    (true, false) => " !Static".into(),
                    (false, true) => " !Runtime".into(),
                    (true, true) => " !Static+Runtime".into(),
                };
                format!("Fn([{}], {}){}", ps.join(", "), self.show_with(&f.ret, names), eff)
            }
            Ty::Record(fs) => format!("{{{}}}", fs.iter().map(|(n, t)| format!("{}: {}", n, self.show_with(t, names))).collect::<Vec<_>>().join(", ")),
            Ty::Variant(fs) if fs.is_empty() => "Never".into(),
            Ty::Variant(fs) => format!("<{}>", fs.iter().map(|(n, t)| format!("{}: {}", n, self.show_with(t, names))).collect::<Vec<_>>().join(", ")),
            Ty::Opaque(id) => format!("Native({:?})", id),
        }
    }
}

#[derive(Default)]
struct InstMaps {
    tys: HashMap<u32, Ty>,
    effs: HashMap<u32, u32>,
}

#[derive(Default)]
pub struct Names {
    map: HashMap<u32, String>,
    shown: Vec<u32>,
}

impl Names {
    fn name(&mut self, v: u32) -> String {
        let n = self.map.len();
        self.map
            .entry(v)
            .or_insert_with(|| {
                let letter = (b'a' + (n % 26) as u8) as char;
                if n < 26 {
                    letter.to_string()
                } else {
                    format!("{}{}", letter, n / 26)
                }
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labeled_normalization() {
        let a = normalize_labeled(vec![(Rc::from("y"), TypeDesc::Int), (Rc::from("x"), TypeDesc::Bool)], "field").unwrap();
        assert_eq!(&*a[0].0, "x");
        assert!(normalize_labeled(vec![(Rc::from("x"), TypeDesc::Int), (Rc::from("x"), TypeDesc::Int)], "field").is_err());
        assert!(normalize_labeled(vec![(Rc::from("1x"), TypeDesc::Int)], "field").is_err());
        assert_eq!(TypeDesc::never().to_string(), "Never");
    }

    #[test]
    fn unify_constraints() {
        let mut t = TypeCtx::default();
        let v = t.fresh_with(1, Cons { num: true, ..Cons::default() });
        assert!(t.unify(&v, &STRING).is_err());
        let w = t.fresh_with(1, Cons { num: true, ..Cons::default() });
        assert!(t.unify(&w, &FLOAT).is_ok());
        let l = t.fresh(1);
        assert!(t.unify(&l, &list(l.clone())).is_err());
    }

    #[test]
    fn field_and_case_constraints() {
        let mut t = TypeCtx::default();
        let f = t.fresh(1);
        let r = t.fresh_with(1, Cons { fields: vec![(Rc::from("x"), f.clone())], ..Cons::default() });
        t.unify(&r, &Ty::Record(vec![(Rc::from("x"), INT)])).unwrap();
        assert!(matches!(t.resolve(&f), Ty::Prim(Prim::Int)));
        let bad = t.fresh_with(1, Cons { fields: vec![(Rc::from("y"), INT)], ..Cons::default() });
        assert!(t.unify(&bad, &Ty::Record(vec![(Rc::from("x"), INT)])).is_err());

        // A handler record determines the variant it matches.
        let ret = t.fresh(1);
        let eff = t.fresh_eff(1, 0);
        let v = t.fresh(1);
        let h = t.fresh_with(1, Cons { handles: vec![Case { other: v.clone(), ret: ret.clone(), eff }], ..Cons::default() });
        let handler = Ty::Fn(Box::new(FnTy { params: vec![STRING], ret: INT, eff: t.fresh_eff(1, 0) }));
        t.unify(&h, &Ty::Record(vec![(Rc::from("a"), handler)])).unwrap();
        assert_eq!(t.to_desc(&v), Some(TypeDesc::Variant(vec![(Rc::from("a"), TypeDesc::String)])));
        assert!(matches!(t.resolve(&ret), Ty::Prim(Prim::Int)));
    }
}
