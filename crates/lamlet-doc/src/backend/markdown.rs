//! The Markdown backend: CommonMark, with GitHub's tables.
//!
//! - `Raw` in [`crate::MARKDOWN`] (or parsed [`crate::COMMONMARK`] prose) is copied; other formats are refused.
//! - `Text` is escaped: ASCII punctuation that could start markup gets a backslash, and so does a `#`, `-`, `+`,
//!   `=`, or `N.` at the start of a line. Escaping more than strictly needed is always valid CommonMark.
//! - `Value` and `Code` are code spans; `Math` is `$tex$` or `$$tex$$` (the common extension).
//! - Heading ids are not written (CommonMark has no syntax for them).
//! - `Span`, `Div`, `Say` write their content; `Anchor` and `Pause` write nothing.

use crate::ir::{Block, Doc, Inline};
use crate::{written_form, Error, COMMONMARK, MARKDOWN};
use lamlet::embed::Location;

/// The document as Markdown.
pub fn write(doc: &Doc) -> Result<String, Error> {
    let mut w = Writer::default();
    w.blocks(&doc.blocks)?;
    Ok(w.out)
}

#[derive(Default)]
struct Writer {
    out: String,
}

fn ours(format: &str) -> bool {
    format == MARKDOWN || format == COMMONMARK
}

fn foreign(format: &str, at: &Option<Location>) -> Error {
    Error::at(at, format!("this is markup for `{}`, which cannot be written as Markdown", format))
}

/// A code span around `text`, with a fence longer than any run of backticks inside it.
pub(crate) fn code_span(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') || text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty() { " " } else { "" };
    format!("{fence}{pad}{text}{pad}{fence}")
}

fn escape(text: &str, mut line_start: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if line_start && c != ' ' && c != '\n' {
            let next = chars.get(i + 1).copied();
            let ordinal = c.is_ascii_digit() && {
                let rest: String = chars[i..].iter().take_while(|c| c.is_ascii_digit()).collect();
                matches!(chars.get(i + rest.len()), Some('.') | Some(')'))
            };
            if matches!(c, '#' | '=') || (matches!(c, '-' | '+') && matches!(next, None | Some(' ') | Some('-'))) {
                out.push('\\');
            } else if ordinal {
                let digits: String = chars[i..].iter().take_while(|c| c.is_ascii_digit()).collect();
                out.push_str(&digits);
                out.push('\\');
                out.extend(chars[i + digits.len()..].iter().take(1));
                // The rest of the line is escaped normally.
                let rest: String = chars[i + digits.len() + 1..].iter().collect();
                out.push_str(&escape(&rest, false));
                return out;
            }
            line_start = false;
        }
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '&' | '|' | '!' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => {
                out.push(c);
                line_start = true;
            }
            c => out.push(c),
        }
    }
    out
}

impl Writer {
    fn at_line_start(&self) -> bool {
        self.out.is_empty() || self.out.ends_with('\n')
    }

    fn blocks(&mut self, bs: &[Block]) -> Result<(), Error> {
        for (i, b) in bs.iter().enumerate() {
            if i > 0 && !matches!(b, Block::Plain(_)) && !matches!(bs[i - 1], Block::Plain(_)) {
                self.out.push_str("\n\n");
            }
            self.block(b)?;
        }
        Ok(())
    }

