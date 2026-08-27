#!/usr/bin/env python3
"""LLT frontend full-volume regression baseline (T19).

Batch-runs `cj-frontend --dump-ast` over every .cj file under
LLT/compiler/{Frontend,Lexer,Parser,Sema,Diagnose} (== the 5353 frontend
cases of the official cangjie_test LLT suite) and classifies each:

  success  — exit 0 and no parser diagnostic emitted
  fail     — exit 0 but parser diagnostic(s) printed to stderr
  crash    — non-zero exit (Rust panic / abort / signal) or timeout

Writes tools/llt_baseline.txt with totals, per-directory breakdown,
crash clusters, and the first 50 failure samples.  This is an evaluation
task: it never modifies cj-frontend logic.

Usage:
  python3 tools/llt_baseline.py                 # defaults, writes tools/llt_baseline.txt
  python3 tools/llt_baseline.py --frontend /path/to/cj-frontend --llt-dir /path/to/LLT/compiler
  python3 tools/llt_baseline.py --jobs 32
"""
from __future__ import annotations

import argparse
import datetime
import os
import re
import subprocess
import sys
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_FRONTEND = REPO_ROOT / "target" / "release" / "cj-frontend"
DEFAULT_LLT_DIR = Path("/root/Code/cangjie/cangjie_test/testsuites/LLT/compiler")
SCOPE_DIRS = ["Frontend", "Lexer", "Parser", "Sema", "Diagnose"]
TIMEOUT = 10  # seconds per file


def git_short_head() -> str:
    try:
        r = subprocess.run(
            ["git", "-C", str(REPO_ROOT), "rev-parse", "--short", "HEAD"],
            capture_output=True, text=True, timeout=5,
        )
        return r.stdout.strip() or "?"
    except Exception:  # noqa: BLE001
        return "?"


def classify(path: str, frontend: str) -> dict:
    """Run cj-frontend --dump-ast on one file; return a result dict."""
    try:
        r = subprocess.run(
            [frontend, "--dump-ast", path],
            capture_output=True, text=True, timeout=TIMEOUT,
        )
        stderr = r.stderr or ""
        has_diag = "error:" in stderr
        if r.returncode != 0:
            status = "crash"
        elif has_diag:
            status = "fail"
        else:
            status = "success"
        first_err = next(
            (ln.strip() for ln in stderr.splitlines() if "error:" in ln), ""
        )
        return {"status": status, "stderr": stderr, "first_err": first_err}
    except subprocess.TimeoutExpired:
        return {"status": "crash", "stderr": "<timeout>", "first_err": "<timeout>"}


def norm_crash(msg: str) -> str:
    """Normalize a crash line for clustering: strip paths/line numbers."""
    s = msg
    s = re.sub(r"0x[0-9a-fA-F]+", "0x..", s)
    s = re.sub(r"at [^ ]+\.rs:\d+:\d+", "at <file>.rs:<l>:<c>", s)
    s = re.sub(r"\d+", "<n>", s)
    return s[:200]


def crash_clue(stderr: str) -> str:
    """Pick the most distinctive line of a crash output."""
    for ln in stderr.splitlines():
        if "panicked" in ln or "Segmentation fault" in ln or "abort" in ln:
            return ln.strip()
    for ln in stderr.splitlines():
        if ln.strip():
            return ln.strip()
    return "<empty stderr>"


