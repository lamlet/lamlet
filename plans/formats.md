# Printing values and building documents into other formats (plan)

`lamlet FILE` prints a document: its text as written, each region replaced by its value. Strings and numbers
print as they are, and everything else prints in Lamlet's written form (`{x: 1.0, y: 2.5}`). This plan adds,
in order:

1. **A document IR, and `lamlet FILE` through it.** Every output, including `lamlet FILE`, goes source → IR →
   writer. Each writer defines how every kind of IR node and value prints.
2. **Content.** Images, links, tables, and the like as Lamlet values that lower to IR nodes.
3. **`lamlet build`.** Writers turn the IR into plain text, Markdown, HTML, JSON, and later LaTeX.
4. **Speech.** IR nodes for what is shown versus spoken, anchors between words, and pauses, plus a writer that
   gives a speech host its utterances.

It names no consumer. The workloads are general: a program that prints a document, a static site or a book
built from Lamlet sources, and a host that narrates text and times visuals to the speech. The tests use
synthetic documents and hosts.

Status: phases 1 to 7 implemented (2026-11-02, unreleased), in `crates/lamlet-doc` and `lamlet build`; the
LaTeX backend (phase 5's "after these") is not. In compiler terms: the `lamlet` crate is the frontend,
`lamlet_doc::lower` and `lamlet_doc::prose` are lowering, and `lamlet_doc::backend` holds the backends. Narrated presentations, built on
the speech backend, are the extension `lamlet-present` (`lamlet present ...`; see `docs/documents.md`,
section 6). Where the implementation differs from the text below:

- The speech backend gives `shown` as plain text, not as inlines.
- Parsed prose is a CommonMark subset (`lamlet_doc::prose` lists it), not all of CommonMark, and it lives in
  `lamlet-doc` itself: it needs no dependency, so a separate crate was not worth it.
- The Markdown backend does not write heading ids, since CommonMark has no syntax for them.
- Parsed prose has no fenced code blocks: a fence is Markdown, not Lamlet syntax, so it is an error at its line,
  and code blocks are `codeBlock` regions.
- `--align ORIGINAL` is given on the translation's build: `lamlet build zh.lam --to speech --align en.lam`.

Tests: `crates/lamlet-doc/tests/identical.rs` (the text backend against the renderer it replaced, on every
fixture and example), `tests/backends.rs` (each node in each backend, golden files for a document using every
node, content errors, parsed prose, speech, alignment), and `crates/lamlet-cli/tests/cli.rs` (`lamlet build`).

## Workloads

| Workload | What it needs |
|---|---|
| Print a document | Values with their own text form; a readable fallback for the rest |
| Build Markdown, HTML, or plain text | Portable content (images, links, tables, code, emphasis), each written in the target's syntax with its escaping |
| Hand content to another tool | The IR as versioned JSON |
| Narrate a document | Which text is spoken and which is shown, where each cue sits between words, pauses, sections with ids |
| Translations | A parallel document matched section by section and paragraph by paragraph |

## Constraints

- **The `lamlet` library keeps zero dependencies** and knows no output format. The IR, the writers, and
  `build` live in a new crate, `lamlet-doc`; the CLI depends on it.
- **No recursive types except `List`** (rationale.md). A document can *build* nested content at a fixed depth,
  such as `{' {#text "a"} {#emph {' {#text "b"}}}}`. But Lamlet cannot express the type of an arbitrary-depth
  tree, so no Lamlet function can walk one. Hence two representations:
  - **In Lamlet, content is a flat list of events.** Each one is `#open`, `#close`, or a leaf. The list has an
    ordinary, ground type, so Lamlet code can produce, filter, and transform it.
  - **In Rust, the IR is a tree** built from those events, which writers walk.
- **Printing never changes evaluation.** Lowering happens after `Instance::evaluate`, on the values, and calls no
  Lamlet code.

## The pipeline

Every output goes through the IR, including `lamlet FILE`:

```text
source ──parse──▶ source IR: Text | Region | Declaration | Comment     (exists: Document::parts)
       ──check, evaluate──▶ the same, each Region holding its value     (static regions are filled by the check)
       ──lower──▶ document IR: blocks and inlines; Raw for prose         (a pure function of the values)
       ──writer──▶ text | Markdown | HTML | JSON (the IR) | utterances (speech) | a host's own writer
```

`lamlet FILE` is `lamlet build FILE --to text`. Decisions:

- **Code stays out of the document IR.** The checker and the VM keep their own representations (syntax tree,
  closure conversion, bytecode). The source IR refers to a region by index and position, never by its code, so
  the output format does not depend on compiler internals.
- **Lowering and writers are pure.** All evaluation happens before lowering. Lowering only reads values, and a
  writer is a function of the IR alone: deterministic, testable from JSON, and writable in another language.
- **The prepared IR is an output.** After the check, static regions have values and runtime regions do not.
  `lamlet build --to ir --prepared` writes that tree with its holes, for a host that evaluates them later. It is
  a structured form of what `lamlet prepare` gives today.
- **`lamlet FILE` keeps its output byte for byte.** Removing lines that held only declarations moves from
  `Document::render` to the source IR, before lowering. The CLI test and every fixture check it.

## Phase 1: the IR core and the text writer

This phase builds `lamlet-doc` with the source IR, the document IR (phase 2's table), lowering, and the text
writer, and switches `lamlet FILE` to them. Done when `lamlet FILE` gives the same bytes as before on every
fixture and example.

**How values print is defined per IR node, in each writer.** A value that is not content lowers to a
`Value { data }` node holding it as plain data (`Data`), and each writer has a rule for every kind of data:

| Data | Text writer | Markdown | HTML |
|---|---|---|---|
| String, Int, Float, Bool | as `lamlet FILE` prints it now | the same, escaped | the same, escaped |
| Unit | nothing | nothing | nothing |
| List | `[a, b]`, each element by these rules | the same, in a code span | the same, in `<code>` |
| Record, variant, Type | Lamlet's written form | in a code span | in `<code>` |

Functions and host objects have no data form. They lower to `Code` with their written form
(`<function map>`), as `lamlet FILE` prints them now.

**No user-defined printing.** A document's own type (a `Point` record) is not an IR node, so no writer prints it
specially. The author converts it with a function that returns a String or content:

```text
$[# showPoint {^ {p} [f "(${.x p}, ${.y p})"]}]
The corners are ${join ", " {map corners showPoint}}.      → The corners are (0.0, 0.0), (1.0, 2.5).
```

This keeps code out of data (records stay comparable and convertible to `Data`), keeps lowering a pure function
of the values (it calls no Lamlet code), and keeps writers pure functions of the IR. Earlier drafts had a
`toString` field on records, and it was dropped for these reasons.

- **Where.** `lamlet-doc`'s lowering and writers. `embed::text` keeps its current behavior for hosts that
  render without the IR.
- **Tests:** every fixture and example through `lamlet FILE`, old and new output compared; one document per
  kind of data in the table.

## Phase 2: the document IR's nodes

Phase 1 builds this tree with the nodes the text writer needs (`Paragraph`, `Text`, `Raw`, `Code`). This phase
adds the rest. It is a small tree, close to CommonMark's model, plus what speech needs (phase 6):

| Blocks | Inlines |
|---|---|
| `Paragraph(inlines)` | `Text(s)` |
| `Heading { level, id, attrs, inlines }` | `Emph`, `Strong`, `Code(s)` |
| `List { ordered, items }` | `Link { href, inlines }`, `Image { src, alt, title }` |
| `Table { header, rows }` | `Math { tex, display }` |
| `CodeBlock { lang, text }` | `Break` |
| `Raw { format, text }` | `Raw { format, text }` |
| `Div { name, attrs, blocks }` | `Span { name, attrs, inlines }` |

- **`Raw`** holds text that belongs to one format, such as the document's prose (phase 4) or `raw`
  content (phase 3). A writer copies `Raw` of its own format and refuses `Raw` of another, with the region's position.
- **`Div` and `Span`** carry named attributes for hosts (classes in HTML, cues for a narrated video). Writers that
  do not know a name write only the children.
- **Positions.** Every node keeps its source `Location`, so a writer's error points at the document.
- **JSON form.** `{"lamletDoc": 1, "blocks": [...]}`, versioned. It is the interface for external tools and for
  writers in other languages.

## Phase 3: content in Lamlet (the `content` package)

Content is written with helpers that produce events, so documents rarely see `#open` and `#close`:

```text
$[Import* std]$[Import* content]
${image "garden.png" "The garden in spring"}
${table {' "Room" "Area"} {' {' "study" "12 m²"} {' "hall" "20 m²"}}}
${emph "only"} the corners fill in.
${code "rust" "let x = 1;"}
```

- **The `Event` type:** `<open: {name, attrs}, close: String, text: String, leaf: {name, attrs}>`, where
  `attrs` is a list of key and value pairs. A helper returns `List(Event)`, and lists concatenate.
- **Building the tree.** The Rust side turns events into the tree and checks that each `open` has its `close`
  and that blocks do not sit inside inlines. A mismatch is an error at the region's line.
- **`raw format text`** gives a `Raw` node, for the rare deliberate case of markup a writer copies unescaped.

## Phase 4: lowering and `lamlet build`

```sh
lamlet build file.lam -o file.md            # the format from the extension
lamlet build file.lam --to html > out.html
lamlet build file.lam --to ir               # the IR as JSON
```

- **Prose.** Prose becomes `Raw { format: <target> }`: the author writes Markdown in a file meant to become
  Markdown. A file with no `--to` and no `-o` is plain text, which is what `lamlet FILE` prints.
- **A region's value lowers by the first rule that applies:**
  1. a list of `Event`s, giving content
  2. a String, number, or Bool, giving `Text`
  3. other data, giving `Value { data }`, which each writer prints by its table (phase 1)
  4. a function or host object, giving `Code` with its written form
- **Escaping.** A `Text` node is escaped for the target (HTML entities; Markdown's special characters only where
  they would change the structure). `Raw` is never escaped. So a string from a region is escaped, and prose and
  `raw` content are not.

  This differs from `lamlet FILE` today, where region strings print as they are. The plain text writer escapes
  nothing, so `lamlet FILE` keeps its output.
- **Assets.** Images keep their paths, and `build` copies nothing in this phase.

## Phase 5: writers

- **text**: content without markup. An image prints its alt text, and a table prints aligned columns.
- **md**: CommonMark, with GitHub tables.
- **html**: a fragment, or with `--standalone` a page.
- **ir**: the JSON form.
- **LaTeX** comes after these, once the first four have shown the IR is enough.

Each writer has golden tests: one document per node kind, and one with everything.

## Phase 6: speech

A narrated document needs to know which text is spoken, which is only shown, and where cues sit between words.
Three nodes cover it, close to SSML's `<sub alias>`, `<mark>`, and `<break>`:

| Node | Meaning |
|---|---|
| `Say { shown: inlines, spoken: String }` | Shown one way and spoken another (a formula, a code name) |
| `Anchor { name, attrs }` | A point between words; it belongs to the next spoken word |
| `Pause { seconds }` | Silence |

A range over words is a `Span` (phase 2) whose attributes a host reads.

**The speech writer** turns the IR into utterances, one per paragraph, as JSON:

```json
{"section": ["1", "c1-room"], "spoken": "This is the study room, lit by its panel.",
 "pieces": ["This is the study room,", "lit by its panel."],
 "anchors": [{"name": "m0", "attrs": {...}, "piece": 0, "kind": "open"}, {"name": "m0", "piece": 1, "kind": "close"}],
 "shown": [...inlines...], "pause": 0.5}
```

- **Anchors point between pieces, not words.** The spoken text is cut at each anchor. The host maps pieces to
  words with its own tokenizer, because word counts depend on how the voice splits words.
- **Sections** come from headings with an `id`: from `Heading` nodes once prose is parsed (phase 7), or from
  content helpers before that.
- **Translations.** A translated document lowers to its own IR. `lamlet build --to speech --align original.lam`
  checks that the sections and the paragraph counts match, and that the translation has no anchors.
- **Timing is not the writer's job.** It ends at utterances. The host synthesizes speech, measures word times,
  and places anchors from them.

## Phase 7: parsed prose (optional)

To build one source into several formats, prose must be format-neutral. With `--prose commonmark`, prose is
parsed as CommonMark into IR blocks instead of `Raw`, and regions become inline or block nodes where they stand.

- The parser goes in a separate crate (`lamlet-doc-markdown`) or behind a feature, so `lamlet-doc` itself stays
  small.
- Headings with `{#id key=value}` attributes give `Heading { id, attrs }` and so speech sections.

## Order and size

| Phase | Size | Depends on |
|---|---|---|
| 1 IR core, text writer | medium | |
| 2 IR nodes | small | 1 |
| 3 content | small | 2 |
| 4 lowering, `build` | medium | 1, 2, 3 |
| 5 writers | medium | 4 |
| 6 speech | medium | 4 |
| 7 parsed prose | large | 5 |

Phases 1 to 5 make `lamlet build -o file.md` useful. Phase 6 can follow phase 4 directly.

## Risks

- **The IR settles too early.** Once hosts read the JSON form, changing it breaks them. Mitigation: a version
  number, and phase 5's writers must work before the form is called stable.
- **Escaping surprises.** Region strings become escaped in HTML and Markdown while prose is not. Mitigation:
  document it with an example, and offer `{raw "…"}` in `content` for the rare deliberate case.

## Open questions

1. **Our IR or Pandoc's AST?** Ours is smaller and stable on our terms. Pandoc's would give every format at once
   but ties us to its schema. Proposed: our own, plus a Pandoc JSON writer later if there is demand.
2. **Should `lamlet FILE` escape?** Proposed: no, since plain text has nothing to escape.
3. **Assets.** Should `build` copy referenced images next to the output?
