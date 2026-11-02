//! The embedding surface: check, prepare, and evaluate Lamlet documents inside a host program.
//!
//! This module is the supported API for hosts (see `plans/embedding.md`). Everything a host needs is
//! reachable from here, including re-exports of the value, type, and native-registry types; the other modules
//! of the crate are implementation details and may change in minor releases.
//!
//! ```
//! use lamlet::embed::*;
//!
//! let engine = Engine::new();
//! // `$[% e]` regions and `$name` lookups of static bindings are computed during the check; other regions
//! // run in an instance (`Document::render_with`).
//! let doc = engine.check(SourceFile::new("room.txt", "$[Import* std]$[# side 4]The room is $[% {* side side}] m².")).unwrap();
//! assert_eq!(doc.render(text).unwrap(), "The room is 16 m².");
//!
//! let doc = engine.check(SourceFile::new("curve.lam", "$[Import* std]$[# twice {^ {x} {* 2.0 x}}]")).unwrap();
//! let twice = doc.binding("twice").unwrap();
//! let mut inst = engine.instance(&doc).unwrap();
//! let y: f64 = inst.call_typed(&twice, (1.25,)).unwrap();
//! assert_eq!(y, 2.5);
//! ```
//!
//! Everything here is single-threaded: `Engine`, `Document`, and `Instance` are `!Send`. [`Data`] is the
//! plain, `Send + Sync` form of a value for hosts that move results between threads.

use crate::host::HostContext;
use crate::source::{self, Segment, SourceMap};
use crate::types::normalize_labeled;
use crate::value::format_float;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub use crate::host::{opaque, NativeCtx, NativeEntry, NativeRegistry, TIME_INPUT};
pub use crate::package::{Exports, MemoryResolver, PackageSource, Resolver};
pub use crate::source::{DiagKind, Diagnostic, Location};
pub use crate::syntax::Availability;
pub use crate::types::{TypeDesc, MAX_TYPE_DEPTH};
pub use crate::value::{display, List, Value};
pub use crate::vm::{Limits, Stats};
pub use crate::Options;

// ---------------------------------------------------------------------------------------------------------
// Sources and errors

/// A document's text and name, optionally positioned inside a larger host file.
#[derive(Clone, Debug)]
pub struct SourceFile {
    name: String,
    text: String,
    line: usize,
    column: usize,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceFile {
        SourceFile { name: name.into(), text: text.into(), line: 1, column: 1 }
    }

    /// Place the text at a 1-based line and column of the host's file, so that every location and diagnostic
    /// names the host file's position (byte offsets stay relative to the text).
    pub fn at(mut self, line: usize, column: usize) -> SourceFile {
        self.line = line.max(1);
        self.column = column.max(1);
        self
    }

    fn into_source(self) -> source::SourceFile {
        source::SourceFile::new(self.name, self.text).with_origin(self.line, self.column)
    }
}

/// Which step of the host boundary failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The document did not check or prepare (syntax, names, types, stages, packages, static evaluation).
    Check,
    /// The program could not be linked against the engine's natives.
    Link,
    /// Evaluation or a call failed (runtime errors, budgets, cancellation).
    Runtime,
    /// A value crossing the boundary had the wrong shape or type.
    Value,
    /// A native function or type could not be registered (a repeated id, an unregistered opaque type).
    Registry,
    /// A bug: Lamlet (or a host native) panicked. The panic was caught; an instance it happened in is unusable,
    /// and later calls on it fail with this kind too.
    Internal,
}

/// The one error type of the embedding surface. It carries the diagnostics and the sources they point into,
/// and displays as rendered diagnostics.
#[derive(Clone)]
pub struct Error {
    kind: ErrorKind,
    diagnostics: Vec<Diagnostic>,
    sources: Rc<SourceMap>,
}

impl Error {
    fn new(kind: ErrorKind, diagnostics: Vec<Diagnostic>, sources: Rc<SourceMap>) -> Error {
        Error { kind, diagnostics, sources }
    }

    fn value(message: impl Into<String>) -> Error {
        let d = Diagnostic::new(DiagKind::Type, message, None);
        Error::new(ErrorKind::Value, vec![d], Rc::new(SourceMap::default()))
    }

    fn internal(message: impl Into<String>) -> Error {
        let d = Diagnostic::new(DiagKind::Static, message, None);
        Error::new(ErrorKind::Internal, vec![d], Rc::new(SourceMap::default()))
    }

    fn registry(message: impl Into<String>) -> Error {
        let d = Diagnostic::new(DiagKind::Native, message, None);
        Error::new(ErrorKind::Registry, vec![d], Rc::new(SourceMap::default()))
    }

    fn runtime(e: crate::vm::RuntimeError, sources: &Rc<SourceMap>) -> Error {
        Error::new(ErrorKind::Runtime, vec![e.to_diagnostic()], sources.clone())
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The location of each diagnostic's primary span, in the host file's coordinates when the document was
    /// given an origin (`SourceFile::at`).
    pub fn locations(&self) -> Vec<Option<Location>> {
        self.diagnostics.iter().map(|d| d.span.map(|s| self.sources.location(s))).collect()
    }

    pub fn render(&self) -> String {
        self.diagnostics.iter().map(|d| d.render(&self.sources)).collect::<Vec<_>>().join("\n")
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.render())
    }
}

impl std::error::Error for Error {}

impl From<crate::Failure> for Error {
    fn from(f: crate::Failure) -> Error {
        Error::new(ErrorKind::Check, f.diagnostics, f.sources)
    }
}

