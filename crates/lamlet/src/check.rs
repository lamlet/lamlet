//! Name resolution, type and capability checking, elaboration to the core IR, and explicit static
//! evaluation. Modules are processed declaration by declaration in source order so that static Type values
//! computed by earlier declarations are available to check later ones.

use crate::builtins::{self, Builtin};
use crate::bytecode::Code;
use crate::compile;
use crate::core::{Core, LocalId, Step};
use crate::host::{NativeRegistry, Stage};
use crate::package::{Exports, Resolver, CORE_PACKAGE};
use crate::source::*;
use crate::syntax::{self, Availability, Decl, Expr, ImportKind, Item};
use crate::types::*;
use crate::value::{LabelFn, NativeFn, Value};
use crate::vm::{Limits, Vm};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

#[derive(Clone)]
pub enum Access {
    Const(Value),
    Local(LocalId),
    Global(u32),
}

#[derive(Clone)]
pub enum Binding {
    Value { def: u64, scheme: Scheme, access: Access, origin: Rc<str> },
    Namespace { def: u64, package: Rc<str>, exports: Rc<HashMap<String, Binding>>, origin: Rc<str> },
    /// A pattern macro, bound under its `!name` key (a separate namespace from values).
    Macro { def: u64, mac: Rc<MacroDef>, origin: Rc<str> },
}

/// A pattern macro with its definition environment (the scopes visible where it was defined).
pub struct MacroDef {
    /// Binding key, `!name`.
    pub name: String,
    pub def: u64,
    pub clauses: Vec<syntax::MacroClause>,
    pub env: Rc<Vec<HashMap<String, Binding>>>,
    pub origin: Rc<str>,
}

/// Maximum nesting of macro expansions, and total expansions per compilation.
const MAX_EXPANSION_DEPTH: usize = 64;
/// Maximum nesting of expressions the checker will descend into (bounds the host stack it uses).
pub const MAX_NESTING: usize = 96;
const MAX_EXPANSIONS: usize = 1_000_000;
/// Total syntax nodes all expansions in one compilation may produce (bounds exponentially growing macros).
const MAX_EXPANSION_NODES: usize = 4_000_000;

impl Binding {
    pub fn def(&self) -> u64 {
        match self {
            Binding::Value { def, .. } | Binding::Namespace { def, .. } | Binding::Macro { def, .. } => *def,
        }
    }
    fn origin(&self) -> &str {
        match self {
            Binding::Value { origin, .. }
            | Binding::Namespace { origin, .. }
            | Binding::Macro { origin, .. } => origin,
        }
    }
}

pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub def: u64,
    pub exports: Rc<HashMap<String, Binding>>,
    pub hash: u64,
}

/// Information about one active region of a checked document.
#[derive(Clone, Debug)]
pub struct RegionInfo {
    pub index: u32,
    pub span: Span,
    pub kind: RegionKind,
    /// Stable identity derived from the source identity and the region's position/structure.
    pub stable_id: String,
    /// `None` for declarations, which have no value.
    pub ty: Option<String>,
    pub ty_desc: Option<TypeDesc>,
    /// Present when the region's value was fixed during preparation.
    pub static_value: Option<Value>,
    pub is_declaration: bool,
    /// The name a declaration region binds (for documentable declarations).
    pub declares: Option<String>,
    /// The declaration's doc: the comment or prose paragraph directly before it.
    pub doc: Option<String>,
}

pub struct Session {
    pub tcx: TypeCtx,
    pub sources: SourceMap,
    pub resolver: Rc<dyn Resolver>,
    pub natives: Rc<NativeRegistry>,
    /// When true, every Native/NativeType declaration must be backed by a registry entry.
    pub require_linked: bool,
    pub static_vm: Vm,
    packages: HashMap<String, Rc<PackageInfo>>,
    loading: Vec<String>,
    pub n_globals: u32,
    pub package_inits: Vec<(String, Rc<Code>)>,
    pub native_requirements: BTreeMap<String, (Vec<TypeDesc>, TypeDesc, Availability)>,
    pub dependencies: Vec<(String, String, u64)>,
    next_def: u64,
    next_local: u32,
    /// Hygiene marks: mark id -> the macro whose expansion introduced it.
    marks: Vec<Rc<MacroDef>>,
    expansions: usize,
    expansion_nodes: usize,
    local_names: HashMap<LocalId, String>,
    /// Docs of documented definitions, by definition identity (survives imports and aliases).
    pub docs: HashMap<u64, String>,
}

type CheckResult<T> = DResult<T>;

fn derr(kind: DiagKind, msg: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::new(kind, msg, Some(span))
}

impl Session {
    pub fn new(resolver: Rc<dyn Resolver>, natives: Rc<NativeRegistry>, require_linked: bool, static_limits: Limits) -> Session {
        Session {
            tcx: TypeCtx::default(),
            sources: SourceMap::default(),
            resolver,
            static_vm: Vm::new(Stage::Static, natives.clone(), static_limits),
            natives,
            require_linked,
            packages: HashMap::new(),
            loading: Vec::new(),
            n_globals: 0,
            package_inits: Vec::new(),
            native_requirements: BTreeMap::new(),
            dependencies: Vec::new(),
            next_def: 1,
            next_local: 0,
            marks: Vec::new(),
            expansions: 0,
            expansion_nodes: 0,
            local_names: HashMap::new(),
            docs: HashMap::new(),
        }
    }

