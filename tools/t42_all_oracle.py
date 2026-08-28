#!/usr/bin/env python3
"""T42: run official cjc-frontend over ALL 5351 LLT files, count how many it
accepts with ZERO diagnostics (the true ceiling for our 'success = no diag'
metric). Also records our own per-file emit for comparison."""
from __future__ import annotations

import os, re, subprocess, sys, json
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed

CJC_ENV = "/root/Code/cangjie/sdk/cangjie/envsetup.sh"
LLT = Path("/root/Code/cangjie/cangjie_test/testsuites/LLT/compiler")
SCOPE = ["Frontend", "Lexer", "Parser", "Sema", "Diagnose"]
OURS = "/root/Code/cangjie/cj-lang/target/release/cj-frontend"

def probe(path: str) -> dict:
    try:
        r = subprocess.run(
            ["bash", "-lc", f"source {CJC_ENV} >/dev/null 2>&1; "
             f"cd /tmp && timeout 45 cjc-frontend --experimental --enable-eh "
             f"--diagnostic-format=noColor --error-count-limit=all {path} -o /tmp/t42_all_probe 2>&1"],
            capture_output=True, text=True, timeout=60,
        )
        out = r.stdout or ""
        # our own emit
        o = subprocess.run([OURS, "--dump-ast", path], capture_output=True, text=True, timeout=15)
        return {"path": path, "rc": r.returncode, "nerr": out.count("error:"),
                "first": next((ln.strip() for ln in out.splitlines() if "error:" in ln), ""),
                "ours_diag": "error:" in (o.stderr or "")}
    except Exception as e:
        return {"path": path, "rc": -999, "nerr": 0, "first": f"<runner {e}>", "ours_diag": None}

def main():
    targets = []
    for d in SCOPE:
        for p in sorted((LLT / d).rglob("*.cj")):
            if p.is_file():
                targets.append(str(p))
    jobs = int(sys.argv[1]) if len(sys.argv) > 1 else 12
    results = []
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        futs = {pool.submit(probe, t): t for t in targets}
        done = 0
        for fut in as_completed(futs):
            results.append(fut.result())
            done += 1
            if done % 1000 == 0:
                print(f"[all] {done}/{len(targets)}", file=sys.stderr, flush=True)
    results.sort(key=lambda x: x["path"])
    with open("/tmp/t42_all_oracle.jsonl", "w") as f:
        for r in results:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    total = len(results)
    off_clean = sum(1 for r in results if r["nerr"] == 0)
    print(f"[all] total={total} official-clean={off_clean} ({off_clean/total*100:.1f}%)")
    print(f"[all] written /tmp/t42_all_oracle.jsonl")

if __name__ == "__main__":
    main()
