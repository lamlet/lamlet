//! Content: the `content` Lamlet package, and the events it produces turned into IR nodes.
//!
//! Lamlet has no recursive types except `List`, so content is a flat list of events (`#open`, `#close`, `#text`,
//! `#leaf`). Here the events become a tree, checking that every opening has its closing and that each element
//! holds what it may: inlines in a paragraph, items in a list, rows in a table.

use crate::ir::{attr, attr_str, Attrs, Block, Inline};
use crate::Error;
use lamlet::embed::{Data, Location};

/// The `content` package's Lamlet source. Hosts add it to their resolver as `content`.
pub const CONTENT_SOURCE: &str = include_str!("content.lam");

/// The written type of `Content` (`List(Event)`): a region of this type lowers to content.
pub const CONTENT_TYPE: &str = "List(<close: String, leaf: {attrs: List({key: String, value: Data}), name: String}, open: {attrs: List({key: String, value: Data}), name: String}, text: String>)";

/// A piece of lowered content: an inline, or a block that interrupts the surrounding inlines.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Inline(Inline),
    Block(Block),
}

enum Event {
    Open(String, Attrs),
    Close(String),
    Text(String),
    Leaf(String, Attrs),
}

/// An element being built, or built: list items, table rows, and cells exist only inside their parents.
enum Node {
    Inline(Inline),
    Block(Block),
    Item(Vec<Block>),
    Row { head: bool, cells: Vec<Vec<Inline>> },
    Cell(Vec<Inline>),
}

/// Content events (the value of a region of type `Content`) as pieces of the IR.
pub fn lower_content(data: &Data, at: &Option<Location>) -> Result<Vec<Piece>, Error> {
    let events = events(data).map_err(|m| Error::at(at, m))?;
    let mut pos = 0;
    let nodes = sequence(&events, &mut pos, None, at)?;
    nodes
        .into_iter()
        .map(|n| match n {
            Node::Inline(i) => Ok(Piece::Inline(i)),
            Node::Block(b) => Ok(Piece::Block(b)),
            Node::Item(_) => Err(Error::at(at, "a list item outside a list")),
            Node::Row { .. } => Err(Error::at(at, "a table row outside a table")),
            Node::Cell(_) => Err(Error::at(at, "a table cell outside a row")),
        })
        .collect()
}

fn events(data: &Data) -> Result<Vec<Event>, String> {
    let Data::List(items) = data else { return Err("content is a list of events".into()) };
    items
        .iter()
        .map(|e| {
            let Data::Variant(tag, payload) = e else { return Err("content holds something that is not an event".into()) };
            Ok(match (tag.as_str(), &**payload) {
                ("text", Data::String(s)) => Event::Text(s.clone()),
                ("close", Data::String(s)) => Event::Close(s.clone()),
                ("open", el) => {
                    let (n, a) = element(el)?;
                    Event::Open(n, a)
                }
                ("leaf", el) => {
                    let (n, a) = element(el)?;
                    Event::Leaf(n, a)
                }
                _ => return Err(format!("`#{}` is not a content event", tag)),
            })
        })
        .collect()
}

fn element(d: &Data) -> Result<(String, Attrs), String> {
    let Data::Record(r) = d else { return Err("an element is a record".into()) };
    let (Some(Data::String(name)), Some(Data::List(attrs))) = (r.get("name"), r.get("attrs")) else {
        return Err("an element has a name and attributes".into());
    };
    let attrs = attrs
        .iter()
        .map(|a| match a {
            Data::Record(r) => match (r.get("key"), r.get("value")) {
                (Some(Data::String(k)), Some(v)) => Ok((k.clone(), v.clone())),
                _ => Err("an attribute has a String key and a value".to_string()),
            },
            _ => Err("an attribute is a record".to_string()),
        })
        .collect::<Result<_, _>>()?;
    Ok((name.clone(), attrs))
}

