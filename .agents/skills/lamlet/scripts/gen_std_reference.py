#!/usr/bin/env python3
"""Regenerate references/std.md from crates/lamlet/std/std.lam (its section headings and per-definition doc paragraphs).

Usage (from the repository root):  python3 .agents/skills/lamlet/scripts/gen_std_reference.py
"""
import pathlib, re

root = pathlib.Path(__file__).resolve().parents[4]
src = (root / "crates" / "lamlet" / "std" / "std.lam").read_text()
out = ["# std reference (generated from crates/lamlet/std/std.lam — do not edit by hand)", "",
       "Each entry: name, then its doc (which starts with its type). `_names` are private and omitted.", ""]
doc = []
decl = re.compile(r"^\$\[(?:# \{?([^\s{}]+)|Macro (!\S+)|Native (\S+))")
for line in src.splitlines():
    m = decl.match(line)
    if m:
        name = m.group(1) or m.group(2) or m.group(3)
        if not name.lstrip("!").startswith("_"):
            text = " ".join(doc).strip()
            out.append(f"- `{name}` — {text}" if text else f"- `{name}`")
        doc = []
    elif re.match(r"^\d+\. \S", line):
        out += ["", f"## {line.strip()}", ""]
        doc = []
    elif line.strip() == "" or line.startswith(" ") or line.startswith("}") or line.startswith("$"):
        doc = []
    else:
        doc.append(line.strip())
(pathlib.Path(__file__).resolve().parents[1] / "references" / "std.md").write_text("\n".join(out) + "\n")
print("wrote references/std.md")