    /// The built-in `core` package: every intrinsic and constant of the intrinsic table, as public exports.
    /// It has no Lamlet source and no initialization; resolvers cannot replace it.
    fn core_package(&mut self) -> Rc<PackageInfo> {
        let mut k = HashMap::new();
        let origin: Rc<str> = Rc::from(format!("core primitive (package `{}`)", CORE_PACKAGE));
        for b in Builtin::all() {
            let def = self.fresh_def();
            let ty = builtins::intrinsic_type(b, &mut self.tcx);
            k.insert(b.name().to_string(), Binding::Value { def, scheme: Scheme { ty, level: 0 }, access: Access::Const(Value::Builtin(b)), origin: origin.clone() });
        }
        for c in builtins::CONSTANTS {
            let def = self.fresh_def();
            let ty = builtins::parse_scheme(c.scheme, &mut self.tcx).unwrap_or_else(|e| panic!("constant K.{}: {}", c.name, e));
            k.insert(c.name.to_string(), Binding::Value { def, scheme: Scheme { ty, level: 0 }, access: Access::Const((c.value)()), origin: origin.clone() });
        }
        let mut fingerprint = String::new();
        for b in Builtin::all() {
            let typing = match b.info().typing {
                builtins::Typing::Scheme(s) => s.to_string(),
                builtins::Typing::Match => "match".to_string(),
            };
            fingerprint.push_str(&format!("{} {} {}\n", b.name(), typing, b.info().arity));
        }
        for c in builtins::CONSTANTS {
            fingerprint.push_str(&format!("{} {}\n", c.name, c.scheme));
        }
        let hash = crate::fnv1a(fingerprint.as_bytes());
        self.dependencies.push((CORE_PACKAGE.to_string(), crate::LANGUAGE_VERSION.to_string(), hash));
        let info = Rc::new(PackageInfo {
            name: CORE_PACKAGE.to_string(),
            version: crate::LANGUAGE_VERSION.to_string(),
            def: self.fresh_def(),
            exports: Rc::new(k),
            hash,
        });
        self.packages.insert(CORE_PACKAGE.to_string(), info.clone());
        info
    }

    fn fresh_def(&mut self) -> u64 {
        self.next_def += 1;
        self.next_def
    }

    /// Resolve a marked key that no enclosing scope binds: the name as it was seen where the macro that
    /// introduced it was defined (the macro itself included, for recursion).
    fn hygienic_fallback(&self, key: &str) -> Option<Binding> {
        let (orig, m) = syntax::split_mark(key)?;
        let mac = self.marks.get(m as usize)?;
        if orig == mac.name {
            return Some(Binding::Macro { def: mac.def, mac: mac.clone(), origin: mac.origin.clone() });
        }
        for scope in mac.env.iter().rev() {
            if let Some(b) = scope.get(orig) {
                return Some(b.clone());
            }
        }
        self.hygienic_fallback(orig)
    }

    fn fresh_local(&mut self, name: &str) -> LocalId {
        self.next_local += 1;
        self.local_names.insert(self.next_local, syntax::base_name(name).to_string());
        self.next_local
    }

    /// Evaluate a closed, checked core expression in the static VM.
    pub fn eval_static(&mut self, core: &Core, span: Span) -> CheckResult<Value> {
        let code = compile::compile_thunk("<static>", core, span).map_err(|m| derr(DiagKind::Static, m, span))?;
        code.verify(self.n_globals as usize).map_err(|m| derr(DiagKind::Static, m, span))?;
        self.static_vm.run_code(code).map_err(|e| {
            let mut d = e.to_diagnostic();
            if d.kind == DiagKind::Runtime {
                d.kind = DiagKind::Static;
            }
            if d.span.is_none() {
                d.span = Some(span);
            }
            d.note("during static evaluation of this expression", Some(span))
        })
    }

