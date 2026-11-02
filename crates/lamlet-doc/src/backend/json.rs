//! The IR as JSON: `{"lamletDoc": 1, "blocks": [...]}`, the interface for other tools and for backends in other
//! languages. Written by hand, since the crate has no dependencies.
//!
//! Every node is an object with a `"type"` (`"paragraph"`, `"text"`, `"link"`, …) and the node's fields, named
//! as in [`crate::ir`] in camel case. Attributes are `[[key, value], ...]`, in order. A position is
//! `{"file", "line", "column", "endLine", "endColumn"}`. A `value` node has its data as JSON (Unit is `null`, a
//! variant `{"tag", "value"}`, a type `{"type": "written form"}`) and its written form as `"text"`.

use crate::ir::{Attrs, Block, Doc, Inline};
use crate::{written_form, Error};
use lamlet::embed::{Data, Location};

/// The version of the JSON form, raised on incompatible changes.
pub const VERSION: u32 = 1;

/// The document's IR as JSON, on one line.
pub fn write(doc: &Doc) -> Result<String, Error> {
    let mut o = Obj::new();
    o.raw("lamletDoc", &VERSION.to_string());
    o.raw("blocks", &array(doc.blocks.iter().map(block))?);
    Ok(o.end() + "\n")
}

/// A JSON string literal.
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON number for a Float: integral values keep a `.0`, as Lamlet writes them.
pub fn float(x: f64) -> String {
    format!("{:?}", x)
}

pub(crate) struct Obj(String);

impl Obj {
    pub(crate) fn new() -> Obj {
        Obj(String::from("{"))
    }
    pub(crate) fn raw(&mut self, key: &str, json: &str) -> &mut Obj {
        if self.0.len() > 1 {
            self.0.push(',');
        }
        self.0.push_str(&string(key));
        self.0.push(':');
        self.0.push_str(json);
        self
    }
    pub(crate) fn str(&mut self, key: &str, value: &str) -> &mut Obj {
        self.raw(key, &string(value))
    }
    pub(crate) fn at(&mut self, at: &Option<Location>) -> &mut Obj {
        if let Some(a) = at {
            self.raw("at", &location(a));
        }
        self
    }
    pub(crate) fn end(&mut self) -> String {
        self.0.push('}');
        std::mem::take(&mut self.0)
    }
}

pub(crate) fn array<I: Iterator<Item = Result<String, Error>>>(items: I) -> Result<String, Error> {
    Ok(format!("[{}]", items.collect::<Result<Vec<_>, _>>()?.join(",")))
}

pub(crate) fn location(a: &Location) -> String {
    let mut o = Obj::new();
    o.str("file", &a.file)
        .raw("line", &a.line.to_string())
        .raw("column", &a.column.to_string())
        .raw("endLine", &a.end_line.to_string())
        .raw("endColumn", &a.end_column.to_string());
    o.end()
}

fn attrs(a: &Attrs) -> String {
    format!("[{}]", a.iter().map(|(k, v)| format!("[{},{}]", string(k), string(v))).collect::<Vec<_>>().join(","))
}

/// Data as JSON.
pub fn data(d: &Data) -> String {
    match d {
        Data::Unit => "null".into(),
        Data::Bool(b) => b.to_string(),
        Data::Int(n) => n.to_string(),
        Data::Float(x) => float(*x),
        Data::String(s) => string(s),
        Data::List(xs) => format!("[{}]", xs.iter().map(data).collect::<Vec<_>>().join(",")),
        Data::Record(r) => {
            let mut o = Obj::new();
            for (k, v) in r {
                o.raw(k, &data(v));
            }
            o.end()
        }
        Data::Variant(tag, v) => {
            let mut o = Obj::new();
            o.str("tag", tag).raw("value", &data(v));
            o.end()
        }
        Data::Type(_) => {
            let text = d.to_value().map(|v| lamlet::embed::display(&v)).unwrap_or_default();
            let mut o = Obj::new();
            o.str("type", &text);
            o.end()
        }
    }
}

fn inlines(xs: &[Inline]) -> Result<String, Error> {
    array(xs.iter().map(inline))
}

fn block(b: &Block) -> Result<String, Error> {
    let mut o = Obj::new();
    match b {
        Block::Plain(xs) => o.str("type", "plain").raw("inlines", &inlines(xs)?),
        Block::Paragraph(xs) => o.str("type", "paragraph").raw("inlines", &inlines(xs)?),
        Block::Heading { level, id, attrs: a, inlines: xs } => {
            o.str("type", "heading").raw("level", &level.to_string());
            if let Some(id) = id {
                o.str("id", id);
            }
            o.raw("attrs", &attrs(a)).raw("inlines", &inlines(xs)?)
        }
        Block::List { ordered, items } => {
            o.str("type", "list").raw("ordered", &ordered.to_string()).raw("items", &array(items.iter().map(|i| array(i.iter().map(block))))?)
        }
        Block::Table { header, rows } => o
            .str("type", "table")
            .raw("header", &array(header.iter().map(|c| inlines(c)))?)
            .raw("rows", &array(rows.iter().map(|r| array(r.iter().map(|c| inlines(c)))))?),
        Block::CodeBlock { lang, text, at } => o.str("type", "codeBlock").str("lang", lang).str("text", text).at(at),
        Block::Raw { format, text, at } => o.str("type", "rawBlock").str("format", format).str("text", text).at(at),
        Block::Div { name, attrs: a, blocks } => o.str("type", "div").str("name", name).raw("attrs", &attrs(a)).raw("blocks", &array(blocks.iter().map(block))?),
    };
    Ok(o.end())
}

fn inline(x: &Inline) -> Result<String, Error> {
    let mut o = Obj::new();
    match x {
        Inline::Text { text, at } => o.str("type", "text").str("text", text).at(at),
        Inline::Raw { format, text, at } => o.str("type", "raw").str("format", format).str("text", text).at(at),
        Inline::Code { text, at } => o.str("type", "code").str("text", text).at(at),
        Inline::Value { data: d, at } => o.str("type", "value").raw("data", &data(d)).str("text", &written_form(d, at)?).at(at),
        Inline::Hole { region, at } => o.str("type", "hole").raw("region", &region.to_string()).at(at),
        Inline::Emph(xs) => o.str("type", "emph").raw("inlines", &inlines(xs)?),
        Inline::Strong(xs) => o.str("type", "strong").raw("inlines", &inlines(xs)?),
        Inline::Link { href, inlines: xs } => o.str("type", "link").str("href", href).raw("inlines", &inlines(xs)?),
        Inline::Image { src, alt, title, at } => {
            o.str("type", "image").str("src", src).str("alt", alt);
            if let Some(t) = title {
                o.str("title", t);
            }
            o.at(at)
        }
        Inline::Math { tex, display, at } => o.str("type", "math").str("tex", tex).raw("display", &display.to_string()).at(at),
        Inline::Break => o.str("type", "break"),
        Inline::Span { name, attrs: a, inlines: xs } => o.str("type", "span").str("name", name).raw("attrs", &attrs(a)).raw("inlines", &inlines(xs)?),
        Inline::Say { shown, spoken } => o.str("type", "say").str("spoken", spoken).raw("shown", &inlines(shown)?),
        Inline::Anchor { name, attrs: a, at } => o.str("type", "anchor").str("name", name).raw("attrs", &attrs(a)).at(at),
        Inline::Pause { seconds, at } => o.str("type", "pause").raw("seconds", &float(*seconds)).at(at),
    };
    Ok(o.end())
}