/// Run `f`, turning a panic into an `Internal` error, so that no input can unwind into the host.
fn guard<T>(what: &str, f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            Err(Error::internal(format!("internal error during {}: {} (please report this as a Lamlet bug)", what, msg)))
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Engine

/// Packages, natives, and options shared by every document a host checks.
pub struct Engine {
    resolver: Rc<dyn Resolver>,
    natives: Rc<NativeRegistry>,
    options: Options,
    limits: Limits,
    /// `std` checked once, for full checks (`[0]`) and interface checks (`[1]`). Cleared whenever the resolver,
    /// natives, or options change, and rebuilt when the resolver's `std` source changes.
    preludes: std::cell::RefCell<[Option<CachedPrelude>; 2]>,
}

struct CachedPrelude {
    std_hash: Option<u64>,
    /// `None` when `std` cannot be preloaded; checks then start fresh.
    prelude: Option<Rc<crate::Prelude>>,
    natives: Rc<NativeRegistry>,
}

impl Default for Engine {
    fn default() -> Engine {
        Engine::new()
    }
}

impl Engine {
    /// An engine with `std` available as a package and the standard natives (`std.time/now`) registered.
    pub fn new() -> Engine {
        Engine {
            resolver: MemoryResolver::new().into_rc(),
            natives: Rc::new(NativeRegistry::new()),
            options: Options::default(),
            limits: Limits::default(),
            preludes: Default::default(),
        }
    }

    /// Replace the package resolver. A `MemoryResolver` keeps `std` available unless it replaces it.
    pub fn set_resolver(&mut self, resolver: impl Resolver + 'static) -> &mut Engine {
        self.resolver = Rc::new(resolver);
        *self.preludes.get_mut() = Default::default();
        self
    }

    /// The native registry, for registering host functions and types. Documents and instances created
    /// earlier keep the registry they were made with.
    pub fn natives_mut(&mut self) -> &mut NativeRegistry {
        *self.preludes.get_mut() = Default::default();
        Rc::make_mut(&mut self.natives)
    }

    pub fn natives(&self) -> &NativeRegistry {
        &self.natives
    }

    /// Checking options: whether every native declaration must be backed by a registered implementation, and
    /// the budgets of static preparation.
    pub fn options_mut(&mut self) -> &mut Options {
        *self.preludes.get_mut() = Default::default();
        &mut self.options
    }

    /// The default budgets of new instances.
    pub fn limits_mut(&mut self) -> &mut Limits {
        &mut self.limits
    }

    /// Check a document and prepare its static regions. `Pure` and `Static` natives may run during
    /// preparation.
    ///
    /// The engine checks `std` once and starts every check from that state, so checking many documents does not
    /// check `std` again each time. The result is the same as checking from scratch.
    pub fn check(&self, file: SourceFile) -> Result<Document, Error> {
        self.check_mode(file, false)
    }

    /// Check a document against native metadata only, with no implementation called and none required:
    /// the build-time check. A static region (`[% ...]`) that calls a native cannot be prepared without its
    /// implementation, so it fails this check; runtime calls check against the metadata.
    pub fn check_interface(&self, file: SourceFile) -> Result<Document, Error> {
        self.check_mode(file, true)
    }

    fn check_mode(&self, file: SourceFile, interface: bool) -> Result<Document, Error> {
        let name = file.name.clone();
        guard("checking", || {
            let (prelude, natives, options) = self.prelude(interface);
            let source = file.into_source();
            let inner = match prelude {
                Some(p) => p.check(source)?,
                None => crate::check_source(source, self.resolver.clone(), natives, &options)?,
            };
            Ok(Document { inner, name })
        })
    }

    /// The preloaded `std` for full or interface checks, built on first use, with the natives and options
    /// it was built with.
    fn prelude(&self, interface: bool) -> (Option<Rc<crate::Prelude>>, Rc<NativeRegistry>, Options) {
        let options = if interface { Options { require_linked: false, ..self.options.clone() } } else { self.options.clone() };
        let std_hash = self.resolver.resolve("std").map(|p| crate::fnv1a(p.source.as_bytes()));
        let mut preludes = self.preludes.borrow_mut();
        let slot = &mut preludes[interface as usize];
        if !matches!(slot, Some(c) if c.std_hash == std_hash) {
            let natives = if interface { Rc::new(self.natives.interface_only()) } else { self.natives.clone() };
            let prelude = std_hash.and_then(|_| crate::Prelude::new(self.resolver.clone(), natives.clone(), &options)).map(Rc::new);
            *slot = Some(CachedPrelude { std_hash, prelude, natives });
        }
        let c = slot.as_ref().expect("the prelude slot was just filled");
        (c.prelude.clone(), c.natives.clone(), options)
    }

    /// A new runtime instance of a checked document, linked against this engine's natives.
    pub fn instance(&self, doc: &Document) -> Result<Instance, Error> {
        let sources = doc.inner.sources.clone();
        guard("linking", || {
            let mut inner = crate::Instance::new(doc.inner.program.clone(), self.natives.clone())
                .map_err(|d| Error::new(ErrorKind::Link, vec![d], sources.clone()))?;
            inner.set_limits(self.limits.clone());
            Ok(Instance { inner, sources: sources.clone(), poisoned: false })
        })
    }
}

// ---------------------------------------------------------------------------------------------------------
// Documents

/// An active region of a document.
#[derive(Clone, Debug)]
pub struct Region {
    /// Index among the document's regions, in source order; the key of `Instance::evaluate` results.
    pub index: u32,
    /// An identity derived from the document's name, content hash, and the region's position. It changes when
    /// the document is edited; it is meant for caches keyed by a document version.
    pub stable_id: String,
    /// The region's type, when it is ground (not for declarations, or for generic values).
    pub ty: Option<TypeDesc>,
    /// The region's type as text (empty for declarations).
    pub type_text: String,
    pub at: Location,
    span: source::Span,
}

/// One piece of a checked document, in source order.
#[derive(Clone, Debug)]
pub enum Part {
    /// Prose (`$$` already unescaped).
    Text { text: String, at: Location },
    /// A disabled region (`$;{...}`, `$;[...]`); hosts normally do not display it.
    Comment { text: String, at: Location },
    /// A declaration region; it has no value.
    Declaration { region: Region },
    /// An expression region. `value` is `Some` when it was fixed during preparation and `None` when it must be
    /// evaluated at runtime (a residual region).
    Region { region: Region, value: Option<Value> },
}

/// A top-level binding whose value was fixed during preparation.
#[derive(Clone)]
pub struct Binding {
    pub name: String,
    /// The binding's type, when it is ground (not for generic functions).
    pub ty: Option<TypeDesc>,
    pub type_text: String,
    pub value: Value,
    pub doc: Option<String>,
}

impl fmt::Debug for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Binding({} : {})", self.name, self.type_text)
    }
}

/// A checked and prepared document.
pub struct Document {
    inner: crate::Document,
    name: String,
}

impl Document {
    /// The document's prose, declarations, regions, and comments, in source order.
    pub fn parts(&self) -> Vec<Part> {
        let mut out = Vec::new();
        let mut regions = self.inner.regions.iter();
        for seg in &self.inner.segments {
            match seg {
                Segment::Text { span, text } => out.push(Part::Text { text: text.clone(), at: self.location(*span) }),
                Segment::Comment { span, text } => out.push(Part::Comment { text: text.clone(), at: self.location(*span) }),
                Segment::Region { .. } => {
                    let Some(r) = regions.next() else { break };
                    let region = Region {
                        index: r.index,
                        stable_id: r.stable_id.clone(),
                        ty: r.ty_desc.clone(),
                        type_text: r.ty.clone().unwrap_or_default(),
                        at: self.location(r.span),
                        span: r.span,
                    };
                    out.push(if r.is_declaration {
                        Part::Declaration { region }
                    } else {
                        Part::Region { region, value: r.static_value.clone() }
                    });
                }
            }
        }
        out
    }

