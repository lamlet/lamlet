//! Lamlet: a typed functional language for documents with inline computation.
//!
//! Pipeline: document scanning (`source`) → region parsing (`syntax`) → resolution, type/capability checking,
//! and static evaluation (`check`) → closure conversion (`compile`) → bytecode execution (`vm`).
//!
//! Conceptual host operations:
//! - [`check_document`]: source + resolver + native metadata → checked [`Document`] (static regions prepared).
//! - [`Document::prepared`]: document spans with ignored text, prepared values, and residual runtime regions.
//! - [`Instance::new`]: verified program + implementation registry → isolated reusable instance.
//! - [`Instance::initialize`], [`Instance::evaluate`], [`Instance::call`], [`Instance::reset`].

pub mod builtins;
pub mod bytecode;
pub mod check;
pub mod compile;
pub mod core;
pub mod embed;
pub mod host;
pub mod package;
pub mod source;
pub mod syntax;
pub mod types;
pub mod value;
pub mod vm;

use crate::bytecode::Code;
use crate::check::{check_module, Access, Binding, ModuleMode, RegionInfo, Session};
use crate::source::{DiagKind, Diagnostic, Segment, SourceMap, Span};
use crate::syntax::Availability;
use crate::types::TypeDesc;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub use host::{HostContext, NativeCtx, NativeEntry, NativeRegistry, Stage};
pub use package::{Exports, MemoryResolver, PackageSource, Resolver};
pub use value::Value;
pub use vm::{Limits, RuntimeError};

pub const LANGUAGE_VERSION: &str = "0.1.0";

/// FNV-1a 64-bit hash (stable across platforms and runs).
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Require every Native/NativeType declaration to be backed by a linked registry entry. When false,
    /// interface-only checking accepts unresolved stubs.
    pub require_linked: bool,
    pub static_limits: Limits,
}

impl Default for Options {
    fn default() -> Options {
        Options { require_linked: false, static_limits: Limits::static_default() }
    }
}

/// A native requirement recorded in a compiled program; it is rebound by API ID when instantiating.
#[derive(Clone, Debug)]
pub struct NativeRequirement {
    pub api_id: String,
    pub params: Vec<TypeDesc>,
    pub ret: TypeDesc,
    pub availability: Availability,
}

/// Immutable compiled code shared by any number of instances.
pub struct Program {
    pub entry: Rc<Code>,
    pub n_globals: usize,
    /// Package initializers in dependency order.
    pub package_inits: Vec<(String, Rc<Code>)>,
    pub native_requirements: Vec<NativeRequirement>,
    /// Cache identity: source, dependencies, language/bytecode versions, and native metadata.
    pub fingerprint: u64,
}

/// A top-level binding of a document whose value was available during preparation.
#[derive(Clone)]
pub struct StaticBinding {
    pub name: String,
    pub ty: String,
    /// The binding's type as a ground descriptor, when it has one (not for generic or variadic functions).
    pub ty_desc: Option<TypeDesc>,
    pub value: Value,
    pub doc: Option<String>,
}

/// A checked document.
pub struct Document {
    pub program: Rc<Program>,
    pub segments: Vec<Segment>,
    pub regions: Vec<RegionInfo>,
    pub sources: Rc<SourceMap>,
    /// (package, version, source hash) for every resolved package, in load order.
    pub dependencies: Vec<(String, String, u64)>,
    pub static_bindings: Vec<StaticBinding>,
    /// Docs of every documented name in the document's scope, including imported ones, keyed by the name as
    /// written in this document (macros as `!name`).
    pub docs: std::collections::BTreeMap<String, String>,
}

/// A failed check/preparation.
pub struct Failure {
    pub diagnostics: Vec<Diagnostic>,
    pub sources: Rc<SourceMap>,
}

impl Failure {
    pub fn render(&self) -> String {
        self.diagnostics.iter().map(|d| d.render(&self.sources)).collect::<Vec<_>>().join("\n")
    }
}

impl std::fmt::Debug for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.render())
    }
}

/// Check (and statically prepare) a document.
pub fn check_document(
    name: &str,
    source: &str,
    resolver: Rc<dyn Resolver>,
    natives: Rc<NativeRegistry>,
    options: &Options,
) -> Result<Document, Failure> {
    check_source(source::SourceFile::new(name, source), resolver, natives, options)
}

/// Check (and statically prepare) a document given as a source file, which may carry an origin in a larger
/// host file (`SourceFile::with_origin`).
pub fn check_source(
    file: source::SourceFile,
    resolver: Rc<dyn Resolver>,
    natives: Rc<NativeRegistry>,
    options: &Options,
) -> Result<Document, Failure> {
    check_in(Session::new(resolver, natives, options.require_linked, options.static_limits.clone()), file)
}

/// A checking session with `std` already checked, for checking many documents with the same resolver, natives,
/// and options. Checking from it gives the same result as `check_source`, without checking `std` again.
pub struct Prelude {
    session: Session,
}

