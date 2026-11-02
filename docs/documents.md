# Building documents

A Lamlet file is a document: text with computed regions. `lamlet FILE` runs it and prints the result, and
`lamlet build` compiles it to Markdown, HTML, plain text, JSON, or speech. This guide covers both. The design is
in [plans/formats.md](../plans/formats.md).

## 1. The pipeline

In compiler terms:

| Stage | Where | What it does |
|---|---|---|
| Frontend | the `lamlet` crate | Parses, type-checks, and evaluates the regions |
| Lowering | `lamlet_doc::lower`, `lamlet_doc::prose` | Turns the document and its values into the document IR |
| Backends | `lamlet_doc::backend::{text, markdown, html, json, speech}` | Turn the IR into one output format each |

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
$[# showPoint {^ {p} [f "(${.x p}, ${.y p})"]}]
```

## 3. `lamlet build`

```sh
lamlet build page.lam -o page.md                         # the format from the extension
lamlet build page.lam --to html --standalone > page.html
lamlet build page.lam --to ir                            # the IR as JSON
lamlet build page.lam --to speech                        # utterances for a speech host
```

| `--to` | Extension of `-o` | Output |
|---|---|---|
| `text` | `.txt` | Plain text (the default; what `lamlet FILE` prints) |
| `md` | `.md` | CommonMark, with GitHub tables |
| `html` | `.html` | An HTML fragment, or a page with `--standalone` |
| `ir` | `.json` | The IR: `{"lamletDoc": 1, "blocks": [...]}` |
| `speech` | | Utterances: `{"lamletSpeech": 1, "utterances": [...]}` |

### Prose

The document's text is handled in one of two ways:

- **Copied (the default).** Prose is markup of the output format, written by you: Markdown in a file built to
  Markdown, HTML in a file built to HTML. `--prose md|html|text` names it explicitly.
- **Parsed (`--prose commonmark`).** Prose is read as CommonMark, so one source builds to every format. Regions
  stand where they are written: a region alone on its lines whose value is a block (a table, a list) becomes that
  block, and any other region is part of the text around it.

The CommonMark subset: ATX headings with optional attributes (`## Title {#id key=value}`), paragraphs, bullet and
numbered lists (nested by indentation), fenced code blocks, GitHub tables, emphasis, strong emphasis, code spans,
links, images, autolinks, backslash escapes, and hard line breaks. Block quotes, thematic breaks, setext
headings, HTML, reference links, and indented code blocks are written as text.

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

## 5. Speech

`--to speech` gives one utterance per paragraph, with what is spoken, what is shown, and where cues fall:

```text
$[Import* std]$[Import* content]
# Light {#light}

The room is lit ${anchor "lamp" {' {attr "intensity" "6"}}}by one lamp${pause 0.5}.

The falloff is ${say {math "1/d^2"} "one over d squared"}.
```

```json
{"section": ["light"], "spoken": "The room is lit by one lamp.",
 "pieces": ["The room is lit", "by one lamp", "."],
 "anchors": [{"piece": 1, "kind": "point", "name": "lamp", "attrs": {"intensity": "6"}},
             {"piece": 2, "kind": "pause", "seconds": 0.5}],
 "shown": "The room is lit by one lamp.", "at": {...}}
```

- **Pieces.** The spoken text is cut at every anchor, span edge, and pause. An anchor's `piece` is the piece it
  stands before, so a host times it to that piece's first word. The backend does not count words, because
  that depends on how the voice splits them.
- **`say shown spoken`** is shown one way and spoken another. A formula must be inside `say`: a voice cannot read
  TeX, and the backend reports a bare formula as an error.
- **Sections** are the ids of the enclosing headings (or their text, when they have no id).
- **Translations.** `lamlet build zh.lam --to speech --align en.lam` checks that the translation has the same
  sections, the same number of paragraphs in each, and no anchors or spans (the cues belong to the original).

## 6. The prepared IR

`lamlet build page.lam --to ir --prepared` writes the IR without evaluating the document. Regions computed during
the check (static bindings, `$[% ...]`) have their values, and the others are holes
(`{"type": "hole", "region": 3}`) for a host that evaluates them later.

## 7. From Rust

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
