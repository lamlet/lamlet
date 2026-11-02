//! Checked intermediate representation: resolved names, explicit control flow, and embedded constants.

use crate::source::Span;
use crate::value::Value;
use std::collections::HashSet;

pub type LocalId = u32;

#[derive(Clone)]
pub enum Core {
    Const(Value),
    Local(LocalId, Span),
    Global(u32, Span),
    /// `self_id`, when present, is bound inside the body to the closure itself (recursive lambda).
    Lambda { name: String, self_id: Option<LocalId>, params: Vec<LocalId>, body: Box<Core>, span: Span },
    Call { f: Box<Core>, args: Vec<Core>, span: Span },
    Let { id: LocalId, init: Box<Core>, body: Box<Core> },
    If(Box<Core>, Box<Core>, Box<Core>),
}

impl Core {
    /// Free local variables (in first-occurrence order).
    pub fn free_locals(&self) -> Vec<(LocalId, Span)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        self.free_into(&mut Vec::new(), &mut seen, &mut out);
        out
    }

    fn free_into(&self, bound: &mut Vec<LocalId>, seen: &mut HashSet<LocalId>, out: &mut Vec<(LocalId, Span)>) {
        match self {
            Core::Const(_) | Core::Global(..) => {}
            Core::Local(id, s) => {
                if !bound.contains(id) && seen.insert(*id) {
                    out.push((*id, *s));
                }
            }
            Core::Lambda { params, body, self_id, .. } => {
                let n = bound.len();
                bound.extend(params);
                bound.extend(self_id);
                body.free_into(bound, seen, out);
                bound.truncate(n);
            }
            Core::Call { f, args, .. } => {
                f.free_into(bound, seen, out);
                args.iter().for_each(|a| a.free_into(bound, seen, out));
            }
            Core::Let { id, init, body } => {
                init.free_into(bound, seen, out);
                bound.push(*id);
                body.free_into(bound, seen, out);
                bound.pop();
            }
            Core::If(c, t, e) => {
                c.free_into(bound, seen, out);
                t.free_into(bound, seen, out);
                e.free_into(bound, seen, out);
            }
        }
    }

    /// Whether the expression reads any package global (whose value is only available after runtime
    /// initialization).
    pub fn uses_globals(&self) -> bool {
        match self {
            Core::Global(..) => true,
            Core::Const(_) | Core::Local(..) => false,
            Core::Lambda { body, .. } => body.uses_globals(),
            Core::Call { f, args, .. } => f.uses_globals() || args.iter().any(|a| a.uses_globals()),
            Core::Let { init, body, .. } => init.uses_globals() || body.uses_globals(),
            Core::If(c, t, e) => c.uses_globals() || t.uses_globals() || e.uses_globals(),
        }
    }
}

/// One step of a module/document entry, in source order.
#[derive(Clone)]
pub enum Step {
    /// Bind a document-level runtime binding (an entry-frame local).
    Bind(LocalId, Core, Span),
    /// Initialize a package-level runtime binding.
    SetGlobal(u32, Core, Span),
    /// Evaluate an expression region and record it as region `index`.
    Emit(u32, Core, Span),
    /// Evaluate and discard (package-level expression regions).
    Eval(Core, Span),
}
