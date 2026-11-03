//! Tokens, syntax trees, parser, surface AST, and pattern macros for the code inside active regions.
//!
//! Code is processed in two steps. The *reader* turns tokens into syntax trees (`Syntax`): atoms and
//! brace/bracket lists. The *parser* turns trees into the AST. Macro uses (`{!name ...}`) keep their operands
//! as unparsed trees; the checker expands them (see `match_clause`/`instantiate`) and parses the result.
//! Macros therefore operate on trees, never on text.
//!
//! Hygiene uses marks: every identifier a template introduces has the expansion's mark appended to its first
//! component (`name` + `MARK_SEP` + id). A marked name first resolves to bindings introduced by the same
//! expansion; otherwise the checker resolves the unmarked name in the macro's definition environment.

use crate::source::{is_word_continue, is_word_start, syntax_err, DResult, RegionKind, Span};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Str(Lit),
    /// `f"{...}"`: a template. Its `Lit` is the text between `f"{` and `}"`, exactly as written.
    Template(Lit),
    Int(i64),
    Float(f64),
    /// A possibly qualified identifier, split into components.
    Ident(Vec<String>),
    /// `!name` or `!pkg::name`: a macro reference (components without the bang).
    Macro(Vec<String>),
    /// `.label`, `.label=`, or `#label`: a label function.
    Label(LabelKind, String),
    /// `.name` or `.pkg::name` written directly after a token: call sugar, `x.f` (the reader desugars it).
    Method(Vec<String>),
    /// `;{...}` or `;[...]`: a comment; the raw text between the delimiters.
    Comment(String),
    /// `;; ...` to the end of the line, inside code: the text after the two semicolons.
    LineComment(String),
}

/// A String literal: its raw source text (between the quotes), where that text starts, and its value after
/// escape processing. Escape errors are deferred until the literal is used as a value.
#[derive(Clone, Debug, PartialEq)]
pub struct Lit {
    pub raw: String,
    /// Absolute byte offset of `raw` in its file.
    pub offset: usize,
    pub cooked: Result<String, (String, usize, usize)>,
    /// Hygiene mark when the literal was introduced by a macro template (applied to names in its holes).
    pub mark: Option<u32>,
}

impl Lit {
    pub fn value(&self) -> Option<&str> {
        self.cooked.as_ref().ok().map(|s| s.as_str())
    }
}

/// The three label-function atoms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    /// `.l`
    Get,
    /// `.l=`
    Set,
    /// `#l`
    Tag,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

pub const SYMBOL_CHARS: &str = "+-*/%'|<>=!^#:&?~@";

/// Separates a name from its hygiene mark inside binding keys.
pub const MARK_SEP: char = '\u{1}';

fn is_symbol(c: char) -> bool {
    SYMBOL_CHARS.contains(c)
}

fn is_delim(c: char) -> bool {
    matches!(c, '{' | '}' | '[' | ']')
}

/// The name without any hygiene mark.
pub fn base_name(key: &str) -> &str {
    key.split(MARK_SEP).next().unwrap_or(key)
}

/// Split a marked key into its unmarked name and the outermost mark.
pub fn split_mark(key: &str) -> Option<(&str, u32)> {
    let i = key.rfind(MARK_SEP)?;
    Some((&key[..i], key[i + MARK_SEP.len_utf8()..].parse().ok()?))
}

fn mark(name: &str, m: u32) -> String {
    format!("{}{}{}", name, MARK_SEP, m)
}

/// Lex the text of a region. `base` is the byte offset of `text` within its file.
pub fn lex(file: u32, text: &str, base: usize) -> DResult<Vec<Token>> {
    let mut toks = Vec::new();
    let mut i = 0usize;
    let sp = |s: usize, e: usize| Span::new(file, base + s, base + e);
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let start = i;
        if c == ';' && (text[i + 1..].starts_with('{') || text[i + 1..].starts_with('[')) {
            let end = crate::source::raw_form_end(text, i + 1).map_err(|(msg, a, b)| {
                crate::source::Diagnostic::new(crate::source::DiagKind::Syntax, msg, Some(sp(a, b)))
            })?;
            toks.push(Token { tok: Tok::Comment(text[i + 2..end - 1].to_string()), span: sp(i, end) });
            i = end;
            continue;
        }
        if text[i..].starts_with(";;") {
            let end = text[i..].find('\n').map_or(text.len(), |k| i + k);
            let body = &text[i + 2..end];
            let body = body.strip_prefix(' ').unwrap_or(body);
            toks.push(Token { tok: Tok::LineComment(body.trim_end().to_string()), span: sp(i, end) });
            i = end;
            continue;
        }
        if c == ';' {
            return syntax_err("a comment must start with `;;`, `;{`, or `;[`", sp(i, i + 1));
        }
        let delim = match c {
            '{' => Some(Tok::LBrace),
            '}' => Some(Tok::RBrace),
            '[' => Some(Tok::LBracket),
            ']' => Some(Tok::RBracket),
            _ => None,
        };
        if let Some(tok) = delim {
            toks.push(Token { tok, span: sp(i, i + 1) });
            i += 1;
            continue;
        }
        match c {
            '"' => {
                let end = string_end(text, i).ok_or_else(|| {
                    crate::source::Diagnostic::new(crate::source::DiagKind::Syntax, "unterminated string literal", Some(sp(i, text.len())))
                })?;
                let raw = &text[i + 1..end - 1];
                let offset = base + i + 1;
                let cooked = cook(raw).map_err(|(m, a, b)| (m, offset + a, offset + b));
                toks.push(Token { tok: Tok::Str(Lit { raw: raw.to_string(), offset, cooked, mark: None }), span: sp(i, end) });
                i = end;
            }
            '$' => return syntax_err("`$` does not open a region inside code; use a String for a literal dollar", sp(i, i + 1)),
            '.' if text[i..].starts_with("...") => {
                toks.push(Token { tok: Tok::Ident(vec!["...".into()]), span: sp(i, i + 3) });
                i += 3;
            }
            '.' if text[i + 1..].starts_with(is_word_start) && i > 0 && !text[..i].ends_with(|p: char| p.is_whitespace() || p == '{' || p == '[') => {
                let (parts, end) = lex_qualified(text, i + 1);
                if parts.iter().any(|p| p.starts_with(is_symbol)) {
                    return syntax_err("a method name after `.` must be a word identifier", sp(i, end));
                }
                toks.push(Token { tok: Tok::Method(parts), span: sp(i, end) });
                i = end;
            }
            '.' if text[i + 1..].starts_with(is_symbol) && i > 0 && !text[..i].ends_with(|p: char| p.is_whitespace() || is_delim(p) && p != '}' && p != ']') => {
                return syntax_err("a method name after `.` must be a word identifier (operators are called as `{+ a b}`)", sp(i, i + 2));
            }
            '.' if text[i + 1..].starts_with(is_word_start) => {
                let (label, mut end) = lex_component(text, i + 1);
                let kind = if text[end..].starts_with('=') {
                    end += 1;
                    LabelKind::Set
                } else {
                    LabelKind::Get
                };
                toks.push(Token { tok: Tok::Label(kind, label), span: sp(i, end) });
                i = end;
            }
            '.' if text[i + 1..].chars().next().is_none_or(|n| n.is_whitespace() || is_delim(n)) => {
                toks.push(Token { tok: Tok::Ident(vec![".".into()]), span: sp(i, i + 1) });
                i += 1;
            }
            '#' if text[i + 1..].starts_with(is_word_start) => {
                let (label, end) = lex_component(text, i + 1);
                toks.push(Token { tok: Tok::Label(LabelKind::Tag, label), span: sp(i, end) });
                i = end;
            }
            '!' if text[i + 1..].starts_with(is_word_start) => {
                let (parts, end) = lex_qualified(text, i + 1);
                if parts.iter().skip(1).any(|p| p.starts_with(is_symbol)) || parts.last().unwrap().starts_with(is_symbol) {
                    return syntax_err("a macro name must be a word identifier", sp(i, end));
                }
                toks.push(Token { tok: Tok::Macro(parts), span: sp(i, end) });
                i = end;
            }
            _ if c.is_ascii_digit() || (c == '-' && text[i + 1..].starts_with(|d: char| d.is_ascii_digit())) => {
                let (tok, end) = lex_number(text, i).map_err(|msg| {
                    crate::source::Diagnostic::new(crate::source::DiagKind::Syntax, msg, Some(sp(start, i + 1)))
                })?;
                toks.push(Token { tok, span: sp(i, end) });
                i = end;
            }
            _ if crate::source::raw_template_open(text, i).is_some() => {
                let body = crate::source::raw_template_open(text, i).unwrap();
                let end = crate::source::raw_template_end(text, body).map_err(|(msg, a, b, _)| {
                    crate::source::Diagnostic::new(crate::source::DiagKind::Syntax, msg, Some(sp(a, b)))
                })?;
                let raw = text[body..end - 2].to_string();
                let lit = Lit { cooked: Ok(raw.clone()), raw, offset: base + body, mark: None };
                toks.push(Token { tok: Tok::Template(lit), span: sp(i, end) });
                i = end;
            }
            _ if is_word_start(c) || is_symbol(c) => {
                let (mut parts, mut j) = lex_qualified(text, i);
                // `Import*` is a single keyword.
                if parts.len() == 1 && parts[0] == "Import" && text[j..].starts_with('*') {
                    parts[0].push('*');
                    j += 1;
                }
                toks.push(Token { tok: Tok::Ident(parts), span: sp(i, j) });
                i = j;
            }
            _ => return syntax_err(format!("unexpected character `{}`", c), sp(i, i + c.len_utf8())),
        }
        // Tokens other than delimiters must be followed by whitespace, a delimiter, or the end.
        if let Some(n) = text[i..].chars().next() {
            let method = n == '.' && text[i + 1..].starts_with(|c: char| is_word_start(c) || is_symbol(c));
            if !n.is_whitespace() && !is_delim(n) && n != ';' && !method {
                return syntax_err("expected whitespace or a delimiter after this token", sp(start, i + n.len_utf8()));
            }
        }
    }
    Ok(toks)
}

