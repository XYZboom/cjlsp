#!/usr/bin/env python3
"""T42: classify the 727 fails as FIXABLE (official PARSER accepts the file;
sema/driver errors only) vs NEGATIVE-TEST (official parser also rejects).

Parser-stage diagnostics come from DiagnosticParser.def. If official emits any
message matching a parser template, the file is a designed-fail negative test
-> stays 'fail' in the metric regardless. If official emits ONLY sema/driver
messages ('main' is missing, undeclared type, ...), the file parses cleanly ->
our emit is a false rejection -> FIXABLE.
"""
from __future__ import annotations

import json, re
from collections import Counter
from pathlib import Path

PARSER_DEF = "/root/Code/cangjie/cangjie_compiler/include/cangjie/Basic/DiagRefactor/DiagnosticParser.def"
LEXER_DEF = "/root/Code/cangjie/cangjie_compiler/include/cangjie/Basic/DiagRefactor/DiagnosticLexer.def"

def template_to_regex(t: str) -> str:
    parts = t.split("%s")
    return r".*?".join(re.escape(p) for p in parts)

def _parse_def(path: str):
    text = Path(path).read_text()
    out = []
    for m in re.finditer(r'(?:ERROR|WARN)\((\w+),\s*"((?:[^"\\]|\\.)*)"', text):
        name, msg = m.group(1), m.group(2)
        # Exclude wildcard / placeholder-only templates ("%s") that match
        # anything — they are not real stage signals.
        if msg == "%s":
            continue
        out.append((name, msg))
    return out

def load_templates():
    return _parse_def(PARSER_DEF) + _parse_def(LEXER_DEF)

def is_parser_stage(first_err: str) -> bool:
    # first_err like: "error: expected %s %s, found %s"  (already position-stripped)
    body = first_err
    if body.startswith("error: "):
        body = body[len("error: "):]
    for name, msg in TEMPLATES:
        if re.match(rf"^{template_to_regex(msg)}$", body):
            return True
    return False

TEMPLATES = load_templates()

def main():
    rows = [json.loads(l) for l in Path("/tmp/t42_oracle2.jsonl").read_text().splitlines() if l.strip()]
    fixable, negative = [], []
    for r in rows:
        errlines = [ln.strip() for ln in r["out"].splitlines() if "error:" in ln]
        if not errlines:
            fixable.append((r, "official-clean"))
            continue
        # use the FIRST error line to decide (parse errors stop further stages)
        fe = re.sub(r" at \d+:\d+$", "", errlines[0])
        fe = re.sub(r"\d+", "<n>", fe)
        if is_parser_stage(fe):
            negative.append((r, fe))
        else:
            fixable.append((r, fe))

    print(f"total={len(rows)}  FIXABLE={len(fixable)}  NEGATIVE={len(negative)}")
    print()
    print("=== FIXABLE (official parser accepts; we wrongly reject) ===")
    c = Counter(why for _, why in fixable)
    print(f"count={len(fixable)}; by official error: {dict(c.most_common(10))}")
    for r, why in fixable:
        print(f"  {r['rel']}  | official[{why[:60]}] ours[{r['first'][:50]}]")

    print()
    print("=== NEGATIVE (official parser also rejects — designed-fail) ===")
    print(f"count={len(negative)}")
    c = Counter(why for _, why in negative)
    for msg, n in c.most_common(30):
        print(f"  {n:>4}  {msg[:110]}")

if __name__ == "__main__":
    main()