    /// Resolve, check, and initialize-plan a package once per session.
    pub fn load_package(&mut self, name: &str, span: Span) -> CheckResult<Rc<PackageInfo>> {
        if let Some(p) = self.packages.get(name) {
            return Ok(p.clone());
        }
        if name == CORE_PACKAGE {
            return Ok(self.core_package());
        }
        if let Some(pos) = self.loading.iter().position(|p| p == name) {
            let mut path: Vec<String> = self.loading[pos..].to_vec();
            path.push(name.to_string());
            return Err(derr(DiagKind::Package, format!("import cycle: {}", path.join(" -> ")), span));
        }
        let src = self
            .resolver
            .resolve(name)
            .ok_or_else(|| derr(DiagKind::Package, format!("package `{}` cannot be resolved", name), span))?;
        self.loading.push(name.to_string());
        let file = self.sources.add(src.file_name.clone(), src.source.clone());
        let result = check_module(self, file, ModuleMode::Package, name);
        self.loading.pop();
        let out = result.map_err(|d| d.note(format!("while checking package `{}`", name), Some(span)))?;

        let mut exports = HashMap::new();
        match &src.exports {
            Exports::All => {
                for n in &out.own {
                    if !n.trim_start_matches('!').starts_with('_') {
                        exports.insert(n.clone(), out.scope[n].clone());
                    }
                }
            }
            Exports::Names(names) => {
                for n in names {
                    match out.scope.get(n) {
                        Some(b) => {
                            exports.insert(n.clone(), b.clone());
                        }
                        None => {
                            return Err(derr(
                                DiagKind::Package,
                                format!("package `{}` declares export `{}`, which it does not define or import", name, n),
                                span,
                            ))
                        }
                    }
                }
            }
        }
        if !out.steps.is_empty() {
            let code = compile::compile_entry(&format!("<init {}>", name), &out.steps, span).map_err(|m| derr(DiagKind::Package, m, span))?;
            self.package_inits.push((name.to_string(), code));
        }
        let hash = crate::fnv1a(src.source.as_bytes());
        self.dependencies.push((name.to_string(), src.version.clone(), hash));
        let info = Rc::new(PackageInfo { name: name.to_string(), version: src.version, def: self.fresh_def(), exports: Rc::new(exports), hash });
        self.packages.insert(name.to_string(), info.clone());
        Ok(info)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModuleMode {
    Document,
    Package,
}

pub struct ModuleOutput {
    pub steps: Vec<Step>,
    pub segments: Vec<Segment>,
    pub regions: Vec<RegionInfo>,
    pub scope: HashMap<String, Binding>,
    pub own: Vec<String>,
}

/// Check one module/document (already added to the session's source map as `file`).
pub fn check_module(s: &mut Session, file: u32, mode: ModuleMode, name: &str) -> CheckResult<ModuleOutput> {
    let text = s.sources.file(file).text.clone();
    let segments = scan_document(file, &text)?;
    // Default import of the core package `K`, in a prelude scope enclosing the module scope.
    let core = s.load_package(CORE_PACKAGE, Span::new(file, 0, 0))?;
    let mut prelude = HashMap::new();
    prelude.insert(
        CORE_PACKAGE.to_string(),
        Binding::Namespace {
            def: core.def,
            package: Rc::from(CORE_PACKAGE),
            exports: core.exports.clone(),
            origin: Rc::from(format!("the default import of package `{}`", CORE_PACKAGE)),
        },
    );
    let mut c = Checker { s, scopes: vec![prelude, HashMap::new()], own: vec![], level: 1, eff: 0, mode, hints: vec![], expand_depth: 0, depth: 0 };
    let mut steps = Vec::new();
    let mut regions = Vec::new();
    let file_hash = crate::fnv1a(text.as_bytes());
    for (si, seg) in segments.iter().enumerate() {
        if let Segment::Region { kind, span, inner } = seg {
            let mut item = syntax::parse_region(file, *kind, &text[inner.start as usize..inner.end as usize], inner.start as usize)?;
            if let Item::Decl(d) = &mut item {
                if let Some((_, slot)) = d.doc_slot() {
                    if slot.is_none() {
                        *slot = crate::source::preceding_doc(&segments, si);
                    }
                }
            }
            let index = regions.len() as u32;
            let stable_id = format!("{}:{:016x}:{}-{}", name, file_hash, span.start, span.end);
            let info = c.top_item(item, index, *span, &mut steps)?;
            regions.push(RegionInfo { index, span: *span, kind: *kind, stable_id, ..info });
        }
    }
    let scope = c.scopes.pop().unwrap();
    let own = c.own;
    Ok(ModuleOutput { steps, segments, regions, scope, own })
}

struct Checker<'s> {
    s: &'s mut Session,
    scopes: Vec<HashMap<String, Binding>>,
    own: Vec<String>,
    level: u32,
    eff: u32,
    mode: ModuleMode,
    hints: Vec<String>,
    expand_depth: usize,
    /// Current expression nesting depth.
    depth: usize,
}

enum DeclOut {
    Done,
    /// A runtime binding the caller must place (local slot, entry local, or package global).
    Bind { name: String, scheme: Scheme, core: Core, def: u64, span: Span },
}

impl<'s> Checker<'s> {
    fn unify_at(&mut self, a: &Ty, b: &Ty, span: Span, what: &str) -> CheckResult<()> {
        self.s.tcx.unify(a, b).map_err(|m| derr(DiagKind::Type, if what.is_empty() { m } else { format!("{}: {}", what, m) }, span))
    }

    fn show(&self, t: &Ty) -> String {
        self.s.tcx.show(t)
    }

    // -----------------------------------------------------------------------------------------------------
    // Scopes

    fn bind(&mut self, name: &str, b: Binding, span: Span) -> CheckResult<()> {
        let scope = self.scopes.last_mut().unwrap();
        if let Some(existing) = scope.get(name) {
            if existing.def() == b.def() {
                return Ok(());
            }
            return Err(derr(
                DiagKind::Name,
                format!("`{}` is already bound in this scope ({}); it cannot also be bound to {}", syntax::base_name(name), existing.origin(), b.origin()),
                span,
            ));
        }
        scope.insert(name.to_string(), b);
        Ok(())
    }

    fn lookup(&self, parts: &[String], span: Span) -> CheckResult<Binding> {
        let first = &parts[0];
        let mut b = match self.lookup_key(first) {
            Some(b) => b,
            None => {
                let shown = syntax::base_name(first);
                let mut d = derr(DiagKind::Name, format!("unknown name `{}`", shown), span);
                if parts.len() == 1 && !shown.starts_with('!') && self.lookup_key(&format!("!{}", first)).is_some() {
                    d = d.note(format!("`!{}` is a macro: write {{!{} ...}}", shown, shown), None);
                }
                // A name written as `x.name` came from the call sugar (its span starts at the dot).
                let text = &self.s.sources.file(span.file).text;
                if text.as_bytes().get(span.start as usize) == Some(&b'.') {
                    d = d.note(
                        format!("`x.{0}` calls a function named `{0}` with x as its first argument; to read a field, write `{{.{0} x}}`", shown),
                        None,
                    );
                }
                return Err(d);
            }
        };
        for (i, part) in parts[1..].iter().enumerate() {
            match b {
                Binding::Namespace { exports, package, .. } => {
                    b = exports.get(part).cloned().ok_or_else(|| {
                        derr(DiagKind::Name, format!("`{}` has no public export `{}`", package, part), span)
                    })?;
                }
                _ => {
                    return Err(derr(
                        DiagKind::Name,
                        format!("`{}` is not a namespace (read a record field with `{{.field record}}`)", parts[..=i].join("::")),
                        span,
                    ))
                }
            }
        }
        Ok(b)
    }

    /// Remember a declaration's doc under the identity of the binding it introduced.
    fn record_doc(&mut self, d: &mut Decl) {
        if let Some((name, slot)) = d.doc_slot() {
            if let (Some(doc), Some(scope)) = (slot.clone(), self.scopes.last()) {
                if let Some(b) = scope.get(name) {
                    self.s.docs.insert(b.def(), doc);
                }
            }
        }
    }

    fn lookup_key(&self, key: &str) -> Option<Binding> {
        for scope in self.scopes.iter().rev() {
            if let Some(x) = scope.get(key) {
                return Some(x.clone());
            }
        }
        self.s.hygienic_fallback(key)
    }

    fn origin(&self, span: Span) -> Rc<str> {
        let loc = self.s.sources.location(span);
        Rc::from(format!("defined at {}", loc))
    }

