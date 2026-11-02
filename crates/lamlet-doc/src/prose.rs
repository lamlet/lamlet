//! Parsed prose (`--prose commonmark`): prose read as a subset of CommonMark into IR blocks, so one source can be
//! written in several formats. Regions stay where they stand: a region alone on its lines is a block when its
//! content is a block, and any other region is an inline in the text around it.
//!
//! The subset:
//!
//! - **Blocks:** ATX headings (`## Title {#id key=value}`), paragraphs, bullet and ordered lists (nested by
//!   indentation), fenced code blocks (```` ``` ```` and `~~~`), and GitHub tables.
//! - **Inlines:** `*emphasis*`, `**strong**` (and `_`/`__`), `` `code` ``, `[links](href)`, `![images](src
//!   "title")`, `<autolinks>`, backslash escapes, and hard line breaks (two spaces or `\` before a newline).
//!
//! Not in the subset (written as text): block quotes, thematic breaks, setext headings, HTML, reference links,
//! and indented code blocks.
//!
//! While the prose is parsed, each region stands in it as one character of Unicode's private use area, so the
//! parser never sees region values as markup.

use crate::content::Piece;
use crate::ir::{Attrs, Block, Inline};
use crate::{Error, COMMONMARK};
use lamlet::embed::Location;

const FIRST_SLOT: u32 = 0xE000;
const LAST_SLOT: u32 = 0xF8FF;

/// Lowered pieces whose prose is `Raw` in [`COMMONMARK`], parsed into blocks.
pub fn parse(pieces: Vec<Piece>) -> Result<Vec<Block>, Error> {
    let mut src = String::new();
    let mut slots = Vec::new();
    let mut at = None;
    for p in pieces {
        match p {
            Piece::Inline(Inline::Raw { format, text, at: a }) if format == COMMONMARK => {
                if at.is_none() {
                    at = a;
                }
                // A source character in the private use area would be read as a region.
                if text.chars().any(is_slot) {
                    return Err(Error::at(&at, "the prose holds a character of Unicode's private use area (U+E000 to U+F8FF), which parsed prose reserves"));
                }
                src.push_str(&text);
            }
            p => {
                let code = FIRST_SLOT + slots.len() as u32;
                if code > LAST_SLOT {
                    return Err(Error::at(&at, "parsed prose holds at most 6400 regions"));
                }
                src.push(char::from_u32(code).unwrap());
                slots.push(p);
            }
        }
    }
    let p = Parser { slots, at };
    let lines: Vec<&str> = src.split('\n').collect();
    p.blocks(&lines)
}

fn is_slot(c: char) -> bool {
    (FIRST_SLOT..=LAST_SLOT).contains(&(c as u32))
}

struct Parser {
    slots: Vec<Piece>,
    at: Option<Location>,
}

fn indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn strip_indent(line: &str, n: usize) -> &str {
    let k = indent(line).min(n);
    &line[k..]
}

/// A list marker at the start of a line: (ordered, width of marker plus following spaces).
fn list_marker(line: &str) -> Option<(bool, usize)> {
    let ind = indent(line);
    if ind > 3 {
        return None;
    }
    let rest = &line[ind..];
    let (ordered, marker_len) = if rest.starts_with(['-', '*', '+']) {
        (false, 1)
    } else {
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 9 || !rest[digits..].starts_with(['.', ')']) {
            return None;
        }
        (true, digits + 1)
    };
    let after = &rest[marker_len..];
    if after.is_empty() {
        return Some((ordered, ind + marker_len + 1));
    }
    if !after.starts_with(' ') {
        return None;
    }
    let spaces = after.chars().take_while(|c| *c == ' ').count().min(4);
    // A thematic break (`- - -`, `***`) is not a list item.
    if !ordered && rest.chars().all(|c| c == rest.chars().next().unwrap() || c == ' ') && rest.chars().filter(|c| *c != ' ').count() >= 3 {
        return None;
    }
    Some((ordered, ind + marker_len + spaces))
}

