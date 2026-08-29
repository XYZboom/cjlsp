#!/usr/bin/env python3
"""Regenerate per-file status of OUR cj-frontend over the LLT scope.

Writes /tmp/llt_ours.jsonl: {rel, status, first_err} and the fail list
/tmp/llt_fail_files.txt. Same metric as tools/llt_baseline.py.
"""
from __future__ import annotations
import json, subprocess, sys
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed

FRONTEND = "/root/Code/cangjie/cj-lang/target/release/cj-frontend"
LLT = Path("/root/Code/cangjie/cangjie_test/testsuites/LLT/compiler")
SCOPE = ["Frontend", "Lexer", "Parser", "Sema", "Diagnose"]
TIMEOUT = 10

def probe(rel: str) -> dict:
    path = LLT / rel
    try:
        r = subprocess.run([FRONTEND, "--dump-ast", str(path)],
                           capture_output=True, text=True, timeout=TIMEOUT)
        stderr = r.stderr or ""
        has_diag = "error:" in stderr
        if r.returncode != 0:
            status = "crash"
        elif has_diag:
            status = "fail"
        else:
            status = "success"
        first = next((ln.strip() for ln in stderr.splitlines() if "error:" in ln), "")
        return {"rel": rel, "status": status, "first_err": first}
    except subprocess.TimeoutExpired:
        return {"rel": rel, "status": "crash", "first_err": "<timeout>"}

def main():
    targets = [str(p.relative_to(LLT)) for d in SCOPE for p in sorted((LLT / d).rglob("*.cj")) if p.is_file()]
    jobs = int(sys.argv[1]) if len(sys.argv) > 1 else 16
    results = []
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        futs = {pool.submit(probe, rel): rel for rel in targets}
        done = 0
        for fut in as_completed(futs):
            results.append(fut.result())
            done += 1
            if done % 1000 == 0:
                print(f"[ours] {done}/{len(targets)}", file=sys.stderr, flush=True)
    results.sort(key=lambda x: x["rel"])
    with open("/tmp/llt_ours.jsonl", "w") as f:
        for r in results:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    fail = [r["rel"] for r in results if r["status"] == "fail"]
    crash = [r["rel"] for r in results if r["status"] == "crash"]
    Path("/tmp/llt_fail_files.txt").write_text("\n".join(fail) + "\n")
    succ = sum(1 for r in results if r["status"] == "success")
    print(f"[ours] total={len(results)} success={succ} fail={len(fail)} crash={len(crash)} "
          f"rate={succ/len(results)*100:.1f}%")
    print(f"[ours] fail list -> /tmp/llt_fail_files.txt ({len(fail)})")

if __name__ == "__main__":
    main()