    fn block(&mut self, b: &Block) -> Result<(), Error> {
        match b {
            Block::Plain(xs) | Block::Paragraph(xs) => self.inlines(xs)?,
            Block::Heading { level, inlines, .. } => {
                self.out.push_str(&"#".repeat(*level as usize));
                self.out.push(' ');
                let text = sub(|w| w.inlines(inlines))?;
                self.out.push_str(&text.replace('\n', " "));
            }
            Block::List { ordered, items } => {
                let mut lines = Vec::new();
                for (n, item) in items.iter().enumerate() {
                    let marker = if *ordered { format!("{}. ", n + 1) } else { "- ".to_string() };
                    let body = sub(|w| w.blocks(item))?;
                    let indent = " ".repeat(marker.len());
                    for (k, line) in body.split('\n').enumerate() {
                        lines.push(if k == 0 { format!("{marker}{line}") } else if line.is_empty() { String::new() } else { format!("{indent}{line}") });
                    }
                }
                self.out.push_str(&lines.join("\n"));
            }
            Block::Table { header, rows } => {
                let cell = |c: &Vec<Inline>| sub(|w| w.inlines(c)).map(|s| s.replace('\n', " "));
                let row = |cells: &Vec<Vec<Inline>>| -> Result<String, Error> {
                    let cs: Vec<String> = cells.iter().map(cell).collect::<Result<_, _>>()?;
                    Ok(format!("| {} |", cs.join(" | ")))
                };
                let mut lines = vec![row(header)?, format!("|{}", " --- |".repeat(header.len()))];
                for r in rows {
                    lines.push(row(r)?);
                }
                self.out.push_str(&lines.join("\n"));
            }
            Block::CodeBlock { lang, text, .. } => {
                let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                let fence = "`".repeat(longest.max(2) + 1);
                self.out.push_str(&format!("{fence}{lang}\n{}\n{fence}", text.trim_end_matches('\n')));
            }
            Block::Raw { format, text, .. } if ours(format) => self.out.push_str(text),
            Block::Raw { format, at, .. } => return Err(foreign(format, at)),
            Block::Div { blocks, .. } => self.blocks(blocks)?,
        }
        Ok(())
    }

    fn inlines(&mut self, xs: &[Inline]) -> Result<(), Error> {
        xs.iter().try_for_each(|x| self.inline(x))
    }

    fn inline(&mut self, node: &Inline) -> Result<(), Error> {
        match node {
            Inline::Text { text, .. } => {
                let start = self.at_line_start();
                self.out.push_str(&escape(text, start));
            }
            Inline::Raw { format, text, .. } if ours(format) => self.out.push_str(text),
            Inline::Raw { format, at, .. } => return Err(foreign(format, at)),
            Inline::Code { text, .. } => self.out.push_str(&code_span(text)),
            Inline::Value { data, at } => self.out.push_str(&code_span(&written_form(data, at)?)),
            Inline::Hole { region, at } => return Err(Error::at(at, format!("region {} is evaluated at runtime and has no value yet", region))),
            Inline::Emph(xs) => self.wrap("*", xs)?,
            Inline::Strong(xs) => self.wrap("**", xs)?,
            Inline::Link { href, inlines } => {
                self.out.push('[');
                self.inlines(inlines)?;
                self.out.push_str(&format!("]({})", destination(href)));
            }
            Inline::Image { src, alt, title, .. } => {
                self.out.push_str(&format!("![{}]({}", escape(alt, false), destination(src)));
                if let Some(t) = title {
                    self.out.push_str(&format!(" \"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"")));
                }
                self.out.push(')');
            }
            Inline::Math { tex, display, .. } => self.out.push_str(&if *display { format!("$${tex}$$") } else { format!("${tex}$") }),
            Inline::Break => self.out.push_str("\\\n"),
            Inline::Span { inlines, .. } | Inline::Say { shown: inlines, .. } => self.inlines(inlines)?,
            Inline::Anchor { .. } | Inline::Pause { .. } => {}
        }
        Ok(())
    }

    fn wrap(&mut self, mark: &str, xs: &[Inline]) -> Result<(), Error> {
        self.out.push_str(mark);
        self.inlines(xs)?;
        self.out.push_str(mark);
        Ok(())
    }
}

/// Write blocks or inlines into a fresh buffer (for list items, cells, headings).
fn sub(f: impl FnOnce(&mut Writer) -> Result<(), Error>) -> Result<String, Error> {
    let mut w = Writer::default();
    f(&mut w)?;
    Ok(w.out)
}

fn destination(href: &str) -> String {
    if href.contains([' ', '(', ')', '<', '>']) {
        format!("<{}>", href.replace('<', "%3C").replace('>', "%3E"))
    } else {
        href.to_string()
    }
}