fn fence(line: &str) -> Option<(char, usize, &str)> {
    let ind = indent(line);
    if ind > 3 {
        return None;
    }
    let rest = &line[ind..];
    let c = rest.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = rest.chars().take_while(|x| *x == c).count();
    if n < 3 {
        return None;
    }
    let info = rest[n..].trim();
    if c == '`' && info.contains('`') {
        return None;
    }
    Some((c, n, info))
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let ind = indent(line);
    if ind > 3 {
        return None;
    }
    let rest = &line[ind..];
    let n = rest.chars().take_while(|c| *c == '#').count();
    if n == 0 || n > 6 {
        return None;
    }
    let after = &rest[n..];
    if !after.is_empty() && !after.starts_with([' ', '\t']) {
        return None;
    }
    let mut text = after.trim();
    // Closing hashes.
    let trimmed = text.trim_end_matches('#');
    if trimmed.is_empty() || trimmed.ends_with(' ') {
        text = trimmed.trim_end();
    }
    Some((n as u8, text))
}

fn is_delimiter_row(line: &str) -> bool {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    !t.is_empty()
        && t.split('|').all(|c| {
            let c = c.trim();
            let c = c.strip_prefix(':').unwrap_or(c);
            let c = c.strip_suffix(':').unwrap_or(c);
            !c.is_empty() && c.chars().all(|x| x == '-')
        })
}

/// Table cells of a row, split at unescaped `|`.
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !t.ends_with("\\|") { &t[..t.len() - 1] } else { t };
    let mut out = vec![String::new()];
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                out.last_mut().unwrap().push('|');
                chars.next();
            }
            '|' => out.push(String::new()),
            c => out.last_mut().unwrap().push(c),
        }
    }
    out.into_iter().map(|c| c.trim().to_string()).collect()
}

/// `## Title {#id key=value .class}`: the title and its attributes.
fn heading_attrs(text: &str) -> (&str, Option<String>, Attrs) {
    let t = text.trim_end();
    let (Some(open), true) = (t.rfind('{'), t.ends_with('}')) else { return (text, None, vec![]) };
    let inner = &t[open + 1..t.len() - 1];
    let mut id = None;
    let mut attrs = Vec::new();
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in inner.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    for tok in tokens {
        if let Some(i) = tok.strip_prefix('#') {
            id = Some(i.to_string());
        } else if let Some(c) = tok.strip_prefix('.') {
            attrs.push(("class".to_string(), c.to_string()));
        } else if let Some((k, v)) = tok.split_once('=') {
            attrs.push((k.to_string(), v.to_string()));
        } else {
            return (text, None, vec![]);
        }
    }
    (t[..open].trim_end(), id, attrs)
}

fn starts_block(line: &str) -> bool {
    heading(line).is_some() || fence(line).is_some() || list_marker(line).is_some_and(|(_, w)| line.len() > w || line.trim().len() > 1)
}

impl Parser {
    fn slot(&self, c: char) -> &Piece {
        &self.slots[(c as u32 - FIRST_SLOT) as usize]
    }

    /// The block a line stands for when it holds only one region whose content is a block.
    fn block_slot(&self, line: &str) -> Option<Block> {
        let t = line.trim();
        let mut chars = t.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else { return None };
        if !is_slot(c) {
            return None;
        }
        match self.slot(c) {
            Piece::Block(b) => Some(b.clone()),
            Piece::Inline(_) => None,
        }
    }

