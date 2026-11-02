//! Native registry (interface metadata plus optional in-process implementations) and explicit host context.

use crate::syntax::Availability;
use crate::types::TypeDesc;
use crate::value::{OpaqueValue, Value};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The stage an execution context runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Explicit compile-time preparation (`[% ...]` including `{!assert ...}`, static Type computation).
    Static,
    /// Runtime evaluation with host context.
    Runtime,
}

/// Call-specific context passed to a native callback. There are no ambient globals: inputs and capabilities
/// are explicit.
pub struct NativeCtx<'a> {
    pub stage: Stage,
    pub api_id: &'a str,
    pub inputs: &'a HashMap<String, Value>,
    pub capabilities: &'a HashSet<String>,
}

impl<'a> NativeCtx<'a> {
    pub fn input(&self, key: &str) -> Result<Value, String> {
        self.inputs.get(key).cloned().ok_or_else(|| format!("host input {:?} is not provided", key))
    }
}

pub type NativeCallback = Rc<dyn Fn(&mut NativeCtx, &[Value]) -> Result<Value, String>>;

#[derive(Clone)]
pub struct NativeEntry {
    pub api_id: String,
    pub version: String,
    pub params: Vec<TypeDesc>,
    pub ret: TypeDesc,
    pub availability: Availability,
    /// Capability names the caller's context must grant.
    pub capabilities: Vec<String>,
    /// `None` in an interface-only registry.
    pub callback: Option<NativeCallback>,
    /// Documentation written above the generated `[Native ...]` declaration (`NativeRegistry::declarations`).
    /// It does not affect checking, linking, or the fingerprint.
    pub doc: Option<String>,
}

#[derive(Clone)]
pub struct NativeTypeEntry {
    pub type_id: String,
    pub version: String,
}

/// Authoritative native metadata and (optionally) implementations.
#[derive(Clone, Default)]
pub struct NativeRegistry {
    pub entries: HashMap<String, NativeEntry>,
    pub types: HashMap<String, NativeTypeEntry>,
}

/// API ID of the standard clock read by `std.time`.
pub const STD_TIME_API: &str = "std.time/now";
/// Host input key holding the Unix timestamp snapshot (Int seconds) for one evaluation or batch.
pub const TIME_INPUT: &str = "std.time";

impl NativeRegistry {
    /// A registry containing the standard natives that `std` declares (currently `std.time/now`).
    pub fn new() -> NativeRegistry {
        let mut r = NativeRegistry::empty();
        r.register_fn(STD_TIME_API, vec![], TypeDesc::Int, Availability::Runtime, |ctx, _| match ctx.input(TIME_INPUT)? {
            Value::Int(t) => Ok(Value::Int(t)),
            other => Err(format!("host input {:?} must be an Int Unix timestamp, found {}", TIME_INPUT, other.type_name())),
        })
        .expect("standard natives");
        r
    }

    /// A registry with no entries at all (programs importing `std` will not link against it).
    pub fn empty() -> NativeRegistry {
        NativeRegistry::default()
    }

    /// Replace (or add) an entry, for hosts that provide their own implementation of a standard API.
    pub fn replace(&mut self, entry: NativeEntry) -> Result<(), String> {
        self.entries.remove(&entry.api_id);
        self.register(entry)
    }

    pub fn register_type(&mut self, type_id: &str, version: &str) -> Result<(), String> {
        if self.types.contains_key(type_id) {
            return Err(format!("native type {:?} is already registered", type_id));
        }
        self.types.insert(type_id.into(), NativeTypeEntry { type_id: type_id.into(), version: version.into() });
        Ok(())
    }

    pub fn register(&mut self, entry: NativeEntry) -> Result<(), String> {
        if self.entries.contains_key(&entry.api_id) {
            return Err(format!("native API {:?} is already registered", entry.api_id));
        }
        for t in entry.params.iter().chain(std::iter::once(&entry.ret)) {
            self.check_marshalable(t)?;
        }
        self.entries.insert(entry.api_id.clone(), entry);
        Ok(())
    }

