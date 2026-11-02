//! Print the docs visible in a document: `cargo run -p lamlet --example docs FILE`.
use lamlet::*;
use std::rc::Rc;
fn main() {
    let src = match std::env::args().nth(1) {
        Some(p) => std::fs::read_to_string(p).unwrap(),
        None => "$[Import* std]".to_string(),
    };
    let doc = check_document("<docs>", &src, MemoryResolver::new().into_rc(), Rc::new(NativeRegistry::new()), &Options::default())
        .unwrap_or_else(|f| panic!("{}", f.render()));
    for (name, d) in &doc.docs {
        println!("{}: {}", name, d.replace('\n', " "));
    }
}