    fn blocks(&self, lines: &[&str]) -> Result<Vec<Block>, Error> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            if line.trim().is_empty() {
                i += 1;
            } else if let Some(b) = self.block_slot(line) {
                out.push(b);
                i += 1;
            } else if let Some((c, n, info)) = fence(line) {
                let ind = indent(line);
                let mut body = Vec::new();
                i += 1;
                while i < lines.len() {
                    if let Some((c2, n2, rest)) = fence(lines[i]) {
                        if c2 == c && n2 >= n && rest.is_empty() {
                            i += 1;
                            break;
                        }
                    }
                    body.push(strip_indent(lines[i], ind));
                    i += 1;
                }
                let text = self.plain(&body.join("\n"))?;
                out.push(Block::CodeBlock { lang: info.split_whitespace().next().unwrap_or("").to_string(), text, at: self.at.clone() });
            } else if let Some((level, text)) = heading(line) {
                let (title, id, attrs) = heading_attrs(text);
                out.push(Block::Heading { level, id, attrs, inlines: self.inlines(title)? });
                i += 1;
            } else if let Some((ordered, _)) = list_marker(line) {
                let (block, next) = self.list(lines, i, ordered)?;
                out.push(block);
                i = next;
            } else if line.contains('|') && i + 1 < lines.len() && is_delimiter_row(lines[i + 1]) {
                let header = cells(line);
                let mut rows = Vec::new();
                i += 2;
                while i < lines.len() && !lines[i].trim().is_empty() && lines[i].contains('|') {
                    let mut r = cells(lines[i]);
                    r.resize(header.len(), String::new());
                    rows.push(r);
                    i += 1;
                }
                let header = header.iter().map(|c| self.inlines(c)).collect::<Result<_, _>>()?;
                let rows = rows.iter().map(|r| r.iter().map(|c| self.inlines(c)).collect::<Result<_, _>>()).collect::<Result<_, _>>()?;
                out.push(Block::Table { header, rows });
            } else {
                let start = i;
                i += 1;
                while i < lines.len() && !lines[i].trim().is_empty() && !starts_block(lines[i]) && self.block_slot(lines[i]).is_none() {
                    i += 1;
                }
                let text: Vec<&str> = lines[start..i].iter().map(|l| l.trim_start()).collect();
                out.push(Block::Paragraph(self.inlines(&text.join("\n"))?));
            }
        }
        Ok(out)
    }

    /// A list starting at line `start`; returns it and the index after it.
    fn list(&self, lines: &[&str], start: usize, ordered: bool) -> Result<(Block, usize), Error> {
        let mut items = Vec::new();
        let mut i = start;
        while i < lines.len() {
            let Some((o, width)) = list_marker(lines[i]) else { break };
            if o != ordered {
                break;
            }
            let first = lines[i].get(width..).unwrap_or("");
            let mut body = vec![first];
            i += 1;
            while i < lines.len() {
                let l = lines[i];
                if l.trim().is_empty() {
                    // A blank line continues the item only if indented content follows.
                    let next = lines[i + 1..].iter().find(|l| !l.trim().is_empty());
                    if next.is_some_and(|n| indent(n) >= width) {
                        body.push("");
                        i += 1;
                        continue;
                    }
                    break;
                }
                if indent(l) >= width {
                    body.push(&l[width..]);
                } else if list_marker(l).is_none() && !starts_block(l) && body.last().is_some_and(|b| !b.trim().is_empty()) {
                    body.push(l.trim_start()); // a lazy continuation of the paragraph
                } else {
                    break;
                }
                i += 1;
            }
            items.push(self.blocks(&body)?);
            // Blank lines between items keep the list going.
            let mut j = i;
            while j < lines.len() && lines[j].trim().is_empty() {
                j += 1;
            }
            if j < lines.len() && list_marker(lines[j]).is_some_and(|(o, _)| o == ordered) {
                i = j;
            } else {
                break;
            }
        }
        // Items of one paragraph are written without paragraph markup.
        let items = items
            .into_iter()
            .map(|bs| match &bs[..] {
                [Block::Paragraph(xs)] => vec![Block::Plain(xs.clone())],
                _ => bs,
            })
            .collect();
        Ok((Block::List { ordered, items }, i))
    }

    /// Text with regions as their plain text (code keeps them literal); a block region is an error.
    fn plain(&self, s: &str) -> Result<String, Error> {
        let mut out = String::new();
        for c in s.chars() {
            if is_slot(c) {
                match self.slot(c) {
                    Piece::Inline(i) => out.push_str(&crate::backend::text::write(&crate::Doc { blocks: vec![Block::Plain(vec![i.clone()])] })?),
                    Piece::Block(_) => return Err(Error::at(&self.at, "a block region inside code or an image's text")),
                }
            } else {
                out.push(c);
            }
        }
        Ok(out)
    }

    fn inlines(&self, s: &str) -> Result<Vec<Inline>, Error> {
        let chars: Vec<char> = s.chars().collect();
        let mut out = Vec::new();
        let mut text = String::new();
        self.run(&chars, &mut out, &mut text)?;
        self.flush(&mut out, &mut text);
        Ok(out)
    }

    fn flush(&self, out: &mut Vec<Inline>, text: &mut String) {
        if !text.is_empty() {
            out.push(Inline::Text { text: std::mem::take(text), at: self.at.clone() });
        }
    }

    fn run(&self, c: &[char], out: &mut Vec<Inline>, text: &mut String) -> Result<(), Error> {
        let mut i = 0;
        while i < c.len() {
            let ch = c[i];
            // Backslash escapes and hard breaks.
            if ch == '\\' && i + 1 < c.len() {
                if c[i + 1] == '\n' {
                    self.flush(out, text);
                    out.push(Inline::Break);
                    i += 2;
                    continue;
                }
                if c[i + 1].is_ascii_punctuation() {
                    text.push(c[i + 1]);
                    i += 2;
                    continue;
                }
            }
            if ch == '\n' {
                let trailing = text.chars().rev().take_while(|x| *x == ' ').count();
                text.truncate(text.len() - trailing);
                if trailing >= 2 {
                    self.flush(out, text);
                    out.push(Inline::Break);
                } else {
                    text.push('\n');
                }
                i += 1;
                continue;
            }
            if is_slot(ch) {
                self.flush(out, text);
                match self.slot(ch) {
                    Piece::Inline(x) => out.push(x.clone()),
                    Piece::Block(_) => {
                        return Err(Error::at(&self.at, "a region whose content is a block stands inside a line; put it on lines of its own"));
                    }
                }
                i += 1;
                continue;
            }
            if ch == '`' {
                let n = c[i..].iter().take_while(|x| **x == '`').count();
                if let Some(end) = find_run(c, i + n, '`', n) {
                    let mut code: String = c[i + n..end].iter().collect::<String>().replace('\n', " ");
                    if code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty() {
                        code = code[1..code.len() - 1].to_string();
                    }
                    self.flush(out, text);
                    out.push(Inline::Code { text: self.plain(&code)?, at: self.at.clone() });
                    i = end + n;
                    continue;
                }
                text.extend(&c[i..i + n]);
                i += n;
                continue;
            }
            if ch == '!' && c.get(i + 1) == Some(&'[') {
                if let Some((label, href, title, next)) = link_parts(c, i + 1) {
                    self.flush(out, text);
                    let alt = self.plain(&label.iter().collect::<String>())?;
                    out.push(Inline::Image { src: href, alt, title, at: self.at.clone() });
                    i = next;
                    continue;
                }
            }
            if ch == '[' {
                if let Some((label, href, _, next)) = link_parts(c, i) {
                    self.flush(out, text);
                    let mut inner = Vec::new();
                    let mut t = String::new();
                    self.run(label, &mut inner, &mut t)?;
                    self.flush(&mut inner, &mut t);
                    out.push(Inline::Link { href, inlines: inner });
                    i = next;
                    continue;
                }
            }
            if ch == '<' {
                if let Some(end) = c[i + 1..].iter().position(|x| *x == '>') {
                    let url: String = c[i + 1..i + 1 + end].iter().collect();
                    if url.contains(':') && !url.contains([' ', '<']) && url.split(':').next().is_some_and(|s| !s.is_empty() && s.chars().all(|x| x.is_ascii_alphanumeric() || "+.-".contains(x))) {
                        self.flush(out, text);
                        out.push(Inline::Link { href: url.clone(), inlines: vec![Inline::Text { text: url, at: self.at.clone() }] });
                        i += end + 2;
                        continue;
                    }
                }
            }
            if ch == '*' || ch == '_' {
                let n = c[i..].iter().take_while(|x| **x == ch).count();
                let prev = if i > 0 { Some(c[i - 1]) } else { None };
                let opens = c.get(i + n).is_some_and(|x| !x.is_whitespace()) && !(ch == '_' && prev.is_some_and(|p| p.is_alphanumeric()));
                if opens && n <= 3 {
                    if let Some(end) = find_closing(c, i + n, ch, n) {
                        self.flush(out, text);
                        let mut inner = Vec::new();
                        let mut t = String::new();
                        self.run(&c[i + n..end], &mut inner, &mut t)?;
                        self.flush(&mut inner, &mut t);
                        out.push(match n {
                            1 => Inline::Emph(inner),
                            2 => Inline::Strong(inner),
                            _ => Inline::Strong(vec![Inline::Emph(inner)]),
                        });
                        i = end + n;
                        continue;
                    }
                }
                text.extend(&c[i..i + n]);
                i += n;
                continue;
            }
            text.push(ch);
            i += 1;
        }
        Ok(())
    }
}

