//! The layout of a document's code: `lamlet fmt`.
//!
//! [`format_document`] rewrites the whitespace inside each `${...}` and `$[...]` region and nothing else: prose,
//! `$name` and `$f"{...}"` regions, disabled regions, and code fences stay as written, and every token keeps its
//! source text. An expression that fits in the width stays on one line; a longer one breaks Lisp-style, its parts at
//! an indent of [`Style::indent`] and its closing brackets at the end of its last line:
//!
//! - a list (`{' ...}`) puts one item per line, or fills its lines when every item is an atom; a record (`{. ...}`)
//!   one label and its value per line; a declaration of several bindings (`[# a 1 b 2]`) one binding per line;
//! - a call keeps its function and its leading atoms on its first line (`{cue "key"`), and a form its header
//!   (`[# {f x}`, `{^ {x}`, `{^ name {x}`, `{!if c`); when everything before the last operand is on the first line,
//!   the last one hangs from it (`[# T [% {Struct {'`);
//! - `{: e T}` keeps its type after the expression.
//!
//! Tokens written against each other (`x.f`) stay together, and a break is the document's line ending (`\r\n` in a
//! document that has one). A region with a line comment (`;; ...`) is left as it is,
//! since its line break is part of it.

use crate::source::{scan_document, DResult, RegionKind, Segment};
use crate::syntax::{self, lex, Tok, Token};

/// How [`format_document`] lays out code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    /// The column a line should not pass (in characters), where a break can keep it inside.
    pub width: usize,
    /// The indent of a broken expression's parts, relative to the line its region starts on.
    pub indent: usize,
}

impl Default for Style {
    fn default() -> Style {
        Style { width: 100, indent: 4 }
    }
}

/// Forms whose first operand stays on their first line.
const HEADED: &[&str] = &["^", "λ", "#", "%", "!if", "!case", "!let", "match"];

enum Tree {
    Atom { text: String, glued: bool },
    List { open: char, close: char, children: Vec<Tree>, glued: bool },
}

impl Tree {
    fn glued(&self) -> bool {
        match self {
            Tree::Atom { glued, .. } | Tree::List { glued, .. } => *glued,
        }
    }

    fn is_atom(&self) -> bool {
        matches!(self, Tree::Atom { .. })
    }

    /// The tree on one line.
    fn flat(&self) -> String {
        match self {
            Tree::Atom { text, .. } => text.clone(),
            Tree::List { open, close, children, .. } => {
                let mut out = open.to_string();
                for (k, c) in children.iter().enumerate() {
                    if k > 0 && !c.glued() {
                        out.push(' ');
                    }
                    out.push_str(&c.flat());
                }
                out.push(*close);
                out
            }
        }
    }
}

/// The document `text` with its code laid out. It fails where the document does not scan or a region does not lex
/// or read (nesting deeper than the reader allows included), with the diagnostics checking it would give.
pub fn format_document(text: &str, style: Style) -> DResult<String> {
    let segments = scan_document(0, text)?;
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut at = 0usize;
    for segment in &segments {
        let Segment::Region { kind: RegionKind::Brace | RegionKind::Bracket, span, inner } = segment else { continue };
        let (start, inner_start, inner_end) = (span.start as usize, inner.start as usize, inner.end as usize);
        let tokens = lex(0, &text[inner_start..inner_end], inner_start)?;
        // The reader's errors (unbalanced brackets, nesting past its depth limit) are the formatter's too.
        syntax::read(&tokens, *inner)?;
        let Some(tree) = read(text, &tokens) else { continue };
        out.push_str(&text[at..start]);
        let line_start = out.rfind('\n').map_or(0, |i| i + 1);
        let column = out[line_start..].chars().count() + 1;
        let base = leading_indent(&out[line_start..]);
        out.push('$');
        let layout = Layout { style, newline: if text.contains("\r\n") { "\r\n" } else { "\n" } };
        out.push_str(&layout.tree(&tree, column, base + style.indent, 0));
        at = span.end as usize;
    }
    out.push_str(&text[at..]);
    Ok(out)
}

fn leading_indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

