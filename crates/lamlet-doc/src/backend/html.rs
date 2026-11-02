//! The HTML backend: a fragment, or with [`standalone`] a whole page.
//!
//! - `Raw` in [`crate::HTML`] is copied; other formats are refused. `Text` is escaped.
//! - `Value` and `Code` are `<code>`. `Math` is `\(tex\)` or `\[tex\]` in a `math` span, as MathJax and KaTeX
//!   read it.
//! - `Span` and `Div` are `<span>` and `<div>` with the name as their class and other attributes as `data-`
//!   attributes. `Say` writes what is shown; `Anchor` and `Pause` write nothing.

use crate::ir::{Attrs, Block, Doc, Inline};
use crate::{written_form, Error, HTML};
use lamlet::embed::Location;

/// The document as an HTML fragment.
pub fn write(doc: &Doc) -> Result<String, Error> {
    let mut out = String::new();
    blocks(&mut out, &doc.blocks)?;
    Ok(out)
}

/// An HTML page around a fragment, with a title.
pub fn standalone(body: &str, title: &str) -> String {
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n</head>\n<body>\n{}{}</body>\n</html>\n",
        escape(title),
        body,
        if body.ends_with('\n') { "" } else { "\n" }
    )
}

/// The text of the document's first heading, for a page title.
pub fn first_heading(doc: &Doc) -> Option<String> {
    fn find(bs: &[Block]) -> Option<&Vec<Inline>> {
        bs.iter().find_map(|b| match b {
            Block::Heading { inlines, .. } => Some(inlines),
            Block::Div { blocks, .. } => find(blocks),
            _ => None,
        })
    }
    find(&doc.blocks).and_then(|xs| crate::backend::text::write(&Doc { blocks: vec![Block::Plain(xs.clone())] }).ok())
}

pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn foreign(format: &str, at: &Option<Location>) -> Error {
    Error::at(at, format!("this is markup for `{}`, which cannot be written as HTML", format))
}

fn attributes(name: &str, attrs: &Attrs) -> String {
    let mut s = format!(" class=\"{}\"", escape(name));
    for (k, v) in attrs {
        let key: String = k.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' }).collect();
        s.push_str(&format!(" data-{}=\"{}\"", key, escape(v)));
    }
    s
}

fn blocks(out: &mut String, bs: &[Block]) -> Result<(), Error> {
    for (i, b) in bs.iter().enumerate() {
        if i > 0 && !matches!(b, Block::Plain(_)) && !matches!(bs[i - 1], Block::Plain(_)) {
            out.push('\n');
        }
        block(out, b)?;
    }
    Ok(())
}

fn block(out: &mut String, b: &Block) -> Result<(), Error> {
    match b {
        Block::Plain(xs) => inlines(out, xs)?,
        Block::Paragraph(xs) => {
            out.push_str("<p>");
            inlines(out, xs)?;
            out.push_str("</p>");
        }
        Block::Heading { level, id, inlines: xs, .. } => {
            match id {
                Some(id) => out.push_str(&format!("<h{} id=\"{}\">", level, escape(id))),
                None => out.push_str(&format!("<h{}>", level)),
            }
            inlines(out, xs)?;
            out.push_str(&format!("</h{}>", level));
        }
        Block::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            out.push_str(&format!("<{}>\n", tag));
            for item in items {
                out.push_str("<li>");
                match &item[..] {
                    [Block::Plain(xs)] => inlines(out, xs)?,
                    bs => blocks(out, bs)?,
                }
                out.push_str("</li>\n");
            }
            out.push_str(&format!("</{}>", tag));
        }
        Block::Table { header, rows } => {
            out.push_str("<table>\n<thead>\n<tr>");
            for c in header {
                out.push_str("<th>");
                inlines(out, c)?;
                out.push_str("</th>");
            }
            out.push_str("</tr>\n</thead>\n<tbody>\n");
            for r in rows {
                out.push_str("<tr>");
                for c in r {
                    out.push_str("<td>");
                    inlines(out, c)?;
                    out.push_str("</td>");
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</tbody>\n</table>");
        }
        Block::CodeBlock { lang, text, .. } => {
            let class = if lang.is_empty() { String::new() } else { format!(" class=\"language-{}\"", escape(lang)) };
            out.push_str(&format!("<pre><code{}>{}</code></pre>", class, escape(text.trim_end_matches('\n'))));
        }
        Block::Raw { format, text, .. } if format == HTML => out.push_str(text),
        Block::Raw { format, at, .. } => return Err(foreign(format, at)),
        Block::Div { name, attrs, blocks: bs } => {
            out.push_str(&format!("<div{}>\n", attributes(name, attrs)));
            blocks(out, bs)?;
            out.push_str("\n</div>");
        }
    }
    Ok(())
}

fn inlines(out: &mut String, xs: &[Inline]) -> Result<(), Error> {
    xs.iter().try_for_each(|x| inline(out, x))
}

fn inline(out: &mut String, node: &Inline) -> Result<(), Error> {
    match node {
        Inline::Text { text, .. } => out.push_str(&escape(text)),
        Inline::Raw { format, text, .. } if format == HTML => out.push_str(text),
        Inline::Raw { format, at, .. } => return Err(foreign(format, at)),
        Inline::Code { text, .. } => out.push_str(&format!("<code>{}</code>", escape(text))),
        Inline::Value { data, at } => out.push_str(&format!("<code>{}</code>", escape(&written_form(data, at)?))),
        Inline::Hole { region, at } => return Err(Error::at(at, format!("region {} is evaluated at runtime and has no value yet", region))),
        Inline::Emph(xs) => {
            out.push_str("<em>");
            inlines(out, xs)?;
            out.push_str("</em>");
        }
        Inline::Strong(xs) => {
            out.push_str("<strong>");
            inlines(out, xs)?;
            out.push_str("</strong>");
        }
        Inline::Link { href, inlines: xs } => {
            out.push_str(&format!("<a href=\"{}\">", escape(href)));
            inlines(out, xs)?;
            out.push_str("</a>");
        }
        Inline::Image { src, alt, title, .. } => {
            out.push_str(&format!("<img src=\"{}\" alt=\"{}\"", escape(src), escape(alt)));
            if let Some(t) = title {
                out.push_str(&format!(" title=\"{}\"", escape(t)));
            }
            out.push('>');
        }
        Inline::Math { tex, display: false, .. } => out.push_str(&format!("<span class=\"math inline\">\\({}\\)</span>", escape(tex))),
        Inline::Math { tex, display: true, .. } => out.push_str(&format!("<span class=\"math display\">\\[{}\\]</span>", escape(tex))),
        Inline::Break => out.push_str("<br>\n"),
        Inline::Span { name, attrs, inlines: xs } => {
            out.push_str(&format!("<span{}>", attributes(name, attrs)));
            inlines(out, xs)?;
            out.push_str("</span>");
        }
        Inline::Say { shown, .. } => inlines(out, shown)?,
        Inline::Anchor { .. } | Inline::Pause { .. } => {}
    }
    Ok(())
}
