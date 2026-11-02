//! A build-time checker: validate the Lamlet blocks of a Markdown file without running any host code, and
//! report errors at their lines in the Markdown file. A build script or CI job would do this.
//!
//! `cargo run -p lamlet --example host_build_check`

use lamlet::embed::*;

const CHAPTER: &str = r#"# Chapter 3

Some prose that is not Lamlet at all.

```lamlet
$[Import* std]$[Import* garden]
The oak has ${leaves "oak"} leaves.
```

More prose.

```lamlet
$[Import* std]
The total is ${+ 1 "two"}.
```
"#;

/// The ```lamlet blocks of a Markdown file, each with the line its text starts on.
fn lamlet_blocks(markdown: &str) -> Vec<(usize, String)> {
    let mut blocks = Vec::new();
    let mut current: Option<(usize, String)> = None;
    for (i, line) in markdown.lines().enumerate() {
        match (&mut current, line.trim()) {
            (None, "```lamlet") => current = Some((i + 2, String::new())),
            (Some(_), "```") => blocks.push(current.take().unwrap()),
            (Some((_, text)), _) => {
                text.push_str(line);
                text.push('\n');
            }
            (None, _) => {}
        }
    }
    blocks
}

fn main() -> Result<(), Error> {
    // At build time the host's functions are described, not implemented. `check_interface` never calls them.
    let mut engine = Engine::new();
    engine
        .natives_mut()
        .function("garden.count/leaves", |_, (_tree,): (String,)| -> Result<i64, String> { unreachable!("not called at build time") })
        .doc("Fn([String], Int). Leaves on one of the garden's trees.")
        .register()?;
    let mut packages = MemoryResolver::new();
    packages.add("garden", &engine.natives().declarations("garden")?, Exports::All);
    engine.set_resolver(packages);

    let mut errors = Vec::new();
    for (line, text) in lamlet_blocks(CHAPTER) {
        // `at(line, 1)` places the block inside chapter.md, so locations name lines of that file.
        match engine.check_interface(SourceFile::new("chapter.md", text).at(line, 1)) {
            Ok(doc) => {
                let packages: Vec<String> = doc.dependencies().into_iter().map(|(name, _)| name).collect();
                println!("chapter.md:{}: ok (packages: {})", line, packages.join(", "));
            }
            Err(e) => {
                println!("{}", e);
                errors.push(e);
            }
        }
    }

    assert_eq!(errors.len(), 1);
    let at = errors[0].locations()[0].clone().expect("the error has a position");
    assert_eq!((at.file.as_str(), at.line), ("chapter.md", 14));
    assert_eq!(errors[0].kind(), ErrorKind::Check);
    Ok(())
}