/// The index of the next run of exactly `n` `ch` characters from `from`.
fn find_run(c: &[char], from: usize, ch: char, n: usize) -> Option<usize> {
    let mut i = from;
    while i < c.len() {
        if c[i] == ch {
            let m = c[i..].iter().take_while(|x| **x == ch).count();
            if m == n {
                return Some(i);
            }
            i += m;
        } else {
            i += 1;
        }
    }
    None
}

/// A closing emphasis run: `n` `ch` characters not preceded by whitespace (and, for `_`, not followed by a
/// letter or digit), skipping code spans.
fn find_closing(c: &[char], from: usize, ch: char, n: usize) -> Option<usize> {
    let mut i = from;
    while i < c.len() {
        if c[i] == '`' {
            let m = c[i..].iter().take_while(|x| **x == '`').count();
            i = find_run(c, i + m, '`', m).map(|e| e + m).unwrap_or(i + m);
            continue;
        }
        if c[i] == '\\' {
            i += 2;
            continue;
        }
        if c[i] == ch {
            let m = c[i..].iter().take_while(|x| **x == ch).count();
            let before_ok = i > from && !c[i - 1].is_whitespace();
            let after_ok = ch != '_' || c.get(i + m).is_none_or(|x| !x.is_alphanumeric());
            if m == n && before_ok && after_ok {
                return Some(i);
            }
            i += m;
            continue;
        }
        i += 1;
    }
    None
}