def norm_err(msg: str) -> str:
    """Normalize a diagnostic line for clustering: strip positions/numbers."""
    s = re.sub(r" at \d+:\d+$", "", msg)
    s = re.sub(r"\d+", "<n>", s)
    return s.strip()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--frontend", default=str(DEFAULT_FRONTEND))
    ap.add_argument("--llt-dir", default=str(DEFAULT_LLT_DIR))
    ap.add_argument("--jobs", "-j", type=int, default=0,
                    help="parallel workers (default: min(32, cpu_count))")
    ap.add_argument("--out", default=str(REPO_ROOT / "tools" / "llt_baseline.txt"))
    args = ap.parse_args()

    frontend = str(Path(args.frontend).resolve())
    llt_root = Path(args.llt_dir)
    if not Path(frontend).exists():
        print(f"error: frontend binary not found: {frontend}", file=sys.stderr)
        return 2
    if not llt_root.is_dir():
        print(f"error: llt dir not found: {llt_root}", file=sys.stderr)
        return 2

    targets: list[tuple[str, str]] = []  # (dir, file)
    skipped: list[str] = []
    for d in SCOPE_DIRS:
        dd = llt_root / d
        for p in sorted(dd.rglob("*.cj")):
            if p.is_file():
                targets.append((d, str(p)))
            else:
                skipped.append(str(p))
    if not targets:
        print(f"error: no .cj files under {llt_root}/{SCOPE_DIRS}", file=sys.stderr)
        return 2
    print(f"[llt] {len(targets)} .cj files under {llt_root} "
          f"({', '.join(SCOPE_DIRS)}); running {Path(frontend).name} --dump-ast",
          file=sys.stderr)
    if skipped:
        print(f"[llt] skipped {len(skipped)} non-regular .cj paths: "
              f"{', '.join(skipped)}", file=sys.stderr)

    jobs = args.jobs or min(32, os.cpu_count() or 1)
    results: dict[str, dict] = {}
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        futures = {pool.submit(classify, t, frontend): (d, t) for d, t in targets}
        done = 0
        for fut in as_completed(futures):
            d, t = futures[fut]
            try:
                results[t] = fut.result()
            except Exception as e:  # noqa: BLE001
                results[t] = {"status": "crash", "stderr": f"<runner error {e}>",
                              "first_err": f"<runner error {e}>"}
            done += 1
            if done % 1000 == 0 or done == len(targets):
                print(f"[llt] {done}/{len(targets)}", file=sys.stderr)

    total = len(targets)
    cnt = Counter(r["status"] for r in results.values())
    success, fail, crash = cnt["success"], cnt["fail"], cnt["crash"]
    rate = success / total * 100 if total else 0.0

    # per-directory breakdown
    per_dir: dict[str, Counter] = {d: Counter() for d in SCOPE_DIRS}
    for d, t in targets:
        per_dir[d][results[t]["status"]] += 1

    # crash clusters
    crash_files = sorted(
        (t for d, t in targets if results[t]["status"] == "crash")
    )
    crash_clusters = Counter(
        norm_crash(crash_clue(results[t]["stderr"])) for t in crash_files
    )

    # failure samples: first 50 'fail' files by path
    fail_samples = sorted(
        (t for d, t in targets if results[t]["status"] == "fail")
    )[:50]

    lines: list[str] = []
    bar = "=" * 78
    lines.append(bar)
    lines.append("cj-lang LLT frontend regression baseline (T19)")
    lines.append(f"generated: {datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    lines.append(f"frontend:  {frontend}")
    lines.append(f"commit:    {git_short_head()}")
    lines.append(f"llt dir:   {llt_root}")
    lines.append(f"scope:     LLT/compiler/{{{','.join(SCOPE_DIRS)}}} "
                 f"({total} .cj files)")
    lines.append(f"mode:      cj-frontend --dump-ast")
    lines.append(bar)
    lines.append("")
    lines.append(
        f"total={total}  success={success}  fail={fail}  crash={crash}  "
        f"success-rate={rate:.1f}%"
    )
    lines.append("")
    lines.append("[by directory]")
    for d in SCOPE_DIRS:
        c = per_dir[d]
        n = sum(c.values())
        lines.append(
            f"  {d:<10} {n:>5}  success={c['success']:>5}  "
            f"fail={c['fail']:>5}  crash={c['crash']:>3}"
        )
    lines.append("")

    lines.append("[crash clusters]")
    if crash_files:
        for msg, n in crash_clusters.most_common(20):
            ex = next(t for t in crash_files
                      if norm_crash(crash_clue(results[t]["stderr"])) == msg)
            lines.append(f"  {n:>3}  {msg}   e.g. {ex}")
        lines.append(f"  (total {len(crash_files)} crash files)")
    else:
        lines.append("  none — zero crash files")
    lines.append("")

    lines.append("[failure samples — first 50 (sorted by path)]")
    for t in fail_samples:
        err = results[t]["first_err"] or "(no error line)"
        lines.append(f"  {t[len(str(llt_root))+1:]}  | {err}")
    if not fail_samples:
        lines.append("  none")
    lines.append("")

    lines.append("[failure message clusters — top 25 (normalized first diag)]")
    fail_clusters = Counter(
        norm_err(results[t]["first_err"])
        for d, t in targets if results[t]["status"] == "fail"
        and results[t]["first_err"]
    )
    if fail_clusters:
        for msg, n in fail_clusters.most_common(25):
            ex = next(
                (t for d, t in targets if results[t]["status"] == "fail"
                 and norm_err(results[t]["first_err"]) == msg), ""
            )
            lines.append(f"  {n:>4}  {msg}   e.g. {ex[len(str(llt_root))+1:]}")
    else:
        lines.append("  none")
    lines.append("")

    if crash_files:
        lines.append("[crash files]")
        for t in crash_files[:100]:
            lines.append(f"  {t[len(str(llt_root))+1:]}  | "
                         f"{crash_clue(results[t]['stderr'])}")
        lines.append("")
        lines.append(
            "  NOTE: high-frequency crash pattern(s) above should be filed "
            "as a follow-up task to fix cj-frontend — T19 is evaluation-only."
        )
        lines.append("")

    lines.append("note: success = exit 0 + no parser diagnostic; "
                 "fail = exit 0 + diagnostic(s); crash = non-zero exit/timeout.")
    lines.append("note: dump-ast renders every parser diag (incl. warnings) as "
                 "'error:' on stderr; fail may include warning-only files.")
    if skipped:
        lines.append(f"note: {len(skipped)} non-regular '.cj' paths excluded "
                     f"(directories, not files): {', '.join(skipped)}")
    lines.append("note: evaluation task — no cj-frontend core logic changed (T19).")
    lines.append("")

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text("\n".join(lines), encoding="utf-8")
    print("\n".join(lines))
    print(f"\n[written] {out_path}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())