/// A region's tokens as one tree; `None` when it holds a line comment (or is not one bracketed form).
fn read(text: &str, tokens: &[Token]) -> Option<Tree> {
    let mut stack: Vec<Tree> = Vec::new();
    let mut done: Option<Tree> = None;
    let mut previous_end: Option<u32> = None;
    for t in tokens {
        let glued = previous_end == Some(t.span.start)
            && matches!(stack.last(), Some(Tree::List { children, .. }) if !children.is_empty());
        previous_end = Some(t.span.end);
        let leaf = match &t.tok {
            Tok::LineComment(_) => return None,
            Tok::LBrace | Tok::LBracket => {
                let (open, close) = if t.tok == Tok::LBrace { ('{', '}') } else { ('[', ']') };
                stack.push(Tree::List { open, close, children: Vec::new(), glued });
                continue;
            }
            Tok::RBrace | Tok::RBracket => stack.pop()?,
            _ => Tree::Atom { text: text[t.span.start as usize..t.span.end as usize].to_string(), glued },
        };
        match stack.last_mut() {
            Some(Tree::List { children, .. }) => children.push(leaf),
            _ if done.is_none() => done = Some(leaf),
            _ => return None,
        }
    }
    if stack.is_empty() { done } else { None }
}

struct Layout {
    style: Style,
    /// The document's line ending.
    newline: &'static str,
}

impl Layout {
    /// `tree` written from `column`, its broken parts at column `indent`, with `tail` characters still to follow
    /// it on its last line.
    fn tree(&self, tree: &Tree, column: usize, indent: usize, tail: usize) -> String {
        let flat = tree.flat();
        let Tree::List { open, close, children, .. } = tree else { return flat };
        if column + flat.chars().count() + tail <= self.style.width {
            return flat;
        }
        let groups = groups(children);
        if groups.len() < 2 {
            return flat;
        }
        let head = group_flat(groups[0]);
        let rest = &groups[1..];
        let first = format!("{open}{head}");
        let newline = format!("{}{}", self.newline, " ".repeat(indent));
        let step = self.style.indent;
        let width = |s: &str| s.chars().count();
        let mut out = first.clone();

        if head == "'" && rest.iter().all(|g| is_atoms(g)) {
            let mut at = column + width(&first);
            for g in rest {
                let t = group_flat(g);
                if at + 1 + width(&t) + 1 > self.style.width {
                    out.push_str(&newline);
                    at = indent + width(&t);
                } else {
                    out.push(' ');
                    at += 1 + width(&t);
                }
                out.push_str(&t);
            }
            out.push(*close);
            return out;
        }
        let pairs = rest.len().is_multiple_of(2) && (head == "." || (head == "#" && rest.len() > 2 && rest.iter().step_by(2).all(|g| is_atoms(g))));
        if pairs {
            for (k, pair) in rest.chunks(2).enumerate() {
                let label = group_flat(pair[0]);
                let last = if (k + 1) * 2 == rest.len() { tail + 1 } else { 0 };
                out.push_str(&newline);
                out.push_str(&label);
                out.push(' ');
                out.push_str(&self.group(pair[1], indent + width(&label) + 1, indent + step, last));
            }
            out.push(*close);
            return out;
        }
        if head == ":" && rest.len() == 2 && width(&group_flat(rest[1])) <= self.style.width / 2 {
            let end = format!(" {}{close}", group_flat(rest[1]));
            return format!("{open}: {}{end}", self.group(rest[0], column + 3, indent, tail + width(&end)));
        }
        let header = if head == "'" || head == "." {
            0
        } else if head == "^" || head == "λ" {
            if is_atoms(rest[0]) && rest.len() > 2 { 2 } else { 1 }
        } else if HEADED.contains(&head.as_str()) {
            1
        } else {
            rest[..rest.len() - 1].iter().take_while(|g| is_atoms(g)).count()
        };
        // The last operand hangs from the first line when every operand before it is in the header.
        if head != "'" && header >= rest.len() - 1 {
            if let [last @ Tree::List { .. }] = rest[rest.len() - 1] {
                let before = rest[..rest.len() - 1].iter().fold(first.clone(), |s, g| s + " " + &group_flat(g));
                if column + width(&before) + 8 <= self.style.width {
                    let hung = self.tree(last, column + width(&before) + 1, indent, tail + 1);
                    if hung.contains('\n') {
                        return format!("{before} {hung}{close}");
                    }
                }
            }
        }
        let mut at = column + width(&first);
        for g in &rest[..header.min(rest.len())] {
            let t = self.group(g, at + 1, indent + step, 0);
            at = match t.rfind('\n') {
                Some(i) => width(&t[i + 1..]),
                None => at + 1 + width(&t),
            };
            out.push(' ');
            out.push_str(&t);
        }
        for (k, g) in rest.iter().enumerate().skip(header) {
            let last = if k == rest.len() - 1 { tail + 1 } else { 0 };
            out.push_str(&newline);
            out.push_str(&self.group(g, indent, indent + step, last));
        }
        out.push(*close);
        out
    }

