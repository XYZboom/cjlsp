#!/usr/bin/env python3
"""T42: deep LLT failure analysis — dump EVERY failing file with full stderr,
cluster by first-error AND by full diagnostic set, so we can pick the biggest
fixable bucket. Read-only evaluation of the current release frontend."""
from __future__ import annotations

import os, re, subprocess, sys
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path("/root/Code/cangjie/cj-lang")
FRONTEND = REPO / "target" / "release" / "cj-frontend"
LLT = Path(os.environ.get("CANGJIE_LLT_DIR", "/root/Code/cangjie/cangjie_test/testsuites/LLT/compiler"))
SCOPE = ["Frontend", "Lexer", "Parser", "Sema", "Diagnose"]
TIMEOUT = 10

def run(f):
    try:
        r = subprocess.run([str(FRONTEND), "--dump-ast", str(f)],
                           capture_output=True, text=True, timeout=TIMEOUT)
        return r.returncode, r.stderr or ""
    except subprocess.TimeoutExpired:
        return -1, "<timeout>"

def norm(msg):
    s = re.sub(r" at \d+:\d+$", "", msg)      # drop trailing pos
    s = re.sub(r"\d+", "<n>", s)              # drop numbers
    return s.strip()

def first_err(stderr):
    for ln in stderr.splitlines():
        if "error:" in ln:
            return ln.strip()
    return "(no error line)"

def all_errors(stderr):
    return [ln.strip() for ln in stderr.splitlines() if "error:" in ln]

targets = []
for d in SCOPE:
    for p in sorted((LLT / d).rglob("*.cj")):
        if p.is_file():
            targets.append((d, p))

fails = []
for d, p in targets:
    rc, err = run(p)
    if rc != 0:
        sys.exit(f"CRASH on {p}: {err[:200]}")
    if "error:" in err:
        fails.append((d, p, err))

print(f"[t42] total={len(targets)} fails={len(fails)}")
print()
print("=== by directory ===")
perdir = Counter(d for d, _, _ in fails)
for d in SCOPE:
    n = perdir.get(d, 0)
    total = sum(1 for dd, _ in targets if dd == d)
    print(f"  {d:<10} {n:>5}/{total:<5} fail")

print()
print("=== full first-error clusters (all 727, sorted desc) ===")
c = Counter(norm(first_err(e)) for _, _, e in fails)
for msg, n in c.most_common():
    ex = next(f"{d}/{p.relative_to(LLT)}" for d, p, e in fails if norm(first_err(e)) == msg)
    print(f"  {n:>4}  {msg}   e.g. {ex}")

print()
print("=== error-SET clusters (files sharing identical full diag sets) — top 40 ===")
cs = Counter(tuple(norm(x) for x in all_errors(e)) for _, _, e in fails)
for sig, n in cs.most_common(40):
    if n < 2:
        continue
    ex = next(f"{d}/{p.relative_to(LLT)}" for d, p, e in fails
              if tuple(norm(x) for x in all_errors(e)) == sig)
    print(f"  {n:>4}  {sig[0] if sig else '(empty)'}")
    print(f"         e.g. {ex}")