    /// The region with this index.
    pub fn region(&self, index: u32) -> Option<Region> {
        self.parts().into_iter().find_map(|p| match p {
            Part::Declaration { region } | Part::Region { region, .. } if region.index == index => Some(region),
            _ => None,
        })
    }

    /// A static top-level binding by name.
    pub fn binding(&self, name: &str) -> Result<Binding, Error> {
        self.bindings().into_iter().find(|b| b.name == name).ok_or_else(|| {
            Error::value(format!("the document has no static top-level binding `{}`", name))
        })
    }

    /// Every static top-level binding, sorted by name.
    pub fn bindings(&self) -> Vec<Binding> {
        self.inner
            .static_bindings
            .iter()
            .map(|b| Binding { name: b.name.clone(), ty: b.ty_desc.clone(), type_text: b.ty.clone(), value: b.value.clone(), doc: b.doc.clone() })
            .collect()
    }

    /// The doc comment of a name in the document's scope (macros as `!name`).
    pub fn doc(&self, name: &str) -> Option<&str> {
        self.inner.docs.get(name).map(|s| s.as_str())
    }

    /// Names and versions of the packages the document loaded, in load order.
    pub fn dependencies(&self) -> Vec<(String, String)> {
        self.inner.dependencies.iter().map(|(n, v, _)| (n.clone(), v.clone())).collect()
    }

    /// Identity of the compiled program: source, dependencies, language and bytecode versions, natives.
    pub fn fingerprint(&self) -> u64 {
        self.inner.program.fingerprint
    }

    /// Prose with each expression region replaced by `format(region, value)`. Declarations and comments
    /// produce nothing, and a line that held only declarations, comments, and whitespace is removed with its
    /// line break. Fails if a region is residual; use [`Document::render_with`] for those.
    pub fn render(&self, format: impl FnMut(&Region, &Value) -> Result<String, String>) -> Result<String, Error> {
        self.render_parts(&HashMap::new(), format)
    }

    /// Like [`Document::render`], evaluating the document in `instance` first so residual regions have values.
    pub fn render_with(&self, instance: &mut Instance, format: impl FnMut(&Region, &Value) -> Result<String, String>) -> Result<String, Error> {
        let values: HashMap<u32, Value> = instance.evaluate()?.into_iter().collect();
        self.render_parts(&values, format)
    }

    fn render_parts(&self, values: &HashMap<u32, Value>, mut format: impl FnMut(&Region, &Value) -> Result<String, String>) -> Result<String, Error> {
        let mut lines = LineWriter::default();
        for part in self.parts() {
            match part {
                Part::Text { text, .. } => lines.text(&text),
                Part::Comment { .. } | Part::Declaration { .. } => lines.invisible(),
                Part::Region { region, value } => {
                    let Some(v) = value.as_ref().or_else(|| values.get(&region.index)) else {
                        return Err(self.region_error(&region, "is evaluated at runtime; render it with `render_with`"));
                    };
                    let s = format(&region, v).map_err(|m| self.region_error(&region, &m))?;
                    lines.value(&s);
                }
            }
        }
        Ok(lines.finish())
    }

    fn region_error(&self, region: &Region, message: &str) -> Error {
        let d = Diagnostic::new(DiagKind::Type, format!("region {} {}", region.index, message), Some(region.span));
        Error::new(ErrorKind::Value, vec![d], self.inner.sources.clone())
    }

    fn location(&self, span: source::Span) -> Location {
        self.inner.sources.location(span)
    }

    /// The underlying document of the internal API. It is not covered by the stability promise.
    #[doc(hidden)]
    pub fn internal(&self) -> &crate::Document {
        &self.inner
    }
}

/// Text of a scalar value, for [`Document::render`]: strings as written, numbers and booleans in their
/// display form, unit as nothing. Other values have no text form and fail.
pub fn text(_region: &Region, value: &Value) -> Result<String, String> {
    match value {
        Value::Str(s) => Ok(s.to_string()),
        Value::Int(n) => Ok(n.to_string()),
        Value::Float(f) => Ok(format_float(*f)),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Unit => Ok(String::new()),
        other => Err(format!("has no text form (a {}: {})", other.type_name(), display(other))),
    }
}

/// Builds rendered text line by line, dropping lines that held only invisible parts and whitespace.
#[derive(Default)]
struct LineWriter {
    out: String,
    line: String,
    invisible: bool,
    visible: bool,
}

impl LineWriter {
    fn text(&mut self, text: &str) {
        let mut rest = text;
        while let Some(i) = rest.find('\n') {
            self.line.push_str(&rest[..=i]);
            self.end_line();
            rest = &rest[i + 1..];
        }
        self.line.push_str(rest);
    }

    fn invisible(&mut self) {
        self.invisible = true;
    }

    fn value(&mut self, s: &str) {
        self.visible = true;
        self.text(s);
    }

    fn end_line(&mut self) {
        let blank = self.invisible && !self.visible && self.line.trim().is_empty();
        if !blank {
            self.out.push_str(&self.line);
        }
        self.line.clear();
        self.invisible = false;
        self.visible = false;
    }

    fn finish(mut self) -> String {
        self.end_line();
        self.out
    }
}

// ---------------------------------------------------------------------------------------------------------
// Instances

/// A runtime instance of a document: package state, host inputs, capabilities, and budgets. It can evaluate
/// the document and call its functions any number of times.
pub struct Instance {
    inner: crate::Instance,
    sources: Rc<SourceMap>,
    /// Set when a panic was caught during a call: the VM's stacks may be inconsistent.
    poisoned: bool,
}

impl Instance {
    /// Set a host input that natives read (`NativeCtx::input`). Inputs persist until changed or removed.
    pub fn set_input(&mut self, key: &str, value: impl IntoLamlet) -> Result<&mut Instance, Error> {
        let v = value.into_lamlet().map_err(Error::value)?;
        self.inner.context_mut().inputs.insert(key.into(), v);
        Ok(self)
    }

    pub fn remove_input(&mut self, key: &str) -> &mut Instance {
        self.inner.context_mut().inputs.remove(key);
        self
    }

    /// Supply the Unix timestamp that `std.time` returns.
    pub fn set_time(&mut self, unix_seconds: i64) -> &mut Instance {
        self.inner.context_mut().inputs.insert(TIME_INPUT.into(), Value::Int(unix_seconds));
        self
    }

    /// Grant a capability that natives may require.
    pub fn grant(&mut self, capability: &str) -> &mut Instance {
        self.inner.context_mut().capabilities.insert(capability.into());
        self
    }

    pub fn revoke(&mut self, capability: &str) -> &mut Instance {
        self.inner.context_mut().capabilities.remove(capability);
        self
    }

    /// Budgets for every later evaluation and call.
    pub fn set_limits(&mut self, limits: Limits) -> &mut Instance {
        self.inner.set_limits(limits);
        self
    }

