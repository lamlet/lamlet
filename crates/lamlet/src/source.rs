//! Source files, spans, diagnostics, and the document region scanner.
//!
//! Only `$name`, `${...}`, and `$[...]` are active. Everything else is ignored text that is retained with its
//! span so a host can present it. `$$` is an escaped literal dollar.

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
                i = raw_form_end(text, i + 1).map_err(|(m, a, b)| Diagnostic::new(DiagKind::Syntax, m, Some(Span::new(file, a, b))))?;
                continue;
            }
            b'{' => stack.push((b'}', i)),
            b'[' => stack.push((b']', i)),
            b'}' | b']' => {
                let (expected, at) = stack.pop().expect("region stack");
                if bytes[i] != expected {
                    return Err(Diagnostic::new(
                        DiagKind::Syntax,
                        format!("mismatched delimiter: expected `{}`, found `{}`", expected as char, bytes[i] as char),
                        Some(Span::new(file, i, i + 1)),
                    )
                    .note("opened here", Some(Span::new(file, at, at + 1))));
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
                        return syntax_err("unterminated string literal", Span::new(file, start, bytes.len()));
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
    syntax_err("unterminated active region", Span::new(file, at - 1, text.len()))
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
    fn unterminated_is_error() {
        assert!(scan_document(0, "${+ 1 2").is_err());
        assert!(scan_document(0, "$[# x }").is_err());
    }
}
