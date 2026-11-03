//! The backends, the `content` package, and parsed prose.
//!
//! `tests/golden/everything.lam` uses every node; its output in each backend is kept in `tests/golden/`. To
//! accept a deliberate change, run with `UPDATE_GOLDEN=1` and review the diff.

use lamlet::embed::*;
use lamlet_doc::backend::{html, json, markdown, text};
use lamlet_doc::{Doc, Error, COMMONMARK, TEXT};
use std::path::Path;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.set_resolver(lamlet_doc::resolver());
    e
}

fn lower(name: &str, source: &str, prose: &str) -> Result<Doc, Error> {
    let e = engine();
    let doc = e.check(SourceFile::new(name, source))?;
    let mut instance = e.instance(&doc)?;
    instance.set_time(0);
    lamlet_doc::evaluate_and_lower(&doc, &mut instance, prose)
}

const HEAD: &str = "$[Import* std]$[Import* content]";

/// A snippet with the imports, in prose `prose`, through `backend`.
fn run(snippet: &str, prose: &str, backend: fn(&Doc) -> Result<String, Error>) -> Result<String, String> {
    let ir = lower("t.lam", &format!("{HEAD}{snippet}"), prose).map_err(|e| e.render())?;
    backend(&ir).map_err(|e| e.render())
}

fn ok(snippet: &str, prose: &str, backend: fn(&Doc) -> Result<String, Error>) -> String {
    run(snippet, prose, backend).unwrap_or_else(|e| panic!("{snippet}: {e}"))
}

#[test]
fn golden_everything() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let source = std::fs::read_to_string(dir.join("everything.lam")).unwrap();
    let ir = lower("everything.lam", &source, COMMONMARK).unwrap_or_else(|e| panic!("{e}"));
    let outputs: [(&str, Result<String, Error>); 4] = [
        ("txt", text::write(&ir)),
        ("md", markdown::write(&ir)),
        ("html", html::write(&ir)),
        ("ir.json", json::write(&ir)),
    ];
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    for (ext, out) in outputs {
        let out = out.unwrap_or_else(|e| panic!("{ext}: {e}"));
        let path = dir.join(format!("everything.{ext}"));
        if update {
            std::fs::write(&path, &out).unwrap();
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} is missing: run with UPDATE_GOLDEN=1", path.display()));
        assert_eq!(out, expected, "everything.{ext} differs; run with UPDATE_GOLDEN=1 to accept");
    }
}

#[test]
fn each_inline_in_each_backend() {
    // (snippet, text, markdown, html)
    let cases = [
        ("${emph \"a\"}", "a", "*a*", "<em>a</em>"),
        ("${strong \"a\"}", "a", "**a**", "<strong>a</strong>"),
        ("${link \"site\" \"https://x.org\"}", "site (https://x.org)", "[site](https://x.org)", "<a href=\"https://x.org\">site</a>"),
        ("${link \"https://x.org\" \"https://x.org\"}", "https://x.org", "[https://x.org](https://x.org)", "<a href=\"https://x.org\">https://x.org</a>"),
        ("${image \"a.png\" \"A\"}", "A", "![A](a.png)", "<img src=\"a.png\" alt=\"A\">"),
        ("${code \"a`b\"}", "a`b", "``a`b``", "<code>a`b</code>"),
        ("${code \"`a\"}", "`a", "`` `a ``", "<code>`a</code>"),
        ("${math \"x^2\"}", "x^2", "$x^2$", "<span class=\"math inline\">\\(x^2\\)</span>"),
        ("${say {code \"K::fold\"} \"fold\"}", "K::fold", "`K::fold`", "<code>K::fold</code>"),
        ("${anchor \"m\" {'}}${pause 1.0}", "", "", ""),
        ("${span \"cue\" {' {attr \"k\" \"v\"}} {text \"s\"}}", "s", "s", "<span class=\"cue\" data-k=\"v\">s</span>"),
        ("${text \"a<b>&c\"}", "a<b>&c", "a\\<b\\>\\&c", "a&lt;b&gt;&amp;c"),
        ("${' 1 2}", "[1, 2]", "`[1, 2]`", "<code>[1, 2]</code>"),
        ("$map", "<function map>", "`<function map>`", "<code>&lt;function map&gt;</code>"),
        ("$empty", "", "", ""),
    ];
    for (snippet, t, m, h) in cases {
        assert_eq!(ok(snippet, TEXT, text::write), t, "text: {snippet}");
        assert_eq!(ok(snippet, "md", markdown::write), m, "md: {snippet}");
        assert_eq!(ok(snippet, "html", html::write), h, "html: {snippet}");
    }
}

