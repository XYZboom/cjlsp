#!/usr/bin/env python3
"""Merge our per-file parse status with the official oracle to find ACTIONABLE
fixes: files where OUR PARSER rejects syntax the OFFICIAL PARSER accepts.

Actionable = official first error (if any) is NOT a parser-stage template
(DiagnosticParser.def / DiagnosticLexer.def) -> official parser accepted the
syntax; only sema/driver errors remain. Our --dump-ast is parser-only, so a
perfect parser flips these to success.

Groups actionable files by OUR first_err to rank parser bugs to fix.
"""
from __future__ import annotations
import json, re
from collections import Counter
from pathlib import Path

PARSER_DEF = "/root/Code/cangjie/cangjie_compiler/include/cangjie/Basic/DiagRefactor/DiagnosticParser.def"
LEXER_DEF = "/root/Code/cangjie/cangjie_compiler/include/cangjie/Basic/DiagRefactor/DiagnosticLexer.def"

def template_to_regex(t: str) -> str:
    return r".*?".join(re.escape(p) for p in t.split("%s"))

def _parse_def(path: str):
    text = Path(path).read_text()
    out = []
    for m in re.finditer(r'(?:ERROR|WARN)\((\w+),\s*"((?:[^"\\]|\\.)*)"', text):
        name, msg = m.group(1), m.group(2)
        if msg == "%s":
            continue
        out.append((name, msg))
    return out

TEMPLATES = _parse_def(PARSER_DEF) + _parse_def(LEXER_DEF)

def is_parser_stage(body: str) -> bool:
    if body.startswith("error: "):
        body = body[len("error: "):]
    for name, msg in TEMPLATES:
        if re.match(rf"^{template_to_regex(msg)}$", body):
            return True
    return False

def norm_err(msg: str) -> str:
    s = re.sub(r" at \d+:\d+$", "", msg)
    s = re.sub(r"\d+", "<n>", s)
    return s.strip()

def main():
    ours = {}
    for l in Path("/tmp/llt_ours.jsonl").read_text().splitlines():
        if not l.strip():
            continue
        r = json.loads(l)
        ours[r["rel"]] = r
    oracle = {}
    for l in Path("/tmp/t42_oracle2.jsonl").read_text().splitlines():
        if not l.strip():
            continue
        r = json.loads(l)
        oracle[r["rel"]] = r

    actionable, negative, other = [], [], []
    for rel, r in ours.items():
        if r["status"] != "fail":
            continue
        o = oracle.get(rel)
        if o is None:
            other.append((rel, "no-oracle"))
            continue
        errlines = [ln.strip() for ln in (o["out"] or "").splitlines() if "error:" in ln]
        if not errlines:
            actionable.append((rel, "official-clean"))
            continue
        fe = re.sub(r"\d+", "<n>", re.sub(r" at \d+:\d+$", "", errlines[0]))
        if is_parser_stage(fe):
            negative.append((rel, fe))
        else:
            actionable.append((rel, fe))

    print(f"OURS-fail={len([r for r in ours.values() if r['status']=='fail'])}  "
          f"ACTIONABLE={len(actionable)}  NEGATIVE={len(negative)}  other={len(other)}")
    print()

    # group actionable by OUR first_err
    print("=== ACTIONABLE grouped by OUR parser error (ranked) ===")
    c = Counter(norm_err(ours[rel]["first_err"]) for rel, _ in actionable)
    for msg, n in c.most_common(30):
        ex = next(rel for rel, _ in actionable if norm_err(ours[rel]["first_err"]) == msg)
        print(f"  {n:>3}  {msg[:90]}   e.g. {ex}")
    print()

    # dump the full actionable list for reference
    Path("/tmp/llt_actionable.txt").write_text(
        "\n".join(f"{rel}\t{why}\t{ours[rel]['first_err']}" for rel, why in sorted(actionable)))
    print(f"[written] /tmp/llt_actionable.txt ({len(actionable)} actionable)")

if __name__ == "__main__":
    main()
