# Embedding Lamlet (Rust host API) and the CLI

## CLI (`crates/lamlet-cli`, binary `lamlet`)

```sh
cargo build --release
./target/release/lamlet eval FILE.lam                  # check then evaluate; JSON on stdout
./target/release/lamlet check FILE.lam                 # types and stages only
./target/release/lamlet prepare FILE.lam               # prose, static values, and residual runtime regions
./target/release/lamlet call FILE.lam area 3 4         # call a static top-level function with JSON arguments
./target/release/lamlet batch REQUEST.json             # several documents/packages in one request
#   options: --package NAME=PATH   --input KEY=JSON   --require-linked   --max-instructions N
cargo run -q -p lamlet --example run -- FILE.lam       # quick: one line per result, "[region] value"
cargo run -q -p lamlet --example docs -- FILE.lam      # list documented names in scope
```

JSON encoding of values:

| Lamlet value | JSON |
|---|---|
| Records | objects |
| Variants | `{"tag": label, "value": payload}` |
| Lists | arrays |
| Unit | `null` |
| Functions | their display string |

Exit codes are nonzero on failure, and diagnostics go to stderr.

## Library (`crates/lamlet`, no dependencies)

```rust
use lamlet::*;                       // check_document, Instance, NativeRegistry, MemoryResolver, HostContext, Limits, Value, Exports
use lamlet::value::{display, List};  // display(&Value) -> String
use lamlet::types::TypeDesc;
use lamlet::syntax::Availability;    // Pure | Static | Runtime
use std::rc::Rc;

let mut resolver = MemoryResolver::new();                 // std is always available
resolver.add("mylib", "$[Import* std] $[# twice {^ {x} {* 2 x}}]", Exports::All); // `_names` stay private

let mut natives = NativeRegistry::new();                  // includes std.time/now
natives.register_fn("app.clock/now", vec![], TypeDesc::Float, Availability::Runtime,
    |ctx, _args| ctx.input("clock"))?;                    // natives read only explicit inputs
let natives = Rc::new(natives);

let doc = check_document("doc.lam", source, resolver.into_rc(), natives.clone(), &Options::default())
    .map_err(|f| f.render())?;                            // types, stages; [% ...] already ran
let mut inst = Instance::new(doc.program.clone(), natives).map_err(|d| d.render(&doc.sources))?;

let ctx = HostContext::default().with_input("clock", Value::Float(1.5)).with_time(1_700_000_000).grant("cap");
let regions: Vec<(u32, Value)> = inst.evaluate(ctx.clone(), Limits::default())?;   // every expression region
let f = doc.static_binding("twice").unwrap().clone();                               // static top-level values
let v = inst.call(&f, vec![Value::Int(21)], ctx, Limits::default())?;              // call any function value
```

### Facts to rely on

- **Compile once, run many times.** `Document`/`Program` are immutable. An `Instance` can evaluate or call any
  number of times, and two instances can share one program. A failed evaluation does not corrupt the next one.
- **Static bindings.** `doc.static_bindings` gives each binding's name, type string, `ty_desc`, value, and doc.
  Use it to check that a document defines what the host expects.
- **Prepared output.** `doc.prepared()` gives the prose spans, static values, and residual runtime regions, for
  rendering a document.
- **Constructing values in Rust.** Use `Value::record(vec![(Rc::from("x"), v)])` (fields are sorted
  automatically), `Value::variant("some", v)`, `Value::str("s")`, and
  `Value::List(List::from_vec(...))`.
- **Reading values in Rust.** Use `v.field("x")`, and match on `Value::Variant(x)` to get `(&x.0, &x.1)`.
- **Limits.** `Limits` caps instructions, frames, stack, allocation, string size, and native calls. Runtime
  defaults are in `Limits::default()`; preparation uses `Limits::static_default()`.
- **Cancellation.** `inst.cancel_flag()` returns an `Arc<AtomicBool>` that stops a running evaluation.
- **Native contracts.** A native is declared in source as
  `[Native name [% {Fn {' A B} R}] "api-id" Pure|Static|Runtime]`. The declaration must match the registry
  exactly (types and availability). Function-typed native arguments are not supported in v1. Opaque host types
  are declared with `[NativeType T "type-id"]` plus `NativeRegistry::register_type`.
- **Interface-only checking.** `natives.interface_only()` keeps the metadata without implementations.
  `Options { require_linked: true, .. }` fails checking on unregistered natives.
