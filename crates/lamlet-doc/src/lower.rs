//! Lowering: a checked document and its region values to the document IR.
//!
//! Lowering only reads values; it calls no Lamlet code. A region's value lowers by the first rule that applies:
//!
//! 1. a value of type `Content` (the `content` package): its inlines and blocks
//! 2. a String, number, or Bool: [`Inline::Text`], written as `lamlet::embed::text` writes it
//! 3. Unit: nothing
//! 4. other data (lists, records, variants, types): [`Inline::Value`]
//! 5. a value with no data form (a function, a host object, or data holding one): [`Inline::Code`] with its
//!    written form
//!
//! Prose becomes [`Inline::Raw`] in the format the caller names, or, for [`crate::COMMONMARK`], is
//! parsed into blocks (`prose`). A line that held only declarations, comments, and whitespace is removed with its
//! line break, so the declarations at the top of a document leave no blank lines. Inlines between blocks form
//! `Plain` blocks.

use crate::content::{lower_content, Piece, CONTENT_TYPE};
use crate::ir::{Block, Doc, Inline};
use crate::{prose, Error, COMMONMARK};
use lamlet::embed::{display, text, Data, Document, Instance, Location, Part, Region, Value};
use std::collections::HashMap;

/// Evaluate the document in `instance` and lower it, with prose in `prose_format`.
pub fn evaluate_and_lower(doc: &Document, instance: &mut Instance, prose_format: &str) -> Result<Doc, Error> {
    let values: HashMap<u32, Value> = instance.evaluate()?.into_iter().collect();
    lower(doc, &values, prose_format)
}

/// Lower a document whose runtime regions have the given values (by region index; regions computed during the
/// check carry their own). A region with no value is an error.
pub fn lower(doc: &Document, values: &HashMap<u32, Value>, prose_format: &str) -> Result<Doc, Error> {
    lower_parts(doc, Some(values), prose_format)
}

/// Lower a checked document without evaluating it: the prepared IR. Regions computed during the check have their
/// values, and the others are [`Inline::Hole`]s.
pub fn lower_prepared(doc: &Document, prose_format: &str) -> Result<Doc, Error> {
    lower_parts(doc, None, prose_format)
}

fn lower_parts(doc: &Document, values: Option<&HashMap<u32, Value>>, prose_format: &str) -> Result<Doc, Error> {
    // Parsed prose is lowered as Raw first and parsed afterwards, with the regions in place.
    let raw_format = if prose_format == COMMONMARK { COMMONMARK } else { prose_format };
    let mut lines = Lines { format: raw_format, ..Lines::default() };
    for part in doc.parts() {
        match part {
            Part::Text { text, at } => lines.text(&text, &at),
            Part::Comment { .. } | Part::Declaration { .. } => lines.invisible = true,
            Part::Region { region, value } => {
                lines.visible = true;
                let at = Some(region.at.clone());
                let Some(v) = value.as_ref().or_else(|| values.and_then(|vs| vs.get(&region.index))) else {
                    if values.is_some() {
                        return Err(Error::at(&at, format!("region {} has no value: it is evaluated at runtime", region.index)));
                    }
                    lines.line.push(Piece::Inline(Inline::Hole { region: region.index, at }));
                    continue;
                };
                for piece in lower_value(&region, v)? {
                    match piece {
                        // A newline in a value's text ends a line too, as it does in prose.
                        Piece::Inline(Inline::Text { text, at }) => lines.value_text(&text, at),
                        piece => lines.line.push(piece),
                    }
                }
            }
        }
    }
    lines.end_line();
    if prose_format == COMMONMARK {
        return Ok(Doc { blocks: prose::parse(lines.out)? });
    }
    Ok(Doc { blocks: assemble(lines.out) })
}

/// Pieces as blocks: runs of inlines become `Plain` blocks.
fn assemble(pieces: Vec<Piece>) -> Vec<Block> {
    let mut out = Vec::new();
    let mut run = Vec::new();
    for p in pieces {
        match p {
            Piece::Inline(i) => run.push(i),
            Piece::Block(b) => {
                if !run.is_empty() {
                    out.push(Block::Plain(std::mem::take(&mut run)));
                }
                out.push(b);
            }
        }
    }
    if !run.is_empty() || out.is_empty() {
        out.push(Block::Plain(run));
    }
    out
}