#[test]
fn each_block_in_each_backend() {
    let cases = [
        ("${heading 2 \"T\"}", "T", "## T", "<h2>T</h2>"),
        ("${headingId 1 \"t\" \"T\"}", "T", "# T", "<h1 id=\"t\">T</h1>"),
        ("${paragraph {text \"p\"}}", "p", "p", "<p>p</p>"),
        ("${bulletsOf {' \"a\" \"b\"}}", "- a\n- b", "- a\n- b", "<ul>\n<li>a</li>\n<li>b</li>\n</ul>"),
        ("${numberedOf {' \"a\"}}", "1. a", "1. a", "<ol>\n<li>a</li>\n</ol>"),
        (
            "${table {' \"k\" \"value\"} {' {' \"a|b\" \"1\"}}}",
            "k    value\n---  -----\na|b  1",
            "| k | value |\n| --- | --- |\n| a\\|b | 1 |",
            "<table>\n<thead>\n<tr><th>k</th><th>value</th></tr>\n</thead>\n<tbody>\n<tr><td>a|b</td><td>1</td></tr>\n</tbody>\n</table>",
        ),
        ("${codeBlock \"rs\" \"a < b\"}", "a < b", "```rs\na < b\n```", "<pre><code class=\"language-rs\">a &lt; b</code></pre>"),
        ("${group \"note\" {'} {paragraph {text \"g\"}}}", "g", "g", "<div class=\"note\">\n<p>g</p>\n</div>"),
        ("${cat {' {heading 1 \"A\"} {paragraph {text \"b\"}}}}", "A\n\nb", "# A\n\nb", "<h1>A</h1>\n<p>b</p>"),
    ];
    for (snippet, t, m, h) in cases {
        assert_eq!(ok(snippet, TEXT, text::write), t, "text: {snippet}");
        assert_eq!(ok(snippet, "md", markdown::write), m, "md: {snippet}");
        assert_eq!(ok(snippet, "html", html::write), h, "html: {snippet}");
    }
}

#[test]
fn markdown_escapes_what_would_start_markup_at_a_line_start() {
    // (The line of imports is removed, so each text starts the output's first line.)
    assert_eq!(ok("\n${text \"# not a heading\"}", "md", markdown::write), "\\# not a heading");
    assert_eq!(ok("\n${text \"1. not a list\"}", "md", markdown::write), "1\\. not a list");
    assert_eq!(ok("\n${text \"- not a list\"}", "md", markdown::write), "\\- not a list");
    assert_eq!(ok("\nx\n${text \"# heading\"}", "md", markdown::write), "x\n\\# heading");
    assert_eq!(ok("x ${text \"# fine\"}", "md", markdown::write), "x # fine");
}

#[test]
fn raw_markup_is_copied_by_its_backend_and_refused_by_others() {
    let s = "${raw \"html\" \"<kbd>K</kbd>\"}";
    assert_eq!(ok(s, "html", html::write), "<kbd>K</kbd>");
    let e = run(s, "md", markdown::write).unwrap_err();
    assert!(e.contains("t.lam:1:33: this is markup for `html`, which cannot be written as Markdown"), "{e}");
    assert!(run("${rawBlock \"md\" \"> q\"}", TEXT, text::write).unwrap_err().contains("markup for `md`"));
    // Raw content next to prose of the same position is not absorbed into it.
    assert_eq!(ok("a ${raw \"html\" \"<b>\"} b", "html", html::write), "a <b> b");
}

#[test]
fn malformed_content_is_an_error_at_its_region() {
    let bad = [
        ("${: {' {#open {. name \"emph\" attrs {'}}}} Content}", "`emph` is opened but never closed"),
        ("${: {' {#close \"emph\"}} Content}", "`emph` is closed but was never opened"),
        ("${: {' {#open {. name \"emph\" attrs {'}}} {#close \"strong\"}} Content}", "`strong` is closed while `emph` is open"),
        ("${emphOf {heading 1 \"x\"}}", "emphasis holds only inline content"),
        ("${tableOf {' {text \"a\"}} {' {' {text \"1\"} {text \"2\"}}}}", "a table row has 2 cells but the header has 1"),
        ("${heading 9 \"x\"}", "a heading's level is 1 to 6, not 9"),
        ("${: {' {#leaf {. name \"blink\" attrs {'}}}} Content}", "`blink` is not a content element"),
        ("${cat {' {_wrapUnknown}}}", "unknown name"),
    ];
    for (snippet, message) in bad {
        let e = run(snippet, TEXT, text::write).unwrap_err();
        assert!(e.contains(message), "{snippet}: {e}");
        if !message.contains("unknown") {
            assert!(e.starts_with("t.lam:1:33:"), "{snippet}: {e}");
        }
    }
}

