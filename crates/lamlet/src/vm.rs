//! Bytecode execution with explicit frames, budgets, and structured errors.
//!
//! User calls never recurse on the Rust stack: every call pushes a VM frame. Eliminators (`K::caseBool`,
//! `K::caseList`, `K::match`) tail-call their selected handler.

use crate::builtins::{self, Outcome};
use crate::bytecode::{CapSrc, Code, Op};
use crate::host::{conforms, HostContext, NativeCtx, NativeRegistry, Stage};
use crate::source::{DiagKind, Diagnostic, Span};
use crate::syntax::Availability;
use crate::value::*;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Resource limits for one evaluation.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_instructions: u64,
    pub max_frames: usize,
    pub max_stack: usize,
    /// Cumulative allocation budget (approximate bytes) for one evaluation.
    pub max_alloc_bytes: usize,
    pub max_string_bytes: usize,
    pub max_native_calls: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_instructions: 50_000_000,
            max_frames: 200_000,
            max_stack: 4_000_000,
            max_alloc_bytes: 1 << 30,
            max_string_bytes: 16 << 20,
            max_native_calls: 1_000_000,
        }
    }
}

impl Limits {
    /// Limits used for static preparation and Type computation.
    pub fn static_default() -> Limits {
        Limits { max_instructions: 10_000_000, max_frames: 100_000, max_alloc_bytes: 256 << 20, ..Limits::default() }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeError {
    pub kind: DiagKind,
    pub message: String,
    pub span: Option<Span>,
    /// Innermost first: function name and call-site span.
    pub stack: Vec<(String, Span)>,
}

impl RuntimeError {
    pub fn to_diagnostic(&self) -> Diagnostic {
        let mut d = Diagnostic::new(self.kind, self.message.clone(), self.span);
        for (name, span) in self.stack.iter().take(12) {
            d = d.note(format!("in {}", name), Some(*span));
        }
        if self.stack.len() > 12 {
            d = d.note(format!("... {} more frames", self.stack.len() - 12), None);
        }
        d
    }
}

struct Frame {
    closure: Rc<Closure>,
    ip: usize,
    base: usize,
}

/// Counters observed after an evaluation (for diagnostics and benchmarks).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub instructions: u64,
    pub alloc_bytes: usize,
    pub native_calls: u64,
    pub max_frames: usize,
}

pub struct Vm {
    pub stage: Stage,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    entry_depth: usize,
    pub globals: Vec<Option<Value>>,
    pub natives: Rc<NativeRegistry>,
    pub context: HostContext,
    pub limits: Limits,
    pub cancel: Option<Arc<AtomicBool>>,
    outputs: Vec<(u32, Value)>,
    pub stats: Stats,
}

type VResult<T> = Result<T, RuntimeError>;

impl Vm {
    pub fn new(stage: Stage, natives: Rc<NativeRegistry>, limits: Limits) -> Vm {
        Vm {
            stage,
            stack: Vec::new(),
            frames: Vec::new(),
            entry_depth: 0,
            globals: Vec::new(),
            natives,
            context: HostContext::default(),
            limits,
            cancel: None,
            outputs: Vec::new(),
            stats: Stats::default(),
        }
    }

    /// A new VM with the same stage, natives, limits, and globals, and nothing executing. Values are immutable,
    /// so the globals are shared, not copied.
    pub fn fork(&self) -> Vm {
        let mut vm = Vm::new(self.stage, self.natives.clone(), self.limits.clone());
        vm.globals = self.globals.clone();
        vm
    }

    /// Release all execution roots (stack, frames, globals, outputs).
    pub fn reset(&mut self) {
        self.stack = Vec::new();
        self.frames = Vec::new();
        self.outputs.clear();
        for g in self.globals.iter_mut() {
            *g = None;
        }
    }

    pub fn take_outputs(&mut self) -> Vec<(u32, Value)> {
        std::mem::take(&mut self.outputs)
    }