    /// A flag that, when set from any thread, stops the running evaluation with a limit error.
    pub fn cancel_flag(&mut self) -> Arc<AtomicBool> {
        self.inner.cancel_flag()
    }

    /// Evaluate the whole document, returning each expression region's value by region index. Fails
    /// atomically: no partial results.
    pub fn evaluate(&mut self) -> Result<Vec<(u32, Value)>, Error> {
        self.run("evaluation", |i| i.inner.evaluate_current().map_err(|e| Error::runtime(e, &i.sources)))
    }

    /// Run a VM operation under the panic guard, poisoning the instance if it panics.
    fn run<T>(&mut self, what: &str, f: impl FnOnce(&mut Instance) -> Result<T, Error>) -> Result<T, Error> {
        if self.poisoned {
            return Err(Error::internal("this instance is unusable after an internal error; create a new one"));
        }
        let r = guard(what, || f(self));
        if matches!(&r, Err(e) if e.kind == ErrorKind::Internal) {
            self.poisoned = true;
        }
        r
    }

    /// Call a function binding with arguments. Argument values are checked against the binding's type when
    /// it is ground.
    pub fn call(&mut self, f: &Binding, args: Vec<Value>) -> Result<Value, Error> {
        if let Some(TypeDesc::Func(params, _)) = &f.ty {
            if params.len() != args.len() {
                return Err(Error::value(format!("`{}` takes {} arguments, not {}", f.name, params.len(), args.len())));
            }
            for (i, (p, a)) in params.iter().zip(&args).enumerate() {
                if !crate::host::conforms(a, p) {
                    return Err(Error::value(format!("argument {} of `{}` must be {}, not {}", i + 1, f.name, p, display(a))));
                }
            }
        } else if !f.value.is_callable() {
            return Err(Error::value(format!("`{}` is a {}, not a function", f.name, f.value.type_name())));
        }
        self.call_value(&f.value, args)
    }

    /// Call any function value (for example one found in a record a call returned).
    pub fn call_value(&mut self, f: &Value, args: Vec<Value>) -> Result<Value, Error> {
        if !f.is_callable() {
            return Err(Error::value(format!("a {} is not a function", f.type_name())));
        }
        self.run("a call", |i| i.inner.call_current(f, args).map_err(|e| Error::runtime(e, &i.sources)))
    }

    /// Call a function binding with Rust arguments (a tuple) and convert its result to a Rust type.
    pub fn call_typed<A: IntoArgs, R: FromLamlet>(&mut self, f: &Binding, args: A) -> Result<R, Error> {
        let args = args.into_args().map_err(Error::value)?;
        let v = self.call(f, args)?;
        R::from_lamlet(&v).map_err(|m| Error::value(format!("result of `{}`: {}", f.name, m)))
    }

    /// Release package state and execution roots; the next call initializes packages again.
    pub fn reset(&mut self) {
        self.inner.reset();
    }

    /// Counters of the last evaluation or call.
    pub fn stats(&self) -> Stats {
        self.inner.stats()
    }

    /// The host context in the form the internal API takes. Not covered by the stability promise.
    #[doc(hidden)]
    pub fn context(&mut self) -> &mut HostContext {
        self.inner.context_mut()
    }
}

// ---------------------------------------------------------------------------------------------------------
// Values in and out

/// Rust values that convert to Lamlet values.
pub trait IntoLamlet {
    fn into_lamlet(self) -> Result<Value, String>;
}

/// Rust values that Lamlet values convert to.
pub trait FromLamlet: Sized {
    fn from_lamlet(v: &Value) -> Result<Self, String>;
}

fn mismatch<T>(want: &str, v: &Value) -> Result<T, String> {
    Err(format!("expected {}, found {}", want, display(v)))
}

impl IntoLamlet for Value {
    fn into_lamlet(self) -> Result<Value, String> {
        Ok(self)
    }
}
impl FromLamlet for Value {
    fn from_lamlet(v: &Value) -> Result<Value, String> {
        Ok(v.clone())
    }
}

impl IntoLamlet for () {
    fn into_lamlet(self) -> Result<Value, String> {
        Ok(Value::Unit)
    }
}
impl FromLamlet for () {
    fn from_lamlet(v: &Value) -> Result<(), String> {
        match v {
            Value::Unit => Ok(()),
            _ => mismatch("Unit", v),
        }
    }
}

impl IntoLamlet for bool {
    fn into_lamlet(self) -> Result<Value, String> {
        Ok(Value::Bool(self))
    }
}
impl FromLamlet for bool {
    fn from_lamlet(v: &Value) -> Result<bool, String> {
        match v {
            Value::Bool(b) => Ok(*b),
            _ => mismatch("Bool", v),
        }
    }
}

impl IntoLamlet for i64 {
    fn into_lamlet(self) -> Result<Value, String> {
        Ok(Value::Int(self))
    }
}
impl FromLamlet for i64 {
    fn from_lamlet(v: &Value) -> Result<i64, String> {
        match v {
            Value::Int(n) => Ok(*n),
            _ => mismatch("Int", v),
        }
    }
}

impl IntoLamlet for i32 {
    fn into_lamlet(self) -> Result<Value, String> {
        Ok(Value::Int(self as i64))
    }
}

/// A Lamlet Float: finite, with negative zero normalized to zero.
fn float(x: f64) -> Result<Value, String> {
    if !x.is_finite() {
        return Err(format!("{} is not a finite Float", x));
    }
    Ok(Value::Float(if x == 0.0 { 0.0 } else { x }))
}

impl IntoLamlet for f64 {
    fn into_lamlet(self) -> Result<Value, String> {
        float(self)
    }
}
impl FromLamlet for f64 {
    fn from_lamlet(v: &Value) -> Result<f64, String> {
        match v {
            Value::Float(f) => Ok(*f),
            _ => mismatch("Float", v),
        }
    }
}

impl IntoLamlet for String {
    fn into_lamlet(self) -> Result<Value, String> {
        Ok(Value::str(&self))
    }
}
impl IntoLamlet for &str {
    fn into_lamlet(self) -> Result<Value, String> {
        Ok(Value::str(self))
    }
}
impl FromLamlet for String {
    fn from_lamlet(v: &Value) -> Result<String, String> {
        match v {
            Value::Str(s) => Ok(s.to_string()),
            _ => mismatch("String", v),
        }
    }
}

impl<T: IntoLamlet> IntoLamlet for Vec<T> {
    fn into_lamlet(self) -> Result<Value, String> {
        let items = self.into_iter().map(T::into_lamlet).collect::<Result<Vec<_>, _>>()?;
        Ok(Value::List(List::from_vec(items)))
    }
}
impl<T: FromLamlet> FromLamlet for Vec<T> {
    fn from_lamlet(v: &Value) -> Result<Vec<T>, String> {
        match v {
            Value::List(l) => l.iter().map(|x| T::from_lamlet(&x)).collect(),
            _ => mismatch("a List", v),
        }
    }
}