/// Nodes up to the closing of `open` (or the end, at the top).
fn sequence(events: &[Event], pos: &mut usize, open: Option<&str>, at: &Option<Location>) -> Result<Vec<Node>, Error> {
    let mut out = Vec::new();
    while *pos < events.len() {
        let e = &events[*pos];
        *pos += 1;
        match e {
            Event::Text(s) => out.push(Node::Inline(Inline::Text { text: s.clone(), at: at.clone() })),
            Event::Leaf(name, attrs) => out.push(leaf(name, attrs, at)?),
            Event::Close(name) => {
                return match open {
                    Some(o) if o == name => Ok(out),
                    Some(o) => Err(Error::at(at, format!("`{}` is closed while `{}` is open", name, o))),
                    None => Err(Error::at(at, format!("`{}` is closed but was never opened", name))),
                };
            }
            Event::Open(name, attrs) => {
                let children = sequence(events, pos, Some(name), at)?;
                out.push(element_node(name, attrs, children, at)?);
            }
        }
    }
    match open {
        Some(o) => Err(Error::at(at, format!("`{}` is opened but never closed", o))),
        None => Ok(out),
    }
}

/// A String attribute the element needs.
fn need<'a>(attrs: &'a Attrs, key: &str, what: &str, at: &Option<Location>) -> Result<&'a str, Error> {
    match attr(attrs, key) {
        Some(Data::String(s)) => Ok(s),
        Some(_) => Err(Error::at(at, format!("{}'s attribute `{}` is a String", what, key))),
        None => Err(Error::at(at, format!("{} needs the attribute `{}`", what, key))),
    }
}

/// The attributes other than the ones an element uses itself.
fn rest(attrs: &Attrs, used: &[&str]) -> Attrs {
    attrs.iter().filter(|(k, _)| !used.contains(&k.as_str())).cloned().collect()
}

fn leaf(name: &str, attrs: &Attrs, at: &Option<Location>) -> Result<Node, Error> {
    let at_ = at.clone();
    Ok(match name {
        "image" => Node::Inline(Inline::Image {
            src: need(attrs, "src", "an image", at)?.into(),
            alt: attr_str(attrs, "alt").unwrap_or("").into(),
            title: attr_str(attrs, "title").map(String::from),
            at: at_,
        }),
        "code" => Node::Inline(Inline::Code { text: need(attrs, "text", "code", at)?.into(), at: at_ }),
        "math" => Node::Inline(Inline::Math { tex: need(attrs, "tex", "a formula", at)?.into(), display: attr_str(attrs, "display") == Some("true"), at: at_ }),
        "break" => Node::Inline(Inline::Break),
        "raw" => Node::Inline(Inline::Raw { format: need(attrs, "format", "raw markup", at)?.into(), text: need(attrs, "text", "raw markup", at)?.into(), at: at_ }),
        "rawBlock" => Node::Block(Block::Raw { format: need(attrs, "format", "raw markup", at)?.into(), text: need(attrs, "text", "raw markup", at)?.into(), at: at_ }),
        "codeBlock" => Node::Block(Block::CodeBlock { lang: attr_str(attrs, "lang").unwrap_or("").into(), text: need(attrs, "text", "a code block", at)?.into(), at: at_ }),
        "anchor" => Node::Inline(Inline::Anchor { name: need(attrs, "name", "an anchor", at)?.into(), attrs: rest(attrs, &["name"]), at: at_ }),
        "pause" => {
            let s = need(attrs, "seconds", "a pause", at)?;
            let seconds: f64 = s.parse().ok().filter(|x: &f64| x.is_finite() && *x >= 0.0).ok_or_else(|| Error::at(at, format!("a pause of {:?} seconds", s)))?;
            Node::Inline(Inline::Pause { seconds, at: at_ })
        }
        other => return Err(Error::at(at, format!("`{}` is not a content element with no children", other))),
    })
}

fn inlines(children: Vec<Node>, what: &str, at: &Option<Location>) -> Result<Vec<Inline>, Error> {
    children
        .into_iter()
        .map(|n| match n {
            Node::Inline(i) => Ok(i),
            _ => Err(Error::at(at, format!("{} holds only inline content (text, emphasis, links, …)", what))),
        })
        .collect()
}