    /// Run a zero-argument code object as a closure with no captures.
    pub fn run_code(&mut self, code: Rc<Code>) -> VResult<Value> {
        let f = Value::Closure(Rc::new(Closure { code, captures: vec![] }));
        self.call_value(f, vec![])
    }

    /// Call any callable value with arguments, running to completion.
    pub fn call_value(&mut self, f: Value, args: Vec<Value>) -> VResult<Value> {
        let saved_stack = self.stack.len();
        let saved_frames = self.frames.len();
        let saved_entry = self.entry_depth;
        self.stats = Stats::default();
        self.entry_depth = self.frames.len();
        let n = args.len();
        self.stack.push(f);
        self.stack.extend(args);
        let result = (|| {
            if let Some(v) = self.call(n as u16)? {
                if let Some(r) = self.deliver(v)? {
                    return Ok(r);
                }
            }
            self.execute()
        })();
        if result.is_err() {
            self.stack.truncate(saved_stack);
            self.frames.truncate(saved_frames);
        }
        self.entry_depth = saved_entry;
        result
    }

    fn error(&self, kind: DiagKind, message: impl Into<String>) -> RuntimeError {
        let mut stack = Vec::new();
        let mut span = None;
        for f in self.frames.iter().rev() {
            let s = f.closure.code.spans[f.ip.saturating_sub(1).min(f.closure.code.spans.len() - 1)];
            if span.is_none() {
                span = Some(s);
            }
            stack.push((f.closure.code.name.clone(), s));
        }
        RuntimeError { kind, message: message.into(), span, stack }
    }

    fn rt(&self, message: impl Into<String>) -> RuntimeError {
        self.error(DiagKind::Runtime, message)
    }

    fn charge(&mut self, bytes: usize) -> VResult<()> {
        self.stats.alloc_bytes += bytes;
        if self.stats.alloc_bytes > self.limits.max_alloc_bytes {
            return Err(self.error(DiagKind::Limit, "allocation budget exceeded"));
        }
        Ok(())
    }

    fn push_frame(&mut self, f: Frame) -> VResult<()> {
        if self.frames.len() - self.entry_depth >= self.limits.max_frames {
            return Err(self.error(DiagKind::Limit, format!("call depth limit ({} frames) exceeded", self.limits.max_frames)));
        }
        self.frames.push(f);
        self.stats.max_frames = self.stats.max_frames.max(self.frames.len() - self.entry_depth);
        Ok(())
    }

    /// Perform a call with the callee and `argc` arguments on the stack. Returns `Some(v)` if the call
    /// completed immediately with a value destined for the current top frame.
    fn call(&mut self, argc: u16) -> VResult<Option<Value>> {
        let argc = argc as usize;
        let callee_idx = self.stack.len() - argc - 1;
        let callee = self.stack[callee_idx].clone();
        match callee {
            Value::Closure(c) => {
                if c.code.arity as usize != argc {
                    return Err(self.rt(format!("{} expects {} arguments, got {}", c.code.name, c.code.arity, argc)));
                }
                let extra = (c.code.n_slots - c.code.arity) as usize;
                if self.stack.len() + extra > self.limits.max_stack {
                    return Err(self.error(DiagKind::Limit, "value stack limit exceeded"));
                }
                for _ in 0..extra {
                    self.stack.push(Value::Unit);
                }
                self.push_frame(Frame { closure: c, ip: 0, base: callee_idx + 1 })?;
                Ok(None)
            }
            Value::Label(l) => {
                if l.arity() != argc {
                    return Err(self.rt(format!("this label function expects {} arguments, got {}", l.arity(), argc)));
                }
                let r = builtins::call_label(&l, &self.stack[callee_idx + 1..]);
                self.stack.truncate(callee_idx);
                let v = r.map_err(|m| self.rt(m))?;
                self.charge(alloc_size(&v))?;
                Ok(Some(v))
            }
            Value::Builtin(b) => {
                let outcome = builtins::call(b, &self.stack[callee_idx + 1..]);
                self.stack.truncate(callee_idx);
                let outcome = outcome.map_err(|m| self.rt(m))?;
                match outcome {
                    Outcome::Value(v) => {
                        let size = alloc_size(&v);
                        self.charge(size)?;
                        if let Value::Str(s) = &v {
                            if s.len() > self.limits.max_string_bytes {
                                return Err(self.error(DiagKind::Limit, "String size limit exceeded"));
                            }
                        }
                        Ok(Some(v))
                    }
                    Outcome::Call(f, args) => {
                        let n = args.len();
                        self.stack.push(f);
                        self.stack.extend(args);
                        self.call(n as u16)
                    }
                }
            }
            Value::Native(n) => {
                let args = self.stack.split_off(callee_idx + 1);
                self.stack.pop();
                self.call_native(&n, args).map(Some)
            }
            other => Err(self.rt(format!("a {} value is not callable", other.type_name()))),
        }
    }