impl Prelude {
    /// `None` when `std` cannot be preloaded (it fails to check, cannot be resolved, or imports other packages).
    pub fn new(resolver: Rc<dyn Resolver>, natives: Rc<NativeRegistry>, options: &Options) -> Option<Prelude> {
        Session::prelude(resolver, natives, options.require_linked, options.static_limits.clone()).map(|session| Prelude { session })
    }

    /// Check a document, starting from the preloaded state.
    pub fn check(&self, file: source::SourceFile) -> Result<Document, Failure> {
        check_in(self.session.fork(), file)
    }
}

fn check_in(mut s: Session, file: source::SourceFile) -> Result<Document, Failure> {
    let natives_fp = s.natives.fingerprint();
    let name = file.name.clone();
    let source_hash = fnv1a(file.text.as_bytes());
    let source_len = file.text.len();
    let name = name.as_str();
    let file = s.sources.add_file(file);
    let fail = |d: Diagnostic, s: Session| Failure { diagnostics: vec![d], sources: Rc::new(s.sources) };
    let out = match check_module(&mut s, file, ModuleMode::Document, name) {
        Ok(o) => o,
        Err(d) => return Err(fail(d, s)),
    };
    let span = Span::new(file, 0, source_len);
    let entry = match compile::compile_entry(&format!("<document {}>", name), &out.steps, span) {
        Ok(c) => c,
        Err(m) => return Err(fail(Diagnostic::new(DiagKind::Static, m, Some(span)), s)),
    };
    let mut fp = format!("lamlet {} bytecode {} natives {:016x} source {:016x}", LANGUAGE_VERSION, bytecode::BYTECODE_VERSION, natives_fp, source_hash);
    for (n, v, h) in &s.dependencies {
        fp.push_str(&format!(" dep {}@{}:{:016x}", n, v, h));
    }
    let mut static_bindings = Vec::new();
    let mut names: Vec<&String> = out.scope.keys().collect();
    names.sort();
    for n in names {
        if let Binding::Value { scheme, access: Access::Const(v), def, .. } = &out.scope[n] {
            let doc = s.docs.get(def).cloned();
            let ty_desc = s.tcx.to_desc(&scheme.ty);
            static_bindings.push(StaticBinding { name: n.clone(), ty: s.tcx.show(&scheme.ty), ty_desc, value: v.clone(), doc });
        }
    }
    let mut docs = std::collections::BTreeMap::new();
    for (n, b) in &out.scope {
        if let Some(d) = s.docs.get(&b.def()) {
            docs.insert(n.clone(), d.clone());
        }
    }
    let program = Program {
        entry,
        n_globals: s.n_globals as usize,
        package_inits: s.package_inits.clone(),
        native_requirements: s
            .native_requirements
            .iter()
            .map(|(id, (p, r, a))| NativeRequirement { api_id: id.clone(), params: p.clone(), ret: r.clone(), availability: *a })
            .collect(),
        fingerprint: fnv1a(fp.as_bytes()),
    };
    let dependencies = s.dependencies.clone();
    Ok(Document {
        program: Rc::new(program),
        segments: out.segments,
        regions: out.regions,
        sources: Rc::new(s.sources),
        dependencies,
        static_bindings,
        docs,
    })
}

/// One span of a prepared document.
#[derive(Clone, Debug)]
pub enum PreparedSpan {
    /// Ignored text (display form: `$$` already unescaped).
    Text { span: Span, text: String },
    /// A declaration region (no value).
    Declaration { region: u32, span: Span },
    /// A region whose value was fixed during preparation.
    Value { region: u32, span: Span, ty: String, value: Value, ty_desc: Option<TypeDesc> },
    /// A region that must be evaluated at runtime.
    Residual { region: u32, span: Span, ty: String },
    /// A disabled region (`$;{...}`/`$;[...]`); hosts normally do not display it.
    Comment { span: Span, text: String },
}

impl Document {
    pub fn prepared(&self) -> Vec<PreparedSpan> {
        let mut out = Vec::new();
        let mut regions = self.regions.iter();
        for seg in &self.segments {
            match seg {
                Segment::Text { span, text } => out.push(PreparedSpan::Text { span: *span, text: text.clone() }),
                Segment::Comment { span, text } => out.push(PreparedSpan::Comment { span: *span, text: text.clone() }),
                Segment::Region { .. } => {
                    let r = regions.next().expect("region info");
                    out.push(if r.is_declaration {
                        PreparedSpan::Declaration { region: r.index, span: r.span }
                    } else if let Some(v) = &r.static_value {
                        PreparedSpan::Value {
                            region: r.index,
                            span: r.span,
                            ty: r.ty.clone().unwrap_or_default(),
                            value: v.clone(),
                            ty_desc: r.ty_desc.clone(),
                        }
                    } else {
                        PreparedSpan::Residual { region: r.index, span: r.span, ty: r.ty.clone().unwrap_or_default() }
                    });
                }
            }
        }
        out
    }

    pub fn static_binding(&self, name: &str) -> Option<&Value> {
        self.static_bindings.iter().find(|b| b.name == name).map(|b| &b.value)
    }

