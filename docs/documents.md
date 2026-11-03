# Building documents

A Lamlet file is a document: text with computed regions. `lamlet FILE` runs it and prints the result, and
`lamlet build` compiles it to Markdown, HTML, plain text, or JSON. This guide covers both.

## 1. The pipeline

In compiler terms:

| Stage | Where | What it does |
|---|---|---|
| Frontend | the `lamlet` crate | Parses, type-checks, and evaluates the regions |
| Lowering | `lamlet_doc::lower`, `lamlet_doc::prose` | Turns the document and its values into the document IR |
| Backends | `lamlet_doc::backend::{text, markdown, html, json}` | Turn the IR into one output format each |

The IR is a small tree of blocks (paragraphs, headings, lists, tables, code blocks) and inlines (text,
emphasis, links, images, code, formulas). Lowering only reads values, and backends only read the IR, so the same
document gives the same output every time. There is no optimizer between lowering and the backends.

## 2. Running a document

```text
$[Import* std]
$[# AGE 3]
Hi! I am $AGE ${!if {> AGE 1} "years" "year"} old.
```

```sh
$ lamlet test.lam
Hi! I am 3 years old.
```

Text is printed as written, each region is replaced by its value, and a line that held only declarations is
removed. Strings and numbers print as they are; other values print in Lamlet's written form
(`[1, 2]`, `{x: 1.0, y: 2.5}`, `#some(3)`). To print a value of your own type differently, write a function for it:

```text
$[# {showPoint p} [f "(${.x p}, ${.y p})"]]
```

## 3. `lamlet build`

```sh
lamlet build page.lam -o page.md                         # the format from the extension
lamlet build page.lam --to html --standalone > page.html
lamlet build page.lam --to ir                            # the IR as JSON
```

| `--to` | Extension of `-o` | Output |
|---|---|---|
| `text` | `.txt` | Plain text (the default; what `lamlet FILE` prints) |
| `md` | `.md` | CommonMark, with GitHub tables |
| `html` | `.html` | An HTML fragment, or a page with `--standalone` |
| `ir` | `.json` | The IR: `{"lamletDoc": 1, "blocks": [...]}` |

### Prose

The document's text is handled in one of two ways:

- **Copied (the default).** Prose is markup of the output format, written by you: Markdown in a file built to
  Markdown, HTML in a file built to HTML. `--prose md|html|text` names it explicitly.
- **Parsed (`--prose commonmark`).** Prose is read as CommonMark, so one source builds to every format. Regions
  stand where they are written: a region alone on its lines whose value is a block (a table, a list) becomes that
  block, and any other region is part of the text around it.

The CommonMark subset: ATX headings with optional attributes (`## Title {#id key=value}`), paragraphs, bullet and
numbered lists (nested by indentation), GitHub tables, emphasis, strong emphasis, code spans, links, images,
autolinks, backslash escapes, and hard line breaks. Block quotes, thematic breaks, setext headings, HTML,
reference links, and indented code blocks are written as text.

Code blocks are written with `codeBlock` (section 4), not with fences: ```` ``` ```` is Markdown, not Lamlet
syntax, so parsed prose reports a fence line as an error at that line. (In copied prose, a fence is part of the
markup you wrote and passes through unchanged, like the rest of it.)

### Escaping

Text from a region is escaped for the output format: `<` becomes `&lt;` in HTML, and `*` becomes `\*` in
Markdown. Prose is never escaped, because you wrote it in that format. To insert markup from a region on
purpose, use `raw`:

```text
Press ${raw "html" "<kbd>K</kbd>"}.
```

A backend copies raw markup of its own format and refuses any other, with the region's position.

## 4. Content

The `content` package gives regions structure that every backend understands. It is always available to the
`lamlet` command; hosts add `lamlet_doc::CONTENT_SOURCE` to their resolver.

```text
$[Import* std]$[Import* content]
# Rooms

The house has ${emph "two"} rooms (${link "plan" "plan.pdf"}):

