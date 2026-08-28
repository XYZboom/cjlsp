#!/usr/bin/env python3
"""T42: cluster the 727 LLT fails by OUR OWN emitted first-error, and
cross-reference official stage. Identifies concrete fixable parser bugs."""
from __future__ import annotations

import json, re, subprocess
from collections import Counter, defaultdict
from pathlib import Path

LLT = Path("/root/Code/cangjie/cangjie_test/testsuites/LLT/compiler")
OURS = "/root/Code/cangjie/cj-lang/target/release/cj-frontend"
FAIL_LIST = Path("/tmp/t42_fail_files.txt")

# official oracle (full output incl first error)
OFF = {json.loads(l)["rel"]: json.loads(l)
       for l in (Path("/tmp/t42_oracle2.jsonl").read_text().splitlines()) if l.strip()}

def our_first(path):
    r = subprocess.run([OURS, "--dump-ast", str(path)],
                       capture_output=True, text=True, timeout=15)
    for ln in (r.stderr or "").splitlines():
        if "error:" in ln:
            return ln.strip()
    return ""

fails = [l.strip() for l in FAIL_LIST.read_text().splitlines() if l.strip()]
clusters = Counter()
samples = defaultdict(list)
for rel in fails:
    path = LLT / rel
    fe = our_first(path)
    fe_n = re.sub(r" at \d+:\d+$", "", fe)
    fe_n = re.sub(r"\d+", "<n>", fe_n)
    clusters[fe_n] += 1
    if len(samples[fe_n]) < 3:
        samples[fe_n].append(rel)

print(f"total fails={len(fails)}  (by OUR first error)")
for msg, n in clusters.most_common(40):
    ex = samples[msg][0]
    off = OFF.get(ex, {}).get("first", "?")[:80]
    print(f"  {n:>4}  {msg[:95]}")
    print(f"         e.g. {ex}  | official: {off}")