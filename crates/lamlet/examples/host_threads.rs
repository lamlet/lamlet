//! Several threads: documents are prepared on worker threads, and the results are used on the main thread.
//!
//! `Engine`, `Document`, and `Instance` stay on the thread that made them (they are not `Send`). Each worker
//! builds its own engine, and results cross threads as plain Rust values: rendered `String`s and `Data`.
//!
//! `cargo run -p lamlet --example host_threads`

use lamlet::embed::*;
use std::sync::mpsc;
use std::thread;

/// What a worker sends back: the rendered text and the document's `summary` binding as plain data.
struct Prepared {
    name: String,
    text: Result<String, String>,
    summary: Option<Data>,
}

/// Every worker builds the same engine. A real host keeps this in one function, so all threads agree.
fn engine() -> Engine {
    let mut e = Engine::new();
    e.natives_mut().function("shop.price/cents", |_, (item,): (String,)| Ok(item.len() as i64 * 125)).pure().register().unwrap();
    let mut packages = MemoryResolver::new();
    packages.add("shop", &e.natives().declarations("shop").unwrap(), Exports::All);
    e.set_resolver(packages);
    e
}

fn main() {
    let documents: Vec<(String, String)> = ["pear", "melon", "fig", "kiwi"]
        .iter()
        .map(|item| {
            let source = format!(
                "$[Import* std]$[Import* shop]$[# price [% {{cents \"{item}\"}}]]$[# summary [% {{. item \"{item}\" cents price}}]]A {item} costs $price cents."
            );
            (format!("{item}.txt"), source)
        })
        .chain(std::iter::once(("broken.txt".to_string(), "$[Import* std]${+ 1 true}".to_string())))
        .collect();

    let (send, receive) = mpsc::channel::<Prepared>();
    let workers: Vec<_> = documents
        .chunks(2)
        .map(|chunk| {
            let chunk = chunk.to_vec();
            let send = send.clone();
            thread::spawn(move || {
                let engine = engine();
                for (name, source) in chunk {
                    let prepared = match engine.check(SourceFile::new(name.clone(), source)) {
                        Ok(doc) => Prepared {
                            text: doc.render(text).map_err(|e| e.to_string()),
                            summary: doc.binding("summary").ok().and_then(|b| Data::from_value(&b.value).ok()),
                            name,
                        },
                        Err(e) => Prepared { name, text: Err(e.to_string()), summary: None },
                    };
                    send.send(prepared).unwrap();
                }
            })
        })
        .collect();
    drop(send);

    let mut results: Vec<Prepared> = receive.iter().collect();
    for w in workers {
        w.join().unwrap();
    }
    results.sort_by(|a, b| a.name.cmp(&b.name));
    for r in &results {
        match (&r.text, &r.summary) {
            (Ok(text), Some(Data::Record(fields))) => println!("{:12} {:24} cents = {:?}", r.name, text, fields["cents"]),
            (Ok(text), _) => println!("{:12} {}", r.name, text),
            (Err(e), _) => println!("{:12} error: {}", r.name, e.lines().next().unwrap_or("")),
        }
    }

    assert_eq!(results.len(), 5);
    assert_eq!(results.iter().filter(|r| r.text.is_err()).count(), 1);
    let melon = results.iter().find(|r| r.name == "melon.txt").unwrap();
    assert_eq!(melon.text.as_deref(), Ok("A melon costs 625 cents."));
}
