//! `lamlet` command-line interface: check, prepare, eval, and a batch JSON protocol.
//!
//! stdout carries only the structured JSON response; rendered diagnostics go to stderr.
//! Exit status: 0 success, 1 language diagnostics (syntax/type/stage/static/runtime/limit), 2 usage or I/O error.

use lamlet::check::RegionInfo;
use lamlet::source::{Diagnostic, SourceMap};
use lamlet::types::TypeDesc;
use lamlet::value::{display, Value};
use lamlet::*;
use serde_json::{json, Map, Value as Json};
use std::io::Read;
use std::rc::Rc;

const PROTOCOL_VERSION: u64 = 1;

const USAGE: &str = "usage:
  lamlet check   FILE [--package NAME=PATH]... [--require-linked]
  lamlet prepare FILE [--package NAME=PATH]...
  lamlet eval    FILE [--package NAME=PATH]... [--input KEY=JSON]... [--max-instructions N]
  lamlet call    FILE FUNCTION [JSON_ARG]... [--package NAME=PATH]... [--input KEY=JSON]...
  lamlet batch   < request.json

`call` evaluates a statically available top-level function of FILE with JSON arguments, converted using the
function's parameter types (numbers, strings, booleans, null for Unit, arrays for lists, objects for records).

`std.time` returns one Unix timestamp snapshot per invocation; override it with --input std.time=SECONDS.
Packages given with --package export every top-level definition not starting with `_`.
`std` is always available. Output is versioned JSON on stdout; diagnostics are also rendered on stderr.";

/// One function call of a `call` request: the function name and its JSON arguments.
type Call = (String, Vec<Json>);

struct Request {
    command: String,
    /// Name, source, and (for `call`) the calls to make: function name and JSON arguments.
    documents: Vec<(String, String, Vec<Call>)>,
    packages: Vec<PackageSource>,
    inputs: Vec<(String, Value)>,
    require_linked: bool,
    max_instructions: Option<u64>,
    /// Unix timestamp snapshot for `std.time`, read once per invocation.
    time: i64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let req = match parse_args(&args) {
        Ok(r) => r,
        Err(m) => {
            eprintln!("lamlet: {}\n\n{}", m, USAGE);
            std::process::exit(2);
        }
    };
    let (response, ok) = run(&req);
    println!("{}", serde_json::to_string_pretty(&response).unwrap());
    std::process::exit(if ok { 0 } else { 1 });
}

fn parse_args(args: &[String]) -> Result<Request, String> {
    let command = args.first().ok_or("missing command")?.clone();
    if command == "batch" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).map_err(|e| format!("reading stdin: {}", e))?;
        return parse_batch(&text);
    }
    if !matches!(command.as_str(), "check" | "prepare" | "eval" | "call") {
        return Err(format!("unknown command `{}`", command));
    }
    let mut positional: Vec<String> = Vec::new();
    let mut req = Request { command, documents: vec![], packages: vec![], inputs: vec![], require_linked: false, max_instructions: None, time: now() };
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        let mut value = || {
            i += 1;
            args.get(i).cloned().ok_or(format!("{} needs a value", a))
        };
        match a.as_str() {
            "--package" => {
                let v = value()?;
                let (name, path) = v.split_once('=').ok_or("--package expects NAME=PATH")?;
                let source = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {}", path, e))?;
                req.packages.push(PackageSource { name: name.into(), version: "0".into(), file_name: path.into(), source, exports: Exports::All });
            }
            "--input" => {
                let v = value()?;
                let (k, j) = v.split_once('=').ok_or("--input expects KEY=JSON")?;
                let j: Json = serde_json::from_str(j).map_err(|e| format!("--input {}: {}", k, e))?;
                req.inputs.push((k.into(), json_to_value(&j)?));
            }
            "--require-linked" => req.require_linked = true,
            "--max-instructions" => req.max_instructions = Some(value()?.parse().map_err(|_| "--max-instructions expects an integer")?),
            arg if !arg.starts_with("--") => positional.push(arg.to_string()),
            other => return Err(format!("unknown option `{}`", other)),
        }
        i += 1;
    }
    let mut positional = positional.into_iter();
    let path = positional.next().ok_or("no input file")?;
    let source = std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {}", path, e))?;
    let mut calls = Vec::new();
    if req.command == "call" {
        let function = positional.next().ok_or("`call` needs a function name")?;
        let args = positional
            .map(|a| serde_json::from_str::<Json>(&a).map_err(|e| format!("argument {:?} is not JSON: {}", a, e)))
            .collect::<Result<Vec<_>, _>>()?;
        calls.push((function, args));
    } else if positional.next().is_some() {
        return Err(format!("`{}` takes one file", req.command));
    }
    req.documents.push((path, source, calls));
    Ok(req)
}