/// `Option<T>` is std's option, `<none: Unit, some: T>`.
impl<T: IntoLamlet> IntoLamlet for Option<T> {
    fn into_lamlet(self) -> Result<Value, String> {
        Ok(match self {
            Some(x) => Value::variant("some", x.into_lamlet()?),
            None => Value::variant("none", Value::Unit),
        })
    }
}
impl<T: FromLamlet> FromLamlet for Option<T> {
    fn from_lamlet(v: &Value) -> Result<Option<T>, String> {
        match v {
            Value::Variant(x) if &*x.0 == "some" => Ok(Some(T::from_lamlet(&x.1)?)),
            Value::Variant(x) if &*x.0 == "none" => Ok(None),
            _ => mismatch("an option (<none: Unit, some: A>)", v),
        }
    }
}

/// A pair `(A, B)` is std's pair, the record `{first: A, second: B}`.
impl<A: IntoLamlet, B: IntoLamlet> IntoLamlet for (A, B) {
    fn into_lamlet(self) -> Result<Value, String> {
        Ok(Value::record(vec![(Rc::from("first"), self.0.into_lamlet()?), (Rc::from("second"), self.1.into_lamlet()?)]))
    }
}
impl<A: FromLamlet, B: FromLamlet> FromLamlet for (A, B) {
    fn from_lamlet(v: &Value) -> Result<(A, B), String> {
        match (v.field("first"), v.field("second")) {
            (Some(a), Some(b)) if matches!(v, Value::Record(fs) if fs.len() == 2) => Ok((A::from_lamlet(a)?, B::from_lamlet(b)?)),
            _ => mismatch("a pair ({first, second})", v),
        }
    }
}

impl IntoLamlet for Data {
    fn into_lamlet(self) -> Result<Value, String> {
        self.to_value()
    }
}
impl FromLamlet for Data {
    fn from_lamlet(v: &Value) -> Result<Data, String> {
        Data::from_value(v)
    }
}

/// An argument list for [`Instance::call_typed`]: a tuple of Rust values, `()` for none, or a `Vec<Value>`.
pub trait IntoArgs {
    fn into_args(self) -> Result<Vec<Value>, String>;
}

impl IntoArgs for Vec<Value> {
    fn into_args(self) -> Result<Vec<Value>, String> {
        Ok(self)
    }
}

impl IntoArgs for () {
    fn into_args(self) -> Result<Vec<Value>, String> {
        Ok(vec![])
    }
}

macro_rules! tuple_args {
    ($($t:ident $i:tt),+) => {
        impl<$($t: IntoLamlet),+> IntoArgs for ($($t,)+) {
            fn into_args(self) -> Result<Vec<Value>, String> {
                Ok(vec![$(self.$i.into_lamlet()?),+])
            }
        }
    };
}
tuple_args!(A 0);
tuple_args!(A 0, B 1);
tuple_args!(A 0, B 1, C 2);
tuple_args!(A 0, B 1, C 2, D 3);
tuple_args!(A 0, B 1, C 2, D 3, E 4);
tuple_args!(A 0, B 1, C 2, D 3, E 4, F 5);

/// Builds a record value field by field: `record().field("x", 1i64).field("name", "a").build()`.
#[derive(Default)]
pub struct RecordBuilder {
    fields: Vec<(Rc<str>, Value)>,
    error: Option<String>,
}

pub fn record() -> RecordBuilder {
    RecordBuilder::default()
}

impl RecordBuilder {
    pub fn field(mut self, label: &str, value: impl IntoLamlet) -> RecordBuilder {
        if self.error.is_none() {
            match value.into_lamlet() {
                Ok(v) => self.fields.push((Rc::from(label), v)),
                Err(m) => self.error = Some(format!("field `{}`: {}", label, m)),
            }
        }
        self
    }

    /// The record, or an error for a bad field value, an invalid label, or a repeated label.
    pub fn build(self) -> Result<Value, Error> {
        if let Some(m) = self.error {
            return Err(Error::value(m));
        }
        check_labels(self.fields.iter().map(|f| &*f.0), "record field").map_err(Error::value)?;
        Ok(Value::record(self.fields))
    }
}

fn check_labels<'a>(labels: impl Iterator<Item = &'a str>, what: &str) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for l in labels {
        if !crate::types::is_label(l) {
            return Err(format!("`{}` is not a valid {} label", l, what));
        }
        if !seen.insert(l) {
            return Err(format!("{} `{}` appears twice", what, l));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Data

/// A plain, owned, `Send + Sync` copy of a Lamlet value that holds no functions or native values. Records
/// are ordered by label.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<Data>),
    Record(BTreeMap<String, Data>),
    /// An alternative's label and its payload.
    Variant(String, Box<Data>),
    Type(TypeData),
}

/// A plain, `Send + Sync` copy of a type descriptor. Record fields and variant alternatives are sorted by
/// label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeData {
    Unit,
    Bool,
    Int,
    Float,
    String,
    Type,
    List(Box<TypeData>),
    Func(Vec<TypeData>, Box<TypeData>),
    Record(Vec<(String, TypeData)>),
    Variant(Vec<(String, TypeData)>),
    /// A registered native type, by type id.
    Opaque(String),
}

impl Data {
    /// Copy a value. Fails on functions and native (opaque) values, naming where in the value they are, and on
    /// values nested deeper than any type may be (`MAX_TYPE_DEPTH`).
    pub fn from_value(v: &Value) -> Result<Data, String> {
        data_at(v, &mut String::new(), 1)
    }

    /// The Lamlet value. Fails on a Float that is not finite, on invalid labels, and on nesting deeper than any
    /// type may be (`MAX_TYPE_DEPTH`).
    pub fn to_value(&self) -> Result<Value, String> {
        self.to_value_at(1)
    }

    fn to_value_at(&self, depth: usize) -> Result<Value, String> {
        if depth > MAX_TYPE_DEPTH {
            return Err(format!("Data nesting limit ({} levels) exceeded", MAX_TYPE_DEPTH));
        }
        let to_value = |x: &Data| x.to_value_at(depth + 1);
        Ok(match self {
            Data::Unit => Value::Unit,
            Data::Bool(b) => Value::Bool(*b),
            Data::Int(n) => Value::Int(*n),
            Data::Float(f) => float(*f)?,
            Data::String(s) => Value::str(s),
            Data::List(xs) => Value::List(List::from_vec(xs.iter().map(to_value).collect::<Result<_, _>>()?)),
            Data::Record(fs) => {
                check_labels(fs.keys().map(|k| k.as_str()), "record field")?;
                Value::record(fs.iter().map(|(k, x)| Ok((Rc::from(k.as_str()), to_value(x)?))).collect::<Result<_, String>>()?)
            }
            Data::Variant(l, x) => {
                check_labels(std::iter::once(l.as_str()), "variant alternative")?;
                Value::variant(l, to_value(x)?)
            }
            Data::Type(t) => Value::Type(Rc::new(t.to_desc()?)),
        })
    }
}