#[test]
fn prepared_ir_has_holes_for_runtime_regions() {
    let e = engine();
    let doc = e.check(SourceFile::new("p.lam", format!("{HEAD}$[# n 3]static $n, runtime ${{+ n 1}}"))).unwrap();
    let ir = lamlet_doc::lower_prepared(&doc, TEXT).unwrap();
    let j = json::write(&ir).unwrap();
    assert!(j.contains(r#"{"type":"text","text":"3""#), "{j}");
    assert!(j.contains(r#"{"type":"hole","region":"#), "{j}");
    let err = text::write(&ir).unwrap_err().render();
    assert!(err.contains("is evaluated at runtime and has no value yet"), "{err}");
}

#[test]
fn parsed_prose_puts_regions_where_they_stand() {
    let md = |s: &str| ok(s, COMMONMARK, markdown::write);
    let h = |s: &str| ok(s, COMMONMARK, html::write);
    assert_eq!(h("\n# Title {#t level=2}\n\nSome *em* and **${emph \"x\"}**."), "<h1 id=\"t\">Title</h1>\n<p>Some <em>em</em> and <strong><em>x</em></strong>.</p>");
    // A block region alone on its line is a block; inside a line it is an error.
    assert_eq!(h("\ntext\n\n${bulletsOf {' \"a\"}}\n\nmore"), "<p>text</p>\n<ul>\n<li>a</li>\n</ul>\n<p>more</p>");
    let e = run("\nsee ${bulletsOf {' \"a\"}} here", COMMONMARK, html::write).unwrap_err();
    assert!(e.contains("stands inside a line"), "{e}");
    // A region inside code is its text; markup in a region's string is text, not markup.
    assert_eq!(h("\n`n = ${+ 1 2}` and ${text \"*not em*\"}"), "<p><code>n = 3</code> and *not em*</p>");
    assert_eq!(md("\n- a\n- b\n  - c\n\n1. x\n2. y"), "- a\n- b\n\n  - c\n\n1. x\n2. y");
    // Code blocks are `codeBlock` regions; a fence is refused at its own line.
    assert_eq!(h("\n${codeBlock \"\" \"*x*\"}"), "<pre><code>*x*</code></pre>");
    let e = run("\nintro\n\n```\n*x*\n```", COMMONMARK, html::write).unwrap_err();
    assert!(e.starts_with("t.lam:4:1: a fenced code block (``` or ~~~) is not Lamlet syntax"), "{e}");
    assert_eq!(h("\n| a | b |\n|---|:-:|\n| 1 | `\\|` |"), "<table>\n<thead>\n<tr><th>a</th><th>b</th></tr>\n</thead>\n<tbody>\n<tr><td>1</td><td><code>|</code></td></tr>\n</tbody>\n</table>");
    assert_eq!(h("\n[a *b*](u) ![i](s.png \"t\") <https://x.y> \\*lit\\*"), "<p><a href=\"u\">a <em>b</em></a> <img src=\"s.png\" alt=\"i\" title=\"t\"> <a href=\"https://x.y\">https://x.y</a> *lit*</p>");
    assert_eq!(h("\nsnake_case_name and 2*3*4"), "<p>snake_case_name and 2<em>3</em>4</p>");
}

#[test]
fn html_page_and_title() {
    let ir = lower("t.lam", &format!("{HEAD}\n# The <Title>\n\nBody."), COMMONMARK).unwrap();
    let title = html::first_heading(&ir).unwrap();
    assert_eq!(title, "The <Title>");
    let page = html::standalone(&html::write(&ir).unwrap(), &title);
    assert!(page.starts_with("<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">"), "{page}");
    assert!(page.contains("<title>The &lt;Title&gt;</title>"), "{page}");
    assert!(page.ends_with("<p>Body.</p>\n</body>\n</html>\n"), "{page}");
}

#[test]
fn json_escapes_and_encodes_data() {
    assert_eq!(json::string("a\"b\\c\n\u{1}"), r#""a\"b\\c\n\u0001""#);
    let j = ok("${. a {some 1.0} b {'}}", TEXT, json::write);
    assert!(j.contains(r#""data":{"a":{"tag":"some","value":1.0},"b":[]}"#), "{j}");
    assert!(j.starts_with(r#"{"lamletDoc":1,"blocks":["#), "{j}");
}