    fn group(&self, group: &[Tree], column: usize, indent: usize, tail: usize) -> String {
        match group {
            [one] => self.tree(one, column, indent, tail),
            _ => group_flat(group),
        }
    }
}

/// The children, each run of glued ones kept together.
fn groups(children: &[Tree]) -> Vec<&[Tree]> {
    let mut out: Vec<&[Tree]> = Vec::new();
    let mut start = 0;
    for k in 1..=children.len() {
        if k == children.len() || !children[k].glued() {
            out.push(&children[start..k]);
            start = k;
        }
    }
    out
}

fn group_flat(group: &[Tree]) -> String {
    group.iter().map(Tree::flat).collect()
}

fn is_atoms(group: &[Tree]) -> bool {
    group.iter().all(Tree::is_atom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(text: &str) -> String {
        format_document(text, Style::default()).unwrap()
    }

    /// Every token of a document's code, and the document's text outside its code.
    fn tokens(text: &str) -> (Vec<String>, String) {
        let mut code = Vec::new();
        let mut prose = String::new();
        let mut at = 0;
        for segment in scan_document(0, text).unwrap() {
            if let Segment::Region { kind: RegionKind::Brace | RegionKind::Bracket, span, inner } = segment {
                prose.push_str(&text[at..span.start as usize]);
                at = span.end as usize;
                let inner_text = &text[inner.start as usize..inner.end as usize];
                code.extend(lex(0, inner_text, 0).unwrap().iter().map(|t| inner_text[t.span.start as usize..t.span.end as usize].to_string()));
            }
        }
        prose.push_str(&text[at..]);
        (code, prose)
    }

    #[test]
    fn short_code_stays_on_one_line() {
        assert_eq!(fmt("Prose.\n$[# {f x} {+   x\n    1}]\n"), "Prose.\n$[# {f x} {+ x 1}]\n");
    }

    #[test]
    fn a_long_list_puts_one_item_per_line_and_hangs() {
        let fields: Vec<String> = (0..6).map(|i| format!("{{Field \"field{i}\" Float}}")).collect();
        let out = fmt(&format!("$[# Thing [% {{Struct {{' {}}}}}]]\n", fields.join(" ")));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "$[# Thing [% {Struct {'");
        assert_eq!(lines[1], "    {Field \"field0\" Float}");
        assert_eq!(lines[6], "    {Field \"field5\" Float}}}]]");
    }

    #[test]
    fn a_list_of_atoms_fills_its_lines() {
        let values: Vec<String> = (0..40).map(|i| format!("{i}.0")).collect();
        let out = fmt(&format!("${{f {{' {}}}}}\n", values.join(" ")));
        assert!(out.lines().all(|l| l.chars().count() <= 100), "{out}");
        assert!(out.lines().count() < 6, "{out}");
    }

    #[test]
    fn records_and_bindings_put_one_pair_per_line() {
        let fields: Vec<String> = (0..8).map(|i| format!("key{i} {{some value{i}}}")).collect();
        let out = fmt(&format!("$[# r {{. {}}}]\n", fields.join(" ")));
        assert!(out.contains("\n    key3 {some value3}\n"), "{out}");
        let bindings: Vec<String> = (0..8).map(|i| format!("name{i} {{compute {i}}}")).collect();
        let out = fmt(&format!("$[# {}]\n", bindings.join(" ")));
        assert!(out.contains("\n    name3 {compute 3}\n"), "{out}");
    }

    #[test]
    fn a_lambda_keeps_its_parameters_and_puts_each_declaration_on_a_line() {
        let out = fmt("$[# handler {^ {click} [# size {viewport}] [# half {/ {.x size} 2.0}] {' {setX {* {.u click} half}} {setY {* {.v click} half}}}}]\n");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "$[# handler {^ {click}");
        assert_eq!(lines[1], "    [# size {viewport}]");
    }

    #[test]
    fn an_ascription_keeps_its_type_last() {
        let body: Vec<String> = (0..12).map(|i| format!("{{set{i} 0.0}}")).collect();
        let out = fmt(&format!("$[# h {{: {{^ {{t}} {{' {}}}}} {{Action Args}}}}]\n", body.join(" ")));
        assert!(out.trim_end().ends_with("}} {Action Args}}]"), "{out}");
    }

    #[test]
    fn lines_stay_inside_the_width() {
        let values: Vec<String> = (0..20).map(|i| format!("{{value{i} {i}.0}}")).collect();
        let out = fmt(&format!("${{cue \"key\" {{' {}}}}} The sentence.\n", values.join(" ")));
        assert!(out.lines().all(|l| l.chars().count() <= 100), "{out}");
        assert!(out.trim_end().ends_with("}} The sentence."), "{out}");
    }

    #[test]
    fn prose_strings_templates_and_glued_tokens_are_kept() {
        let args: Vec<String> = (0..20).map(|i| format!("arg{i}")).collect();
        let source = format!(
            "Prose with $$[Import* x], `$$y`, and $name.\n$[# s {{x.f \"a }} ] \\\" b\" f\"{{x ${{spoken {{n}}}} y}}\" ;{{ note }} {}}}]\n",
            args.join(" ")
        );
        let out = fmt(&source);
        assert_eq!(tokens(&out).0, tokens(&source).0);
        assert!(out.starts_with("Prose with $$[Import* x], `$$y`, and $name.\n$[# s {x.f \"a } ] \\\" b\""), "{out}");
        assert!(out.contains("f\"{x ${spoken {n}} y}\""), "{out}");
    }

    #[test]
    fn code_fences_disabled_regions_and_line_comments_are_left_alone() {
        let long: Vec<String> = (0..30).map(|i| format!("{{arg{i}}}")).collect();
        let long = long.join(" ");
        let source = format!("```\n${{f {long}}}\n```\n$;{{f {long}}}\n$[# x ;; why\n {{f {long}}}]\n");
        assert_eq!(fmt(&source), source);
    }

    #[test]
    fn a_region_inside_an_indented_line_breaks_from_that_indent() {
        let values: Vec<String> = (0..20).map(|i| format!("{{value{i} {i}.0}}")).collect();
        let out = fmt(&format!("  - an item ${{f {{' {}}}}}\n", values.join(" ")));
        assert!(out.lines().nth(1).unwrap().starts_with("      {value"), "{out}");
    }

    #[test]
    fn a_crlf_document_breaks_with_crlf() {
        let values: Vec<String> = (0..20).map(|i| format!("{{value{i} {i}.0}}")).collect();
        let out = fmt(&format!("Text.\r\n${{f {{' {}}}}}\r\n", values.join(" ")));
        assert!(out.contains("{'\r\n    {value0 0.0}\r\n"), "{out:?}");
        assert!(!out.replace("\r\n", "").contains('\n'), "{out:?}");
    }

    #[test]
    fn nesting_past_the_readers_limit_is_its_error() {
        let deep = format!("${{{}{}}}\n", "{id ".repeat(5000), "}".repeat(5000));
        assert!(format_document(&deep, Style::default()).is_err());
    }

    #[test]
    fn formatting_is_stable_and_keeps_every_token() {
        let std = include_str!("../std/std.lam");
        let once = fmt(std);
        assert_eq!(tokens(&once), tokens(std));
        assert_eq!(fmt(&once), once);
    }
}