fn lex_qualified(text: &str, start: usize) -> (Vec<String>, usize) {
    let mut parts = Vec::new();
    let mut j = start;
    loop {
        let (part, end) = lex_component(text, j);
        parts.push(part);
        j = end;
        // A symbolic component consumes `:` itself, so `::` separates only after a word component.
        if !part_is_symbol(&parts) && text[j..].starts_with("::") {
            if let Some(n) = text[j + 2..].chars().next() {
                if is_word_start(n) || is_symbol(n) {
                    j += 2;
                    continue;
                }
            }
        }
        break;
    }
    (parts, j)
}

fn part_is_symbol(parts: &[String]) -> bool {
    parts.last().is_some_and(|p| p.starts_with(is_symbol))
}

fn lex_component(text: &str, start: usize) -> (String, usize) {
    let first = text[start..].chars().next().unwrap();
    let mut i = start;
    if is_symbol(first) {
        while let Some(c) = text[i..].chars().next() {
            if is_symbol(c) {
                i += c.len_utf8();
            } else {
                break;
            }
        }
    } else {
        let mut first = true;
        while let Some(c) = text[i..].chars().next() {
            if (first && is_word_start(c)) || (!first && is_word_continue(c)) {
                i += c.len_utf8();
                first = false;
            } else if !first && c == '-' && text[i + 1..].starts_with(is_word_continue) {
                i += 1;
            } else {
                break;
            }
        }
    }
    (text[start..i].to_string(), i)
}

fn lex_number(text: &str, start: usize) -> Result<(Tok, usize), String> {
    let b = text.as_bytes();
    let mut i = start;
    if b[i] == b'-' {
        i += 1;
    }
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut is_float = false;
    if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
        is_float = true;
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            is_float = true;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let s = &text[start..i];
    if is_float {
        let f: f64 = s.parse().map_err(|_| format!("invalid Float literal `{}`", s))?;
        if !f.is_finite() {
            return Err(format!("Float literal `{}` is not finite", s));
        }
        Ok((Tok::Float(if f == 0.0 { 0.0 } else { f }), i))
    } else {
        let n: i64 = s.parse().map_err(|_| format!("Int literal `{}` is outside the signed 64-bit range", s))?;
        Ok((Tok::Int(n), i))
    }
}

