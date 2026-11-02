//! Closure conversion and bytecode generation from the checked core.

use crate::bytecode::{CapSrc, Code, Op};
use crate::core::{Core, LocalId, Step};
use crate::source::Span;
use crate::value::Value;
use std::collections::HashMap;
use std::rc::Rc;

struct FnState {
    name: String,
    arity: u16,
    slots: HashMap<LocalId, u16>,
    n_slots: u16,
    captures: Vec<CapSrc>,
    capmap: HashMap<LocalId, u16>,
    self_id: Option<LocalId>,
    ops: Vec<Op>,
    spans: Vec<Span>,
    consts: Vec<Value>,
    children: Vec<Rc<Code>>,
    span: Span,
}

impl FnState {
    fn new(name: String, self_id: Option<LocalId>, params: &[LocalId], span: Span) -> FnState {
        let mut slots = HashMap::new();
        for (i, p) in params.iter().enumerate() {
            slots.insert(*p, i as u16);
        }
        FnState {
            name,
            arity: params.len() as u16,
            slots,
            n_slots: params.len() as u16,
            captures: vec![],
            capmap: HashMap::new(),
            self_id,
            ops: vec![],
            spans: vec![],
            consts: vec![],
            children: vec![],
            span,
        }
    }
    fn finish(self) -> Code {
        Code {
            name: self.name,
            arity: self.arity,
            n_slots: self.n_slots,
            captures: self.captures,
            ops: self.ops,
            spans: self.spans,
            consts: self.consts,
            children: self.children,
            span: self.span,
        }
    }
}

enum Access {
    Local(u16),
    Capture(u16),
    SelfClosure,
}

pub struct Compiler {
    fns: Vec<FnState>,
}

pub type CResult<T> = Result<T, String>;

impl Compiler {
    fn top(&mut self) -> &mut FnState {
        self.fns.last_mut().unwrap()
    }

    fn emit(&mut self, op: Op, span: Span) -> usize {
        let f = self.top();
        f.ops.push(op);
        f.spans.push(span);
        f.ops.len() - 1
    }

    fn patch(&mut self, at: usize) {
        let target = self.top().ops.len() as u32;
        let f = self.top();
        f.ops[at] = match f.ops[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            Op::JumpIfTrue(_) => Op::JumpIfTrue(target),
            o => o,
        };
    }

    fn constant(&mut self, v: Value, span: Span) {
        let f = self.top();
        f.consts.push(v);
        let i = (f.consts.len() - 1) as u32;
        self.emit(Op::Const(i), span);
    }

    fn resolve(&mut self, level: usize, id: LocalId) -> Option<Access> {
        if let Some(s) = self.fns[level].slots.get(&id) {
            return Some(Access::Local(*s));
        }
        if let Some(c) = self.fns[level].capmap.get(&id) {
            return Some(Access::Capture(*c));
        }
        if self.fns[level].self_id == Some(id) {
            return Some(Access::SelfClosure);
        }
        if level == 0 {
            return None;
        }
        let src = match self.resolve(level - 1, id)? {
            Access::Local(s) => CapSrc::Local(s),
            Access::Capture(k) => CapSrc::Capture(k),
            Access::SelfClosure => CapSrc::SelfClosure,
        };
        let f = &mut self.fns[level];
        f.captures.push(src);
        let k = (f.captures.len() - 1) as u16;
        f.capmap.insert(id, k);
        Some(Access::Capture(k))
    }

    fn new_slot(&mut self, id: LocalId) -> CResult<u16> {
        let f = self.top();
        if f.n_slots == u16::MAX {
            return Err("too many local bindings in one function".into());
        }
        let s = f.n_slots;
        f.n_slots += 1;
        f.slots.insert(id, s);
        Ok(s)
    }

    fn expr(&mut self, e: &Core, tail: bool, span: Span) -> CResult<()> {
        match e {
            Core::Const(v) => self.constant(v.clone(), span),
            Core::Local(id, s) => {
                let level = self.fns.len() - 1;
                match self.resolve(level, *id) {
                    Some(Access::Local(slot)) => self.emit(Op::Local(slot), *s),
                    Some(Access::Capture(k)) => self.emit(Op::Capture(k), *s),
                    Some(Access::SelfClosure) => self.emit(Op::SelfClosure, *s),
                    None => return Err(format!("internal error: unbound local {}", id)),
                };
            }
            Core::Global(g, s) => {
                self.emit(Op::Global(*g), *s);
            }
            Core::Lambda { name, self_id, params, body, span } => {
                if params.len() > u16::MAX as usize / 2 {
                    return Err("too many parameters".into());
                }
                self.fns.push(FnState::new(name.clone(), *self_id, params, *span));
                self.expr(body, true, *span)?;
                self.emit(Op::Return, *span);
                let code = self.fns.pop().unwrap().finish();
                let f = self.top();
                f.children.push(Rc::new(code));
                let i = (f.children.len() - 1) as u32;
                self.emit(Op::Closure(i), *span);
            }
            Core::Call { f, args, span } => {
                self.expr(f, false, *span)?;
                for a in args {
                    self.expr(a, false, *span)?;
                }
                let n = args.len() as u16;
                self.emit(if tail { Op::TailCall(n) } else { Op::Call(n) }, *span);
            }
            Core::Let { id, init, body } => {
                self.expr(init, false, span)?;
                let s = self.new_slot(*id)?;
                self.emit(Op::SetLocal(s), span);
                self.expr(body, tail, span)?;
            }
            Core::If(c, t, f) => {
                self.expr(c, false, span)?;
                let j1 = self.emit(Op::JumpIfFalse(0), span);
                self.expr(t, tail, span)?;
                let j2 = self.emit(Op::Jump(0), span);
                self.patch(j1);
                self.expr(f, tail, span)?;
                self.patch(j2);
            }
        }
        Ok(())
    }
}

/// Compile a closed expression into a zero-argument code object.
pub fn compile_thunk(name: &str, e: &Core, span: Span) -> CResult<Rc<Code>> {
    let mut c = Compiler { fns: vec![FnState::new(name.into(), None, &[], span)] };
    c.expr(e, false, span)?;
    c.emit(Op::Return, span);
    Ok(Rc::new(c.fns.pop().unwrap().finish()))
}

/// Compile a module/document entry: steps run in source order in one frame.
pub fn compile_entry(name: &str, steps: &[Step], span: Span) -> CResult<Rc<Code>> {
    let mut c = Compiler { fns: vec![FnState::new(name.into(), None, &[], span)] };
    for step in steps {
        match step {
            Step::Bind(id, e, s) => {
                c.expr(e, false, *s)?;
                let slot = c.new_slot(*id)?;
                c.emit(Op::SetLocal(slot), *s);
            }
            Step::SetGlobal(g, e, s) => {
                c.expr(e, false, *s)?;
                c.emit(Op::SetGlobal(*g), *s);
            }
            Step::Emit(i, e, s) => {
                c.expr(e, false, *s)?;
                c.emit(Op::Emit(*i), *s);
            }
            Step::Eval(e, s) => {
                c.expr(e, false, *s)?;
                c.emit(Op::Pop, *s);
            }
        }
    }
    c.constant(Value::Unit, span);
    c.emit(Op::Return, span);
    Ok(Rc::new(c.fns.pop().unwrap().finish()))
}