    fn call_native(&mut self, n: &NativeFn, args: Vec<Value>) -> VResult<Value> {
        match (self.stage, n.availability) {
            (Stage::Static, Availability::Runtime) => {
                return Err(self.error(DiagKind::Stage, format!("Runtime native `{}` cannot be called during static preparation", n.api_id)))
            }
            (Stage::Runtime, Availability::Static) => {
                return Err(self.error(DiagKind::Stage, format!("Static native `{}` cannot be called at runtime", n.api_id)))
            }
            _ => {}
        }
        if args.len() != n.params.len() {
            return Err(self.rt(format!("native `{}` expects {} arguments, got {}", n.name, n.params.len(), args.len())));
        }
        let natives = self.natives.clone();
        let entry = match natives.entries.get(&n.api_id) {
            Some(e) => e,
            None => return Err(self.error(DiagKind::Native, format!("native API `{}` is not linked", n.api_id))),
        };
        let callback = match &entry.callback {
            Some(c) => c.clone(),
            None => return Err(self.error(DiagKind::Native, format!("native API `{}` has no implementation in this registry", n.api_id))),
        };
        for cap in &entry.capabilities {
            if !self.context.capabilities.contains(cap) {
                return Err(self.error(DiagKind::Native, format!("native `{}` requires capability {:?}, which the host context does not grant", n.api_id, cap)));
            }
        }
        for (a, t) in args.iter().zip(&n.params) {
            if !conforms(a, t) {
                return Err(self.error(DiagKind::Native, format!("argument to native `{}` does not conform to {}", n.api_id, t)));
            }
        }
        self.stats.native_calls += 1;
        if self.stats.native_calls > self.limits.max_native_calls {
            return Err(self.error(DiagKind::Limit, "native call budget exceeded"));
        }
        let mut ctx = NativeCtx { stage: self.stage, api_id: &n.api_id, inputs: &self.context.inputs, capabilities: &self.context.capabilities };
        let result = callback(&mut ctx, &args).map_err(|m| self.error(DiagKind::Native, format!("native `{}` failed: {}", n.api_id, m)))?;
        if !conforms(&result, &n.ret) {
            return Err(self.error(DiagKind::Native, format!("native `{}` returned a value that does not conform to {}", n.api_id, n.ret)));
        }
        self.charge(alloc_size(&result))?;
        Ok(result)
    }

    /// Deliver a returned value to the top frame. Returns `Some` when the evaluation is complete.
    fn deliver(&mut self, v: Value) -> VResult<Option<Value>> {
        if self.frames.len() == self.entry_depth {
            return Ok(Some(v));
        }
        self.stack.push(v);
        Ok(None)
    }

