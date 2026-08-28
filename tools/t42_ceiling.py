#!/usr/bin/env python3
"""T42 final classification.

For each of the 5351 files we have:
  - official cjc-frontend (--experimental --enable-eh) full output
  - our own --dump-ast emit (boolean)

The metric (llt_baseline) counts success = WE emit NO diagnostic.

Correctness model:
  - file is parse-VALID if official emits NO Parser.def/Lexer.def diagnostic
    (sema/driver-only errors are fine: our parser-only frontend may accept).
  - For a parse-valid file, success is correct behavior and FIXABLE.
  - For a parse-invalid file, emitting a diag is correct -> permanent 'fail'.

This reports the TRUE fixable pool and the achievable ceiling.
"""
from __future__ import annotations

import json, re
from collections import Counter
from pathlib import Path

PARSER_DEF = "/root/Code/cangjie/cangjie_compiler/include/cangjie/Basic/DiagRefactor/DiagnosticParser.def"
LEXER_DEF = "/root/Code/cangjie/cangjie_compiler/include/cangjie/Basic/DiagRefactor/DiagnosticLexer.def"

def t2r(t: str) -> str:
    return r".*?".join(re.escape(p) for p in t.split("%s"))

def _defs(path: str):
    text = Path(path).read_text()
    out = []
    for m in re.finditer(r'(?:ERROR|WARN)\((\w+),\s*"((?:[^"\\]|\\.)*)"', text):
        if m.group(2) == "%s":
            continue
        out.append(m.group(2))
    return out

STAGE_MSGS = _defs(PARSER_DEF) + _defs(LEXER_DEF)

def has_stage_diag(first: str) -> bool:
    body = first
    if body.startswith("error:"):
        body = body[len("error:"):].strip()
    body = re.sub(r" at \d+:\d+$", "", body)
    body = re.sub(r"\d+", "<n>", body)
    for msg in STAGE_MSGS:
        if re.match(rf"^{t2r(msg)}$", body):
            return True
    return False

def main():
    rows = [json.loads(l) for l in Path("/tmp/t42_all_oracle.jsonl").read_text().splitlines() if l.strip()]
    fixable, negative = [], []
    for r in rows:
        if not r["ours_diag"]:
            continue  # already success
        if has_stage_diag(r["first"]):
            negative.append(r)
        else:
            fixable.append(r)
    print(f"ours-fail={len(fixable)+len(negative)}  "
          f"FIXABLE(parse-valid)={len(fixable)}  NEGATIVE(parse-invalid)={len(negative)}")
    print(f"ceiling if all fixable fixed: {5351-len(negative)}/5351 = {(5351-len(negative))/5351*100:.2f}%")
    print()
    print("=== FIXABLE files (official parser accepts; we wrongly emit) ===")
    # cluster by OUR first error
    c = Counter()
    for r in fixable:
        # re-derive our first error
        c[re.sub(r"\d+", "<n>", r.get("our_first", ""))] += 1
    # (our_first not stored in all_oracle; recompute separately below)
    for r in sorted(fixable, key=lambda x: x["path"]):
        print(f"  {r['path'][len('/root/Code/cangjie/cangjie_test/testsuites/LLT/compiler/'):]}"
              f"  | off: {r['first'][:70]}")

if __name__ == "__main__":
    main()
