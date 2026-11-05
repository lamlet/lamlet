use serde_json::Value as Json;
use std::io::Write;
use std::process::{Command, Stdio};

fn lamlet(args: &[&str], stdin: Option<&str>) -> (i32, Json, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lamlet"));
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    if let Some(s) = stdin {
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    } else {
        drop(child.stdin.take());
    }
    let out = child.wait_with_output().unwrap();
    let json = serde_json::from_slice(&out.stdout).unwrap_or(Json::Null);
    (out.status.code().unwrap(), json, String::from_utf8_lossy(&out.stderr).into())
}

fn example() -> String {
    format!("{}/../../examples/hello.lam", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn eval_example_matches_library() {
    let (code, j, _) = lamlet(&["eval", &example()], None);
    assert_eq!(code, 0);
    let shown: Vec<String> = j["documents"][0]["results"].as_array().unwrap().iter().map(|r| r["value"]["display"].as_str().unwrap().to_string()).collect();
    let src = std::fs::read_to_string(example()).unwrap();
    let engine = lamlet::embed::Engine::new();
    let doc = engine.check(lamlet::embed::SourceFile::new("hello.lam", src)).unwrap();
    let lib: Vec<String> = engine.instance(&doc).unwrap().evaluate().unwrap().iter().map(|(_, v)| format!("{:?}", v)).collect();
    assert_eq!(shown, lib);
    assert_eq!(shown.last().unwrap(), "\"The total is 9.\"");
}

#[test]
fn prepare_is_replaced_by_build() {
    // `lamlet prepare` became `lamlet build --to ir --prepared`, which gives the same static values and holes.
    let (code, _, _) = lamlet(&["prepare", &example()], None);
    assert_eq!(code, 2);
    let out = Command::new(env!("CARGO_BIN_EXE_lamlet")).args(["build", &example(), "--to", "ir", "--prepared"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let ir: Json = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(ir["lamletDoc"], 2);
}

#[test]
fn check_reports_docs() {
    let (code, j, _) = lamlet(&["batch"], Some(r#"{"version": 1, "command": "check",
        "documents": [{"name": "d", "source": "$[Import* std]\nSquares x.\n$[# square {^ {x} {* x x}}]"}]}"#));
    assert_eq!(code, 0);
    let r = &j["documents"][0]["regions"][1];
    assert_eq!(r["declares"], "square");
    assert_eq!(r["doc"], "Squares x.");
}

#[test]
fn batch_protocol() {
    let req = r#"{"version": 1, "command": "eval",
        "packages": [{"name": "greet", "source": "$[Import std] $[# hello {^ {n} {std::++ \"hi \" n}}]", "exports": ["hello"]}],
        "inputs": {"unused": 1},
        "documents": [
            {"name": "a", "source": "$[Import greet] ${greet::hello \"there\"}"},
            {"name": "b", "source": "$[Import* std] ${+ 1 2.0}"}
        ]}"#;
    let (code, j, stderr) = lamlet(&["batch"], Some(req));
    assert_eq!(code, 1);
    assert_eq!(j["version"], 1);
    assert_eq!(j["documents"][0]["results"][0]["value"]["data"], "hi there");
    assert_eq!(j["documents"][1]["diagnostics"][0]["kind"], "type");
    assert!(stderr.contains("type error"));
}

#[test]
fn usage_errors_exit_2() {
    let (code, _, _) = lamlet(&["frobnicate"], None);
    assert_eq!(code, 2);
    let (code, _, _) = lamlet(&["batch"], Some("{\"version\": 99}"));
    assert_eq!(code, 2);
}

#[test]
fn runtime_inputs_and_limits() {
    let (code, j, _) = lamlet(&["eval", &example(), "--max-instructions", "5"], None);
    assert_eq!(code, 1);
    assert_eq!(j["documents"][0]["diagnostics"][0]["kind"], "limit");
}

#[test]
fn running_a_file_prints_its_text_with_values() {
    let dir = std::env::temp_dir().join(format!("lamlet-cli-run-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("age.lam");
    std::fs::write(&f, "$[Import* std]\n$[# AGE 3]\nHi! I am $AGE ${!if {> AGE 1} \"years\" \"year\"} old, $$5.\n").unwrap();
    for args in [vec![f.to_str().unwrap()], vec!["run", f.to_str().unwrap()]] {
        let out = Command::new(env!("CARGO_BIN_EXE_lamlet")).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "Hi! I am 3 years old, $5.\n");
    }
    std::fs::write(&f, "$[# AGE 3]\nHi ${nope AGE}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_lamlet")).arg(&f).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("age.lam:2:6: name error: unknown name `nope`"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn time_snapshot() {
    let dir = std::env::temp_dir().join(format!("lamlet-cli-time-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.lam");
    std::fs::write(&f, "$[Import* std] ${time} ${= {time} {time}}").unwrap();
    let (code, j, _) = lamlet(&["eval", f.to_str().unwrap(), "--input", "std.time=42"], None);
    assert_eq!(code, 0);
    assert_eq!(j["documents"][0]["results"][0]["value"]["data"], 42);
    let (code, j, _) = lamlet(&["eval", f.to_str().unwrap()], None);
    assert_eq!(code, 0);
    assert!(j["documents"][0]["results"][0]["value"]["data"].as_i64().unwrap() > 1_600_000_000);
    assert_eq!(j["documents"][0]["results"][1]["value"]["data"], true);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn call_functions_with_typed_json_arguments() {
    let source = "$[Import* std]\n$[# frames {^ {seconds fps} {round {* seconds {float fps}}}}]\n$[# P [% {Struct {' {Field \"x\" Float} {Field \"label\" String}}}]]\n$[# describe {^ {p} [# label {.label {: p P}}] [# x {fixed {.x p} 2}] f\"{$label at $x}\"}]\n$[# n 3]\n$[# live {time}]";
    let req = serde_json::json!({"version": 1, "command": "call", "inputs": {"std.time": 5},
        "documents": [{"name": "t", "source": source, "calls": [
            {"function": "frames", "args": [1.25, 30]},
            {"function": "frames", "args": [2, 24]},
            {"function": "describe", "args": [{"x": 0.125, "label": "probe"}]},
            {"function": "frames", "args": ["x", 30]},
            {"function": "frames", "args": [1.0]},
            {"function": "n", "args": []},
            {"function": "live", "args": []},
            {"function": "missing", "args": []},
            {"function": "frames", "args": [1e300, 30]}
        ]}]});
    let (code, j, _) = lamlet(&["batch"], Some(&req.to_string()));
    assert_eq!(code, 1);
    let calls = j["documents"][0]["calls"].as_array().unwrap();
    assert_eq!(calls[0]["value"]["data"], 38);
    assert_eq!(calls[1]["value"]["data"], 48, "an integer JSON number is accepted for a Float parameter");
    assert_eq!(calls[2]["value"]["data"], "probe at 0.12");
    assert!(calls[3]["message"].as_str().unwrap().contains("expected Float"));
    assert!(calls[4]["message"].as_str().unwrap().contains("takes 2 argument(s)"));
    assert!(calls[5]["message"].as_str().unwrap().contains("not a function"));
    assert!(calls[6]["message"].as_str().unwrap().contains("not a statically available"));
    assert!(calls[7]["message"].as_str().unwrap().contains("not a statically available"));
    assert_eq!(calls[8]["diagnostics"][0]["kind"], "runtime", "a failed call reports a diagnostic and later calls still run");
    // Positional form.
    let dir = std::env::temp_dir().join(format!("lamlet-cli-call-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.lam");
    std::fs::write(&f, source).unwrap();
    let (code, j, _) = lamlet(&["call", f.to_str().unwrap(), "frames", "0.5", "60"], None);
    assert_eq!(code, 0);
    assert_eq!(j["documents"][0]["calls"][0]["value"]["data"], 30);
    let (code, _, _) = lamlet(&["call", f.to_str().unwrap()], None);
    assert_eq!(code, 2, "a missing function name is a usage error");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn build_writes_each_format() {
    let dir = std::env::temp_dir().join(format!("lamlet-cli-build-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("page.lam");
    std::fs::write(&f, "$[Import* std]$[Import* content]\n# Rooms {#rooms}\n\nOnly ${emph \"two\"} rooms & ${+ 1 1} doors.\n").unwrap();
    let file = f.to_str().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_lamlet")).args(args).output().unwrap();
        (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    // The format comes from -o's extension; prose parsed as CommonMark builds to every format.
    let md = dir.join("page.md");
    assert_eq!(run(&["build", file, "-o", md.to_str().unwrap(), "--prose", "commonmark"]).0, 0);
    assert_eq!(std::fs::read_to_string(&md).unwrap(), "# Rooms\n\nOnly *two* rooms \\& 2 doors.\n");
    let (code, html, _) = run(&["build", file, "--to", "html", "--prose", "commonmark", "--standalone"]);
    assert_eq!(code, 0);
    assert!(html.contains("<title>Rooms</title>") && html.contains("<h1 id=\"rooms\">Rooms</h1>\n<p>Only <em>two</em> rooms &amp; 2 doors.</p>"), "{html}");
    // Speech is an extension's job (`lamlet present speech`), not a format of `build`.
    assert_eq!(run(&["build", file, "--to", "speech"]).0, 2);
    let (_, ir, _) = run(&["build", file, "--to", "ir", "--prepared"]);
    assert!(ir.contains(r#"{"type":"hole""#), "{ir}");
    // Without options, `build` is what `lamlet FILE` prints.
    assert_eq!(run(&["build", file]).1, run(&[file]).1);
    // Usage errors.
    assert_eq!(run(&["build", file, "-o", "page.xyz"]).0, 2);
    assert_eq!(run(&["build", file, "--prepared"]).0, 2);
    assert_eq!(run(&[file, "--to", "md"]).0, 2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_content_package_is_available_to_every_command() {
    let dir = std::env::temp_dir().join(format!("lamlet-cli-content-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("c.lam");
    std::fs::write(&f, "$[Import* content]${emph \"x\"}").unwrap();
    let (code, _, _) = lamlet(&["check", f.to_str().unwrap()], None);
    assert_eq!(code, 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn unknown_commands_run_extensions_from_the_path() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("lamlet-cli-ext-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ext = dir.join("lamlet-hello");
    std::fs::write(&ext, "#!/bin/sh\necho \"hello $*\"\n[ -n \"$LAMLET\" ] && echo has-lamlet\nexit 3\n").unwrap();
    std::fs::set_permissions(&ext, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default());
    let run = |args: &[&str], cwd: &std::path::Path| Command::new(env!("CARGO_BIN_EXE_lamlet")).args(args).env("PATH", &path).current_dir(cwd).output().unwrap();
    let out = run(&["hello", "a", "b"], &dir);
    assert_eq!(out.status.code(), Some(3), "the extension's exit status is lamlet's");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hello a b\nhas-lamlet\n");
    // A file of that name is run as a document instead, and a name with no extension is read as a file.
    let docs = dir.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("hello"), "just text\n").unwrap();
    assert_eq!(String::from_utf8_lossy(&run(&["hello"], &docs).stdout), "just text\n");
    let out = run(&["nothing-here"], &dir);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("reading nothing-here"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_command_reports_each_test() {
    let dir = std::env::temp_dir().join(format!("lamlet-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(
        dir.join("math.lam"),
        "$[Import* std]\n$[# _tests {!tests\n  {!test \"adds\" {expectEqual {+ 1 2} 3}}\n  {!test \"wrong\" {expectEqual {+ 1 2} 4}}\n}]\n",
    )
    .unwrap();
    std::fs::write(dir.join("sub/golden.lam"), "$[Import* std]\n${+ 1 2}\nexpect: 3\n").unwrap();
    let path = dir.to_str().unwrap();

    let (code, j, _) = lamlet(&["test", path, "--json"], None);
    assert_eq!(code, 1);
    let files = j["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let math = files.iter().find(|f| f["file"].as_str().unwrap().ends_with("math.lam")).unwrap();
    let outcomes: Vec<&str> = math["tests"].as_array().unwrap().iter().map(|t| t["outcome"].as_str().unwrap()).collect();
    assert_eq!(outcomes, ["passed", "failed"]);
    assert_eq!(math["tests"][1]["failures"][0]["message"], "expected 4, got 3");
    let golden = files.iter().find(|f| f["file"].as_str().unwrap().ends_with("golden.lam")).unwrap();
    assert_eq!(golden["ok"], true);

    let (code, j, _) = lamlet(&["test", path, "--filter", "adds", "--json"], None);
    assert_eq!(code, 0, "{}", j);

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lamlet"));
    let out = cmd.args(["test", path]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("  FAIL   wrong\n         check 1 of 1: expected 4, got 3"), "{}", text);
    assert!(text.ends_with("2 files, 2 tests: 1 passed, 1 failed, 0 errored\n"), "{}", text);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn fmt_lays_out_code_in_place_and_check_changes_nothing() {
    let dir = std::env::temp_dir().join(format!("lamlet-fmt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("doc.lam");
    let file = file.to_str().unwrap();
    let fields: Vec<String> = (0..8).map(|i| format!("{{Field \"field{i}\" Float}}")).collect();
    let source = format!("Text.\n$[# Thing [% {{Struct {{' {}}}}}]]\n${{3}}\n", fields.join(" "));
    std::fs::write(file, &source).unwrap();

    let (code, _, _) = lamlet(&["fmt", "--check", file], None);
    assert_eq!(code, 1);
    assert_eq!(std::fs::read_to_string(file).unwrap(), source);

    let (code, _, _) = lamlet(&["fmt", file], None);
    assert_eq!(code, 0);
    let formatted = std::fs::read_to_string(file).unwrap();
    assert!(formatted.starts_with("Text.\n$[# Thing [% {Struct {'\n    {Field \"field0\" Float}\n"), "{formatted}");
    assert!(formatted.ends_with("}}]]\n${3}\n"), "{formatted}");

    let (code, _, _) = lamlet(&["fmt", "--check", file], None);
    assert_eq!(code, 0);
    let (code, _, _) = lamlet(&["fmt", "--width"], None);
    assert_eq!(code, 2);
    std::fs::remove_dir_all(&dir).unwrap();
}