    fn execute(&mut self) -> VResult<Value> {
        loop {
            self.stats.instructions += 1;
            if self.stats.instructions > self.limits.max_instructions {
                return Err(self.error(DiagKind::Limit, format!("instruction budget ({}) exhausted", self.limits.max_instructions)));
            }
            if self.stats.instructions & 4095 == 0 {
                if let Some(c) = &self.cancel {
                    if c.load(Ordering::Relaxed) {
                        return Err(self.error(DiagKind::Limit, "evaluation cancelled"));
                    }
                }
            }
            let fi = self.frames.len() - 1;
            let (op, base) = {
                let f = &mut self.frames[fi];
                let op = f.closure.code.ops[f.ip];
                f.ip += 1;
                (op, f.base)
            };
            match op {
                Op::Const(i) => {
                    let v = self.frames[fi].closure.code.consts[i as usize].clone();
                    self.stack.push(v);
                }
                Op::Local(s) => {
                    let v = self.stack[base + s as usize].clone();
                    self.stack.push(v);
                }
                Op::SetLocal(s) => {
                    let v = self.stack.pop().unwrap();
                    self.stack[base + s as usize] = v;
                }
                Op::Capture(c) => {
                    let v = self.frames[fi].closure.captures[c as usize].clone();
                    self.stack.push(v);
                }
                Op::SelfClosure => {
                    let v = Value::Closure(self.frames[fi].closure.clone());
                    self.stack.push(v);
                }
                Op::Global(g) => match &self.globals[g as usize] {
                    Some(v) => {
                        let v = v.clone();
                        self.stack.push(v)
                    }
                    None => return Err(self.rt("a package binding was used before its package was initialized")),
                },
                Op::SetGlobal(g) => {
                    let v = self.stack.pop().unwrap();
                    self.globals[g as usize] = Some(v);
                }
                Op::Closure(ci) => {
                    let (child, captures) = {
                        let closure = &self.frames[fi].closure;
                        let child = closure.code.children[ci as usize].clone();
                        let caps = child
                            .captures
                            .iter()
                            .map(|c| match *c {
                                CapSrc::Local(s) => self.stack[base + s as usize].clone(),
                                CapSrc::Capture(k) => closure.captures[k as usize].clone(),
                                CapSrc::SelfClosure => Value::Closure(closure.clone()),
                            })
                            .collect::<Vec<_>>();
                        (child, caps)
                    };
                    self.charge(32 + 16 * captures.len())?;
                    self.stack.push(Value::Closure(Rc::new(Closure { code: child, captures })));
                }
                Op::Call(n) => {
                    if let Some(v) = self.call(n)? {
                        if let Some(r) = self.deliver(v)? {
                            return Ok(r);
                        }
                    }
                }
                Op::TailCall(n) => {
                    let n = n as usize;
                    // The frame is released before the call, so remember the call site: an error raised
                    // while making the call (a failing native or builtin, an arity mismatch) belongs to it.
                    let site = {
                        let f = &self.frames[fi];
                        (f.closure.code.name.clone(), f.closure.code.spans[f.ip - 1])
                    };
                    let moved = self.stack.split_off(self.stack.len() - n - 1);
                    self.frames.pop();
                    self.stack.truncate(base - 1);
                    self.stack.extend(moved);
                    let called = self.call(n as u16).map_err(|mut e| {
                        e.span = Some(site.1);
                        e.stack.insert(0, site);
                        e
                    });
                    if let Some(v) = called? {
                        if let Some(r) = self.deliver(v)? {
                            return Ok(r);
                        }
                    }
                }
                Op::Return => {
                    let v = self.stack.pop().unwrap();
                    self.frames.pop();
                    self.stack.truncate(base - 1);
                    if let Some(r) = self.deliver(v)? {
                        return Ok(r);
                    }
                }
                Op::Jump(t) => self.set_ip(fi, t),
                Op::JumpIfFalse(t) | Op::JumpIfTrue(t) => {
                    let b = match self.stack.pop().unwrap() {
                        Value::Bool(b) => b,
                        other => return Err(self.rt(format!("condition is {}, not Bool", other.type_name()))),
                    };
                    let jump_on = matches!(op, Op::JumpIfTrue(_));
                    if b == jump_on {
                        self.set_ip(fi, t);
                    }
                }
                Op::Pop => {
                    self.stack.pop();
                }
                Op::Emit(i) => {
                    let v = self.stack.pop().unwrap();
                    self.outputs.push((i, v));
                }
            }
        }
    }

    fn set_ip(&mut self, fi: usize, t: u32) {
        self.frames[fi].ip = t as usize;
    }
}