/// Batch request:
/// `{"version":1, "command":"check|prepare|eval|call",
///   "documents":[{"name":..., "source":..., "calls":[{"function":..., "args":[...]}]}],
///   "packages":[{"name":..., "source":..., "exports":["a","b"] | "all"}], "inputs":{"key": json}}`
fn parse_batch(text: &str) -> Result<Request, String> {
    let j: Json = serde_json::from_str(text).map_err(|e| format!("invalid JSON request: {}", e))?;
    if j.get("version").and_then(Json::as_u64) != Some(PROTOCOL_VERSION) {
        return Err(format!("unsupported request version (this tool speaks version {})", PROTOCOL_VERSION));
    }
    let command = j.get("command").and_then(Json::as_str).ok_or("request needs a command")?.to_string();
    if !matches!(command.as_str(), "check" | "prepare" | "eval" | "call") {
        return Err(format!("unknown command `{}`", command));
    }
    let str_field = |o: &Json, f: &str| o.get(f).and_then(Json::as_str).map(String::from).ok_or(format!("missing string field `{}`", f));
    let mut documents = Vec::new();
    for d in j.get("documents").and_then(Json::as_array).ok_or("request needs documents")? {
        let mut calls = Vec::new();
        for c in d.get("calls").and_then(Json::as_array).cloned().unwrap_or_default() {
            let args = c.get("args").and_then(Json::as_array).cloned().unwrap_or_default();
            calls.push((str_field(&c, "function")?, args));
        }
        documents.push((str_field(d, "name")?, str_field(d, "source")?, calls));
    }
    let mut packages = Vec::new();
    for p in j.get("packages").and_then(Json::as_array).cloned().unwrap_or_default() {
        let exports = match p.get("exports") {
            None => Exports::All,
            Some(Json::String(s)) if s == "all" => Exports::All,
            Some(Json::Array(xs)) => Exports::Names(xs.iter().filter_map(|x| x.as_str().map(String::from)).collect()),
            Some(_) => return Err("`exports` must be \"all\" or a list of names".into()),
        };
        let name = str_field(&p, "name")?;
        packages.push(PackageSource {
            version: p.get("version").and_then(Json::as_str).unwrap_or("0").into(),
            file_name: format!("<{}>", name),
            name,
            source: str_field(&p, "source")?,
            exports,
        });
    }
    let mut inputs = Vec::new();
    if let Some(Json::Object(m)) = j.get("inputs") {
        for (k, v) in m {
            inputs.push((k.clone(), json_to_value(v)?));
        }
    }
    Ok(Request {
        command,
        documents,
        packages,
        inputs,
        require_linked: j.get("require_linked").and_then(Json::as_bool).unwrap_or(false),
        max_instructions: j.get("max_instructions").and_then(Json::as_u64),
        time: now(),
    })
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn run(req: &Request) -> (Json, bool) {
    let mut resolver = MemoryResolver::new();
    for p in &req.packages {
        resolver.add_package(p.clone());
    }
    let resolver: Rc<dyn Resolver> = Rc::new(resolver);
    let natives = Rc::new(NativeRegistry::new());
    let options = Options { require_linked: req.require_linked, ..Options::default() };
    let mut all_ok = true;
    let mut results = Vec::new();
    for (name, source, calls) in &req.documents {
        let (doc_json, ok) = run_document(req, name, source, calls, resolver.clone(), natives.clone(), &options);
        all_ok &= ok;
        results.push(doc_json);
    }
    let response = json!({
        "version": PROTOCOL_VERSION,
        "language": LANGUAGE_VERSION,
        "command": req.command,
        "ok": all_ok,
        "documents": results,
    });
    (response, all_ok)
}

fn run_document(req: &Request, name: &str, source: &str, calls: &[(String, Vec<Json>)], resolver: Rc<dyn Resolver>, natives: Rc<NativeRegistry>, options: &Options) -> (Json, bool) {
    let doc = match check_document(name, source, resolver, natives.clone(), options) {
        Ok(d) => d,
        Err(f) => {
            for d in &f.diagnostics {
                eprintln!("{}", d.render(&f.sources));
            }
            let diags: Vec<Json> = f.diagnostics.iter().map(|d| diag_json(d, &f.sources)).collect();
            return (json!({"name": name, "ok": false, "diagnostics": diags}), false);
        }
    };
    let mut out = Map::new();
    out.insert("name".into(), json!(name));
    out.insert("fingerprint".into(), json!(format!("{:016x}", doc.program.fingerprint)));
    out.insert(
        "dependencies".into(),
        Json::Array(doc.dependencies.iter().map(|(n, v, h)| json!({"package": n, "version": v, "hash": format!("{:016x}", h)})).collect()),
    );
    out.insert("natives".into(), Json::Array(doc.program.native_requirements.iter().map(|r| json!(r.api_id)).collect()));
    match req.command.as_str() {
        "check" => {
            out.insert("regions".into(), Json::Array(doc.regions.iter().map(|r| region_json(r, &doc.sources)).collect()));
        }
        "prepare" => {
            let spans: Vec<Json> = doc
                .prepared()
                .into_iter()
                .map(|p| match p {
                    PreparedSpan::Text { span, text } => json!({"kind": "text", "start": span.start, "end": span.end, "text": text}),
                    PreparedSpan::Comment { span, text } => json!({"kind": "comment", "start": span.start, "end": span.end, "text": text}),
                    PreparedSpan::Declaration { region, span } => json!({"kind": "declaration", "region": region, "start": span.start, "end": span.end}),
                    PreparedSpan::Value { region, span, ty, value, .. } => json!({
                        "kind": "value", "region": region, "start": span.start, "end": span.end, "type": ty,
                        "value": value_json(&value),
                    }),
                    PreparedSpan::Residual { region, span, ty } => json!({"kind": "residual", "region": region, "start": span.start, "end": span.end, "type": ty}),
                })
                .collect();
            out.insert("spans".into(), Json::Array(spans));
        }
        "eval" | "call" => {
            let mut inst = match Instance::new(doc.program.clone(), natives) {
                Ok(i) => i,
                Err(d) => {
                    eprintln!("{}", d.render(&doc.sources));
                    out.insert("ok".into(), json!(false));
                    out.insert("diagnostics".into(), json!([diag_json(&d, &doc.sources)]));
                    return (Json::Object(out), false);
                }
            };
            // One time snapshot per batch, unless the request supplies `std.time` explicitly.
            let mut ctx = HostContext::default().with_time(req.time);
            for (k, v) in &req.inputs {
                ctx = ctx.with_input(k, v.clone());
            }
            let mut limits = Limits::default();
            if let Some(n) = req.max_instructions {
                limits.max_instructions = n;
            }
            if req.command == "call" {
                let mut ok = true;
                let results: Vec<Json> = calls
                    .iter()
                    .map(|(function, args)| {
                        let r = call_one(&doc, &mut inst, function, args, ctx.clone(), limits.clone());
                        ok &= r.get("ok") == Some(&json!(true));
                        r
                    })
                    .collect();
                out.insert("calls".into(), Json::Array(results));
                out.insert("ok".into(), json!(ok));
                return (Json::Object(out), ok);
            }
            match inst.evaluate(ctx, limits) {
                Ok(values) => {
                    let results: Vec<Json> = values
                        .iter()
                        .map(|(i, v)| {
                            let r = &doc.regions[*i as usize];
                            let loc = doc.sources.location(r.span);
                            json!({
                                "region": i, "id": r.stable_id, "line": loc.line, "column": loc.column,
                                "type": r.ty, "value": value_json(v),
                            })
                        })
                        .collect();
                    out.insert("results".into(), Json::Array(results));
                    let st = inst.stats();
                    out.insert("stats".into(), json!({"instructions": st.instructions, "alloc_bytes": st.alloc_bytes, "native_calls": st.native_calls}));
                }
                Err(e) => {
                    let d = e.to_diagnostic();
                    eprintln!("{}", d.render(&doc.sources));
                    out.insert("ok".into(), json!(false));
                    out.insert("diagnostics".into(), json!([diag_json(&d, &doc.sources)]));
                    return (Json::Object(out), false);
                }
            }
        }
        _ => unreachable!(),
    }
    out.insert("ok".into(), json!(true));
    (Json::Object(out), true)
}

/// One host call of a document function; failures are reported per call and do not affect later calls.
fn call_one(doc: &Document, inst: &mut Instance, function: &str, args: &[Json], ctx: HostContext, limits: Limits) -> Json {
    let fail = |message: String| {
        eprintln!("lamlet: call {}: {}", function, message);
        json!({"function": function, "ok": false, "message": message})
    };
    let binding = match doc.static_bindings.iter().find(|b| b.name == function) {
        Some(b) => b,
        None => return fail(format!("`{}` is not a statically available top-level binding of the document", function)),
    };
    if !binding.value.is_callable() {
        return fail(format!("`{}` is a {}, not a function", function, binding.ty));
    }
    let (values, _ret) = match &binding.ty_desc {
        Some(TypeDesc::Func(params, ret)) => {
            if params.len() != args.len() {
                return fail(format!("`{}` takes {} argument(s), but {} were given", function, params.len(), args.len()));
            }
            let mut values = Vec::new();
            for (i, (a, t)) in args.iter().zip(params).enumerate() {
                match json_to_typed(a, t) {
                    Ok(v) => values.push(v),
                    Err(m) => return fail(format!("argument {} ({}): {}", i + 1, t, m)),
                }
            }
            (values, Some((**ret).clone()))
        }
        // Generic or variadic functions: arguments are untyped JSON values, checked at runtime.
        _ => match args.iter().map(json_to_value).collect::<Result<Vec<_>, _>>() {
            Ok(v) => (v, None),
            Err(m) => return fail(m),
        },
    };
    match inst.call(&binding.value, values, ctx, limits) {
        Ok(v) => json!({"function": function, "ok": true, "type": binding.ty, "value": value_json(&v)}),
        Err(e) => {
            let d = e.to_diagnostic();
            eprintln!("{}", d.render(&doc.sources));
            json!({"function": function, "ok": false, "diagnostics": [diag_json(&d, &doc.sources)]})
        }
    }
}

/// Convert JSON to a value of a known type (the inverse of `data_json`).
fn json_to_typed(j: &Json, t: &TypeDesc) -> Result<Value, String> {
    let mismatch = || format!("expected {}, found {}", t, j);
    Ok(match t {
        TypeDesc::Unit if j.is_null() => Value::Unit,
        TypeDesc::Bool => Value::Bool(j.as_bool().ok_or_else(mismatch)?),
        TypeDesc::Int => Value::Int(j.as_i64().ok_or_else(mismatch)?),
        TypeDesc::Float => lamlet::builtins::check_float(j.as_f64().ok_or_else(mismatch)?)?,
        TypeDesc::String => Value::str(j.as_str().ok_or_else(mismatch)?),
        TypeDesc::List(e) => Value::List(lamlet::value::List::from_vec(
            j.as_array().ok_or_else(mismatch)?.iter().map(|x| json_to_typed(x, e)).collect::<Result<_, _>>()?,
        )),
        TypeDesc::Record(fields) => {
            let obj = j.as_object().ok_or_else(mismatch)?;
            if obj.len() != fields.len() || fields.iter().any(|(n, _)| !obj.contains_key(&**n)) {
                return Err(format!("expected exactly the fields of {}", t));
            }
            let mut out = Vec::new();
            for (n, ft) in fields {
                out.push((n.clone(), json_to_typed(&obj[&**n], ft).map_err(|m| format!("field {:?}: {}", n, m))?));
            }
            Value::Record(Rc::new(out))
        }
        TypeDesc::Variant(alts) => {
            let tag = j.get("tag").and_then(Json::as_str).ok_or_else(|| format!("expected {{\"tag\": label, \"value\": ...}} for {}", t))?;
            let (n, pt) = alts.iter().find(|a| &*a.0 == tag).ok_or_else(|| format!("#{} is not an alternative of {}", tag, t))?;
            let payload = if *pt == TypeDesc::Unit && j.get("value").is_none() { Value::Unit } else { json_to_typed(&j["value"], pt)? };
            Value::Variant(Rc::new((n.clone(), payload)))
        }
        _ => return Err(format!("{} values cannot be passed from JSON", t)),
    })
}

fn region_json(r: &RegionInfo, sources: &SourceMap) -> Json {
    let loc = sources.location(r.span);
    json!({
        "region": r.index,
        "id": r.stable_id,
        "line": loc.line,
        "column": loc.column,
        "start": r.span.start,
        "end": r.span.end,
        "declaration": r.is_declaration,
        "declares": r.declares,
        "doc": r.doc,
        "type": r.ty,
        "static": r.static_value.is_some(),
    })
}

fn diag_json(d: &Diagnostic, sources: &SourceMap) -> Json {
    let loc = |s: Option<lamlet::source::Span>| {
        s.map(|s| {
            let l = sources.location(s);
            json!({"file": l.file, "line": l.line, "column": l.column, "end_line": l.end_line, "end_column": l.end_column, "start": l.start, "end": l.end})
        })
    };
    json!({
        "kind": d.kind.as_str(),
        "message": d.message,
        "location": loc(d.span),
        "notes": d.notes.iter().map(|(m, s)| json!({"message": m, "location": loc(*s)})).collect::<Vec<_>>(),
    })
}

fn value_json(v: &Value) -> Json {
    json!({"display": display(v), "data": data_json(v)})
}

/// A structural JSON encoding of data: records are objects, variants are `{"tag", "value"}`, and functions,
/// types, and native values are encoded by their display.
fn data_json(v: &Value) -> Json {
    match v {
        Value::Unit => Json::Null,
        Value::Bool(b) => json!(b),
        Value::Int(n) => json!(n),
        Value::Float(f) => json!(f),
        Value::Str(s) => json!(&**s),
        Value::List(l) => Json::Array(l.iter().map(|x| data_json(&x)).collect()),
        Value::Record(fs) => {
            let mut m = Map::new();
            for (n, x) in fs.iter() {
                m.insert(n.to_string(), data_json(x));
            }
            Json::Object(m)
        }
        Value::Variant(x) => json!({"tag": &*x.0, "value": data_json(&x.1)}),
        other => json!(display(other)),
    }
}

fn json_to_value(j: &Json) -> Result<Value, String> {
    Ok(match j {
        Json::Null => Value::Unit,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => {
                let f = n.as_f64().ok_or("number out of range")?;
                lamlet::builtins::check_float(f)?
            }
        },
        Json::String(s) => Value::str(s),
        Json::Array(xs) => Value::List(lamlet::value::List::from_vec(xs.iter().map(json_to_value).collect::<Result<_, _>>()?)),
        Json::Object(_) => return Err("object inputs are not supported; use scalars or arrays".into()),
    })
}
