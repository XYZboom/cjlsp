#!/usr/bin/env python3
"""T42: THE fixable classification.

Metric (llt_baseline): success = OUR frontend emits NO diagnostic.

A fail is FIXABLE iff official's PARSER accepts the file (no Parser.def /
Lexer.def diagnostic in official output) — even if official later rejects it
at SEMANTIC stage (can not find package, mismatched types, 'main' is missing,
undeclared type name, ...). Those semantic errors are checked by SEMA, which
our --dump-ast frontend does not run; a semantically-rejected-but-parse-clean
file must still yield NO diagnostic from us.

A fail is NEGATIVE iff official emits a parser/lexer-stage diagnostic (the
file is genuinely malformed; official parser rejects it too) -> we correctly
emit -> permanent 'fail' under this metric.
"""
from __future__ import annotations

import json, re, subprocess
from collections import Counter, defaultdict
from pathlib import Path

LLT = Path("/root/Code/cangjie/cangjie_test/testsuites/LLT/compiler")
OURS = "/root/Code/cangjie/cj-lang/target/release/cj-frontend"
PARSER_DEF = "/root/Code/cangjie/cangjie_compiler/include/cangjie/Basic/DiagRefactor/DiagnosticParser.def"
LEXER_DEF = "/root/Code/cangjie/cangjie_compiler/include/cangjie/Basic/DiagRefactor/DiagnosticLexer.def"

def t2r(t): return r".*?".join(re.escape(p) for p in t.split("%s"))
def _defs(p):
    out=[]
    for m in re.finditer(r'(?:ERROR|WARN)\((\w+),\s*"((?:[^"\\]|\\.)*)"', Path(p).read_text()):
        if m.group(2)!="%s": out.append(m.group(2))
    return out
STAGE = _defs(PARSER_DEF)+_defs(LEXER_DEF)

def is_parser_stage_error(first: str) -> bool:
    body = first[len("error:"):].strip() if first.startswith("error:") else first
    body = re.sub(r" at \d+:\d+$","",body)
    body = re.sub(r"\d+","<n>",body)
    return any(re.match(rf"^{t2r(m)}$", body) for m in STAGE)

def our_first(path):
    r = subprocess.run([OURS,"--dump-ast",str(path)],capture_output=True,text=True,timeout=15)
    return next((ln.strip() for ln in (r.stderr or "").splitlines() if "error:" in ln),"")

# official oracle v3: full out stored (rerun cjc-frontend needing full output?)
# Use oracle2 (first error stored) - first error is best proxy: parse errors come first.
ORACLE = {json.loads(l)["rel"]: json.loads(l)
          for l in Path("/tmp/t42_oracle2.jsonl").read_text().splitlines() if l.strip()}

fails = [l.strip() for l in Path("/tmp/t42_fail_files.txt").read_text().splitlines() if l.strip()]
fixable, negative = [], []
for rel in fails:
    off = ORACLE.get(rel)
    if off is None:
        continue
    if off["first"] and is_parser_stage_error(off["first"]):
        negative.append(rel)
    else:
        fixable.append(rel)

print(f"fails={len(fails)}  FIXABLE(parse-valid)={len(fixable)}  NEGATIVE(parser-rejected)={len(negative)}")
print()
print("=== FIXABLE (official PARSER ok; we wrongly emit a parse diag) ===")
by_our = Counter()
samp = defaultdict(list)
for rel in fixable:
    fe = re.sub(r"\d+","<n>",re.sub(r" at \d+:\d+$","",our_first(LLT/rel)))
    by_our[fe]+=1
    if len(samp[fe])<4: samp[fe].append(rel)
for msg,n in by_our.most_common(30):
    print(f"  {n:>3}  {msg[:100]}")
    for ex in samp[msg]:
        off = ORACLE.get(ex,{}).get("first","")[:70]
        print(f"         {ex}  | off: {off}")
print()
print("=== NEGATIVE by official error (designed-fail; permanent) ===")
by_off = Counter()
for rel in negative:
    fe = ORACLE[rel]["first"]
    by_off[re.sub(r"\d+","<n>",fe)]+=1
for msg,n in by_off.most_common(25):
    print(f"  {n:>3}  {msg[:105]}")