/// Children as blocks: runs of inlines become `Plain` blocks.
fn blocks(children: Vec<Node>, what: &str, at: &Option<Location>) -> Result<Vec<Block>, Error> {
    let mut out = Vec::new();
    let mut run = Vec::new();
    for n in children {
        match n {
            Node::Inline(i) => run.push(i),
            Node::Block(b) => {
                if !run.is_empty() {
                    out.push(Block::Plain(std::mem::take(&mut run)));
                }
                out.push(b);
            }
            _ => return Err(Error::at(at, format!("{} holds blocks and inlines, not list items, rows, or cells", what))),
        }
    }
    if !run.is_empty() {
        out.push(Block::Plain(run));
    }
    Ok(out)
}

fn element_node(name: &str, attrs: &Attrs, children: Vec<Node>, at: &Option<Location>) -> Result<Node, Error> {
    Ok(match name {
        "emph" => Node::Inline(Inline::Emph(inlines(children, "emphasis", at)?)),
        "strong" => Node::Inline(Inline::Strong(inlines(children, "strong emphasis", at)?)),
        "link" => Node::Inline(Inline::Link { href: need(attrs, "href", "a link", at)?.into(), inlines: inlines(children, "a link", at)? }),
        "span" => Node::Inline(Inline::Span { name: need(attrs, "name", "a span", at)?.into(), attrs: rest(attrs, &["name"]), inlines: inlines(children, "a span", at)? }),
        "say" => Node::Inline(Inline::Say { spoken: need(attrs, "spoken", "`say`", at)?.into(), shown: inlines(children, "`say`", at)? }),
        "paragraph" => Node::Block(Block::Paragraph(inlines(children, "a paragraph", at)?)),
        "heading" => {
            let l = need(attrs, "level", "a heading", at)?;
            let level = l.parse::<u8>().ok().filter(|n| (1..=6).contains(n)).ok_or_else(|| Error::at(at, format!("a heading's level is 1 to 6, not {}", l)))?;
            Node::Block(Block::Heading { level, id: attr_str(attrs, "id").map(String::from), attrs: rest(attrs, &["level", "id"]), inlines: inlines(children, "a heading", at)? })
        }
        "list" => {
            let items = children
                .into_iter()
                .map(|n| match n {
                    Node::Item(b) => Ok(b),
                    _ => Err(Error::at(at, "a list holds only items")),
                })
                .collect::<Result<_, _>>()?;
            Node::Block(Block::List { ordered: attr_str(attrs, "ordered") == Some("true"), items })
        }
        "item" => Node::Item(blocks(children, "a list item", at)?),
        "table" => {
            let mut header = None;
            let mut rows = Vec::new();
            for n in children {
                match n {
                    Node::Row { head: true, cells } if header.is_none() && rows.is_empty() => header = Some(cells),
                    Node::Row { head: true, .. } => return Err(Error::at(at, "a table has one header row, before the others")),
                    Node::Row { head: false, cells } => rows.push(cells),
                    _ => return Err(Error::at(at, "a table holds only rows")),
                }
            }
            let header = header.ok_or_else(|| Error::at(at, "a table needs a header row"))?;
            if let Some(r) = rows.iter().find(|r| r.len() != header.len()) {
                return Err(Error::at(at, format!("a table row has {} cells but the header has {}", r.len(), header.len())));
            }
            Node::Block(Block::Table { header, rows })
        }
        "head" | "row" => {
            let cells = children
                .into_iter()
                .map(|n| match n {
                    Node::Cell(c) => Ok(c),
                    _ => Err(Error::at(at, "a table row holds only cells")),
                })
                .collect::<Result<_, _>>()?;
            Node::Row { head: name == "head", cells }
        }
        "cell" => Node::Cell(inlines(children, "a table cell", at)?),
        "group" => Node::Block(Block::Div { name: need(attrs, "name", "a group", at)?.into(), attrs: rest(attrs, &["name"]), blocks: blocks(children, "a group", at)? }),
        other => return Err(Error::at(at, format!("`{}` is not a content element", other))),
    })
}
