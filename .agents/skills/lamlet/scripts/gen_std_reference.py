#!/usr/bin/env python3
"""Regenerate the skills' package references from the packages' sources (their section headings and
per-definition doc paragraphs):

- `lamlet/references/std.md` from crates/lamlet/std/std.lam
- `lamlet-documents/references/content.md` from crates/lamlet-doc/src/content.lam, with each export's type
  asked of the built CLI (`cargo build --release` first), since content's docs do not state them

Usage (from the repository root):  python3 .agents/skills/lamlet/scripts/gen_std_reference.py
"""
import json, pathlib, re, subprocess, tempfile

root = pathlib.Path(__file__).resolve().parents[4]
skills = root / ".agents" / "skills"
decl = re.compile(r"^\$\[(?:# \{?([^\s{}]+)|Macro (!\S+)|Native (\S+))")


def types_of(names: list[str], imports: str) -> dict[str, str]:
    """Each name's type, from `lamlet eval` of a document holding one lookup region per name, with content's
    structural types written by their names."""
    with tempfile.TemporaryDirectory() as d:
        doc = pathlib.Path(d) / "types.lam"
        doc.write_text(imports + "\n" + " ".join(f"${n}" for n in names) + "\n")
        out = subprocess.run([str(root / "target" / "release" / "lamlet"), "eval", str(doc)], capture_output=True, text=True, check=True)
    attr = "{key: String, value: String}"
    element = f"{{attrs: List({attr}), name: String}}"
    event = f"<close: String, leaf: {element}, open: {element}, text: String>"
    named = [(f"List({event})", "Content"), (event, "Event"), (element, "Element"), (attr, "Attr")]
    types = {}
    for name, result in zip(names, json.loads(out.stdout)["documents"][0]["results"]):
        t = result["type"]
        for long, short in named:
            t = t.replace(long, short)
        types[name] = t
    return types


def reference(source: pathlib.Path, title: str, out: pathlib.Path, imports: str = "") -> None:
    lines = [f"# {title} reference (generated from {source.relative_to(root)} — do not edit by hand)", "",
             "Each entry: name, then its doc (which starts with its type when it has one). `_names` are private and omitted.", ""]
    # With `imports` (content's layout), a doc paragraph may stand one blank line above its definitions, and
    # applies to each of the definitions that follow it; they are listed with their types.
    entries, doc, after_decl = [], [], False
    for line in source.read_text().splitlines():
        m = decl.match(line)
        if m:
            name = m.group(1) or m.group(2) or m.group(3)
            if not name.lstrip("!").startswith("_"):
                entries.append((len(lines), name, " ".join(doc).strip()))
                lines.append(None)
            if not imports:
                doc = []
            after_decl = True
        elif re.match(r"^\d+\. \S", line):
            lines += ["", f"## {line.strip()}", ""]
            doc, after_decl = [], False
        elif line.strip() == "" or line.startswith(" ") or line.startswith("}") or line.startswith("$"):
            if not imports:
                doc = []
        else:
            if after_decl:
                doc, after_decl = [], False
            doc.append(line.strip())
    types = types_of([n for _, n, _ in entries], imports) if imports else {}
    for i, name, text in entries:
        head = f"`{name}` : `{types[name]}`" if name in types else f"`{name}`"
        lines[i] = f"- {head} — {text}" if text else f"- {head}"
    out.write_text("\n".join(lines) + "\n")
    print(f"wrote {out.relative_to(root)}")


reference(root / "crates" / "lamlet" / "std" / "std.lam", "std", skills / "lamlet" / "references" / "std.md")
reference(root / "crates" / "lamlet-doc" / "src" / "content.lam", "content", skills / "lamlet-documents" / "references" / "content.md",
          "$[Import* std]$[Import* content]")
