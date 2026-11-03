//! Injected package resolution. The core never searches directories or reads manifests itself.

use std::collections::HashMap;
use std::rc::Rc;

/// The package-author export descriptor (v1). `All` exports every own top-level definition whose name does
/// not start with `_`; `Names` lists exports explicitly and may deliberately re-export imported bindings or
/// namespaces (which is how submodule namespaces are published).
#[derive(Clone, Debug)]
pub enum Exports {
    All,
    Names(Vec<String>),
}

/// A resolved package: canonical identity, exact version, Lamlet document source, and export descriptor.
#[derive(Clone, Debug)]
pub struct PackageSource {
    pub name: String,
    pub version: String,
    pub file_name: String,
    pub source: String,
    pub exports: Exports,
}

pub trait Resolver {
    fn resolve(&self, name: &str) -> Option<PackageSource>;
}

/// The built-in package of core primitives (the intrinsic table). It is always resolved by the compiler
/// itself (a resolver entry with this name is ignored) and every module imports it by default, as if by
/// `[Import K]` in an enclosing prelude scope, so a module may shadow the name `K`.
pub const CORE_PACKAGE: &str = "K";

/// The embedded standard library source.
pub const STD_SOURCE: &str = include_str!("../std/std.lam");

pub fn std_package() -> PackageSource {
    PackageSource {
        name: "std".into(),
        version: crate::LANGUAGE_VERSION.into(),
        file_name: "<std>/std.lam".into(),
        source: STD_SOURCE.into(),
        exports: Exports::All,
    }
}

/// An in-memory resolver. `std` is always available unless explicitly replaced.
#[derive(Clone, Default)]
pub struct MemoryResolver {
    pub packages: HashMap<String, PackageSource>,
}

impl MemoryResolver {
    pub fn new() -> MemoryResolver {
        MemoryResolver::default()
    }
    pub fn add(&mut self, name: &str, source: &str, exports: Exports) -> &mut Self {
        self.packages.insert(
            name.into(),
            PackageSource {
                name: name.into(),
                version: "0".into(),
                file_name: format!("<{}>", name),
                source: source.into(),
                exports,
            },
        );
        self
    }
    pub fn add_package(&mut self, p: PackageSource) -> &mut Self {
        self.packages.insert(p.name.clone(), p);
        self
    }
    pub fn into_rc(self) -> Rc<dyn Resolver> {
        Rc::new(self)
    }
}

impl Resolver for MemoryResolver {
    fn resolve(&self, name: &str) -> Option<PackageSource> {
        match self.packages.get(name) {
            Some(p) => Some(p.clone()),
            None if name == "std" => Some(std_package()),
            None => None,
        }
    }
}
