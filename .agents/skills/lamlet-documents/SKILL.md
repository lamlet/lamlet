---
name: lamlet-documents
description: >-
  Write and build Lamlet documents: prose with computed regions, turned into text, Markdown, HTML, or the document
  IR (JSON) by `lamlet FILE` and `lamlet build` (this repository's crates/lamlet-doc). Use this skill whenever a
  .lam file is meant to be read as a page (headings, tables, lists, links, formulas, code blocks), when you use
  the `content` package (`emph`, `table`, `codeBlock`, `span`, `say`, `anchor`, ...), choose between copied and
  CommonMark prose, read or produce the IR, or work on lamlet-doc's lowering and backends. For the language
  itself (syntax, types, std), use the `lamlet` skill as well.
---

# Lamlet documents

A Lamlet file is a document: prose with `$`-regions. Lamlet evaluates the regions; **lamlet-doc** turns the
prose and the values into a format-neutral IR, and a backend writes the IR as text, Markdown, HTML, or JSON.
The language is the `lamlet` skill's subject; this skill is about the page. The normative guide is
`docs/documents.md`.

**Verify every document you write** by building it (§6). `references/example.lam` is a complete document, and
`references/example.md` is exactly what it builds to; a test (`crates/lamlet-doc/tests/backends.rs`,
`skill_example_builds_as_shown`) keeps the two equal.

## 1. What `#` means here

`#` appears in three unrelated roles, and documents use all three:

| Written | Where | Meaning |
|---|---|---|
| `$[# total 41]`, `$[# {row r} ...]` | a bracket region | a Lamlet declaration (a binding or a function definition); its line disappears from the output |
| `{#some 3}`, `#none` | inside code | a variant constructor (a tagged value) |
| `# The house {#house}` | prose, at the start of a line | a Markdown heading, level 1, with the id `house`; `##` is level 2 |

The heading form is prose, not Lamlet: it means a heading only when prose is parsed as CommonMark (§3).

## 2. The pipeline

| Stage | Where | What it does |
|---|---|---|
| Frontend | `crates/lamlet` | Parses, type-checks, and evaluates the regions |
| Lowering | `lamlet_doc::lower`, `lamlet_doc::prose` | Turns the prose and the region values into the IR |
| Backends | `lamlet_doc::backend::{text, markdown, html, json}` | Each writes the IR in one format |

Lowering only reads values and backends only read the IR, so a document gives the same output every time.

How a region's value becomes output:

| Value | Output |
|---|---|
| String, Int, Float, Bool | its text, escaped for the format (`<` is `&lt;` in HTML, `*` is `\*` in Markdown) |
| Unit, `empty` | nothing |
| `Content` (the `content` package, §4) | structure: emphasis, links, tables, ... |
| anything else (a list, a record, a variant, a function) | its written form as code: `` `[1, 2]` ``, `` `#some(3)` `` |

To show your own type well, convert it: `$[# {showPoint p} f"{(${.x p}, ${.y p})}"]`.

Text with values in it is a template region, `$f"{...}"`: `She said $f"{"hi", I am $who}" today.` Its text
is copied as written and escaped like any String (it is a String). `${f"{...}"}` is wrong: `${...}` holds a
form, and the form `{x}` calls `x`.

A line that held only declarations (and spaces) is removed, so the imports and bindings at the top leave no
blank lines.

## 3. Prose: copied or parsed

| Mode | Flag | Prose is | Use it when |
|---|---|---|---|
| Copied (the default) | `--prose md`, `--prose html`, `--prose text` (named explicitly) | markup of the output format, written by you, passed through unchanged | the document targets one format |
| Parsed | `--prose commonmark` | CommonMark, read into the IR | one source builds to every format |

`lamlet FILE` prints with copied text prose, so `# The house {#house}` stays as written there; build with
`--prose commonmark` to make it a heading.

The parsed CommonMark subset:

- ATX headings with optional attributes: `## Title {#id key=value}`. Only an explicit `{#id}` gives a heading an
  id (`<h2 id="...">`); lamlet-doc makes none from the text.
- Paragraphs, bullet and numbered lists (nested by indentation), GitHub tables.
- Emphasis, strong emphasis, code spans, links, images, autolinks, backslash escapes, hard line breaks.
- Not parsed, written as text: block quotes, thematic breaks, setext headings, HTML, reference links, indented
  code blocks.

Rules for regions in parsed prose:

- **A region alone on its lines whose value is a block** (a table, a list, a code block, a heading, a group)
  becomes that block.
- **Any other region is inline,** part of the text around it. A block-valued region inside a line is an error:
  `a region whose content is a block stands inside a line; put it on lines of its own`.
- **No code fences.** ```` ``` ```` is Markdown, not Lamlet, and parsed prose rejects a fence line at its line.
  Write `${codeBlock "toml" "..."}`. (In copied prose a fence is your markup and passes through.)

Escaping: text from a region is escaped; prose never is, since you wrote it in the format. Markup from a region
on purpose is `${raw "html" "<kbd>K</kbd>"}`: the HTML backend copies it, and the others refuse it with the
region's position (`this is markup for `html`, which cannot be written as Markdown`).

## 4. The `content` package

`$[Import* std]$[Import* content]` at the top. It is always available to the `lamlet` command; a host adds
`lamlet_doc::CONTENT_SOURCE` (or uses `lamlet_doc::resolver()`). Every export with its type is in
`references/content.md` (generated). In short (`s` is a String, `c` is Content):

| Inline | Block |
|---|---|
| `text s` | `paragraph c` |
| `emph s`, `emphOf c`, `strong s`, `strongOf c` | `heading level s`, `headingId level id s`, `headingOf level id c` |
| `link label href`, `linkOf c href` | `bullets {' c ...}`, `numbered {' c ...}` |
| `image src alt`, `imageTitled src alt title` | `bulletsOf {' s ...}`, `numberedOf {' s ...}` |
| `code s`, `math tex`, `displayMath tex` | `table header rows` (Strings), `tableOf header rows` (Content) |
| `lineBreak`, `raw format s` | `codeBlock lang s`, `rawBlock format s` |
| `span name attrs c` | `group name attrs c` |

- **The `…Of` form takes Content; the plain one a String** (the naming convention for two forms of one
  function).
- **Joining:** `{cat {' c1 c2 ...}}`; nothing is `empty`. Content is a list, so `flatten`, `map`, and `fold` work
  on it too.
- **Attributes:** `{attr "key" "value"}`, in a list, for `span`, `group`, and `anchor`. Values are Strings:
  convert numbers with `str`.
- **`span` and `group` are for hosts:** HTML writes the name as `class` and the attributes as `data-`
  attributes; the other backends write only the content.
- **Content is flat events** (`#open`, `#close`, `#text`, `#leaf`), because List is Lamlet's only recursive type.
  lamlet-doc builds the tree and reports at the region's line: an element never closed, a block inside an inline
  (`strong emphasis holds only inline content`), a table row with the wrong number of cells
  (`a table row has 1 cells but the header has 2`).

### Patterns

```text
$[Import* std]$[Import* content]
$[# rooms [% {' {. name "study" area 12} {. name "hall" area 20}}]]
$[# {row r} {' {.name r} f"{${.area r} m²}"}]

${table {' "Room" "Area"} {map rooms row}}

${bulletsOf {map rooms .name}}

${numbered {map rooms {^ {r} {cat {' {strong {.name r}} {text f"{: ${.area r} m²}"}}}}}}

The study is ${!if {> 12 10} {strong "large"} {text "small"}}.
```

- A table or list from data: `map` the records to rows or items.
- Both branches of `!if` must be the same type, so wrap a String as `{text s}` next to `{strong s}`.
- A helper that returns Content is an ordinary function; put shared helpers in a package (`--package NAME=PATH`).

## 5. Speech nodes

Three kinds of content describe speech. Lamlet's own backends write what is shown and drop the rest; a speech host
(the extension `lamlet-present`) uses them fully.

| Content | Lamlet's backends | A speech host |
|---|---|---|
| `say shown spoken` | write `shown` | speaks `spoken`, shows `shown` |
| `anchor name attrs` | nothing | times a cue to the next spoken word |
| `pause seconds` | nothing | adds silence |

```text
The room is lit ${anchor "lamp" {' {attr "intensity" "6"}}}by one lamp${pause 0.5}.
The falloff is ${say {math "1/d^2"} "one over d squared"}.
```

A formula meant to be heard belongs in `say`, since a voice cannot read TeX. Narrated presentations (headings as
sections, paragraphs as sentences, cues, timing) are `lamlet-present`'s format, not Lamlet's: see its own
repository and `docs/documents.md` §6.

## 6. Building and checking

```sh
cargo build --release                       # the CLI is ./target/release/lamlet
lamlet page.lam                             # print it: copied text prose
lamlet build page.lam --prose commonmark -o page.md          # the format from the extension
lamlet build page.lam --prose commonmark --to html --standalone > page.html
lamlet build page.lam --to ir               # the IR as JSON: {"lamletDoc": 1, "blocks": [...]}
lamlet build page.lam --to ir --prepared    # without evaluating: runtime regions are {"type": "hole", "region": N}
lamlet check page.lam                       # types only, no output
```

| `--to` | `-o` extension | Output |
|---|---|---|
| `text` | `.txt` | plain text (the default) |
| `md` | `.md` | CommonMark with GitHub tables |
| `html` | `.html` | a fragment, or a page with `--standalone` |
| `ir` | `.json` | the IR |

Workflow:

1. Build to the target format and read the output, not just the exit code.
2. On a diagnostic, the location is the region's (or the prose line's): `file:line:col: message`.
3. For a value whose type you doubt, `lamlet eval page.lam` lists each region's type; Content shows as a long
   `List(<close: …>)`, which is expected.
4. `--prepared` shows which regions are computed during the check: `$name` of a static binding and `$[% e]`
   are; any `${...}` call is a hole.

## 7. From Rust, and working on lamlet-doc

```rust
use lamlet::embed::*;
use lamlet_doc::backend::markdown;

let mut engine = Engine::new();
engine.set_resolver(lamlet_doc::resolver());                    // std and content
let doc = engine.check(SourceFile::new("page.lam", source))?;
let mut instance = engine.instance(&doc)?;
let ir = lamlet_doc::evaluate_and_lower(&doc, &mut instance, lamlet_doc::COMMONMARK)?;  // or lamlet_doc::TEXT, ...
let md = markdown::write(&ir)?;
```

A host with a format of its own walks `lamlet_doc::Doc` (or reads the JSON) and writes it, as the backends in
`crates/lamlet-doc/src/backend/` do. The `lamlet-embedding` skill covers the engine and natives.

Changing lamlet-doc:

| Path | Holds |
|---|---|
| `crates/lamlet-doc/src/content.lam` | the `content` package (Lamlet source; doc paragraphs feed `references/content.md`) |
| `crates/lamlet-doc/src/content.rs` | events to tree, with the nesting checks |
| `crates/lamlet-doc/src/ir.rs` | `Doc`, `Block`, `Inline` |
| `crates/lamlet-doc/src/lower.rs`, `prose.rs` | lowering, and the CommonMark subset |
| `crates/lamlet-doc/src/backend/` | text, Markdown, HTML, JSON |
| `crates/lamlet-doc/tests/backends.rs` | each node in each backend; `tests/golden/everything.*` (accept a deliberate change with `UPDATE_GOLDEN=1` and review the diff) |
| `crates/lamlet-doc/tests/identical.rs` | `lamlet FILE` output unchanged on every fixture and example |

After changing `content.lam`, regenerate the reference with
`python3 .agents/skills/lamlet/scripts/gen_std_reference.py` (it needs the release CLI built), and run
`cargo test -p lamlet-doc`.

## 8. Not available

Don't write these expecting them to work: a LaTeX backend; block quotes, thematic breaks, setext headings,
HTML, reference links, or indented code in parsed prose (they stay text); code fences in parsed prose (an
error); the `--to speech` backend (it moved to `lamlet-present`).