    pub fn region_type(&self, index: u32) -> Option<&TypeDesc> {
        self.regions.get(index as usize).and_then(|r| r.ty_desc.as_ref())
    }
}

/// An isolated execution instance of a compiled program.
pub struct Instance {
    program: Rc<Program>,
    vm: vm::Vm,
    initialized: bool,
}

impl Instance {
    /// Verify the program and link its native requirements against an implementation registry. Linking
    /// inspects metadata only; it does not call native functions.
    pub fn new(program: Rc<Program>, natives: Rc<NativeRegistry>) -> Result<Instance, Diagnostic> {
        let verify = |c: &Code| c.verify(program.n_globals).map_err(|m| Diagnostic::new(DiagKind::Static, format!("invalid bytecode: {}", m), None));
        verify(&program.entry)?;
        for (_, c) in &program.package_inits {
            verify(c)?;
        }
        for req in &program.native_requirements {
            let e = natives.entries.get(&req.api_id).ok_or_else(|| {
                Diagnostic::new(DiagKind::Native, format!("native API `{}` required by the program is not registered", req.api_id), None)
            })?;
            if e.params != req.params || e.ret != req.ret || e.availability != req.availability {
                return Err(Diagnostic::new(DiagKind::Native, format!("native API `{}` is registered with an incompatible contract", req.api_id), None));
            }
            if e.callback.is_none() {
                return Err(Diagnostic::new(DiagKind::Native, format!("native API `{}` has no implementation", req.api_id), None));
            }
        }
        let mut vm = vm::Vm::new(Stage::Runtime, natives, Limits::default());
        vm.globals = vec![None; program.n_globals];
        Ok(Instance { program, vm, initialized: false })
    }

    pub fn cancel_flag(&mut self) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.vm.cancel = Some(flag.clone());
        flag
    }

    fn prepare_vm(&mut self, ctx: HostContext, limits: Limits) {
        self.vm.context = ctx;
        self.vm.limits = limits;
    }

    /// The host context used by the `*_current` methods; it persists between calls.
    pub(crate) fn context_mut(&mut self) -> &mut HostContext {
        &mut self.vm.context
    }

    pub(crate) fn set_limits(&mut self, limits: Limits) {
        self.vm.limits = limits;
    }

    /// Initialize package bindings once for this execution context.
    pub fn initialize(&mut self, ctx: HostContext, limits: Limits) -> Result<(), RuntimeError> {
        if self.initialized {
            return Ok(());
        }
        self.prepare_vm(ctx, limits);
        self.initialize_current()
    }

    fn initialize_current(&mut self) -> Result<(), RuntimeError> {
        if self.initialized {
            return Ok(());
        }
        for (_, code) in self.program.package_inits.clone() {
            if let Err(e) = self.vm.run_code(code) {
                for g in self.vm.globals.iter_mut() {
                    *g = None;
                }
                return Err(e);
            }
        }
        self.initialized = true;
        Ok(())
    }

    /// Evaluate the whole document entry with fresh local bindings, returning each expression region's value
    /// in source order. Fails atomically: no partial results are returned.
    pub fn evaluate(&mut self, ctx: HostContext, limits: Limits) -> Result<Vec<(u32, Value)>, RuntimeError> {
        self.prepare_vm(ctx, limits);
        self.evaluate_current()
    }

    /// `evaluate` with the instance's current context and limits.
    pub(crate) fn evaluate_current(&mut self) -> Result<Vec<(u32, Value)>, RuntimeError> {
        self.initialize_current()?;
        self.vm.take_outputs();
        let r = self.vm.run_code(self.program.entry.clone());
        let outputs = self.vm.take_outputs();
        r.map(|_| outputs)
    }

    /// Call a function value (for example a static binding of the document) with explicit arguments.
    pub fn call(&mut self, f: &Value, args: Vec<Value>, ctx: HostContext, limits: Limits) -> Result<Value, RuntimeError> {
        self.prepare_vm(ctx, limits);
        self.call_current(f, args)
    }

    /// `call` with the instance's current context and limits.
    pub(crate) fn call_current(&mut self, f: &Value, args: Vec<Value>) -> Result<Value, RuntimeError> {
        self.initialize_current()?;
        self.vm.call_value(f.clone(), args)
    }

    /// Release execution roots and package state; the compiled program remains shared.
    pub fn reset(&mut self) {
        self.vm.reset();
        self.initialized = false;
    }

    pub fn stats(&self) -> vm::Stats {
        self.vm.stats
    }
}

/// Convenience: check and evaluate a document with the default resolver and no natives.
pub fn run_source(source: &str) -> Result<Vec<(u32, Value)>, String> {
    let natives = Rc::new(NativeRegistry::new());
    let doc = check_document("<input>", source, MemoryResolver::new().into_rc(), natives.clone(), &Options::default()).map_err(|f| f.render())?;
    let mut inst = Instance::new(doc.program.clone(), natives).map_err(|d| d.render(&doc.sources))?;
    inst.evaluate(HostContext::default(), Limits::default()).map_err(|e| e.to_diagnostic().render(&doc.sources))
}
