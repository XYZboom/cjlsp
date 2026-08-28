#!/usr/bin/env python3
"""T42 oracle v2: run official cjc-frontend --enable-eh on every failing LLT
file (parser + frontend sema, no codegen/link). Record rc + first error + full
output. Better oracle than full cjc: no linker/'main is missing' noise from
back-end, and --enable-eh admits effect-handler files."""
from __future__ import annotations

import os, re, subprocess, sys, json
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed

CJC_ENV = "/root/Code/cangjie/sdk/cangjie/envsetup.sh"
FAIL_LIST = Path("/tmp/t42_fail_files.txt")
OUT = Path("/tmp/t42_oracle2.jsonl")

def probe(rel: str) -> dict:
    path = f"/root/Code/cangjie/cangjie_test/testsuites/LLT/compiler/{rel}"
    try:
        r = subprocess.run(
            ["bash", "-lc", f"source {CJC_ENV} >/dev/null 2>&1; "
             f"cd /tmp && timeout 45 cjc-frontend --experimental --enable-eh "
             f"--diagnostic-format=noColor --error-count-limit=all {path} -o /tmp/t42_cjf_probe 2>&1"],
            capture_output=True, text=True, timeout=60,
        )
        out = r.stdout or ""
        first = next((ln.strip() for ln in out.splitlines() if "error:" in ln), "")
        return {"rel": rel, "rc": r.returncode, "first": first,
                "nerr": out.count("error:"), "out": out[:1200]}
    except Exception as e:
        return {"rel": rel, "rc": -999, "first": f"<runner {e}>", "nerr": 0, "out": ""}

def main() -> int:
    files = [l.strip() for l in FAIL_LIST.read_text().splitlines() if l.strip()]
    jobs = int(sys.argv[1]) if len(sys.argv) > 1 else 12
    results = []
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        futs = {pool.submit(probe, rel): rel for rel in files}
        done = 0
        for fut in as_completed(futs):
            results.append(fut.result())
            done += 1
            if done % 100 == 0:
                print(f"[oracle2] {done}/{len(files)}", file=sys.stderr, flush=True)
    with open(OUT, "w") as f:
        for r in sorted(results, key=lambda x: x["rel"]):
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"[oracle2] done {len(results)} -> {OUT}", file=sys.stderr)
    return 0

if __name__ == "__main__":
    sys.exit(main())
