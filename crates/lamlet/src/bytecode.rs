//! Compact stack bytecode with explicit calls, closures, jumps, and tail calls.

use crate::source::Span;
use crate::value::Value;
use std::rc::Rc;

/// Version of the instruction set and value semantics.
pub const BYTECODE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Push `consts[i]`.
    Const(u32),
    /// Push local slot.
    Local(u16),
    /// Pop into local slot.
    SetLocal(u16),
    /// Push captured value.
    Capture(u16),
    /// Push the closure of the executing frame (a recursive lambda's self reference).
    SelfClosure,
    /// Push a package global (error if not initialized).
    Global(u32),
    /// Pop into a package global.
    SetGlobal(u32),
    /// Create a closure from `children[i]`, capturing per its capture list.
    Closure(u32),
    /// Call the callee below `n` arguments.
    Call(u16),
    /// Call in tail position (the current frame is released first).
    TailCall(u16),
    Return,
    Jump(u32),
    /// Pop a Bool; jump if false.
    JumpIfFalse(u32),
    /// Pop a Bool; jump if true.
    JumpIfTrue(u32),
    Pop,
    /// Pop a value and record it as the result of document region `i`.
    Emit(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapSrc {
    Local(u16),
    Capture(u16),
    /// The enclosing frame's own closure.
    SelfClosure,
}

pub struct Code {
    pub name: String,
    pub arity: u16,
    pub n_slots: u16,
    pub captures: Vec<CapSrc>,
    pub ops: Vec<Op>,
    pub spans: Vec<Span>,
    pub consts: Vec<Value>,
    pub children: Vec<Rc<Code>>,
    pub span: Span,
}

impl Code {
    /// Verify operands, jump targets, and indexes before execution.
    pub fn verify(&self, n_globals: usize) -> Result<(), String> {
        if self.arity > self.n_slots || self.spans.len() != self.ops.len() {
            return Err(format!("{}: inconsistent frame layout", self.name));
        }
        let n = self.ops.len() as u32;
        for op in &self.ops {
            let ok = match *op {
                Op::Const(i) => (i as usize) < self.consts.len(),
                Op::Local(s) | Op::SetLocal(s) => s < self.n_slots,
                Op::Capture(c) => (c as usize) < self.captures.len(),
                Op::Global(g) | Op::SetGlobal(g) => (g as usize) < n_globals,
                Op::Closure(c) => (c as usize) < self.children.len(),
                Op::Jump(t) | Op::JumpIfFalse(t) | Op::JumpIfTrue(t) => t < n,
                _ => true,
            };
            if !ok {
                return Err(format!("{}: invalid operand in {:?}", self.name, op));
            }
        }
        if !matches!(self.ops.last(), Some(Op::Return) | Some(Op::TailCall(_))) {
            return Err(format!("{}: code does not end with a return", self.name));
        }
        for child in &self.children {
            for c in &child.captures {
                let ok = match *c {
                    CapSrc::Local(s) => s < self.n_slots,
                    CapSrc::Capture(i) => (i as usize) < self.captures.len(),
                    CapSrc::SelfClosure => true,
                };
                if !ok {
                    return Err(format!("{}: invalid capture source", child.name));
                }
            }
            child.verify(n_globals)?;
        }
        Ok(())
    }
}