/// One region's value as pieces (none for Unit).
fn lower_value(region: &Region, v: &Value) -> Result<Vec<Piece>, Error> {
    let at = Some(region.at.clone());
    Ok(match v {
        Value::Unit => vec![],
        Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_) => {
            vec![Piece::Inline(Inline::Text { text: text(region, v).expect("a scalar has a text form"), at })]
        }
        _ => match Data::from_value(v) {
            Ok(data) if region.type_text == CONTENT_TYPE => lower_content(&data, &at)?,
            Ok(data) => vec![Piece::Inline(Inline::Value { data, at })],
            Err(_) => vec![Piece::Inline(Inline::Code { text: display(v), at })],
        },
    })
}

/// The position of line `n` (from 0) of a text part that starts at `at`. Lines after the first start at
/// column 1. The byte offsets stay the part's own: messages do not use them, and `$$` in the source makes
/// the text shorter than its span.
fn line_at(at: &Location, n: usize) -> Location {
    if n == 0 {
        return at.clone();
    }
    Location { line: at.line + n, column: 1, end_line: at.line + n, end_column: 1, ..at.clone() }
}

/// Whether a piece is prose (`Raw` in the prose format), which adjacent prose joins.
fn is_prose(p: &Piece, format: &str) -> bool {
    matches!(p, Piece::Inline(Inline::Raw { format: f, .. }) if f == format)
}

/// Collects pieces line by line, dropping lines that held only invisible parts and whitespace.
#[derive(Default)]
struct Lines<'f> {
    format: &'f str,
    out: Vec<Piece>,
    line: Vec<Piece>,
    invisible: bool,
    visible: bool,
}

impl Lines<'_> {
    fn text(&mut self, text: &str, at: &Location) {
        let mut rest = text;
        let mut line = 0;
        while let Some(i) = rest.find('\n') {
            self.raw(&rest[..=i], &line_at(at, line));
            self.end_line();
            rest = &rest[i + 1..];
            line += 1;
        }
        if !rest.is_empty() {
            self.raw(rest, &line_at(at, line));
        }
    }

    fn value_text(&mut self, text: &str, at: Option<Location>) {
        let mut rest = text;
        while let Some(i) = rest.find('\n') {
            self.line.push(Piece::Inline(Inline::Text { text: rest[..=i].to_string(), at: at.clone() }));
            self.end_line();
            rest = &rest[i + 1..];
        }
        if !rest.is_empty() {
            self.line.push(Piece::Inline(Inline::Text { text: rest.to_string(), at }));
        }
    }

    fn raw(&mut self, text: &str, at: &Location) {
        let format = self.format;
        if let Some(Piece::Inline(Inline::Raw { text: last, .. })) = self.line.last_mut().filter(|p| is_prose(p, format)) {
            last.push_str(text);
        } else {
            self.line.push(Piece::Inline(Inline::Raw { format: self.format.to_string(), text: text.to_string(), at: Some(at.clone()) }));
        }
    }

    fn end_line(&mut self) {
        let blank = self.invisible
            && !self.visible
            && self.line.iter().all(|p| matches!(p, Piece::Inline(Inline::Raw { text, .. } | Inline::Text { text, .. }) if text.trim().is_empty()));
        let line = std::mem::take(&mut self.line);
        if !blank {
            for piece in line {
                // Parsed prose keeps one piece per line, so the parser knows each line's position.
                let format = self.format;
                let joins = format != COMMONMARK;
                match (self.out.last_mut().filter(|p| joins && is_prose(p, format)), piece) {
                    (Some(Piece::Inline(Inline::Raw { text: last, .. })), Piece::Inline(Inline::Raw { text, format: f, .. })) if f == format => last.push_str(&text),
                    // The pieces of one value's text, split at its newlines above, join again.
                    (Some(Piece::Inline(Inline::Text { text: last, at: a })), Piece::Inline(Inline::Text { text, at: b })) if a.as_ref() == b.as_ref() => last.push_str(&text),
                    (_, piece) => self.out.push(piece),
                }
            }
        }
        self.invisible = false;
        self.visible = false;
    }
}
