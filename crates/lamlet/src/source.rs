//! Source files, spans, diagnostics, and the document region scanner.
//!
//! Only `$name`, `${...}`, and `$[...]` are active. Everything else is ignored text that is retained with its
//! span so a host can present it. `$$` is an escaped literal dollar. A code fence (a line of three or more
//! backticks or tildes, up to a closing line) is verbatim prose: nothing in it is active, `$$` included.

use std::fmt;

/// A byte range in one source file. Offsets are UTF-8 byte offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub file: u32,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: u32, start: usize, end: usize) -> Span {
        Span { file, start: start as u32, end: end as u32 }
    }
    pub fn to(self, other: Span) -> Span {
        Span { file: self.file, start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

/// A named source text with a line index for display positions.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub name: String,
    pub text: String,
    line_starts: Vec<usize>,
    /// 1-based line and column of the text's first character in the host's file (1, 1 for a whole file).
    origin: (usize, usize),
}

impl SourceFile {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceFile {
        let text = text.into();
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        SourceFile { name: name.into(), text, line_starts, origin: (1, 1) }
    }

    /// The same text, positioned at a 1-based line and column of a larger host file: display positions
    /// (diagnostics, locations) are reported in the host file's coordinates. Byte offsets are unchanged.
    pub fn with_origin(mut self, line: usize, column: usize) -> SourceFile {
        self.origin = (line.max(1), column.max(1));
        self
    }

    /// 1-based line and column (column counted in Unicode scalars).
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.line_starts[line];
        let col = self.text[start..offset].chars().count();
        let (origin_line, origin_col) = self.origin;
        let col = if line == 0 { col + origin_col - 1 } else { col };
        (line + origin_line, col + 1)
    }
}

/// All source files known to one compilation session.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    pub files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> u32 {
        self.files.push(SourceFile::new(name, text));
        (self.files.len() - 1) as u32
    }
    pub fn add_file(&mut self, file: SourceFile) -> u32 {
        self.files.push(file);
        (self.files.len() - 1) as u32
    }
    pub fn file(&self, id: u32) -> &SourceFile {
        &self.files[id as usize]
    }
    pub fn location(&self, span: Span) -> Location {
        let f = self.file(span.file);
        let (line, column) = f.line_col(span.start as usize);
        let (end_line, end_column) = f.line_col(span.end as usize);
        Location { file: f.name.clone(), line, column, end_line, end_column, start: span.start, end: span.end }
    }
    pub fn snippet(&self, span: Span) -> &str {
        let f = self.file(span.file);
        let s = (span.start as usize).min(f.text.len());
        let e = (span.end as usize).min(f.text.len()).max(s);
        &f.text[s..e]
    }
}

/// A display location for a span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub start: u32,
    pub end: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// Diagnostic categories; they form the initial error catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagKind {
    Syntax,
    Name,
    Type,
    Stage,
    Package,
    Native,
    Static,
    Runtime,
    Limit,
}

impl DiagKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DiagKind::Syntax => "syntax",
            DiagKind::Name => "name",
            DiagKind::Type => "type",
            DiagKind::Stage => "stage",
            DiagKind::Package => "package",
            DiagKind::Native => "native",
            DiagKind::Static => "static-evaluation",
            DiagKind::Runtime => "runtime",
            DiagKind::Limit => "limit",
        }
    }
}

/// A structured diagnostic with an optional primary span and notes (for example a call path).
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub kind: DiagKind,
    pub message: String,
    pub span: Option<Span>,
    pub notes: Vec<(String, Option<Span>)>,
}

impl Diagnostic {
    pub fn new(kind: DiagKind, message: impl Into<String>, span: Option<Span>) -> Diagnostic {
        Diagnostic { kind, message: message.into(), span, notes: Vec::new() }
    }
    pub fn note(mut self, message: impl Into<String>, span: Option<Span>) -> Diagnostic {
        self.notes.push((message.into(), span));
        self
    }
    pub fn render(&self, map: &SourceMap) -> String {
        let mut out = String::new();
        match self.span {
            Some(s) => {
                let loc = map.location(s);
                out.push_str(&format!("{}: {} error: {}", loc, self.kind.as_str(), self.message));
                let snip = map.snippet(s);
                let snip: String = snip.chars().take(80).collect();
                if !snip.is_empty() {
                    out.push_str(&format!("\n    | {}", snip.replace('\n', "\n    | ")));
                }
            }
            None => out.push_str(&format!("{} error: {}", self.kind.as_str(), self.message)),
        }
        for (n, s) in &self.notes {
            match s {
                Some(s) => out.push_str(&format!("\n  note: {} (at {})", n, map.location(*s))),
                None => out.push_str(&format!("\n  note: {}", n)),
            }
        }
        out
    }
}