/// `[label](href "title")` from `open` (the `[`): the label, the destination, the title, and the index after.
fn link_parts(c: &[char], open: usize) -> Option<(&[char], String, Option<String>, usize)> {
    let mut depth = 0;
    let mut i = open;
    let close = loop {
        match c.get(i)? {
            '\\' => i += 1,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break i;
                }
            }
            _ => {}
        }
        i += 1;
    };
    if c.get(close + 1) != Some(&'(') {
        return None;
    }
    let mut j = close + 2;
    while c.get(j) == Some(&' ') {
        j += 1;
    }
    let mut href = String::new();
    if c.get(j) == Some(&'<') {
        j += 1;
        while *c.get(j)? != '>' {
            href.push(c[j]);
            j += 1;
        }
        j += 1;
    } else {
        let mut parens = 0;
        while let Some(&x) = c.get(j) {
            if x == ' ' || (x == ')' && parens == 0) {
                break;
            }
            if x == '(' {
                parens += 1;
            }
            if x == ')' {
                parens -= 1;
            }
            href.push(x);
            j += 1;
        }
    }
    while c.get(j) == Some(&' ') {
        j += 1;
    }
    let mut title = None;
    if let Some(&q) = c.get(j).filter(|x| **x == '"' || **x == '\'') {
        let start = j + 1;
        let end = start + c[start..].iter().position(|x| *x == q)?;
        title = Some(c[start..end].iter().collect());
        j = end + 1;
        while c.get(j) == Some(&' ') {
            j += 1;
        }
    }
    if c.get(j) != Some(&')') {
        return None;
    }
    Some((&c[open + 1..close], href, title, j + 1))
}
