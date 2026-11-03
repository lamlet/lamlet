# content reference (generated from crates/lamlet-doc/src/content.lam — do not edit by hand)

Each entry: name, then its doc (which starts with its type when it has one). `_names` are private and omitted.


## 1. Types

- `Attr` : `Type` — An attribute of an element: a key and its value as text.
- `Element` : `Type` — An element's name and attributes.
- `Event` : `Type` — One event: the opening of an element, its closing (by name), text, or a leaf element with no children.
- `Content` : `Type` — Content: a list of events. Lists of content join with `cat` (or std's `flatten`).

## 2. Building blocks

- `attr` : `Fn([String, String], Attr)`
- `cat` : `Fn([List(Content)], Content)` — Content from pieces of content, in order.
- `empty` : `Content` — Nothing.

## 3. Inlines

- `text` : `Fn([String], Content)` — Text. Writers escape it for their format.
- `emph` : `Fn([String], Content)` — Emphasis and strong emphasis, of a String or of content.
- `emphOf` : `Fn([Content], Content)` — Emphasis and strong emphasis, of a String or of content.
- `strong` : `Fn([String], Content)` — Emphasis and strong emphasis, of a String or of content.
- `strongOf` : `Fn([Content], Content)` — Emphasis and strong emphasis, of a String or of content.
- `link` : `Fn([String, String], Content)` — A link with a String label, or with content.
- `linkOf` : `Fn([Content, String], Content)` — A link with a String label, or with content.
- `image` : `Fn([String, String], Content)` — An image, with its alternative text, and optionally a title.
- `imageTitled` : `Fn([String, String, String], Content)` — An image, with its alternative text, and optionally a title.
- `code` : `Fn([String], Content)` — Code inside a line.
- `math` : `Fn([String], Content)` — A formula in TeX, inline or displayed.
- `displayMath` : `Fn([String], Content)` — A formula in TeX, inline or displayed.
- `lineBreak` : `Content` — A line break.
- `raw` : `Fn([String, String], Content)` — Markup for one format, copied unescaped by that format's backend and refused by the others: inline, or as a block.
- `rawBlock` : `Fn([String, String], Content)` — Markup for one format, copied unescaped by that format's backend and refused by the others: inline, or as a block.
- `span` : `Fn([String, List(Attr), Content], Content)` — A named span with attributes, for hosts: a class in HTML, a cue for a narrated document. Writers that do not know the name write only its content.

## 4. Blocks

- `paragraph` : `Fn([Content], Content)` — A paragraph.
- `heading` : `Fn([a:Scalar, String], Content)` — A heading of level 1 to 6, from a String; `headingId` also gives it an id (a speech section, an HTML anchor).
- `headingId` : `Fn([a:Scalar, String, String], Content)` — A heading of level 1 to 6, from a String; `headingId` also gives it an id (a speech section, an HTML anchor).
- `headingOf` : `Fn([a:Scalar, String, Content], Content)` — A heading of level 1 to 6, from a String; `headingId` also gives it an id (a speech section, an HTML anchor).
- `bullets` : `Fn([List(Content)], Content)` — Lists of items, each item content.
- `numbered` : `Fn([List(Content)], Content)` — Lists of items, each item content.
- `bulletsOf` : `Fn([List(String)], Content)` — Lists of Strings, as bullets or numbered.
- `numberedOf` : `Fn([List(String)], Content)` — Lists of Strings, as bullets or numbered.
- `tableOf` : `Fn([List(Content), List(List(Content))], Content)` — A table from a header row and rows of Strings, or of content.
- `table` : `Fn([List(String), List(List(String))], Content)` — A table from a header row and rows of Strings, or of content.
- `codeBlock` : `Fn([String, String], Content)` — A block of code in a language (`""` for none).
- `group` : `Fn([String, List(Attr), Content], Content)` — A named group of blocks with attributes, for hosts, like `span` for inlines.

## 5. Speech

- `say` : `Fn([Content, String], Content)` — Shown as `shown` and spoken as `spoken`: a formula, a code name, an abbreviation.
- `anchor` : `Fn([String, List(Attr)], Content)` — A named point between words, with attributes, that a speech host times to the next spoken word.
- `pause` : `Fn([a:Scalar], Content)` — Silence, in seconds.