    // -----------------------------------------------------------------------------------------------------
    // Top level

    fn top_item(&mut self, item: Item, index: u32, span: Span, steps: &mut Vec<Step>) -> CheckResult<RegionInfo> {
        self.level = 1;
        self.eff = self.s.tcx.fresh_eff(1, 0);
        let mut info = RegionInfo {
            index,
            span,
            kind: RegionKind::Brace,
            stable_id: String::new(),
            ty: None,
            ty_desc: None,
            static_value: None,
            is_declaration: false,
            declares: None,
            doc: None,
        };
        match item {
            Item::Decl(mut d) => {
                info.is_declaration = true;
                if let Some((name, slot)) = d.doc_slot() {
                    info.declares = Some(syntax::base_name(name).to_string());
                    info.doc = slot.clone();
                }
                let out = self.decl(&d, true)?;
                self.check_runtime_effect(span)?;
                if let DeclOut::Bind { name, scheme, core, def, span: dspan } = out {
                    let origin = self.origin(dspan);
                    let access = match self.mode {
                        ModuleMode::Document => {
                            let id = self.s.fresh_local(&name);
                            steps.push(Step::Bind(id, core, dspan));
                            Access::Local(id)
                        }
                        ModuleMode::Package => {
                            let g = self.s.n_globals;
                            self.s.n_globals += 1;
                            steps.push(Step::SetGlobal(g, core, dspan));
                            Access::Global(g)
                        }
                    };
                    self.bind(&name, Binding::Value { def, scheme, access, origin }, dspan)?;
                }
                self.record_doc(&mut d);
            }
            Item::Expr(e) => {
                let (t, c) = self.expr(&e)?;
                self.check_runtime_effect(span)?;
                self.s.tcx.default_type(&t);
                info.ty = Some(self.show(&t));
                info.ty_desc = self.s.tcx.to_desc(&t);
                if let Core::Const(v) = &c {
                    info.static_value = Some(v.clone());
                }
                match self.mode {
                    ModuleMode::Document => steps.push(Step::Emit(index, c, span)),
                    ModuleMode::Package => steps.push(Step::Eval(c, span)),
                }
            }
        }
        Ok(info)
    }