pub type DResult<T> = Result<T, Diagnostic>;

pub fn syntax_err<T>(message: impl Into<String>, span: Span) -> DResult<T> {
    Err(Diagnostic::new(DiagKind::Syntax, message, Some(span)))
}

/// The three active region spellings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// `$name` or `$a.b`
    Lookup,
    /// `${...}`
    Brace,
    /// `$[...]`
    Bracket,
    /// `$f"{...}"`: a template
    Template,
}

/// A scanned document segment.
#[derive(Clone, Debug)]
pub enum Segment {
    /// Ignored text. `text` is the display text (`$$` becomes `$`); `span` covers the original bytes.
    Text { span: Span, text: String },
    /// An active region. `span` includes the dollar; `inner` excludes it.
    Region { kind: RegionKind, span: Span, inner: Span },
    /// A disabled region `$;{...}` or `$;[...]`: inert, and not display prose. `text` is the raw body.
    Comment { span: Span, text: String },
}

/// The doc for a declaration region at `index`: an adjacent comment segment, or the prose paragraph directly
/// above it (no blank line in between). Prose on the same line as an earlier region is not part of it.
pub fn preceding_doc(segments: &[Segment], index: usize) -> Option<String> {
    let prev = segments.get(index.checked_sub(1)?)?;
    let (text, first_is_continuation) = match prev {
        Segment::Comment { text, .. } => return non_empty(text.trim()),
        Segment::Region { .. } => return None,
        Segment::Text { text, .. } => (text, index >= 2),
    };
    let mut lines: Vec<&str> = text.split('\n').collect();
    // Text on the region's own line before it must be blank: an inline region has no doc.
    if !lines.pop().unwrap_or("").trim().is_empty() {
        return None;
    }
    if lines.is_empty() {
        // The region directly follows another segment on the same line.
        return match (index >= 2).then(|| &segments[index - 2]) {
            Some(Segment::Comment { text, .. }) => non_empty(text.trim()),
            _ => None,
        };
    }
    if first_is_continuation {
        lines.remove(0);
        if lines.is_empty() {
            return match &segments[index - 2] {
                Segment::Comment { text, .. } => non_empty(text.trim()),
                _ => None,
            };
        }
    }
    let mut para = Vec::new();
    for line in lines.iter().rev() {
        if line.trim().is_empty() {
            break;
        }
        para.push(line.trim());
    }
    para.reverse();
    non_empty(&para.join("\n"))
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

pub fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

pub fn is_word_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Scan a document into ignored text and active regions.
pub fn scan_document(file: u32, text: &str) -> DResult<Vec<Segment>> {
    let mut segments = Vec::new();
    let mut display = String::new();
    let mut text_start = 0usize;
    let mut i = 0usize;
    let bytes = text.as_bytes();

    let flush = |segments: &mut Vec<Segment>, display: &mut String, start: usize, end: usize| {
        if end > start {
            segments.push(Segment::Text { span: Span::new(file, start, end), text: std::mem::take(display) });
        }
    };

    while i < text.len() {
        if i == 0 || bytes[i - 1] == b'\n' {
            if let Some((fence, run)) = fence_open(line_at(text, i)) {
                let end = fence_end(text, i, fence, run).ok_or_else(|| {
                    let open = line_at(text, i);
                    let c = fence as char;
                    Diagnostic::new(
                        DiagKind::Syntax,
                        format!("unterminated code fence: close it with a line of {run} or more `{c}` (everything in a fence is verbatim prose)"),
                        Some(Span::new(file, i, i + open.len())),
                    )
                })?;
                display.push_str(&text[i..end]);
                i = end;
                continue;
            }
        }
        if bytes[i] == b'$' {
            let next = text[i + 1..].chars().next();
            match next {
                Some('$') => {
                    display.push('$');
                    i += 2;
                }
                Some(';') if text[i + 2..].starts_with('{') || text[i + 2..].starts_with('[') => {
                    flush(&mut segments, &mut display, text_start, i);
                    let end = raw_form_end(text, i + 2).map_err(|(m, a, b)| {
                        Diagnostic::new(DiagKind::Syntax, m, Some(Span::new(file, a, b)))
                    })?;
                    segments.push(Segment::Comment { span: Span::new(file, i, end), text: text[i + 3..end - 1].to_string() });
                    i = end;
                    text_start = i;
                }
                Some('{') | Some('[') => {
                    flush(&mut segments, &mut display, text_start, i);
                    let end = match_region(file, text, i + 1)?;
                    let kind = if next == Some('{') { RegionKind::Brace } else { RegionKind::Bracket };
                    segments.push(Segment::Region {
                        kind,
                        span: Span::new(file, i, end),
                        inner: Span::new(file, i + 1, end),
                    });
                    i = end;
                    text_start = i;
                }
                Some(_) if raw_template_head(text, i + 1).is_some() => {
                    flush(&mut segments, &mut display, text_start, i);
                    let body = raw_template_head(text, i + 1).unwrap();
                    let end = raw_template_end(text, body).map_err(|(m, a, b, _)| Diagnostic::new(DiagKind::Syntax, m, Some(Span::new(file, a, b))))?;
                    segments.push(Segment::Region {
                        kind: RegionKind::Template,
                        span: Span::new(file, i, end),
                        inner: Span::new(file, i + 1, end),
                    });
                    i = end;
                    text_start = i;
                }
                Some(c) if is_word_start(c) => {
                    flush(&mut segments, &mut display, text_start, i);
                    let end = scan_qualified_word(text, i + 1);
                    segments.push(Segment::Region {
                        kind: RegionKind::Lookup,
                        span: Span::new(file, i, end),
                        inner: Span::new(file, i + 1, end),
                    });
                    i = end;
                    text_start = i;
                }
                _ => {
                    display.push('$');
                    i += 1;
                }
            }
        } else {
            let c = text[i..].chars().next().unwrap();
            display.push(c);
            i += c.len_utf8();
        }
    }
    flush(&mut segments, &mut display, text_start, i);
    Ok(segments)
}

/// The line of `text` that starts at `start`, without its newline.
fn line_at(text: &str, start: usize) -> &str {
    let rest = &text[start..];
    &rest[..rest.find('\n').unwrap_or(rest.len())]
}

/// Whether `line` opens a code fence: optional indentation, then three or more backticks or tildes. A backtick
/// fence's info string (the rest of the line) holds no backtick, so ```` ```x``` ```` is inline code, not a
/// fence. Returns the fence character and the length of its run.
pub fn fence_open(line: &str) -> Option<(u8, usize)> {
    let rest = line.trim_start_matches([' ', '\t']);
    let fence = *rest.as_bytes().first().filter(|b| matches!(b, b'`' | b'~'))?;
    let run = rest.bytes().take_while(|b| *b == fence).count();
    (run >= 3 && !(fence == b'`' && rest[run..].contains('`'))).then_some((fence, run))
}

/// Whether `line` closes a fence opened by `run` `fence` characters: optional indentation, at least `run` of
/// them, and nothing else but whitespace.
pub fn fence_closes(line: &str, fence: u8, run: usize) -> bool {
    let rest = line.trim_start_matches([' ', '\t']);
    let n = rest.bytes().take_while(|b| *b == fence).count();
    n >= run && rest[n..].trim().is_empty()
}

/// End (exclusive, before its newline) of the closing line of a fence whose opening line starts at `start`.
fn fence_end(text: &str, start: usize, fence: u8, run: usize) -> Option<usize> {
    let mut at = start + line_at(text, start).len();
    while at < text.len() {
        let line = line_at(text, at + 1);
        at += 1 + line.len();
        if fence_closes(line, fence, run) {
            return Some(at);
        }
    }
    None
}

/// Consume a word identifier (hyphens allowed between word characters) followed by `::component`s.
pub fn scan_qualified_word(text: &str, start: usize) -> usize {
    let mut i = scan_word(text, start);
    while text[i..].starts_with("::") && text[i + 2..].starts_with(is_word_start) {
        i = scan_word(text, i + 2);
    }
    i
}

fn scan_word(text: &str, start: usize) -> usize {
    let mut i = start;
    let mut first = true;
    while let Some(c) = text[i..].chars().next() {
        if (first && is_word_start(c)) || (!first && is_word_continue(c)) {
            i += c.len_utf8();
            first = false;
        } else if !first && c == '-' {
            // A hyphen belongs to the name only when a word character follows.
            match text[i + 1..].chars().next() {
                Some(n) if is_word_continue(n) => i += 1,
                _ => break,
            }
        } else {
            break;
        }
    }
    i
}

/// End (exclusive) of a raw `;{...}`/`;[...]` comment body whose opener is at `open`: braces and brackets
/// balance and Strings close; nothing else (including `;;`) is special. Errors carry offsets into `text`.
pub fn raw_form_end(text: &str, open: usize) -> Result<usize, (String, usize, usize)> {
    let b = text.as_bytes();
    let mut stack: Vec<u8> = Vec::new();
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.pop() != Some(b[i]) {
                    return Err(("mismatched delimiter inside a block comment".into(), i, i + 1));
                }
                if stack.is_empty() {
                    return Ok(i + 1);
                }
            }
            b'f' | b'l' if raw_template_open(text, i).is_some() => {
                let body = raw_template_open(text, i).unwrap();
                i = raw_template_end(text, body).map_err(|(m, a, b, _)| (m, a, b))?;
                continue;
            }
            b'"' => {
                let start = i;
                i += 1;
                loop {
                    if i >= b.len() {
                        return Err(("unterminated string inside a block comment".into(), start, b.len()));
                    }
                    match b[i] {
                        b'\\' => i += 2,
                        b'"' => break,
                        _ => i += 1,
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err(("unterminated block comment".into(), open.saturating_sub(1), text.len()))
}

/// Find the end (exclusive) of a balanced active region starting at the opener at `open`.
fn match_region(file: u32, text: &str, open: usize) -> DResult<usize> {
    code_end(text, open).map_err(|(m, a, b, opened)| {
        let d = Diagnostic::new(DiagKind::Syntax, m, Some(Span::new(file, a, b)));
        match opened {
            Some(at) => d.note("opened here", Some(Span::new(file, at, at + 1))),
            None => d,
        }
    })
}

/// An error found while balancing code: its message, its byte range in the text, and where the delimiter it
/// concerns was opened, if it has one.
pub type ScanError = (String, usize, usize, Option<usize>);

/// End (exclusive) of balanced code whose opener (`{` or `[`) is at `open`: Strings, comments, and templates
/// are skipped whole, so delimiters inside them do not count.
pub fn code_end(text: &str, open: usize) -> Result<usize, ScanError> {
    let bytes = text.as_bytes();
    let mut stack: Vec<(u8, usize)> = Vec::new();
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            // A `;;` line comment: nothing in it counts toward balancing.
            b';' if bytes.get(i + 1) == Some(&b';') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            // A raw `;{...}`/`;[...]` comment body.
            b';' if matches!(bytes.get(i + 1), Some(b'{') | Some(b'[')) => {
                i = raw_form_end(text, i + 1).map_err(|(m, a, b)| (m, a, b, None))?;
                continue;
            }
            // A template, `f"{...}"`.
            b'f' | b'l' if raw_template_open(text, i).is_some() => {
                i = raw_template_end(text, raw_template_open(text, i).unwrap())?;
                continue;
            }
            b'{' => stack.push((b'}', i)),
            b'[' => stack.push((b']', i)),
            b'}' | b']' => {
                let (expected, at) = stack.pop().expect("region stack");
                if bytes[i] != expected {
                    return Err((
                        format!("mismatched delimiter: expected `{}`, found `{}`", expected as char, bytes[i] as char),
                        i,
                        i + 1,
                        Some(at),
                    ));
                }
                if stack.is_empty() {
                    return Ok(i + 1);
                }
            }
            b'"' => {
                let start = i;
                i += 1;
                loop {
                    if i >= bytes.len() {
                        return Err(("unterminated string literal".into(), start, bytes.len(), None));
                    }
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => break,
                        _ => i += 1,
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    let at = stack.first().map(|s| s.1).unwrap_or(open);
    Err(("unterminated active region".into(), at.saturating_sub(1), text.len(), None))
}

/// When a template (`f"{...}"`) opens at `i`, where its body starts. The head must start a
/// token: it follows the start of the text, whitespace, or a delimiter.
pub fn raw_template_open(text: &str, i: usize) -> Option<usize> {
    let starts_token = text[..i].chars().next_back().is_none_or(|p| p.is_whitespace() || matches!(p, '{' | '[' | '}' | ']'));
    if !starts_token {
        return None;
    }
    raw_template_head(text, i)
}

/// Where a template's body starts, when `text` at `i` starts with `f"{`.
fn raw_template_head(text: &str, i: usize) -> Option<usize> {
    text[i..].starts_with("f\"{").then_some(i + 3)
}

/// End (exclusive, after the closing `}"`) of a template whose body starts at `body`. The body ends at the
/// first `}"` outside a hole; `$$` is skipped, and a `${...}`/`$[...]` hole is balanced code (§`code_end`), so it
/// may hold Strings and other templates.
pub fn raw_template_end(text: &str, body: usize) -> Result<usize, ScanError> {
    let b = text.as_bytes();
    let mut i = body;
    while i < b.len() {
        match b[i] {
            b'}' if b.get(i + 1) == Some(&b'"') => return Ok(i + 2),
            b'$' if b.get(i + 1) == Some(&b'$') => i += 2,
            b'$' if matches!(b.get(i + 1), Some(b'{') | Some(b'[')) => i = code_end(text, i + 1)?,
            _ => i += 1,
        }
    }
    let head = text[..body].rfind(['f', 'l']).unwrap_or(body);
    Err(("unterminated template: its text ends at the first `}\"` outside a `$` hole".into(), head, text.len(), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<String> {
        scan_document(0, text)
            .unwrap()
            .into_iter()
            .map(|s| match s {
                Segment::Text { text, .. } => format!("T({})", text),
                Segment::Comment { text, .. } => format!("C({})", text),
                Segment::Region { kind, span, .. } => {
                    format!("{:?}({})", kind, &text[span.start as usize..span.end as usize])
                }
            })
            .collect()
    }

    #[test]
    fn scans_regions_and_text() {
        assert_eq!(
            kinds("a {+ 1 2} $x. ${f \"}\"} $[# y 2] $$z $50 $ {q}"),
            vec![
                "T(a {+ 1 2} )",
                "Lookup($x)",
                "T(. )",
                "Brace(${f \"}\"})",
                "T( )",
                "Bracket($[# y 2])",
                "T( $z $50 $ {q})"
            ]
        );
    }

    #[test]
    fn scans_disabled_regions_as_raw_comments() {
        assert_eq!(
            kinds("before $;{raw ;; $missing} middle $;[nested {;} \"]\"] after ${+ 1 2}"),
            vec![
                "T(before )",
                "C(raw ;; $missing)",
                "T( middle )",
                "C(nested {;} \"]\")",
                "T( after )",
                "Brace(${+ 1 2})",
            ]
        );
        assert_eq!(
            kinds("prose;{inert} ;; text $$;{literal} ${\";{ ;; ;[\"}"),
            vec!["T(prose;{inert} ;; text $;{literal} )", "Brace(${\";{ ;; ;[\"})"]
        );
    }

    #[test]
    fn rejects_malformed_block_comments() {
        for (src, message) in [
            ("${+ 1 ;{bad] 2}", "mismatched delimiter inside a block comment"),
            ("$;[bad}", "mismatched delimiter inside a block comment"),
            ("${+ 1 ;[unfinished", "unterminated block comment"),
            ("$;{unfinished", "unterminated block comment"),
            ("$;{\"unfinished}", "unterminated string inside a block comment"),
        ] {
            let err = scan_document(0, src).unwrap_err();
            assert_eq!(err.kind, DiagKind::Syntax);
            assert_eq!(err.message, message, "{}", src);
        }
    }

    #[test]
    fn qualified_lookup_stops_at_sentence_dot() {
        assert_eq!(kinds("$pn::content. $AGE-years $a- $p.name"), vec!["Lookup($pn::content)", "T(. )", "Lookup($AGE-years)", "T( )", "Lookup($a)", "T(- )", "Lookup($p)", "T(.name)"]);
    }

    #[test]
    fn code_fences_are_verbatim_prose() {
        assert_eq!(
            kinds("a $x\n```sh $y\necho $HOME ${q} $$ $[z]\n  ````\n$w"),
            vec!["T(a )", "Lookup($x)", "T(\n```sh $y\necho $HOME ${q} $$ $[z]\n  ````\n)", "Lookup($w)"]
        );
        // Tildes, a longer opening run, and a shorter run inside that does not close it.
        assert_eq!(kinds("~~~~\n~~~\n$x\n~~~~"), vec!["T(~~~~\n~~~\n$x\n~~~~)"]);
        // Not fences: inline code with backticks after the run, a run mid-line, and two backticks.
        assert_eq!(kinds("```a``` $x"), vec!["T(```a``` )", "Lookup($x)"]);
        assert_eq!(kinds("see ``` $x"), vec!["T(see ``` )", "Lookup($x)"]);
        assert_eq!(kinds("`` $x"), vec!["T(`` )", "Lookup($x)"]);
        // A fence inside code is not prose.
        assert_eq!(kinds("${\"\n```\"}\n$x"), vec!["Brace(${\"\n```\"})", "T(\n)", "Lookup($x)"]);
        let err = scan_document(0, "text\n```\n$x\n``").unwrap_err();
        assert!(err.message.starts_with("unterminated code fence: close it with a line of 3 or more `"), "{}", err.message);
        assert_eq!((err.span.unwrap().start, err.span.unwrap().end), (5, 8));
    }

    #[test]
    fn unterminated_is_error() {
        assert!(scan_document(0, "${+ 1 2").is_err());
        assert!(scan_document(0, "$[# x }").is_err());
    }
}
