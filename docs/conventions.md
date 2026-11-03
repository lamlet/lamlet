# Naming conventions

These are Lamlet's recommended names, for Lamlet code, packages, documents, and the tools around them. `std`,
`content`, and Lamlet's own tools follow them. They are recommendations: the language accepts any valid
identifier (spec §2.1, letters, digits, and `_`, starting with a letter or `_`), and nothing is checked.

The rule behind all of them: **names inside Lamlet code are identifiers, names outside it are words joined by
dashes.** A Lamlet identifier cannot hold a dash, so a name that must be written in code (a package, a binding)
uses camel case or `_`. A name that only travels as data or on the command line (an id, a program, a file)
uses dashes.

## In Lamlet code

| Kind | Convention | Examples |
|---|---|---|
| Values and functions | lower camel case; one word when one is enough | `map`, `fold`, `isEmpty`, `headingId`, `bulletsOf` |
| Types (`Type` values) | upper camel case | `Float`, `List`, `Content`, `Event`, `Attr` |
| Record fields | lower camel case | `{. startFrame 0 endFrame 30}` |
| Variant alternatives | lower camel case | `#some`, `#none`, `#lightOnly` |
| Macros | `!` and lower camel case | `!if`, `!and`, `!assert` |
| Private definitions (not exported) | a leading `_` | `_quote`, `_attr` |
| Parameters and local bindings | short lower camel case | `x`, `xs`, `acc`, `name` |

- **Predicates** start with `is` or `has`: `isEmpty`, `hasLabel`.
- **Two forms of one function:** the plain name takes the simple argument, and `…Of` takes the general one:
  `emph "word"` and `emphOf content`, `bulletsOf {' "a" "b"}` and `bullets {' c1 c2}`.
- **Built-in primitives** are the one exception: the core package is `K` (`K::fold`), capitalized so it never
  collides with a document's names.

## Packages

| Kind | Convention | Examples |
|---|---|---|
| Package names | lower case; several words joined by `_` | `std`, `content`, `present`, `garden_tools`, `light_lab` |
| Package files | the package name and `.lam` | `std.lam`, `content.lam`, `present.lam` |
| Aliases (`ImportAs`) | a short lower case word | `$[ImportAs garden_tools gt]` |

Package names use `_`, not camel case. They are often named after something outside Lamlet (a directory, a
crate, a project), where words are joined by `_` or `-`, and `_` keeps that name recognizable in code. It also
keeps a package visibly apart from a binding: `light_lab::lampIntensity`.

A project whose name has a dash uses `_` for its package: `light-lab` is `light_lab`, and render-lab's own
package is `render_lab`.

## Names that are data

Ids that documents pass to hosts as Strings are never written as identifiers, so they use dashes:

| Kind | Convention | Examples |
|---|---|---|
| Heading and section ids | lower case words joined by `-` | `{#c1-room}`, `{#bounced-light}` (a heading without an id gets this form from its text) |
| Cue, span, and group names | the same | `${cue "light-up" …}`, `${span "warm-glow" …}`, `${group "aside" …}` |
| Native API ids | dotted package, `/`, dashed name | `garden.count/leaves`, `garden.tree/make-widget` (its Lamlet name is `makeWidget`) |

Attribute **keys** are a host's vocabulary and keep the host's spelling: `lampIntensity`, `camera.yaw`,
`ui.label`. A generated vocabulary names its functions in lower camel case from them (`cameraYaw`).

## Files and documents

- Lamlet documents and packages end in `.lam`. A document's file name uses lower case words joined by `-`:
  `point-light.lam`, `storyboard.lam`, `storyboard.zh-CN.lam` (a language tag keeps its own case).
- Generated packages are named for what they describe and are not edited by hand: `lights.lam` from
  `keys.json`.

## Tools and extensions

| Kind | Convention | Examples |
|---|---|---|
| An extension's program | `lamlet-` and dashed words | `lamlet-present` |
| Its command | `lamlet` and the same words | `lamlet present schedule …` |
| Its repository and crate | the program's name | `lamlet/lamlet-present`, crate `lamlet-present` |
| The Lamlet packages it carries | package names, as above | `present` |
| Lamlet's own crates | `lamlet-` and a word | `lamlet`, `lamlet-doc`, `lamlet-cli` |

**JSON** that Lamlet and its tools write uses lower camel case keys, and a `lamlet…` key with the format's
version: `{"lamletSchedule": 1, "startFrame": 0}`.

## Hosts

A host keeps its own conventions for its own code. Where its names meet Lamlet (the packages it provides,
the ids its documents use, the keys of its vocabulary), it follows this page: package names with `_`, ids with
`-`, and keys as the host spells them.
