//! A text preparer: check a document, fill in its computed values, and hand the finished text to a slow step
//! (here it is printed; a real host would send it to speech synthesis or a typesetter).
//!
//! `cargo run -p lamlet --example host_text`

use lamlet::embed::*;

const DOCUMENT: &str = r#"$[Import* std]$[Import* garden]
$[# trees {' "oak" "elm" "ash"}]
$[# total [% {fold trees 0 {^ {sum t} {+ sum {leaves t}}}}]]
The garden has $[% {length trees}] trees and $total leaves in all.
$[% {#picture {. file "garden.png" caption "The garden in spring"}}]
Today is day ${today} of the season.
"#;

fn main() -> Result<(), Error> {
    let mut engine = Engine::new();

    // A host function. `pure()` lets the document call it during the check, inside `[% ...]`.
    engine
        .natives_mut()
        .function("garden.count/leaves", |_, (tree,): (String,)| match tree.as_str() {
            "oak" => Ok(42_289i64),
            "elm" => Ok(31_007),
            "ash" => Ok(18_550),
            other => Err(format!("no tree named {:?}", other)),
        })
        .pure()
        .doc("Fn([String], Int). Leaves on one of the garden's trees.")
        .register()?;
    // A runtime host function: it reads an input the host sets on the instance.
    engine.natives_mut().function("garden.calendar/today", |ctx, ()| ctx.input_as::<i64>("day")).register()?;

    // Documents import the host functions as a package generated from the registry.
    let package = engine.natives().declarations("garden")?;
    let mut packages = MemoryResolver::new();
    packages.add("garden", &package, Exports::All);
    engine.set_resolver(packages);

    let doc = engine.check(SourceFile::new("garden.txt", DOCUMENT))?;

    // `parts()` lists the document piece by piece. Regions computed during the check carry their value;
    // `${today}` needs runtime input, so it has none yet.
    for part in doc.parts() {
        if let Part::Region { region, value } = part {
            let shown = value.as_ref().map(display).unwrap_or_else(|| "(computed at runtime)".into());
            println!("region {} at line {}: {} = {}", region.index, region.at.line, region.type_text, shown);
        }
    }

    // The host decides how each kind of value appears: text as itself, a picture description as markup.
    let format = |region: &Region, value: &Value| match Data::from_value(value)? {
        Data::Variant(tag, payload) if tag == "picture" => match *payload {
            Data::Record(fields) => match (&fields["file"], &fields["caption"]) {
                (Data::String(file), Data::String(caption)) => Ok(format!("[image {} \"{}\"]", file, caption)),
                _ => Err("a picture needs a file and a caption".to_string()),
            },
            _ => Err("a picture's payload is a record".to_string()),
        },
        _ => text(region, value),
    };

    let mut instance = engine.instance(&doc)?;
    instance.set_input("day", 12i64)?;
    let prepared = doc.render_with(&mut instance, format)?;
    println!("\n{}", prepared);

    assert_eq!(
        prepared,
        "The garden has 3 trees and 91846 leaves in all.\n[image garden.png \"The garden in spring\"]\nToday is day 12 of the season.\n"
    );
    Ok(())
}