${table {' "Room" "Area"} {' {' "study" "12 m²"} {' "hall" "20 m²"}}}
```

| Inline | Block |
|---|---|
| `text s` | `paragraph c` |
| `emph s`, `emphOf c` | `heading level s`, `headingId level id s`, `headingOf level id c` |
| `strong s`, `strongOf c` | `bullets {' c ...}`, `numbered {' c ...}` |
| `link label href`, `linkOf c href` | `bulletsOf {' s ...}`, `numberedOf {' s ...}` |
| `image src alt`, `imageTitled src alt title` | `table header rows` (Strings), `tableOf header rows` (content) |
| `code s` | `codeBlock lang s` |
| `math tex`, `displayMath tex` | `group name attrs c` |
| `lineBreak` | `rawBlock format s` |
| `raw format s` | |
| `span name attrs c` | |

`s` is a String and `c` is content. `cat {' c1 c2 ...}` joins content, `empty` is none, and `attr key value`
makes an attribute for `span`, `group`, and `anchor`. `span` and `group` carry names and attributes for hosts:
HTML writes them as `class` and `data-` attributes, and the other backends write only their content.

A region's value is content when its type is `Content`, which every function above returns. Content is a flat
list of events (`#open`, `#close`, `#text`, `#leaf`), because Lamlet has no recursive types except `List`;
`lamlet-doc` builds the tree and reports an element opened and never closed, a block inside emphasis, or a table
row with the wrong number of cells, at the region's line.

## 5. Speech nodes

Three kinds of content describe how a document is spoken. They are part of the document, so every backend
knows what to do with them, but only a speech host (such as the extension `lamlet-present`, section 6) uses
them fully:

| Content | IR node | Lamlet's backends | A speech host |
|---|---|---|---|
| `say shown spoken` | `Say` | write what is shown | speaks `spoken`, shows `shown` |
| `anchor name attrs` | `Anchor` | write nothing | times a cue to the next spoken word |
| `pause seconds` | `Pause` | write nothing | adds silence |

```text
$[Import* std]$[Import* content]
The room is lit ${anchor "lamp" {' {attr "intensity" "6"}}}by one lamp${pause 0.5}.
The falloff is ${say {math "1/d^2"} "one over d squared"}.
```

A formula meant to be spoken belongs inside `say`, since a voice cannot read TeX. A `span` (section 4) marks a
stretch of words the same way an anchor marks a point.

## 6. Extensions

Tools for one kind of document live outside Lamlet, as *extensions*. `lamlet NAME ARGS...` runs the program
`lamlet-NAME` with `ARGS`, when `NAME` is neither a command nor a file, as `git` and `cargo` do. The extension
gets the path of the `lamlet` program in the `LAMLET` environment variable.

An extension is a git repository with a Rust program that depends on `lamlet` and `lamlet-doc` by git tag:

```sh
cargo install --git https://github.com/lamlet/lamlet-present
lamlet present schedule deck.lam --fps 30        # runs lamlet-present schedule deck.lam --fps 30
```

There is no package registry. An extension carries the Lamlet packages it defines inside the program, and can
write them out for other tools (`lamlet present source > present.lam`, then `lamlet build deck.lam --package
present=present.lam`).

`lamlet-present` is the extension for narrated presentations: a document whose headings are sections and whose
paragraphs are sentences, timed from measured speech, with cues, animation curves, and slides. It reads the
speech nodes of section 5: `lamlet present speech` lists every sentence with its spoken text, pieces, and cues, and
checks that a translation lines up with its original (`--align`).

## 7. The prepared IR

`lamlet build page.lam --to ir --prepared` writes the IR without evaluating the document. Regions computed during
the check (static bindings, `$[% ...]`) have their values, and the others are holes
(`{"type": "hole", "region": 3}`) for a host that evaluates them later.

## 8. From Rust

```rust
use lamlet::embed::*;
use lamlet_doc::backend::markdown;

let mut engine = Engine::new();
engine.set_resolver(lamlet_doc::resolver());                    // std and content
let doc = engine.check(SourceFile::new("page.lam", source))?;
let mut instance = engine.instance(&doc)?;
let ir = lamlet_doc::evaluate_and_lower(&doc, &mut instance, lamlet_doc::COMMONMARK)?;
let md = markdown::write(&ir)?;
```

A host with a format of its own walks `lamlet_doc::Doc` (or reads the JSON form) and writes it, as the backends
in `crates/lamlet-doc/src/backend/` do.

## 9. Not yet available

- A LaTeX backend. The IR already has what it needs (formulas, code, tables); text, Markdown, and HTML came
  first to show the IR is enough.
- The rest of CommonMark in parsed prose: block quotes, thematic breaks, setext headings, HTML, reference links,
  and indented code blocks are written as text (section 3).