fn data_at(v: &Value, path: &mut String, depth: usize) -> Result<Data, String> {
    let here = |path: &str| if path.is_empty() { String::new() } else { format!(" at {}", path) };
    if depth > MAX_TYPE_DEPTH {
        return Err(format!("the value is nested deeper than the type nesting limit ({} levels){}", MAX_TYPE_DEPTH, here(path)));
    }
    Ok(match v {
        Value::Unit => Data::Unit,
        Value::Bool(b) => Data::Bool(*b),
        Value::Int(n) => Data::Int(*n),
        Value::Float(f) => Data::Float(*f),
        Value::Str(s) => Data::String(s.to_string()),
        Value::List(l) => {
            let mut out = Vec::new();
            for (i, x) in l.iter().enumerate() {
                let len = path.len();
                path.push_str(&format!("[{}]", i));
                out.push(data_at(&x, path, depth + 1)?);
                path.truncate(len);
            }
            Data::List(out)
        }
        Value::Record(fs) => {
            let mut out = BTreeMap::new();
            for (n, x) in fs.iter() {
                let len = path.len();
                path.push_str(&format!(".{}", n));
                out.insert(n.to_string(), data_at(x, path, depth + 1)?);
                path.truncate(len);
            }
            Data::Record(out)
        }
        Value::Variant(x) => {
            let len = path.len();
            path.push_str(&format!("#{}", x.0));
            let payload = data_at(&x.1, path, depth + 1)?;
            path.truncate(len);
            Data::Variant(x.0.to_string(), Box::new(payload))
        }
        Value::Type(t) => Data::Type(TypeData::from_desc(t)),
        Value::Opaque(o) => return Err(format!("a native value of type {:?}{} has no plain-data form", o.type_id, here(path))),
        f => return Err(format!("a {}{} has no plain-data form", f.type_name(), here(path))),
    })
}

impl TypeData {
    pub fn from_desc(t: &TypeDesc) -> TypeData {
        let labeled = |fs: &[(Rc<str>, TypeDesc)]| fs.iter().map(|(n, t)| (n.to_string(), TypeData::from_desc(t))).collect();
        match t {
            TypeDesc::Unit => TypeData::Unit,
            TypeDesc::Bool => TypeData::Bool,
            TypeDesc::Int => TypeData::Int,
            TypeDesc::Float => TypeData::Float,
            TypeDesc::String => TypeData::String,
            TypeDesc::Type => TypeData::Type,
            TypeDesc::List(e) => TypeData::List(Box::new(TypeData::from_desc(e))),
            TypeDesc::Func(ps, r) => TypeData::Func(ps.iter().map(TypeData::from_desc).collect(), Box::new(TypeData::from_desc(r))),
            TypeDesc::Record(fs) => TypeData::Record(labeled(fs)),
            TypeDesc::Variant(fs) => TypeData::Variant(labeled(fs)),
            TypeDesc::Opaque(id) => TypeData::Opaque(id.to_string()),
        }
    }

    /// The descriptor, normalized (labels sorted). Fails on invalid or repeated labels and on nesting deeper
    /// than `MAX_TYPE_DEPTH`.
    pub fn to_desc(&self) -> Result<TypeDesc, String> {
        self.to_desc_at(1)
    }

    fn to_desc_at(&self, depth: usize) -> Result<TypeDesc, String> {
        if depth > MAX_TYPE_DEPTH {
            return Err(format!("type nesting limit ({} levels) exceeded", MAX_TYPE_DEPTH));
        }
        let sub = |t: &TypeData| t.to_desc_at(depth + 1);
        let labeled = |fs: &[(String, TypeData)], what: &str| -> Result<Vec<(Rc<str>, TypeDesc)>, String> {
            check_labels(fs.iter().map(|f| f.0.as_str()), what)?;
            let items = fs.iter().map(|(n, t)| Ok((Rc::from(n.as_str()), sub(t)?))).collect::<Result<Vec<_>, String>>()?;
            normalize_labeled(items, what)
        };
        Ok(match self {
            TypeData::Unit => TypeDesc::Unit,
            TypeData::Bool => TypeDesc::Bool,
            TypeData::Int => TypeDesc::Int,
            TypeData::Float => TypeDesc::Float,
            TypeData::String => TypeDesc::String,
            TypeData::Type => TypeDesc::Type,
            TypeData::List(e) => TypeDesc::List(Rc::new(sub(e)?)),
            TypeData::Func(ps, r) => TypeDesc::Func(ps.iter().map(sub).collect::<Result<_, _>>()?, Rc::new(sub(r)?)),
            TypeData::Record(fs) => TypeDesc::Record(labeled(fs, "record field")?),
            TypeData::Variant(fs) => TypeDesc::Variant(labeled(fs, "variant alternative")?),
            TypeData::Opaque(id) => TypeDesc::Opaque(Rc::from(id.as_str())),
        })
    }
}

impl fmt::Display for TypeData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_desc() {
            Ok(d) => write!(f, "{}", d),
            Err(m) => write!(f, "<invalid type: {}>", m),
        }
    }
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine").field("natives", &self.natives.entries.len()).field("options", &self.options).finish_non_exhaustive()
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Document({:?}, {} regions, fingerprint {:016x})", self.name, self.inner.regions.len(), self.fingerprint())
    }
}

impl fmt::Debug for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Instance")
    }
}

// ---------------------------------------------------------------------------------------------------------
// Natives from Rust types

/// Rust types with a fixed Lamlet type, usable in native signatures ([`NativeRegistry::function`]).
pub trait LamletType {
    fn lamlet_type() -> TypeDesc;
}

impl LamletType for () {
    fn lamlet_type() -> TypeDesc {
        TypeDesc::Unit
    }
}
impl LamletType for bool {
    fn lamlet_type() -> TypeDesc {
        TypeDesc::Bool
    }
}
impl LamletType for i64 {
    fn lamlet_type() -> TypeDesc {
        TypeDesc::Int
    }
}
impl LamletType for f64 {
    fn lamlet_type() -> TypeDesc {
        TypeDesc::Float
    }
}
impl LamletType for String {
    fn lamlet_type() -> TypeDesc {
        TypeDesc::String
    }
}
impl<T: LamletType> LamletType for Vec<T> {
    fn lamlet_type() -> TypeDesc {
        TypeDesc::List(Rc::new(T::lamlet_type()))
    }
}
impl<T: LamletType> LamletType for Option<T> {
    fn lamlet_type() -> TypeDesc {
        TypeDesc::Variant(vec![(Rc::from("none"), TypeDesc::Unit), (Rc::from("some"), T::lamlet_type())])
    }
}
impl<A: LamletType, B: LamletType> LamletType for (A, B) {
    fn lamlet_type() -> TypeDesc {
        TypeDesc::Record(vec![(Rc::from("first"), A::lamlet_type()), (Rc::from("second"), B::lamlet_type())])
    }
}