    fn check_runtime_effect(&mut self, span: Span) -> CheckResult<()> {
        if self.s.tcx.eff_atoms(self.eff) & EFF_STATIC != 0 {
            return Err(derr(
                DiagKind::Stage,
                "this runtime code requires Static capabilities (it calls a Static native); evaluate it explicitly with [% ...]",
                span,
            ));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------------------
    // Declarations

    fn decl(&mut self, d: &Decl, top: bool) -> CheckResult<DeclOut> {
        match d {
            Decl::Def { name, name_span, init, span, .. } => {
                if self.scopes.last().unwrap().contains_key(name) {
                    let existing = self.scopes.last().unwrap()[name].origin().to_string();
                    return Err(derr(DiagKind::Name, format!("`{}` is already bound in this scope ({})", name, existing), *name_span));
                }
                self.level += 1;
                self.hints.push(name.clone());
                let r = self.expr(init);
                self.hints.pop();
                self.level -= 1;
                let (t, c) = r?;
                let scheme = Scheme { ty: t, level: self.level };
                let def = self.s.fresh_def();
                if top {
                    self.own.push(name.clone());
                }
                match self.static_value_of(&c, *span)? {
                    Some(v) => {
                        let origin = self.origin(*span);
                        self.bind(name, Binding::Value { def, scheme, access: Access::Const(v), origin }, *name_span)?;
                        Ok(DeclOut::Done)
                    }
                    None => Ok(DeclOut::Bind { name: name.clone(), scheme, core: c, def, span: *span }),
                }
            }
            Decl::Import { kind, package, alias, span } => {
                let info = self.s.load_package(package, *span)?;
                let ns = Binding::Namespace {
                    def: info.def,
                    package: Rc::from(package.as_str()),
                    exports: info.exports.clone(),
                    origin: Rc::from(format!("the namespace of package `{}`", package)),
                };
                let mut new: Vec<(String, Binding)> = Vec::new();
                match kind {
                    ImportKind::Qualified => new.push((package.clone(), ns)),
                    ImportKind::Alias => new.push((alias.clone().unwrap(), ns)),
                    ImportKind::Wildcard => {
                        new.push((package.clone(), ns));
                        let mut names: Vec<&String> = info.exports.keys().collect();
                        names.sort();
                        for n in names {
                            let b = info.exports[n].clone();
                            let b = match b {
                                Binding::Value { def, scheme, access, .. } => Binding::Value {
                                    def,
                                    scheme,
                                    access,
                                    origin: Rc::from(format!("`{}` imported from package `{}`", n, package)),
                                },
                                Binding::Macro { def, mac, .. } => {
                                    Binding::Macro { def, mac, origin: Rc::from(format!("`{}` imported from package `{}`", n, package)) }
                                }
                                ns @ Binding::Namespace { .. } => ns,
                            };
                            new.push((n.clone(), b));
                        }
                    }
                }
                // Atomic: check every introduced name before binding any.
                {
                    let scope = self.scopes.last().unwrap();
                    for (n, b) in &new {
                        if let Some(existing) = scope.get(n) {
                            if existing.def() != b.def() {
                                return Err(derr(
                                    DiagKind::Name,
                                    format!(
                                        "import conflict: `{}` is already bound ({}) and this import would bind it to {}",
                                        n,
                                        existing.origin(),
                                        b.origin()
                                    ),
                                    *span,
                                ));
                            }
                        }
                    }
                }
                for (n, b) in new {
                    self.scopes.last_mut().unwrap().insert(n, b);
                }
                Ok(DeclOut::Done)
            }
            Decl::Native { name, name_span, ty, api_id, availability, span, .. } => {
                let desc = self.static_type(ty)?;
                let (params, ret) = match desc {
                    TypeDesc::Func(p, r) => (p, (*r).clone()),
                    other => {
                        return Err(derr(DiagKind::Native, format!("a Native declaration needs a function type, not {}", other), ty.span()))
                    }
                };
                match self.s.natives.entries.get(api_id) {
                    Some(e) => {
                        if e.params != params || e.ret != ret {
                            return Err(derr(
                                DiagKind::Native,
                                format!(
                                    "native `{}` is registered as {} but declared as {}",
                                    api_id,
                                    TypeDesc::Func(e.params.clone(), Rc::new(e.ret.clone())),
                                    TypeDesc::Func(params.clone(), Rc::new(ret.clone()))
                                ),
                                *span,
                            ));
                        }
                        if e.availability != *availability {
                            return Err(derr(
                                DiagKind::Native,
                                format!(
                                    "native `{}` is registered as {}; a declaration cannot change it to {}",
                                    api_id,
                                    e.availability.as_str(),
                                    availability.as_str()
                                ),
                                *span,
                            ));
                        }
                        if self.s.require_linked && e.callback.is_none() {
                            return Err(derr(DiagKind::Native, format!("native `{}` has no linked implementation", api_id), *span));
                        }
                    }
                    None if self.s.require_linked => {
                        return Err(derr(DiagKind::Native, format!("native API `{}` is not registered", api_id), *span))
                    }
                    None => {}
                }
                self.s.native_requirements.insert(api_id.clone(), (params.clone(), ret.clone(), *availability));
                let atoms = match availability {
                    Availability::Pure => 0,
                    Availability::Static => EFF_STATIC,
                    Availability::Runtime => EFF_RUNTIME,
                };
                let fdesc = TypeDesc::Func(params.clone(), Rc::new(ret.clone()));
                let t = self.s.tcx.from_desc(&fdesc, GENERIC_LEVEL);
                if let Ty::Fn(f) = &t {
                    self.s.tcx.eff_add(f.eff, atoms);
                }
                let v = Value::Native(Rc::new(NativeFn {
                    name: name.clone(),
                    api_id: api_id.clone(),
                    params,
                    ret,
                    availability: *availability,
                }));
                let def = self.s.fresh_def();
                let origin = Rc::from(format!("native `{}` declared at {}", api_id, self.s.sources.location(*span)));
                self.bind(name, Binding::Value { def, scheme: Scheme { ty: t, level: 0 }, access: Access::Const(v), origin }, *name_span)?;
                if top {
                    self.own.push(name.clone());
                }
                Ok(DeclOut::Done)
            }
            Decl::NativeType { name, name_span, type_id, span, .. } => {
                if self.s.require_linked && !self.s.natives.types.contains_key(type_id) {
                    return Err(derr(DiagKind::Native, format!("native type `{}` is not registered", type_id), *span));
                }
                let def = self.s.fresh_def();
                let origin = self.origin(*span);
                let v = Value::Type(Rc::new(TypeDesc::Opaque(Rc::from(type_id.as_str()))));
                self.bind(name, Binding::Value { def, scheme: Scheme::mono(TYPE), access: Access::Const(v), origin }, *name_span)?;
                if top {
                    self.own.push(name.clone());
                }
                Ok(DeclOut::Done)
            }
            Decl::Macro { name, name_span, clauses, span, .. } => {
                let def = self.s.fresh_def();
                let origin: Rc<str> = Rc::from(format!("macro `{}` defined at {}", syntax::base_name(name), self.s.sources.location(*span)));
                let mac = Rc::new(MacroDef {
                    name: name.clone(),
                    def,
                    clauses: clauses.clone(),
                    env: Rc::new(self.scopes.clone()),
                    origin: origin.clone(),
                });
                self.bind(name, Binding::Macro { def, mac, origin }, *name_span)?;
                if top {
                    self.own.push(name.clone());
                }
                Ok(DeclOut::Done)
            }
        }
    }

    /// If an elaborated expression's value is available without running effects (a constant, or a closure
    /// that captures only constants), compute it now.
    fn static_value_of(&mut self, c: &Core, span: Span) -> CheckResult<Option<Value>> {
        match c {
            Core::Const(v) => Ok(Some(v.clone())),
            Core::Lambda { .. } if c.free_locals().is_empty() && !c.uses_globals() => Ok(Some(self.s.eval_static(c, span)?)),
            _ => Ok(None),
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // Static helpers

    /// Check an expression that must evaluate statically; returns its type and checked core (not yet run).
    fn check_static_operand(&mut self, e: &Expr) -> CheckResult<(Ty, Core)> {
        let saved = self.eff;
        self.eff = self.s.tcx.fresh_eff(self.level, 0);
        let r = self.expr(e);
        let atoms = self.s.tcx.eff_atoms(self.eff);
        self.eff = saved;
        let (t, c) = r?;
        if atoms & EFF_RUNTIME != 0 {
            return Err(derr(
                DiagKind::Stage,
                "this static expression requires Runtime capabilities and cannot be evaluated during preparation",
                e.span(),
            ));
        }
        if let Some((id, s)) = c.free_locals().first() {
            let name = self.s.local_names.get(id).cloned().unwrap_or_default();
            return Err(derr(
                DiagKind::Stage,
                format!("a static expression cannot use the runtime binding `{}`", name),
                *s,
            )
            .note("only literals, closures over static values, imported constants, and [% ...] results are statically available", None));
        }
        if c.uses_globals() {
            return Err(derr(DiagKind::Stage, "a static expression cannot use a package's runtime-initialized binding", e.span()));
        }
        Ok((t, c))
    }

    fn static_eval(&mut self, e: &Expr) -> CheckResult<(Ty, Value)> {
        let (t, c) = self.check_static_operand(e)?;
        let v = self.s.eval_static(&c, e.span())?;
        Ok((t, v))
    }

    /// A Type argument that must already be static (a Type name, literal binding, or `[% ...]`).
    fn static_type(&mut self, e: &Expr) -> CheckResult<TypeDesc> {
        let (t, c) = self.expr(e)?;
        self.unify_at(&t, &TYPE, e.span(), "expected a Type")?;
        match c {
            Core::Const(Value::Type(d)) => Ok((*d).clone()),
            _ => Err(derr(
                DiagKind::Stage,
                "this Type must be statically known: use a Type name or compute it with [% ...]",
                e.span(),
            )),
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // Expressions

    fn expr(&mut self, e: &Expr) -> CheckResult<(Ty, Core)> {
        if self.depth >= MAX_NESTING {
            return Err(derr(DiagKind::Limit, format!("expression nesting limit ({}) exceeded", MAX_NESTING), e.span()));
        }
        self.depth += 1;
        let r = self.expr_inner(e);
        self.depth -= 1;
        r
    }

    /// A label function (or literal constructor) as a constant with its generic type.
    fn label_fn(&mut self, f: LabelFn) -> CheckResult<(Ty, Core)> {
        let g = builtins::label_scheme(&mut self.s.tcx, &f);
        let level = self.level;
        let t = self.s.tcx.instantiate(&Scheme { ty: g, level: 0 }, level);
        Ok((t, Core::Const(Value::Label(Rc::new(f)))))
    }

    /// A list/record/template literal whose components are all constants is itself a constant (spec §5).
    fn fold_literal(&mut self, c: Core, span: Span) -> CheckResult<Core> {
        if let Core::Call { f, args, .. } = &c {
            if matches!(**f, Core::Const(_)) && args.iter().all(|a| matches!(a, Core::Const(_))) {
                return Ok(Core::Const(self.s.eval_static(&c, span)?));
            }
        }
        Ok(c)
    }

    fn expr_inner(&mut self, e: &Expr) -> CheckResult<(Ty, Core)> {
        match e {
            Expr::Int(n, _) => Ok((INT, Core::Const(Value::Int(*n)))),
            Expr::Float(f, _) => Ok((FLOAT, Core::Const(Value::Float(*f)))),
            Expr::Str(s, _) => Ok((STRING, Core::Const(Value::str(s)))),
            Expr::Bool(b, _) => Ok((BOOL, Core::Const(Value::Bool(*b)))),
            Expr::Name(parts, span) => match self.lookup(parts, *span)? {
                Binding::Value { scheme, access, .. } => {
                    let level = self.level;
                    let t = self.s.tcx.instantiate(&scheme, level);
                    let c = match access {
                        Access::Const(v) => Core::Const(v),
                        Access::Local(id) => Core::Local(id, *span),
                        Access::Global(g) => Core::Global(g, *span),
                    };
                    Ok((t, c))
                }
                Binding::Namespace { .. } => {
                    Err(derr(
                        DiagKind::Name,
                        format!("`{0}` is a package namespace, not a value (an export is written `{0}::name`)", parts.join("::")),
                        *span,
                    ))
                }
                Binding::Macro { .. } => unreachable!("macro keys start with `!`"),
            },
            Expr::Call(f, args, span) => {
                // Optimization (spec §3.3): `K::caseBool` applied to two literal thunks compiles to a jump. The
                // callee is identified by the identity of its resolved constant, never by its name.
                if let [c, Expr::Lambda(None, p1, b1, _), Expr::Lambda(None, p2, b2, _)] = args.as_slice() {
                    if p1.is_empty() && p2.is_empty() && self.is_case_bool(f) {
                        return self.conditional(c, b1, b2);
                    }
                }
                let (tf, cf) = self.expr(f)?;
                self.call(tf, cf, args, *span)
            }
            Expr::Lambda(self_name, params, body, span) => self.lambda(self_name.as_ref(), params, body, *span),
            Expr::Label(kind, label, _) => {
                let l: Rc<str> = Rc::from(label.as_str());
                let f = match kind {
                    syntax::LabelKind::Get => LabelFn::Get(l),
                    syntax::LabelKind::Set => LabelFn::Set(l),
                    syntax::LabelKind::Tag => LabelFn::Tag(l),
                };
                self.label_fn(f)
            }
            Expr::List(items, span) => {
                let ctor = LabelFn::List(items.len());
                let (tf, cf) = self.label_fn(ctor)?;
                let (t, c) = self.call(tf, cf, items, *span)?;
                Ok((t, self.fold_literal(c, *span)?))
            }
            Expr::Record(fields, span) => {
                let ctor = LabelFn::Record(fields.iter().map(|f| Rc::from(f.0.as_str())).collect());
                let (tf, cf) = self.label_fn(ctor)?;
                let args: Vec<Expr> = fields.iter().map(|f| f.2.clone()).collect();
                let (t, c) = self.call(tf, cf, &args, *span)?;
                Ok((t, self.fold_literal(c, *span)?))
            }
            Expr::Ascribe(x, t, _) => {
                let desc = self.static_type(t)?;
                let (tx, cx) = self.expr(x)?;
                let level = self.level;
                let expected = self.s.tcx.from_desc(&desc, level);
                self.unify_at(&tx, &expected, x.span(), &format!("the expression does not have the ascribed type {}", desc))?;
                Ok((tx, cx))
            }
            Expr::Static(x, _) => {
                let (t, v) = self.static_eval(x)?;
                Ok((t, Core::Const(v)))
            }
            Expr::Macro(parts, operands, span) => self.expand(parts, operands, *span),
            Expr::Template(kind, parts, span) => self.template(*kind, parts, *span),
        }
    }

    /// `[f ...]`/`[latex ...]`: append the parts left to right; holes are evaluated once, in order. A Latex
    /// value is the record `{latex: String}`; in a `[latex]` hole it contributes its source, while scalars are
    /// formatted and (for Strings) escaped. Holes of any other type are type errors.
    fn template(&mut self, kind: syntax::TemplateKind, parts: &[syntax::Part], span: Span) -> CheckResult<(Ty, Core)> {
        let prim = |name: &str| Box::new(Core::Const(Value::Builtin(Builtin::by_name(name).expect("intrinsic"))));
        let call1 = |name: &str, a: Core, span: Span| Core::Call { f: prim(name), args: vec![a], span };
        let latex_label: Rc<str> = Rc::from(LATEX_LABEL);
        let mut acc: Option<Core> = None;
        for p in parts {
            let piece = match p {
                syntax::Part::Lit(s) => Core::Const(Value::str(s)),
                syntax::Part::Hole(e) => {
                    let (t, c) = self.expr(e)?;
                    let is_latex = matches!(self.s.tcx.resolve(&t), Ty::Record(fs) if fs.len() == 1 && fs[0].0 == latex_label);
                    if kind == syntax::TemplateKind::Latex && is_latex {
                        let get = Box::new(Core::Const(Value::Label(Rc::new(LabelFn::Get(latex_label.clone())))));
                        Core::Call { f: get, args: vec![c], span: e.span() }
                    } else {
                        let scalar = Cons { scalar: true, ..Cons::default() };
                        let level = self.level;
                        self.s.tcx.check_cons(&t, scalar, level).map_err(|_| {
                            let allowed = match kind {
                                syntax::TemplateKind::Format => "a String, Int, Float, Bool, or Unit",
                                syntax::TemplateKind::Latex => "Latex, a String (escaped), Int, Float, Bool, or Unit",
                            };
                            derr(DiagKind::Type, format!("a template hole must be {}, not {}", allowed, self.show(&t)), e.span())
                        })?;
                        let text = call1("scalarString", c, e.span());
                        match kind {
                            syntax::TemplateKind::Format => text,
                            syntax::TemplateKind::Latex => call1("latexEscape", text, e.span()),
                        }
                    }
                }
            };
            acc = Some(match acc {
                None => piece,
                Some(a) => Core::Call { f: prim("stringAppend"), args: vec![a, piece], span },
            });
        }
        let mut core = acc.unwrap_or_else(|| Core::Const(Value::str("")));
        let ty = match kind {
            syntax::TemplateKind::Format => STRING,
            syntax::TemplateKind::Latex => {
                let ctor = Box::new(Core::Const(Value::Label(Rc::new(LabelFn::Record(vec![latex_label.clone()])))));
                core = Core::Call { f: ctor, args: vec![core], span };
                Ty::Record(vec![(latex_label, STRING)])
            }
        };
        if core.free_locals().is_empty() && !core.uses_globals() && !matches!(core, Core::Const(_)) {
            core = Core::Const(self.s.eval_static(&core, span)?);
        }
        Ok((ty, core))
    }

    fn is_case_bool(&self, g: &Expr) -> bool {
        match g {
            Expr::Name(parts, span) => matches!(
                self.lookup(parts, *span),
                Ok(Binding::Value { access: Access::Const(Value::Builtin(b)), .. }) if Some(b) == Builtin::by_name("caseBool")
            ),
            _ => false,
        }
    }

    /// Check `{K::caseBool c {^ {} yes} {^ {} no}}` as a conditional: both thunk bodies are checked (and their
    /// requirements counted, since one of them runs now), but only the selected one is executed.
    fn conditional(&mut self, c: &Expr, yes: &syntax::Body, no: &syntax::Body) -> CheckResult<(Ty, Core)> {
        let (tc, cc) = self.expr(c)?;
        self.unify_at(&tc, &BOOL, c.span(), "the condition must be Bool")?;
        let (ty, cy) = self.inline_body(yes)?;
        let (tn, cn) = self.inline_body(no)?;
        self.unify_at(&ty, &tn, no.result.span(), "both branches must have the same type")?;
        Ok((ty, Core::If(Box::new(cc), Box::new(cy), Box::new(cn))))
    }

    fn inline_body(&mut self, b: &syntax::Body) -> CheckResult<(Ty, Core)> {
        self.scopes.push(HashMap::new());
        let r = self.body(b);
        self.scopes.pop();
        r
    }

    /// Expand a pattern macro use and check the expansion.
    fn expand(&mut self, parts: &[String], operands: &[syntax::Syntax], span: Span) -> CheckResult<(Ty, Core)> {
        let mut key = parts.to_vec();
        let last = key.len() - 1;
        key[last] = format!("!{}", key[last]);
        let shown = format!("!{}", parts.iter().map(|p| syntax::base_name(p)).collect::<Vec<_>>().join("::"));
        let mac = match self.lookup(&key, span) {
            Ok(Binding::Macro { mac, .. }) => mac,
            Ok(_) => return Err(derr(DiagKind::Name, format!("`{}` is not a macro", shown), span)),
            Err(_) => return Err(derr(DiagKind::Name, format!("unknown macro `{}`", shown), span)),
        };
        if self.expand_depth >= MAX_EXPANSION_DEPTH {
            return Err(derr(DiagKind::Limit, format!("macro expansion depth limit ({}) exceeded", MAX_EXPANSION_DEPTH), span));
        }
        self.s.expansions += 1;
        if self.s.expansions > MAX_EXPANSIONS {
            return Err(derr(DiagKind::Limit, "macro expansion budget exceeded", span));
        }
        for clause in &mac.clauses {
            if let Some(bindings) = syntax::match_clause(&clause.pattern, operands) {
                let m = self.s.marks.len() as u32;
                self.s.marks.push(mac.clone());
                let note = |d: Diagnostic| d.note(format!("in the expansion of `{}`", shown), Some(span));
                let expanded = syntax::instantiate(&clause.template, &bindings, m)
                    .map_err(|e| note(derr(DiagKind::Syntax, e, clause.template.span())))?;
                self.s.expansion_nodes += expanded.size();
                if self.s.expansion_nodes > MAX_EXPANSION_NODES {
                    return Err(note(derr(DiagKind::Limit, "macro expansion produced too much syntax (size budget exceeded)", span)));
                }
                let e = syntax::parse_expr(&expanded).map_err(note)?;
                self.expand_depth += 1;
                let r = self.expr(&e);
                self.expand_depth -= 1;
                return r.map_err(note);
            }
        }
        let mut d = derr(DiagKind::Syntax, format!("no clause of `{}` matches this use", shown), span);
        for c in &mac.clauses {
            d = d.note("a clause expects operands shaped like this", Some(c.pattern.first().map_or(c.template.span(), |p| p.span())));
        }
        Err(d)
    }

    fn lambda(&mut self, self_name: Option<&(String, Span)>, params: &[(String, Span)], body: &syntax::Body, span: Span) -> CheckResult<(Ty, Core)> {
        let level = self.level;
        let mut scope = HashMap::new();
        let ret = self.s.tcx.fresh(level);
        let mut ids = Vec::new();
        let mut tys = Vec::new();
        for (p, _) in params {
            let id = self.s.fresh_local(p);
            let t = self.s.tcx.fresh(level);
            let def = self.s.fresh_def();
            let origin = Rc::from(format!("parameter `{}`", p));
            scope.insert(p.clone(), Binding::Value { def, scheme: Scheme::mono(t.clone()), access: Access::Local(id), origin });
            ids.push(id);
            tys.push(t);
        }
        let e = self.s.tcx.fresh_eff(level, 0);
        let fn_ty = Ty::Fn(Box::new(FnTy { params: tys.clone(), ret: ret.clone(), eff: e }));
        // A recursive lambda's name is bound, monomorphically, to the function itself.
        let self_id = match self_name {
            Some((n, _)) => {
                let id = self.s.fresh_local(n);
                let def = self.s.fresh_def();
                let origin = Rc::from(format!("recursive lambda `{}`", syntax::base_name(n)));
                scope.insert(n.clone(), Binding::Value { def, scheme: Scheme::mono(fn_ty.clone()), access: Access::Local(id), origin });
                Some(id)
            }
            None => None,
        };
        let saved_eff = self.eff;
        self.eff = e;
        self.scopes.push(scope);
        let name = match (self_name, self.hints.last()) {
            (Some((n, _)), _) => syntax::base_name(n).to_string(),
            (None, Some(h)) => syntax::base_name(h).to_string(),
            (None, None) => {
                let loc = self.s.sources.location(span);
                format!("lambda at {}:{}", loc.line, loc.column)
            }
        };
        self.hints.push(format!("{}/lambda", name));
        let r = self.body(body);
        self.hints.pop();
        self.scopes.pop();
        self.eff = saved_eff;
        let (rt, rc) = r?;
        self.unify_at(&ret, &rt, body.result.span(), "the function's result")?;
        let t = fn_ty;
        let core = Core::Lambda { name, self_id, params: ids, body: Box::new(rc), span };
        // Closed lambdas are created once, during checking.
        if core.free_locals().is_empty() && !core.uses_globals() {
            let v = self.s.eval_static(&core, span)?;
            return Ok((t, Core::Const(v)));
        }
        Ok((t, core))
    }

    fn body(&mut self, body: &syntax::Body) -> CheckResult<(Ty, Core)> {
        let mut lets: Vec<(LocalId, Core)> = Vec::new();
        for d in &body.decls {
            if let DeclOut::Bind { name, scheme, core, def, span } = self.decl(d, false)? {
                let id = self.s.fresh_local(&name);
                let origin = self.origin(span);
                self.bind(&name, Binding::Value { def, scheme, access: Access::Local(id), origin }, span)?;
                lets.push((id, core));
            }
            self.record_doc(&mut d.clone());
        }
        let (t, mut c) = self.expr(&body.result)?;
        for (id, init) in lets.into_iter().rev() {
            c = Core::Let { id, init: Box::new(init), body: Box::new(c) };
        }
        Ok((t, c))
    }

    fn call(&mut self, tf: Ty, cf: Core, args: &[Expr], span: Span) -> CheckResult<(Ty, Core)> {
        let ft = self.s.tcx.resolve(&tf);
        let mut cores = Vec::with_capacity(args.len());
        let ret = match ft {
            Ty::Fn(f) => {
                if args.len() != f.params.len() {
                    // A namespace passed as an argument (`std.map` reads as `{map std}`) is the real error, not the arity.
                    for a in args {
                        if let Expr::Name(parts, nspan) = a {
                            if let Ok(Binding::Namespace { .. }) = self.lookup(parts, *nspan) {
                                self.expr(a)?;
                            }
                        }
                    }
                    return Err(derr(
                        DiagKind::Type,
                        format!("this function expects {} arguments, but {} were given", f.params.len(), args.len()),
                        span,
                    )
                    .note(format!("its type is {}", self.show(&Ty::Fn(f.clone()))), None));
                }
                for (i, (a, p)) in args.iter().zip(&f.params).enumerate() {
                    let (ta, ca) = self.expr(a)?;
                    self.unify_at(p, &ta, a.span(), &format!("argument {}", i + 1))?;
                    cores.push(ca);
                }
                self.s.tcx.eff_merge(f.eff, self.eff);
                f.ret.clone()
            }
            Ty::Var(_) => {
                let mut tys = Vec::new();
                for a in args {
                    let (ta, ca) = self.expr(a)?;
                    tys.push(ta);
                    cores.push(ca);
                }
                let level = self.level;
                let ret = self.s.tcx.fresh(level);
                let e = self.s.tcx.fresh_eff(level, 0);
                let ft = Ty::Fn(Box::new(FnTy { params: tys, ret: ret.clone(), eff: e }));
                self.unify_at(&tf, &ft, span, "")?;
                self.s.tcx.eff_merge(e, self.eff);
                ret
            }
            other => {
                return Err(derr(DiagKind::Type, format!("a value of type {} is not callable", self.show(&other)), span));
            }
        };
        Ok((ret, Core::Call { f: Box::new(cf), args: cores, span }))
    }


}

/// The label of the single field of a Latex value (`{latex: String}`).
pub const LATEX_LABEL: &str = "latex";
