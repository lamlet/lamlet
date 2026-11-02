//! The plain text backend: what `lamlet FILE` prints. Content is written without markup.
//!
//! | Node | Written as |
//! |---|---|
//! | `Text`, `Code`, `Math` | as it is (plain text escapes nothing) |
//! | `Raw` in [`crate::TEXT`] | as it is; `Raw` in another format is an error |
//! | `Value` | Lamlet's written form: `[1, 2]`, `{x: 1.0, y: 2.5}`, `#some(3)` |
//! | `Emph`, `Strong`, `Span`, `Say` | their content (`Say`: what is shown) |
//! | `Link` | its text, then the target in parentheses when it differs |
//! | `Image` | its alternative text |
//! | `Anchor`, `Pause` | nothing |
//! | `Plain` | its inlines |
//! | `Paragraph`, `Heading` | their inlines |
//! | `List` | `- item` or `1. item`, one per line |
//! | `Table` | aligned columns, with a rule under the header |
//! | `CodeBlock` | its text |
//!
//! Blocks that come from content are separated by a blank line; prose around them keeps its own line breaks.

use crate::ir::{Block, Doc, Inline};
use crate::{written_form, Error, TEXT};

/// The document as plain text.
pub fn write(doc: &Doc) -> Result<String, Error> {
    blocks(&doc.blocks)
}

fn blocks(bs: &[Block]) -> Result<String, Error> {
    let mut out = String::new();
    for (i, b) in bs.iter().enumerate() {
        if i > 0 && !matches!(b, Block::Plain(_)) && !matches!(bs[i - 1], Block::Plain(_)) {
            out.push_str("\n\n");
        }
        block(&mut out, b)?;
    }
    Ok(out)
}

fn block(out: &mut String, b: &Block) -> Result<(), Error> {
    match b {
        Block::Plain(xs) | Block::Paragraph(xs) | Block::Heading { inlines: xs, .. } => out.push_str(&inlines(xs)?),
        Block::List { ordered, items } => {
            let mut lines = Vec::new();
            for (n, item) in items.iter().enumerate() {
                let marker = if *ordered { format!("{}. ", n + 1) } else { "- ".to_string() };
                let body = blocks(item)?;
                let indent = " ".repeat(marker.len());
                let mut first = true;
                for line in body.split('\n') {
                    lines.push(if first { format!("{marker}{line}") } else if line.is_empty() { String::new() } else { format!("{indent}{line}") });
                    first = false;
                }
            }
            out.push_str(&lines.join("\n"));
        }
        Block::Table { header, rows } => {
            let cell = |c: &Vec<Inline>| inlines(c).map(|s| s.replace('\n', " "));
            let header: Vec<String> = header.iter().map(cell).collect::<Result<_, _>>()?;
            let rows: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(cell).collect::<Result<_, _>>()).collect::<Result<_, _>>()?;
            let widths: Vec<usize> =
                (0..header.len()).map(|i| rows.iter().map(|r| r[i].chars().count()).chain([header[i].chars().count()]).max().unwrap_or(0)).collect();
            let line = |cells: &[String]| {
                let padded: Vec<String> = cells.iter().zip(&widths).map(|(c, w)| format!("{}{}", c, " ".repeat(w - c.chars().count()))).collect();
                padded.join("  ").trim_end().to_string()
            };
            let mut lines = vec![line(&header), widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("  ")];
            lines.extend(rows.iter().map(|r| line(r)));
            out.push_str(&lines.join("\n"));
        }
        Block::CodeBlock { text, .. } => out.push_str(text.trim_end_matches('\n')),
        Block::Raw { format, text, .. } if format == TEXT => out.push_str(text),
        Block::Raw { format, at, .. } => return Err(foreign(format, at)),
        Block::Div { blocks: bs, .. } => out.push_str(&blocks(bs)?),
    }
    Ok(())
}

fn foreign(format: &str, at: &Option<lamlet::embed::Location>) -> Error {
    Error::at(at, format!("this is markup for `{}`, which cannot be written as plain text", format))
}

fn inlines(xs: &[Inline]) -> Result<String, Error> {
    let mut out = String::new();
    for x in xs {
        inline(&mut out, x)?;
    }
    Ok(out)
}

fn inline(out: &mut String, node: &Inline) -> Result<(), Error> {
    match node {
        Inline::Text { text, .. } | Inline::Code { text, .. } => out.push_str(text),
        Inline::Raw { format, text, .. } if format == TEXT => out.push_str(text),
        Inline::Raw { format, at, .. } => return Err(foreign(format, at)),
        Inline::Value { data, at } => out.push_str(&written_form(data, at)?),
        Inline::Hole { region, at } => return Err(Error::at(at, format!("region {} is evaluated at runtime and has no value yet", region))),
        Inline::Emph(xs) | Inline::Strong(xs) | Inline::Span { inlines: xs, .. } | Inline::Say { shown: xs, .. } => out.push_str(&inlines(xs)?),
        Inline::Link { href, inlines: xs } => {
            let label = inlines(xs)?;
            out.push_str(&label);
            if label != *href {
                out.push_str(&format!(" ({})", href));
            }
        }
        Inline::Image { alt, .. } => out.push_str(alt),
        Inline::Math { tex, .. } => out.push_str(tex),
        Inline::Break => out.push('\n'),
        Inline::Anchor { .. } | Inline::Pause { .. } => {}
    }
    Ok(())
}