/// The parameter list of a typed native: `()` or a tuple of [`LamletType`] values.
pub trait NativeArgs: Sized {
    fn types() -> Vec<TypeDesc>;
    fn from_args(args: &[Value]) -> Result<Self, String>;
}

impl NativeArgs for () {
    fn types() -> Vec<TypeDesc> {
        vec![]
    }
    fn from_args(args: &[Value]) -> Result<(), String> {
        match args.len() {
            0 => Ok(()),
            n => Err(format!("expected no arguments, got {}", n)),
        }
    }
}

macro_rules! native_args {
    ($n:literal; $($t:ident $i:tt),+) => {
        impl<$($t: FromLamlet + LamletType),+> NativeArgs for ($($t,)+) {
            fn types() -> Vec<TypeDesc> {
                vec![$($t::lamlet_type()),+]
            }
            fn from_args(args: &[Value]) -> Result<Self, String> {
                if args.len() != $n {
                    return Err(format!("expected {} arguments, got {}", $n, args.len()));
                }
                Ok(($($t::from_lamlet(&args[$i]).map_err(|m| format!("argument {}: {}", $i + 1, m))?,)+))
            }
        }
    };
}
native_args!(1; A 0);
native_args!(2; A 0, B 1);
native_args!(3; A 0, B 1, C 2);
native_args!(4; A 0, B 1, C 2, D 3);
native_args!(5; A 0, B 1, C 2, D 3, E 4);
native_args!(6; A 0, B 1, C 2, D 3, E 4, F 5);

impl NativeRegistry {
    /// Register a Rust closure as a native, with its Lamlet signature taken from the closure's types. The
    /// closure receives the native context and its arguments as a tuple:
    ///
    /// ```
    /// # use lamlet::embed::*;
    /// let mut r = NativeRegistry::new();
    /// r.function("host.geo/area", |_, (w, h): (f64, f64)| Ok(w * h)).pure().doc("Fn([Float, Float], Float). An area.").register()?;
    /// r.function("host.clock/now", |ctx, ()| ctx.input_as::<f64>("clock")).register()?; // Runtime by default
    /// # Ok::<(), Error>(())
    /// ```
    ///
    /// The returned builder sets availability (Runtime unless changed), capabilities, version, and docs, and
    /// [`NativeBuilder::register`] adds the entry.
    pub fn function<A, R, F>(&mut self, api_id: &str, f: F) -> NativeBuilder<'_>
    where
        A: NativeArgs,
        R: IntoLamlet + LamletType,
        F: Fn(&mut NativeCtx, A) -> Result<R, String> + 'static,
    {
        let callback: crate::host::NativeCallback = Rc::new(move |ctx: &mut NativeCtx, args: &[Value]| f(ctx, A::from_args(args)?)?.into_lamlet());
        let entry = NativeEntry {
            api_id: api_id.into(),
            version: "1".into(),
            params: A::types(),
            ret: R::lamlet_type(),
            availability: Availability::Runtime,
            capabilities: vec![],
            callback: Some(callback),
            doc: None,
        };
        NativeBuilder { registry: self, entry }
    }

    /// Register the opaque host type `T` under its `TYPE_ID`.
    pub fn opaque<T: NativeType>(&mut self) -> Result<(), Error> {
        self.register_type(T::TYPE_ID, T::VERSION).map_err(Error::registry)
    }

    /// A Lamlet package declaring every native type and function whose id is `prefix` or starts with
    /// `prefix.` or `prefix/`, plus every native type their signatures use. Each declaration carries the
    /// entry's doc. Names come from the last segment of the id: `host.ui/make-widget` becomes the function
    /// `makeWidget`, and the type `host.ui/widget` becomes `Widget`. Fails when two names collide or a name
    /// is not an identifier.
    pub fn declarations(&self, prefix: &str) -> Result<String, Error> {
        let selected = |id: &str| id == prefix || id.starts_with(&format!("{}.", prefix)) || id.starts_with(&format!("{}/", prefix));
        let mut functions: Vec<&NativeEntry> = self.entries.values().filter(|e| selected(&e.api_id)).collect();
        functions.sort_by(|a, b| a.api_id.cmp(&b.api_id));
        let mut type_ids: std::collections::BTreeSet<String> = self.types.keys().filter(|id| selected(id)).cloned().collect();
        for e in &functions {
            for t in e.params.iter().chain(std::iter::once(&e.ret)) {
                opaque_ids(t, &mut type_ids);
            }
        }
        let mut used: HashMap<String, String> = HashMap::new();
        let mut claim = |name: String, id: &str| -> Result<String, Error> {
            if !crate::types::is_label(&name) || ["true", "false"].contains(&name.as_str()) {
                return Err(Error::registry(format!("`{}` (from {:?}) is not a usable Lamlet name", name, id)));
            }
            if let Some(other) = used.insert(name.clone(), id.to_string()) {
                return Err(Error::registry(format!("{:?} and {:?} would both be declared as `{}`", other, id, name)));
            }
            Ok(name)
        };
        let mut type_names = HashMap::new();
        for id in &type_ids {
            type_names.insert(id.clone(), claim(id_name(id, true), id)?);
        }
        let mut out = format!(
            "Native interface for `{}`, generated by `NativeRegistry::declarations`. Regenerate it instead of editing it.\n\n$[Import std]\n",
            prose(prefix)
        );
        for id in &type_ids {
            out.push_str(&format!("\n$[NativeType {} {:?}]\n", type_names[id], id));
        }
        for e in functions {
            let name = claim(id_name(&e.api_id, false), &e.api_id)?;
            let params: Vec<String> = e.params.iter().map(|t| type_expr(t, &type_names)).collect();
            let sig = format!("{{std::Fn {{' {}}} {}}}", params.join(" "), type_expr(&e.ret, &type_names)).replace("{' }", "{'}");
            out.push('\n');
            if let Some(doc) = &e.doc {
                // A doc is the prose paragraph directly above the declaration: no blank lines, no active `$`.
                let lines: Vec<&str> = doc.lines().filter(|l| !l.trim().is_empty()).collect();
                out.push_str(&prose(&lines.join("\n")));
                out.push('\n');
            }
            out.push_str(&format!("$[Native {} [% {}] {:?} {}]\n", name, sig, e.api_id, e.availability.as_str()));
        }
        Ok(out)
    }
}