/// Index after the closing quote of the String starting at `start`. A backslash always skips the next
/// character (the same rule the region scanner uses), even in raw forms.
fn string_end(text: &str, start: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// Process escapes: `\"`, `\\`, `\n`, `\t`, `\r`, `\0`, and `\u{hex}`. Error positions are relative to `raw`.
pub fn cook(raw: &str) -> Result<String, (String, usize, usize)> {
    let mut out = String::new();
    let mut i = 0;
    while let Some(c) = raw[i..].chars().next() {
        if c != '\\' {
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        let e = raw[i + 1..].chars().next().ok_or(("unterminated escape".to_string(), i, i + 1))?;
        match e {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            'u' => {
                let rest = &raw[i + 2..];
                if !rest.starts_with('{') {
                    return Err(("expected `{` after `\\u`".into(), i, i + 2));
                }
                let close = rest.find('}').ok_or(("unterminated `\\u{...}` escape".to_string(), i, i + 2))?;
                let hex = &rest[1..close];
                match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => return Err((format!("invalid Unicode scalar escape `{}`", hex), i, i + 3 + close)),
                }
                i += 2 + close + 1;
                continue;
            }
            other => {
                let hint = if other.is_ascii_alphabetic() { " (for LaTeX, write a template f\"{...}\", where backslashes are literal)" } else { "" };
                return Err((format!("unknown escape `\\{}`{}", other, hint), i, i + 1 + other.len_utf8()));
            }
        }
        i += 1 + e.len_utf8();
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------
// Syntax trees (the reader)

#[derive(Clone, Debug)]
pub enum Syntax {
    Atom(Token),
    Brace(Vec<Syntax>, Span),
    /// A bracket form and the doc comment directly preceding it, if any.
    Bracket(Vec<Syntax>, Span, Option<String>),
}

impl Syntax {
    /// Number of nodes in the tree (used to bound macro expansion output).
    pub fn size(&self) -> usize {
        match self {
            Syntax::Atom(_) => 1,
            Syntax::Brace(xs, _) | Syntax::Bracket(xs, _, _) => 1 + xs.iter().map(Syntax::size).sum::<usize>(),
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Syntax::Atom(t) => t.span,
            Syntax::Brace(_, s) | Syntax::Bracket(_, s, _) => *s,
        }
    }
    /// The unmarked identifier, if this is a single-component identifier atom.
    fn ident(&self) -> Option<&str> {
        match self {
            Syntax::Atom(Token { tok: Tok::Ident(p), .. }) if p.len() == 1 => Some(base_name(&p[0])),
            _ => None,
        }
    }
    fn is_ellipsis(&self) -> bool {
        self.ident() == Some("...")
    }
}

type OpenForm = (Tok, Span, Vec<Syntax>, Option<String>);

/// Maximum nesting of braces and brackets in source. Bounds the host stack used by parsing, macro
/// instantiation, and dropping syntax trees; the checker separately bounds expression nesting.
pub const MAX_SYNTAX_DEPTH: usize = 200;

/// Read tokens into a sequence of syntax trees.
pub fn read(toks: &[Token], region: Span) -> DResult<Vec<Syntax>> {
    // A comment directly before a bracket form becomes its doc; comments are otherwise dropped.
    // Open forms: the opener, its span, the enclosing form's items so far, and its doc comment.
    let mut stack: Vec<OpenForm> = Vec::new();
    let mut top: Vec<Syntax> = Vec::new();
    // Whether `top` is exactly a method-call head built from `{r.f ...}` with no arguments added yet.
    let mut method_head = false;
    let mut pending: Option<String> = None;
    let mut line_run = false;
    for t in toks {
        if !matches!(t.tok, Tok::LineComment(_)) {
            line_run = false;
        }
        match &t.tok {
            Tok::Comment(text) => {
                pending = Some(text.trim().to_string());
                line_run = false;
            }
            // Consecutive `;;` lines form one comment.
            Tok::LineComment(text) => {
                pending = match (pending.take(), line_run) {
                    (Some(p), true) => Some(format!("{}\n{}", p, text)),
                    _ => Some(text.clone()),
                };
                line_run = true;
                continue;
            }
            Tok::LBrace | Tok::LBracket => {
                if stack.len() >= MAX_SYNTAX_DEPTH {
                    return Err(crate::source::Diagnostic::new(
                        crate::source::DiagKind::Limit,
                        format!("syntax nesting limit ({} levels) exceeded", MAX_SYNTAX_DEPTH),
                        Some(t.span),
                    ));
                }
                let doc = if t.tok == Tok::LBracket { pending.take() } else { None };
                pending = None;
                stack.push((t.tok.clone(), t.span, std::mem::take(&mut top), doc));
                method_head = false;
            }
            Tok::RBrace | Tok::RBracket => {
                pending = None;
                let (open, ospan, parent, doc) = match stack.pop() {
                    Some(x) => x,
                    None => return syntax_err("unexpected closing delimiter", t.span),
                };
                let items = std::mem::replace(&mut top, parent);
                let span = ospan.to(t.span);
                let node = match (open, &t.tok) {
                    (Tok::LBrace, Tok::RBrace) => Syntax::Brace(items, span),
                    (Tok::LBracket, Tok::RBracket) => Syntax::Bracket(items, span, doc),
                    _ => return syntax_err("mismatched delimiter", t.span),
                };
                // The closed form is an argument of its parent, so the parent is no longer a bare method head.
                method_head = false;
                top.push(node);
            }
            // Call sugar (spec §2.6): `r.f` is `{f r}`; at the head of a brace, `{r.f a b}` is `{f r a b}`.
            Tok::Method(parts) => {
                pending = None;
                let name = Syntax::Atom(Token { tok: Tok::Ident(parts.clone()), span: t.span });
                let in_brace = matches!(stack.last(), Some((Tok::LBrace, ..)));
                if in_brace && (top.len() == 1 || method_head) {
                    // The receiver is the whole head so far: `{r.f.g ...}` is `{g {f r} ...}`.
                    let recv = if method_head {
                        let span = top[1].span().to(t.span);
                        Syntax::Brace(std::mem::take(&mut top), span)
                    } else {
                        top.pop().unwrap()
                    };
                    top = vec![name, recv];
                    method_head = true;
                } else {
                    let recv = match top.pop() {
                        Some(r) => r,
                        None => return syntax_err("a `.method` call needs a receiver before the dot", t.span),
                    };
                    let span = recv.span().to(t.span);
                    top.push(Syntax::Brace(vec![name, recv], span));
                }
            }
            _ => {
                pending = None;
                method_head = false;
                top.push(Syntax::Atom(t.clone()))
            }
        }
    }
    if let Some((_, s, _, _)) = stack.last() {
        return syntax_err("unterminated form", s.to(region));
    }
    Ok(top)
}

// ---------------------------------------------------------------------------------------------------------
// AST

#[derive(Clone, Debug)]
pub enum Expr {
    Int(i64, Span),
    Float(f64, Span),
    Str(String, Span),
    Bool(bool, Span),
    /// Name components; the first may carry a hygiene mark.
    Name(Vec<String>, Span),
    Call(Box<Expr>, Vec<Expr>, Span),
    /// Optional self name (recursive lambda), parameters, body.
    Lambda(Option<(String, Span)>, Vec<(String, Span)>, Body, Span),
    /// `{' e ...}`
    List(Vec<Expr>, Span),
    /// `{. l e ...}`: labels in source order.
    Record(Vec<(String, Span, Expr)>, Span),
    /// `.l`, `.l=`, `#l`
    Label(LabelKind, String, Span),
    Ascribe(Box<Expr>, Box<Expr>, Span),
    Static(Box<Expr>, Span),
    /// `{!name operands...}`: operands stay unparsed until expansion.
    Macro(Vec<String>, Vec<Syntax>, Span),
    /// `f"{...}"`: literal text and `$` holes, a String.
    Template(Vec<Part>, Span),
}

#[derive(Clone, Debug)]
pub enum Part {
    Lit(String),
    Hole(Expr),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int(_, s)
            | Expr::Float(_, s)
            | Expr::Str(_, s)
            | Expr::Bool(_, s)
            | Expr::Name(_, s)
            | Expr::Call(_, _, s)
            | Expr::Lambda(_, _, _, s)
            | Expr::List(_, s)
            | Expr::Record(_, s)
            | Expr::Label(_, _, s)
            | Expr::Ascribe(_, _, s)
            | Expr::Static(_, s)
            | Expr::Macro(_, _, s)
            | Expr::Template(_, s) => *s,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Body {
    pub decls: Vec<Decl>,
    pub result: Box<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportKind {
    Qualified,
    Wildcard,
    Alias,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Pure,
    Static,
    Runtime,
}

impl Availability {
    pub fn as_str(self) -> &'static str {
        match self {
            Availability::Pure => "Pure",
            Availability::Static => "Static",
            Availability::Runtime => "Runtime",
        }
    }
}

/// One `{pattern template}` clause of a macro.
#[derive(Clone, Debug)]
pub struct MacroClause {
    pub pattern: Vec<Syntax>,
    pub template: Syntax,
}

#[derive(Clone, Debug)]
pub enum Decl {
    Def { name: String, name_span: Span, init: Expr, span: Span, doc: Option<String> },
    Import { kind: ImportKind, package: String, alias: Option<String>, span: Span },
    Native { name: String, name_span: Span, ty: Expr, api_id: String, availability: Availability, span: Span, doc: Option<String> },
    NativeType { name: String, name_span: Span, type_id: String, span: Span, doc: Option<String> },
    /// `[Macro !name {pattern template}...]`; `name` is the binding key (`!name`).
    Macro { name: String, name_span: Span, clauses: Vec<MacroClause>, span: Span, doc: Option<String> },
    /// `[Type {name params...} roll unroll body]` (or a bare `name` with no parameters): a recursive type.
    Type {
        name: String,
        name_span: Span,
        params: Vec<(String, Span)>,
        roll: (String, Span),
        unroll: (String, Span),
        body: Expr,
        span: Span,
        doc: Option<String>,
    },
}

impl Decl {
    pub fn span(&self) -> Span {
        match self {
            Decl::Def { span, .. }
            | Decl::Import { span, .. }
            | Decl::Native { span, .. }
            | Decl::NativeType { span, .. }
            | Decl::Macro { span, .. }
            | Decl::Type { span, .. } => *span,
        }
    }

    /// The declared name (binding key) and doc slot of documentable declarations.
    pub fn doc_slot(&mut self) -> Option<(&str, &mut Option<String>)> {
        match self {
            Decl::Def { name, doc, .. }
            | Decl::Native { name, doc, .. }
            | Decl::NativeType { name, doc, .. }
            | Decl::Macro { name, doc, .. }
            | Decl::Type { name, doc, .. } => {
                Some((name.as_str(), doc))
            }
            Decl::Import { .. } => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Item {
    /// One bracket declaration; `[# a 1 b 2]` holds several, in source order.
    Decls(Vec<Decl>),
    Expr(Expr),
}

// ---------------------------------------------------------------------------------------------------------
// Parser (syntax trees to AST)

/// `λ` is the other spelling of `^`.
const RESERVED: &[&str] = &["^", "λ", ":", "'", ".", "true", "false", "..."];

fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&base_name(name))
}

/// Parse a syntax tree as an item (an expression or a bracket declaration).
pub fn parse_item(s: &Syntax) -> DResult<Item> {
    match s {
        Syntax::Bracket(items, span, doc) => bracket(items, *span, doc.clone()),
        _ => Ok(Item::Expr(parse_expr(s)?)),
    }
}

pub fn parse_expr(s: &Syntax) -> DResult<Expr> {
    match s {
        Syntax::Atom(t) => match &t.tok {
            Tok::Int(n) => Ok(Expr::Int(*n, t.span)),
            Tok::Float(f) => Ok(Expr::Float(*f, t.span)),
            Tok::Str(l) => match &l.cooked {
                Ok(x) => Ok(Expr::Str(x.clone(), t.span)),
                Err((m, a, b)) => syntax_err(m.clone(), Span::new(t.span.file, *a, *b)),
            },
            Tok::Template(l) => template(l, t.span.file, t.span),
            Tok::Ident(parts) => name_expr(parts, t.span),
            Tok::Macro(p) => syntax_err(format!("a macro must be applied: write {{!{} ...}}", p.join("::")), t.span),
            Tok::Label(k, l) => Ok(Expr::Label(*k, base_name(l).to_string(), t.span)),
            _ => unreachable!("delimiters are not atoms"),
        },
        Syntax::Brace(items, span) => brace(items, *span),
        Syntax::Bracket(items, span, _) => match bracket(items, *span, None)? {
            Item::Expr(e) => Ok(e),
            Item::Decls(_) => syntax_err("a declaration has no value and cannot be used as an expression", *span),
        },
    }
}

fn name_expr(parts: &[String], span: Span) -> DResult<Expr> {
    if parts.len() == 1 {
        match base_name(&parts[0]) {
            "true" => return Ok(Expr::Bool(true, span)),
            "false" => return Ok(Expr::Bool(false, span)),
            p if is_reserved(p) => return syntax_err(format!("`{}` is reserved syntax and cannot be used as a value", p), span),
            _ => {}
        }
    }
    Ok(Expr::Name(parts.to_vec(), span))
}

fn brace(items: &[Syntax], span: Span) -> DResult<Expr> {
    let head = match items.first() {
        Some(h) => h,
        None => return syntax_err("empty application `{}`", span),
    };
    match head {
        Syntax::Atom(Token { tok: Tok::Macro(parts), .. }) => return Ok(Expr::Macro(parts.clone(), items[1..].to_vec(), span)),
        _ => match head.ident() {
            Some("^") | Some("λ") => return lambda(items, span),
            Some("'") => return Ok(Expr::List(items[1..].iter().map(parse_expr).collect::<DResult<Vec<_>>>()?, span)),
            Some(".") => return record_literal(&items[1..], span),
            Some(":") => {
                if items.len() != 3 {
                    return syntax_err("a `:` ascription takes exactly an expression and a Type", span);
                }
                return Ok(Expr::Ascribe(Box::new(parse_expr(&items[1])?), Box::new(parse_expr(&items[2])?), span));
            }
            _ => {}
        },
    }
    let f = parse_expr(head)?;
    let args = items[1..].iter().map(parse_expr).collect::<DResult<Vec<_>>>()?;
    Ok(Expr::Call(Box::new(f), args, span))
}

fn param(s: &Syntax, what: &str) -> DResult<(String, Span)> {
    match s {
        Syntax::Atom(Token { tok: Tok::Ident(p), span }) if p.len() == 1 && !is_reserved(&p[0]) => Ok((p[0].clone(), *span)),
        _ => syntax_err(format!("expected {} (an unqualified, unreserved identifier)", what), s.span()),
    }
}

fn record_literal(items: &[Syntax], span: Span) -> DResult<Expr> {
    if !items.len().is_multiple_of(2) {
        return syntax_err("a record literal `{. label value ...}` needs a value after every label", span);
    }
    let mut fields: Vec<(String, Span, Expr)> = Vec::new();
    for pair in items.chunks(2) {
        let (label, lspan) = match &pair[0] {
            Syntax::Atom(Token { tok: Tok::Ident(p), span }) if p.len() == 1 && crate::types::is_label(base_name(&p[0])) => {
                (base_name(&p[0]).to_string(), *span)
            }
            other => return syntax_err("expected a field label (a word identifier)", other.span()),
        };
        if fields.iter().any(|f| f.0 == label) {
            return syntax_err(format!("duplicate field `{}` in a record literal", label), lspan);
        }
        fields.push((label, lspan, parse_expr(&pair[1])?));
    }
    Ok(Expr::Record(fields, span))
}

fn lambda(items: &[Syntax], span: Span) -> DResult<Expr> {
    let (self_name, at) = match items.get(1) {
        Some(s @ Syntax::Atom(Token { tok: Tok::Ident(_), .. })) => (Some(param(s, "a recursive lambda's name")?), 2),
        _ => (None, 1),
    };
    let ps = match items.get(at) {
        Some(Syntax::Brace(ps, _)) => ps,
        _ => return syntax_err("expected a `{...}` parameter list after `^` or `λ` (or after `^ name`)", items.get(at).map_or(span, |s| s.span())),
    };
    lambda_parts(self_name, ps, &items[at + 1..], span)
}

/// A lambda from its optional self name, its parameter syntax, and its body items.
fn lambda_parts(self_name: Option<(String, Span)>, ps: &[Syntax], body_items: &[Syntax], span: Span) -> DResult<Expr> {
    let mut params: Vec<(String, Span)> = Vec::new();
    for p in ps {
        let (name, pspan) = param(p, "a lambda parameter")?;
        if params.iter().any(|(n, _)| *n == name) {
            return Err(crate::source::Diagnostic::new(
                crate::source::DiagKind::Name,
                format!("duplicate parameter `{}`", base_name(&name)),
                Some(pspan),
            ));
        }
        params.push((name, pspan));
    }
    if let Some((n, ns)) = &self_name {
        if params.iter().any(|(p, _)| base_name(p) == base_name(n)) {
            return syntax_err(format!("`{}` is both the lambda's name and a parameter", base_name(n)), *ns);
        }
    }
    let body = body(body_items, span)?;
    Ok(Expr::Lambda(self_name, params, body, span))
}

/// Declarations followed by exactly one result expression.
fn body(items: &[Syntax], span: Span) -> DResult<Body> {
    let mut decls = Vec::new();
    let mut result: Option<Expr> = None;
    for s in items {
        match parse_item(s)? {
            Item::Decls(ds) => {
                if let Some(r) = &result {
                    return syntax_err("declarations must precede the body's result expression", r.span().to(s.span()));
                }
                decls.extend(ds)
            }
            Item::Expr(e) => {
                if result.is_some() {
                    return syntax_err("a function body has exactly one result expression", e.span());
                }
                result = Some(e)
            }
        }
    }
    match result {
        Some(r) => Ok(Body { decls, result: Box::new(r) }),
        None => syntax_err("a function body needs a result expression", span),
    }
}

fn string(s: Option<&Syntax>, what: &str, span: Span) -> DResult<String> {
    match s {
        Some(Syntax::Atom(Token { tok: Tok::Str(l), .. })) if l.value().is_some_and(|x| !x.is_empty()) => Ok(l.value().unwrap().to_string()),
        _ => syntax_err(format!("expected {} (a nonempty String literal)", what), s.map_or(span, |s| s.span())),
    }
}

fn arity(items: &[Syntax], n: usize, form: &str, span: Span) -> DResult<()> {
    if items.len() != n {
        return syntax_err(format!("{} takes exactly {} operand(s)", form, n - 1), span);
    }
    Ok(())
}

fn bracket(items: &[Syntax], span: Span, doc: Option<String>) -> DResult<Item> {
    let head = items.first().ok_or_else(|| {
        crate::source::Diagnostic::new(crate::source::DiagKind::Syntax, "empty bracket form `[]`", Some(span))
    })?;
    let name = match head.ident() {
        Some(n) => n.to_string(),
        None => return syntax_err("expected a bracket form name such as `#`, `%`, `Import`, or `Macro`", head.span()),
    };
    let item = match name.as_str() {
        "#" => Item::Decls(bindings(&items[1..], span, doc)?),
        "%" => {
            arity(items, 2, "`[% expression]`", span)?;
            Item::Expr(Expr::Static(Box::new(parse_expr(&items[1])?), span))
        }
        "Import" | "Import*" | "ImportAs" => {
            let n = if name == "ImportAs" { 3 } else { 2 };
            arity(items, n, &format!("`[{} ...]`", name), span)?;
            let (pkg, _) = param(&items[1], "a package name")?;
            let (kind, alias) = match name.as_str() {
                "Import" => (ImportKind::Qualified, None),
                "Import*" => (ImportKind::Wildcard, None),
                _ => (ImportKind::Alias, Some(param(&items[2], "a namespace alias")?.0)),
            };
            Item::Decls(vec![Decl::Import { kind, package: base_name(&pkg).to_string(), alias, span }])
        }
        "Native" => {
            arity(items, 5, "`[Native name Type \"api-id\" availability]`", span)?;
            let (n, ns) = param(&items[1], "a native binding name")?;
            let ty = parse_expr(&items[2])?;
            let api_id = string(items.get(3), "an API ID", span)?;
            let availability = match items[4].ident() {
                Some("Pure") => Availability::Pure,
                Some("Static") => Availability::Static,
                Some("Runtime") => Availability::Runtime,
                _ => return syntax_err("availability must be Pure, Static, or Runtime", items[4].span()),
            };
            Item::Decls(vec![Decl::Native { name: n, name_span: ns, ty, api_id, availability, span, doc }])
        }
        "NativeType" => {
            arity(items, 3, "`[NativeType name \"type-id\"]`", span)?;
            let (n, ns) = param(&items[1], "a native type name")?;
            let type_id = string(items.get(2), "a native type ID", span)?;
            Item::Decls(vec![Decl::NativeType { name: n, name_span: ns, type_id, span, doc }])
        }
        "Macro" => macro_decl(items, span, doc)?,
        "Type" => {
            arity(items, 5, "`[Type {Name params...} roll unroll body]`", span)?;
            let (n, ns, params) = match &items[1] {
                Syntax::Brace(head, hspan) => {
                    let (n, ns) = match head.first() {
                        Some(h) => param(h, "a type name")?,
                        None => return syntax_err("a type head `{Name params...}` needs a name", *hspan),
                    };
                    let params = head[1..].iter().map(|p| param(p, "a type parameter name")).collect::<DResult<Vec<_>>>()?;
                    (n, ns, params)
                }
                other => {
                    let (n, ns) = param(other, "a type name, or a type head `{Name params...}`")?;
                    (n, ns, vec![])
                }
            };
            let roll = param(&items[2], "the name of the roll function")?;
            let unroll = param(&items[3], "the name of the unroll function")?;
            let body = parse_expr(&items[4])?;
            Item::Decls(vec![Decl::Type { name: n, name_span: ns, params, roll, unroll, body, span, doc }])
        }
        _ => return syntax_err(format!("unknown bracket form `[{} ...]`", name), head.span()),
    };
    Ok(item)
}

/// The bindings of `[# ...]`: one or more `name expression` pairs, or function definitions
/// `{name params...} decl... result`, which are `name {^ name {params...} decl... result}`. The region's doc
/// goes to the first binding.
fn bindings(items: &[Syntax], span: Span, mut doc: Option<String>) -> DResult<Vec<Decl>> {
    if items.is_empty() {
        return syntax_err("`[# name expression]` needs a name, or a function head `{name params...}`", span);
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let target = &items[i];
        let rest = &items[i + 1..];
        let (n, ns, init, used) = match target {
            Syntax::Brace(head, hspan) => {
                let (n, ns) = match head.first() {
                    Some(h) => param(h, "a function name")?,
                    None => return syntax_err("a function head `{name params...}` needs a name", *hspan),
                };
                // The body: declarations, then exactly one result expression (where the next binding starts).
                let len = match rest.iter().position(|s| !is_decl_form(s)) {
                    Some(k) => k + 1,
                    None => return syntax_err(format!("the definition of `{}` needs a result expression", base_name(&n)), target.span()),
                };
                let lspan = target.span().to(rest[len - 1].span());
                (n.clone(), ns, lambda_parts(Some((n, ns)), &head[1..], &rest[..len], lspan)?, len)
            }
            _ => {
                let (n, ns) = param(target, "a binding name, or a function head `{name params...}`")?;
                match rest.first() {
                    Some(e) => (n, ns, parse_expr(e)?, 1),
                    None => return syntax_err(format!("the binding of `{}` needs an expression", base_name(&n)), target.span()),
                }
            }
        };
        let dspan = if items.len() == 1 + used { span } else { target.span().to(rest[used - 1].span()) };
        out.push(Decl::Def { name: n, name_span: ns, init, span: dspan, doc: doc.take() });
        i += 1 + used;
    }
    Ok(out)
}

/// Whether a syntax tree is a declaration form, as opposed to an expression (`[% ...]` and templates are
/// expressions).
fn is_decl_form(s: &Syntax) -> bool {
    match s {
        Syntax::Bracket(items, _, _) => {
            matches!(items.first().and_then(|h| h.ident()), Some("#" | "Import" | "Import*" | "ImportAs" | "Native" | "NativeType" | "Macro" | "Type"))
        }
        _ => false,
    }
}

fn macro_decl(items: &[Syntax], span: Span, doc: Option<String>) -> DResult<Item> {
    let (name, name_span) = match items.get(1) {
        Some(Syntax::Atom(Token { tok: Tok::Macro(p), span })) if p.len() == 1 => (format!("!{}", p[0]), *span),
        other => return syntax_err("expected a macro name such as `!unless`", other.map_or(span, |s| s.span())),
    };
    if items.len() < 3 {
        return syntax_err("a macro needs at least one `{pattern template}` clause", span);
    }
    let mut clauses = Vec::new();
    for c in &items[2..] {
        match c {
            Syntax::Brace(parts, cspan) if parts.len() == 2 => match &parts[0] {
                Syntax::Brace(pattern, _) => {
                    let mut vars = Vec::new();
                    check_pattern(pattern, &mut vars)?;
                    clauses.push(MacroClause { pattern: pattern.clone(), template: parts[1].clone() });
                }
                other => return syntax_err("a macro pattern is a `{...}` list of operand patterns", other.span()),
            },
            other => {
                return syntax_err("a macro clause is `{pattern template}`", match other {
                    Syntax::Brace(_, s) => *s,
                    o => o.span(),
                })
            }
        }
    }
    Ok(Item::Decls(vec![Decl::Macro { name, name_span, clauses, span, doc }]))
}

/// Validate a pattern list: at most one `...` per list, following an element; variables are unique.
fn check_pattern(items: &[Syntax], vars: &mut Vec<String>) -> DResult<()> {
    let mut seen_ellipsis = false;
    for (i, p) in items.iter().enumerate() {
        if p.is_ellipsis() {
            if i == 0 && items.len() > 1 {
                return syntax_err("a splice group `{... }` is a template form; patterns cannot use it", p.span());
            }
            if i == 0 || seen_ellipsis {
                return syntax_err("`...` must follow a pattern element, at most once per list", p.span());
            }
            seen_ellipsis = true;
            continue;
        }
        match p {
            Syntax::Atom(Token { tok: Tok::Ident(parts), span }) => {
                if parts.len() != 1 {
                    return syntax_err("pattern variables are unqualified identifiers", *span);
                }
                let v = &parts[0];
                if v != "_" && v != "true" && v != "false" {
                    if vars.contains(v) {
                        return syntax_err(format!("pattern variable `{}` appears twice", v), *span);
                    }
                    vars.push(v.clone());
                }
            }
            Syntax::Atom(Token { tok: Tok::Macro(_), span }) => return syntax_err("a macro name cannot appear in a pattern", *span),
            Syntax::Atom(_) => {}
            Syntax::Brace(xs, _) | Syntax::Bracket(xs, _, _) => check_pattern(xs, vars)?,
        }
    }
    Ok(())
}

/// Read and parse the inner text of an active region into exactly one item.
pub fn parse_region(file: u32, kind: RegionKind, text: &str, base: usize) -> DResult<Item> {
    let toks = lex(file, text, base)?;
    let region = Span::new(file, base, base + text.len());
    if kind == RegionKind::Lookup {
        return match toks.as_slice() {
            [Token { tok: Tok::Ident(parts), span }] => Ok(Item::Expr(name_expr(parts, *span)?)),
            _ => syntax_err("expected a name", region),
        };
    }
    let trees = read(&toks, region)?;
    match trees.as_slice() {
        [one] => parse_item(one),
        [] => syntax_err("empty region", region),
        [_, extra, ..] => syntax_err("unexpected tokens after the end of the region's form", extra.span()),
    }
}

// ---------------------------------------------------------------------------------------------------------
// Templates: f"{...}"

enum RawPart {
    /// Literal text.
    Text(String),
    Hole(Expr),
}

/// Parse a template literal. Holes use the document convention: `$name`/`$a.b`, `${expr}`, `$[expr]`, and
/// `$$` for a literal dollar; any other `$` is literal. A template starting with a newline has that newline,
/// the indentation shared by its lines, and a final whitespace-only line removed.
///
/// The text has no escapes: it is exactly as written, and holes may contain String literals, since a quote
/// does not end a template.
fn template(lit: &Lit, file: u32, span: Span) -> DResult<Expr> {
    let raw = lit.raw.as_str();
    let base = lit.offset;
    let mut parts: Vec<RawPart> = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    let push_text = |parts: &mut Vec<RawPart>, text: &mut String| {
        if !text.is_empty() {
            parts.push(RawPart::Text(std::mem::take(text)));
        }
    };
    while let Some(c) = raw[i..].chars().next() {
        if c != '$' {
            text.push(c);
            i += c.len_utf8();
            continue;
        }
        let next = raw[i + 1..].chars().next();
        match next {
            Some('$') => {
                text.push('$');
                i += 2;
            }
            Some('{') | Some('[') => {
                let close = crate::source::code_end(raw, i + 1).ok().ok_or_else(|| {
                    crate::source::Diagnostic::new(
                        crate::source::DiagKind::Syntax,
                        "unterminated `$` hole in a template",
                        Some(Span::new(file, base + i, base + raw.len())),
                    )
                })?;
                push_text(&mut parts, &mut text);
                let code = &raw[i + 1..close];
                let toks = lex(file, code, base + i + 1)?;
                let hole_span = Span::new(file, base + i, base + close);
                let mut trees = read(&toks, hole_span)?;
                if trees.len() != 1 {
                    return syntax_err("a `$` hole contains exactly one form", hole_span);
                }
                let mut tree = trees.pop().unwrap();
                if let Some(m) = lit.mark {
                    tree = instantiate(&tree, &HashMap::new(), m).map_err(|e| crate::source::Diagnostic::new(crate::source::DiagKind::Syntax, e, Some(hole_span)))?;
                }
                parts.push(RawPart::Hole(parse_expr(&tree)?));
                i = close;
            }
            Some(n) if crate::source::is_word_start(n) => {
                push_text(&mut parts, &mut text);
                let end = crate::source::scan_qualified_word(raw, i + 1);
                let mut names: Vec<String> = raw[i + 1..end].split("::").map(String::from).collect();
                if let Some(m) = lit.mark {
                    names[0] = mark(&names[0], m);
                }
                parts.push(RawPart::Hole(name_expr(&names, Span::new(file, base + i, base + end))?));
                i = end;
            }
            _ => {
                text.push('$');
                i += 1;
            }
        }
    }
    push_text(&mut parts, &mut text);
    dedent(raw, &mut parts);
    let mut out = Vec::new();
    for p in parts {
        match p {
            RawPart::Hole(e) => out.push(Part::Hole(e)),
            RawPart::Text(t) => {
                if !t.is_empty() {
                    out.push(Part::Lit(t));
                }
            }
        }
    }
    Ok(Expr::Template(out, span))
}

/// Strip common indentation from a template that starts with a newline (see `template`).
fn dedent(raw: &str, parts: &mut [RawPart]) {
    let body = match raw.strip_prefix("\r\n").or_else(|| raw.strip_prefix('\n')) {
        Some(b) => b,
        None => return,
    };
    let lines: Vec<&str> = body.split('\n').collect();
    let last_blank = lines.last().is_some_and(|l| l.trim().is_empty());
    let content = if last_blank { &lines[..lines.len() - 1] } else { &lines[..] };
    let indent = content
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').count())
        .min()
        .unwrap_or(0);
    let mut first = true;
    let mut at_line_start = true;
    let count = parts.len();
    for (pi, p) in parts.iter_mut().enumerate() {
        match p {
            RawPart::Hole(_) => at_line_start = false,
            RawPart::Text(t) => {
                let mut out = String::new();
                let mut chars = t.chars().peekable();
                if first {
                    if chars.peek() == Some(&'\r') {
                        chars.next();
                    }
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                }
                let mut skipped = 0;
                for c in chars {
                    if at_line_start && skipped < indent && (c == ' ' || c == '\t') {
                        skipped += 1;
                        continue;
                    }
                    at_line_start = false;
                    out.push(c);
                    if c == '\n' {
                        at_line_start = true;
                        skipped = 0;
                    }
                }
                if pi == count - 1 && last_blank {
                    let keep = out.trim_end_matches([' ', '\t']).len();
                    out.truncate(keep);
                }
                *t = out;
            }
        }
        first = false;
    }
}

// ---------------------------------------------------------------------------------------------------------
// Pattern matching and template instantiation

#[derive(Clone, Debug)]
pub enum MatchBinding {
    One(Syntax),
    Many(Vec<MatchBinding>),
}

pub type Bindings = HashMap<String, MatchBinding>;

fn atom_eq(a: &Tok, b: &Tok) -> bool {
    match (a, b) {
        (Tok::Int(x), Tok::Int(y)) => x == y,
        (Tok::Float(x), Tok::Float(y)) => x == y,
        (Tok::Str(x), Tok::Str(y)) => x.value().is_some() && x.value() == y.value(),
        (Tok::Template(x), Tok::Template(y)) => x.raw == y.raw,
        (Tok::Ident(x), Tok::Ident(y)) => x.len() == 1 && y.len() == 1 && base_name(&x[0]) == base_name(&y[0]),
        (Tok::Label(k1, x), Tok::Label(k2, y)) => k1 == k2 && x == y,
        _ => false,
    }
}

/// Match operands against a clause's pattern list.
pub fn match_clause(pattern: &[Syntax], operands: &[Syntax]) -> Option<Bindings> {
    let mut b = HashMap::new();
    if match_list(pattern, operands, &mut b) {
        Some(b)
    } else {
        None
    }
}

fn match_list(pats: &[Syntax], items: &[Syntax], b: &mut Bindings) -> bool {
    let ell = pats.iter().position(|p| p.is_ellipsis());
    match ell {
        None => pats.len() == items.len() && pats.iter().zip(items).all(|(p, s)| match_one(p, s, b)),
        Some(e) => {
            let (before, rep, after) = (&pats[..e - 1], &pats[e - 1], &pats[e + 1..]);
            if items.len() < before.len() + after.len() {
                return false;
            }
            let mid_end = items.len() - after.len();
            if !before.iter().zip(&items[..before.len()]).all(|(p, s)| match_one(p, s, b)) {
                return false;
            }
            if !after.iter().zip(&items[mid_end..]).all(|(p, s)| match_one(p, s, b)) {
                return false;
            }
            let mut vars = Vec::new();
            pattern_vars(rep, &mut vars);
            let mut seqs: HashMap<String, Vec<MatchBinding>> = vars.iter().map(|v| (v.clone(), Vec::new())).collect();
            for s in &items[before.len()..mid_end] {
                let mut inner = HashMap::new();
                if !match_one(rep, s, &mut inner) {
                    return false;
                }
                for (k, v) in inner {
                    seqs.get_mut(&k).unwrap().push(v);
                }
            }
            for (k, v) in seqs {
                b.insert(k, MatchBinding::Many(v));
            }
            true
        }
    }
}

fn match_one(p: &Syntax, s: &Syntax, b: &mut Bindings) -> bool {
    match p {
        Syntax::Atom(Token { tok: Tok::Ident(parts), .. }) if parts.len() == 1 && parts[0] != "true" && parts[0] != "false" => {
            if parts[0] != "_" {
                b.insert(parts[0].clone(), MatchBinding::One(s.clone()));
            }
            true
        }
        Syntax::Atom(pt) => matches!(s, Syntax::Atom(st) if atom_eq(&pt.tok, &st.tok)),
        Syntax::Brace(ps, _) => matches!(s, Syntax::Brace(ss, _) if match_list(ps, ss, b)),
        Syntax::Bracket(ps, _, _) => matches!(s, Syntax::Bracket(ss, _, _) if match_list(ps, ss, b)),
    }
}

fn pattern_vars(p: &Syntax, out: &mut Vec<String>) {
    match p {
        Syntax::Atom(Token { tok: Tok::Ident(parts), .. })
            if parts.len() == 1 && !matches!(parts[0].as_str(), "_" | "..." | "true" | "false") =>
        {
            out.push(parts[0].clone())
        }
        Syntax::Brace(xs, _) | Syntax::Bracket(xs, _, _) => xs.iter().for_each(|x| pattern_vars(x, out)),
        _ => {}
    }
}

/// Instantiate a template: substitute pattern variables, and mark every identifier the template introduces
/// with `m` (`call` is the use site's span, recorded for diagnostics).
pub fn instantiate(t: &Syntax, b: &Bindings, m: u32) -> Result<Syntax, String> {
    match t {
        Syntax::Atom(tok) => match &tok.tok {
            Tok::Ident(parts) if parts.len() == 1 && b.contains_key(&parts[0]) => match &b[&parts[0]] {
                MatchBinding::One(s) => Ok(s.clone()),
                MatchBinding::Many(_) => Err(format!(
                    "pattern variable `{}` matched a sequence; follow it with `...` (to repeat several elements together, group them: `{{... a b}} ...`)",
                    parts[0]
                )),
            },
            Tok::Ident(parts) => {
                let mut parts = parts.clone();
                parts[0] = mark(&parts[0], m);
                Ok(Syntax::Atom(Token { tok: Tok::Ident(parts), span: tok.span }))
            }
            Tok::Macro(parts) => {
                let mut parts = parts.clone();
                parts[0] = mark(&parts[0], m);
                Ok(Syntax::Atom(Token { tok: Tok::Macro(parts), span: tok.span }))
            }
            Tok::Str(l) if l.mark.is_none() => Ok(Syntax::Atom(Token { tok: Tok::Str(Lit { mark: Some(m), ..l.clone() }), span: tok.span })),
            Tok::Template(l) if l.mark.is_none() => Ok(Syntax::Atom(Token { tok: Tok::Template(Lit { mark: Some(m), ..l.clone() }), span: tok.span })),
            _ => Ok(t.clone()),
        },
        Syntax::Brace(..) if splice_group(t).is_some() => Err("a splice group `{... }` must be an element of a list".into()),
        Syntax::Brace(xs, s) => Ok(Syntax::Brace(instantiate_list(xs, b, m)?, *s)),
        Syntax::Bracket(xs, s, d) => Ok(Syntax::Bracket(instantiate_list(xs, b, m)?, *s, d.clone())),
    }
}

/// The elements of a template splice group `{... t1 ... tn}`, which instantiates in place as `t1 ... tn`.
fn splice_group(t: &Syntax) -> Option<&[Syntax]> {
    match t {
        Syntax::Brace(xs, _) if xs.first().is_some_and(|h| h.is_ellipsis()) => Some(&xs[1..]),
        _ => None,
    }
}

/// Instantiate one template element into `out`: a splice group adds its elements, anything else adds itself.
fn instantiate_into(t: &Syntax, b: &Bindings, m: u32, out: &mut Vec<Syntax>) -> Result<(), String> {
    match splice_group(t) {
        Some(xs) => out.extend(instantiate_list(xs, b, m)?),
        None => out.push(instantiate(t, b, m)?),
    }
    Ok(())
}

fn instantiate_list(xs: &[Syntax], b: &Bindings, m: u32) -> Result<Vec<Syntax>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < xs.len() {
        if i + 1 < xs.len() && xs[i + 1].is_ellipsis() {
            let mut vars = Vec::new();
            pattern_vars(&xs[i], &mut vars);
            let seq: Vec<&String> = vars.iter().filter(|v| matches!(b.get(*v), Some(MatchBinding::Many(_)))).collect();
            if seq.is_empty() {
                return Err("`...` in a template must follow an element containing a repeated pattern variable".into());
            }
            let len = match &b[seq[0]] {
                MatchBinding::Many(v) => v.len(),
                _ => unreachable!(),
            };
            for v in &seq {
                if let MatchBinding::Many(items) = &b[*v] {
                    if items.len() != len {
                        return Err("repeated pattern variables in one `...` template have different lengths".into());
                    }
                }
            }
            for j in 0..len {
                let mut inner = b.clone();
                for v in &seq {
                    if let MatchBinding::Many(items) = &b[*v] {
                        inner.insert((*v).clone(), items[j].clone());
                    }
                }
                instantiate_into(&xs[i], &inner, m, &mut out)?;
            }
            i += 2;
        } else {
            instantiate_into(&xs[i], b, m, &mut out)?;
            i += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> DResult<Item> {
        let kind = if s.starts_with('{') { RegionKind::Brace } else { RegionKind::Bracket };
        parse_region(0, kind, s, 0)
    }

    fn trees(s: &str) -> Vec<Syntax> {
        read(&lex(0, s, 0).unwrap(), Span::default()).unwrap()
    }

    #[test]
    fn parses_forms() {
        assert!(matches!(parse("{+ 1 2}").unwrap(), Item::Expr(Expr::Call(..))));
        assert!(matches!(parse("{^ {x} [# y x] y}").unwrap(), Item::Expr(Expr::Lambda(..))));
        assert!(matches!(parse("[# x {f}]").unwrap(), Item::Decls(ds) if matches!(ds.as_slice(), [Decl::Def { .. }])));
        assert!(matches!(parse("[Import* std]").unwrap(), Item::Decls(ds) if matches!(ds.as_slice(), [Decl::Import { kind: ImportKind::Wildcard, .. }])));
        assert!(matches!(parse("{std::+ 1 -8}").unwrap(), Item::Expr(Expr::Call(..))));
        assert!(matches!(parse("{- 8}").unwrap(), Item::Expr(Expr::Call(..))));
        assert!(matches!(parse("{!if c {f} 2}").unwrap(), Item::Expr(Expr::Macro(..))));
        assert!(matches!(parse("{!= 1 2}").unwrap(), Item::Expr(Expr::Call(..))));
        assert!(matches!(parse("[Macro !m {{a ...} {f a ...}}]").unwrap(), Item::Decls(ds) if matches!(ds.as_slice(), [Decl::Macro { .. }])));
        assert!(matches!(parse("{' 1 2}").unwrap(), Item::Expr(Expr::List(..))));
        assert!(matches!(parse("{. x 1 y 2}").unwrap(), Item::Expr(Expr::Record(..))));
        assert!(matches!(parse("{^ go {n} {go n}}").unwrap(), Item::Expr(Expr::Lambda(Some(_), ..))));
        assert!(matches!(parse("{.x r}").unwrap(), Item::Expr(Expr::Call(..))));
        assert!(matches!(parse("{map rs .x=}").unwrap(), Item::Expr(Expr::Call(..))));
        assert!(matches!(parse("{#some 1}").unwrap(), Item::Expr(Expr::Call(..))));
        assert!(matches!(parse("[# a 1 b 2]").unwrap(), Item::Decls(ds) if ds.len() == 2));
        assert!(matches!(parse("[# {f x} [# y x] y]").unwrap(),
            Item::Decls(ds) if matches!(ds.as_slice(), [Decl::Def { init: Expr::Lambda(Some(_), ps, b, _), .. }] if ps.len() == 1 && b.decls.len() == 1)));
        assert!(matches!(parse("[# {f} 1 g {f}]").unwrap(), Item::Decls(ds) if ds.len() == 2));
    }

    #[test]
    fn rejects_bad_forms() {
        assert!(parse("{}").is_err());
        assert!(parse("{f [# x 1]}").is_err());
        assert!(parse("{^ {x} 1 2}").is_err());
        assert!(parse("{^ {x} 1 [# y 2]}").is_err());
        assert!(parse("[Foo 1]").is_err());
        assert!(parse("{f \"a\"b}").is_err());
        assert!(parse("{f !if}").is_err());
        assert!(parse("[Macro !m {{a a} a}]").is_err());
        assert!(parse("[Macro !m {{... a} a}]").is_err());
        assert!(parse("{. x}").is_err());
        assert!(parse("{. x 1 x 2}").is_err());
        assert!(parse("{^ f {f} 1}").is_err());
        assert!(parse("[# ' 1]").is_err());
        assert!(parse("[#]").is_err());
        assert!(parse("[# a 1 b]").is_err());
        assert!(parse("[# {f x}]").is_err());
        assert!(parse("[# {f x} [# y 1]]").is_err());
        assert!(parse("[# {} 1]").is_err());
        assert!(parse("[# {f f} 1]").is_err());
    }

    #[test]
    fn lexes_numbers_and_strings() {
        let t = lex(0, "-8 1.5 2e3 \"a\\\"b\\u{41}\" x::y !a::b ...", 0).unwrap();
        assert_eq!(t[0].tok, Tok::Int(-8));
        assert_eq!(t[1].tok, Tok::Float(1.5));
        assert_eq!(t[2].tok, Tok::Float(2000.0));
        assert!(matches!(&t[3].tok, Tok::Str(l) if l.value() == Some("a\"bA") && l.raw == "a\\\"b\\u{41}"));
        assert_eq!(t[4].tok, Tok::Ident(vec!["x".into(), "y".into()]));
        assert_eq!(t[5].tok, Tok::Macro(vec!["a".into(), "b".into()]));
        assert_eq!(t[6].tok, Tok::Ident(vec!["...".into()]));
    }

    #[test]
    fn method_call_sugar_reads_as_calls() {
        // `r.f` is `{f r}`; at a brace head the arguments follow the receiver; chains nest.
        fn tree(s: &Syntax) -> String {
            match s {
                Syntax::Atom(Token { tok: Tok::Str(l), .. }) => format!("{:?}", l.value()),
                Syntax::Atom(t) => format!("{:?}", t.tok),
                Syntax::Brace(xs, _) => format!("{{{}}}", xs.iter().map(tree).collect::<Vec<_>>().join(" ")),
                Syntax::Bracket(xs, _, _) => format!("[{}]", xs.iter().map(tree).collect::<Vec<_>>().join(" ")),
            }
        }
        let shape = |s: &str| trees(s).iter().map(tree).collect::<Vec<_>>().join(" ");
        assert_eq!(shape("{x.f a b}"), shape("{f x a b}"));
        assert_eq!(shape("{x.f.g a}"), shape("{g {f x} a}"));
        assert_eq!(shape("{h x.f}"), shape("{h {f x}}"));
        assert_eq!(shape("{{f x}.g}"), shape("{g {f x}}"));
        assert_eq!(shape("{\"s\".len}"), shape("{len \"s\"}"));
        assert_eq!(shape("{x.m::f 1}"), shape("{m::f x 1}"));
        // A dot after whitespace or an opener is a getter, not a call.
        assert!(matches!(&lex(0, "{x .f}", 0).unwrap()[2].tok, Tok::Label(LabelKind::Get, _)));
        assert!(matches!(&lex(0, "{.f x}", 0).unwrap()[1].tok, Tok::Label(LabelKind::Get, _)));
        assert!(matches!(&lex(0, "x.f", 0).unwrap()[1].tok, Tok::Method(_)));
        assert!(lex(0, "x.+", 0).is_err());
    }

    #[test]
    fn lexes_semicolon_comments() {
        let source = "1;{raw ;; [\"}\"]}2;[ignored]3;; tail } \"\r\n4";
        let tokens = lex(0, source, 0).unwrap();
        assert_eq!(
            tokens.iter().map(|t| t.tok.clone()).collect::<Vec<_>>(),
            vec![
                Tok::Int(1),
                Tok::Comment("raw ;; [\"}\"]".into()),
                Tok::Int(2),
                Tok::Comment("ignored".into()),
                Tok::Int(3),
                Tok::LineComment("tail } \"".into()),
                Tok::Int(4),
            ]
        );
        let tokens = lex(0, "\";{ ;; ;[ $;{\"", 0).unwrap();
        assert!(matches!(&tokens[0].tok, Tok::Str(l) if l.value() == Some(";{ ;; ;[ $;{")));
        let tokens = lex(0, ";;; extra semicolon", 0).unwrap();
        assert_eq!(tokens[0].tok, Tok::LineComment("; extra semicolon".into()));
    }

    #[test]
    fn rejects_single_semicolon_comments() {
        for source in ["; old comment", "{+ 1 2; old comment}"] {
            let err = lex(0, source, 0).unwrap_err();
            assert_eq!(err.message, "a comment must start with `;;`, `;{`, or `;[`");
        }
    }

    #[test]
    fn line_comments_become_docs() {
        let t = trees("{^ {x}\n ;; Doubles x.\n ;; Twice.\n [# d {* 2 x}] ;; trailing\n d}");
        match &t[0] {
            Syntax::Brace(items, _) => match &items[2] {
                Syntax::Bracket(_, _, doc) => assert_eq!(doc.as_deref(), Some("Doubles x.\nTwice.")),
                other => panic!("{:?}", other),
            },
            other => panic!("{:?}", other),
        }
        // A form between two comment lines separates them.
        let t = trees(";; first\n1\n;; second\n[# y 2]");
        assert!(matches!(&t[1], Syntax::Bracket(_, _, Some(d)) if d == "second"));
    }

    #[test]
    fn matches_and_instantiates_with_ellipsis() {
        let pat = trees("first rest ...");
        let ops = trees("a {b c} d");
        let b = match_clause(&pat, &ops).unwrap();
        let tpl = &trees("{g first {h rest} ...}")[0];
        let out = instantiate(tpl, &b, 7).unwrap();
        let shown = format!("{:?}", out);
        assert!(shown.contains("g\\u{1}7") && shown.contains("h\\u{1}7"));
        assert!(match_clause(&trees("x y"), &trees("1")).is_none());
        assert!(match_clause(&trees("x \"lit\""), &trees("1 \"lit\"")).is_some());
        assert!(match_clause(&trees("x \"lit\""), &trees("1 \"other\"")).is_none());
    }

    #[test]
    fn splice_groups_repeat_several_elements() {
        let show = |s: &Syntax| format!("{:?}", s);
        let b = match_clause(&trees("{k v} ..."), &trees("{a 1} {b 2}")).unwrap();
        let out = instantiate(&trees("{. {... k {f v}} ...}")[0], &b, 3).unwrap();
        let Syntax::Brace(xs, _) = &out else { panic!() };
        assert_eq!(xs.len(), 5, "{}", show(&out));
        // Without `...` a group splices once; alone, it is an error.
        let b = match_clause(&trees("x y"), &trees("1 2")).unwrap();
        let Syntax::Brace(xs, _) = instantiate(&trees("{f {... x y} 3}")[0], &b, 3).unwrap() else { panic!() };
        assert_eq!(xs.len(), 4);
        assert!(instantiate(&trees("{... x y}")[0], &b, 3).is_err());
        assert!(parse("[Macro !m {{{... a b}} 1}]").is_err());
    }
}