    /// Convenience registration of an in-process Rust callback.
    pub fn register_fn(
        &mut self,
        api_id: &str,
        params: Vec<TypeDesc>,
        ret: TypeDesc,
        availability: Availability,
        f: impl Fn(&mut NativeCtx, &[Value]) -> Result<Value, String> + 'static,
    ) -> Result<(), String> {
        self.register(NativeEntry {
            api_id: api_id.into(),
            version: "1".into(),
            params,
            ret,
            availability,
            capabilities: vec![],
            callback: Some(Rc::new(f)),
            doc: None,
        })
    }

    /// Metadata without implementations, for interface-only checking.
    pub fn interface_only(&self) -> NativeRegistry {
        let mut r = self.clone();
        for e in r.entries.values_mut() {
            e.callback = None;
        }
        r
    }

    fn check_marshalable(&self, t: &TypeDesc) -> Result<(), String> {
        match t {
            TypeDesc::Opaque(id) if !self.types.contains_key(&**id) => {
                Err(format!("native type {:?} is used in a signature but not registered", id))
            }
            TypeDesc::Func(..) => Err("function-typed native arguments/results (callbacks) are not supported in v1".into()),
            TypeDesc::List(t) => self.check_marshalable(t),
            TypeDesc::Record(fs) | TypeDesc::Variant(fs) => fs.iter().try_for_each(|(_, t)| self.check_marshalable(t)),
            _ => Ok(()),
        }
    }

    /// Stable fingerprint of the registry metadata (for cache identity).
    pub fn fingerprint(&self) -> u64 {
        let mut ids: Vec<String> = self
            .entries
            .values()
            .map(|e| {
                format!(
                    "{}@{}:{:?}->{}:{}:{:?}",
                    e.api_id,
                    e.version,
                    e.params.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
                    e.ret,
                    e.availability.as_str(),
                    e.capabilities
                )
            })
            .collect();
        ids.extend(self.types.values().map(|t| format!("type {}@{}", t.type_id, t.version)));
        ids.sort();
        crate::fnv1a(ids.join("\n").as_bytes())
    }
}

/// Create an opaque native value of a registered type.
pub fn opaque(type_id: &str, data: impl Any, release: Option<crate::value::Release>) -> Value {
    Value::Opaque(Rc::new(OpaqueValue { type_id: Rc::from(type_id), data: Box::new(data), release }))
}

/// Explicit host inputs and granted capabilities for one call or batch.
#[derive(Clone, Default)]
pub struct HostContext {
    pub inputs: HashMap<String, Value>,
    pub capabilities: HashSet<String>,
}

impl HostContext {
    pub fn with_input(mut self, key: &str, v: Value) -> HostContext {
        self.inputs.insert(key.into(), v);
        self
    }
    /// Supply the Unix timestamp snapshot returned by `std.time` for this evaluation.
    pub fn with_time(self, unix_seconds: i64) -> HostContext {
        self.with_input(TIME_INPUT, Value::Int(unix_seconds))
    }

    /// Supply the current system time as the snapshot (the host's explicit choice to read its clock once).
    pub fn with_current_time(self) -> HostContext {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.with_time(now)
    }

    pub fn grant(mut self, cap: &str) -> HostContext {
        self.capabilities.insert(cap.into());
        self
    }
}

/// Check that a value conforms to a descriptor (used to validate native arguments and results).
pub fn conforms(v: &Value, t: &TypeDesc) -> bool {
    match (t, v) {
        (TypeDesc::Unit, Value::Unit) => true,
        (TypeDesc::Bool, Value::Bool(_)) => true,
        (TypeDesc::Int, Value::Int(_)) => true,
        (TypeDesc::Float, Value::Float(f)) => f.is_finite() && !(*f == 0.0 && f.is_sign_negative()),
        (TypeDesc::String, Value::Str(_)) => true,
        (TypeDesc::Type, Value::Type(_)) => true,
        (TypeDesc::List(e), Value::List(l)) => l.iter().all(|x| conforms(&x, e)),
        (TypeDesc::Record(fs), Value::Record(r)) => {
            fs.len() == r.len() && fs.iter().zip(r.iter()).all(|((n, t), (m, x))| n == m && conforms(x, t))
        }
        (TypeDesc::Variant(alts), Value::Variant(x)) => alts.iter().any(|(n, t)| *n == x.0 && conforms(&x.1, t)),
        (TypeDesc::Opaque(id), Value::Opaque(o)) => *id == o.type_id,
        _ => false,
    }
}