/// Prose that Lamlet reads back as the same text.
fn prose(text: &str) -> String {
    text.replace('$', "$$")
}

fn opaque_ids(t: &TypeDesc, out: &mut std::collections::BTreeSet<String>) {
    match t {
        TypeDesc::Opaque(id) => {
            out.insert(id.to_string());
        }
        TypeDesc::List(e) => opaque_ids(e, out),
        TypeDesc::Func(ps, r) => ps.iter().chain(std::iter::once(&**r)).for_each(|t| opaque_ids(t, out)),
        TypeDesc::Record(fs) | TypeDesc::Variant(fs) => fs.iter().for_each(|(_, t)| opaque_ids(t, out)),
        _ => {}
    }
}

/// The Lamlet name for an id's last segment: camelCase for functions, PascalCase for types.
fn id_name(id: &str, capitalize: bool) -> String {
    let last = id.rsplit(['/', '.']).next().unwrap_or(id);
    let mut out = String::new();
    for (i, word) in last.split(['-', '_']).filter(|w| !w.is_empty()).enumerate() {
        let mut cs = word.chars();
        if let Some(c) = cs.next() {
            if i > 0 || capitalize {
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
            out.push_str(cs.as_str());
        }
    }
    out
}

/// A Lamlet expression computing the type, using `std` qualified so the package defines nothing else.
fn type_expr(t: &TypeDesc, names: &HashMap<String, String>) -> String {
    let labeled = |ctor: &str, fs: &[(Rc<str>, TypeDesc)]| {
        let items: Vec<String> = fs.iter().map(|(n, t)| format!("{{std::Field {:?} {}}}", n, type_expr(t, names))).collect();
        format!("{{{} {{' {}}}}}", ctor, items.join(" ")).replace("{' }", "{'}")
    };
    match t {
        TypeDesc::Unit => "std::Unit".into(),
        TypeDesc::Bool => "std::Bool".into(),
        TypeDesc::Int => "std::Int".into(),
        TypeDesc::Float => "std::Float".into(),
        TypeDesc::String => "std::String".into(),
        TypeDesc::Type => "std::Type".into(),
        TypeDesc::List(e) => format!("{{std::List {}}}", type_expr(e, names)),
        TypeDesc::Func(ps, r) => {
            let ps: Vec<String> = ps.iter().map(|p| type_expr(p, names)).collect();
            format!("{{std::Fn {{' {}}} {}}}", ps.join(" "), type_expr(r, names)).replace("{' }", "{'}")
        }
        TypeDesc::Record(fs) => labeled("std::Struct", fs),
        TypeDesc::Variant(fs) if fs.is_empty() => "std::Never".into(),
        TypeDesc::Variant(fs) => labeled("std::Variant", fs),
        TypeDesc::Opaque(id) => names.get(&**id).cloned().unwrap_or_else(|| id_name(id, true)),
    }
}

impl NativeCtx<'_> {
    /// A host input converted to a Rust type.
    pub fn input_as<T: FromLamlet>(&self, key: &str) -> Result<T, String> {
        T::from_lamlet(&self.input(key)?).map_err(|m| format!("host input {:?}: {}", key, m))
    }
}

/// Settings of a native being registered; [`NativeBuilder::register`] adds it.
#[must_use = "a native is only added by `register()`"]
pub struct NativeBuilder<'r> {
    registry: &'r mut NativeRegistry,
    entry: NativeEntry,
}

impl NativeBuilder<'_> {
    /// Callable at any stage, including static preparation (`[% ...]`). For deterministic functions only.
    pub fn pure(mut self) -> Self {
        self.entry.availability = Availability::Pure;
        self
    }

    /// Callable only during static preparation.
    pub fn static_(mut self) -> Self {
        self.entry.availability = Availability::Static;
        self
    }

    /// Callable only at runtime (the default).
    pub fn runtime(mut self) -> Self {
        self.entry.availability = Availability::Runtime;
        self
    }

    /// Require a capability that the instance must grant (`Instance::grant`).
    pub fn capability(mut self, name: &str) -> Self {
        self.entry.capabilities.push(name.into());
        self
    }

    /// The contract version (default "1"); part of the program fingerprint.
    pub fn version(mut self, version: &str) -> Self {
        self.entry.version = version.into();
        self
    }

    /// Documentation for the generated declaration. By convention it starts with the Lamlet type.
    pub fn doc(mut self, doc: &str) -> Self {
        self.entry.doc = Some(doc.into());
        self
    }

    /// Add the native. Fails when the id is already registered or the signature uses an unregistered opaque
    /// type.
    pub fn register(self) -> Result<(), Error> {
        self.registry.register(self.entry).map_err(Error::registry)
    }
}

/// A Rust type that Lamlet code can hold as an opaque value of type `Native("TYPE_ID")`.
pub trait NativeType: std::any::Any {
    const TYPE_ID: &'static str;
    const VERSION: &'static str = "1";
}

/// A typed handle to a host value. Cloning shares the value; it is dropped (running `T`'s `Drop`) when the
/// last handle, in Rust or in Lamlet, is gone.
pub struct Opaque<T> {
    rc: Rc<crate::value::OpaqueValue>,
    _t: std::marker::PhantomData<T>,
}

impl<T: NativeType> Opaque<T> {
    pub fn new(data: T) -> Opaque<T> {
        let rc = Rc::new(crate::value::OpaqueValue { type_id: Rc::from(T::TYPE_ID), data: Box::new(data), release: None });
        Opaque { rc, _t: std::marker::PhantomData }
    }

    pub fn get(&self) -> &T {
        // Every constructor checks the type id and the Rust type.
        self.rc.data.downcast_ref::<T>().expect("opaque handle holds its declared type")
    }
}

impl<T> Clone for Opaque<T> {
    fn clone(&self) -> Self {
        Opaque { rc: self.rc.clone(), _t: std::marker::PhantomData }
    }
}

impl<T: NativeType> std::ops::Deref for Opaque<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get()
    }
}

impl<T: NativeType> LamletType for Opaque<T> {
    fn lamlet_type() -> TypeDesc {
        TypeDesc::Opaque(Rc::from(T::TYPE_ID))
    }
}

impl<T: NativeType> IntoLamlet for Opaque<T> {
    fn into_lamlet(self) -> Result<Value, String> {
        Ok(Value::Opaque(self.rc))
    }
}

impl<T: NativeType> FromLamlet for Opaque<T> {
    fn from_lamlet(v: &Value) -> Result<Opaque<T>, String> {
        match v {
            Value::Opaque(o) if &*o.type_id == T::TYPE_ID && o.data.is::<T>() => Ok(Opaque { rc: o.clone(), _t: std::marker::PhantomData }),
            _ => mismatch(&format!("a native value of type {:?}", T::TYPE_ID), v),
        }
    }
}
