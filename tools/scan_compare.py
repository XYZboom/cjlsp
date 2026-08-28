#!/usr/bin/env python3
"""SCAN-block comparison harness for our Rust frontend.

Extracts /* SCAN ... */ blocks from cangjie_test .cj files (the official
expected diagnostic output) and compares against `cj-frontend` stderr output.

Maple semantics (compare.py): only a SINGLE compare target is exercised. We
implement the common case: the first `| compare %f` pipe, comparing our
frontend's stderr against the SCAN / SCAN-IN block.

Usage:
  python3 tools/scan_compare.py <file.cj>...
  python3 tools/scan_compare.py --dir <Parser-dir>   # all *.cj under dir
"""
from __future__ import annotations
import argparse
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

import os as _os
FRONTEND = _os.environ.get("CJ_FRONTEND", "/root/Code/cangjie/cj-lang/target/debug/cj-frontend")

# SCAN block: /* SCAN ... */ or /* SCAN-IN ... */ or /* SCAN-OUT ... */
SCAN_RE = re.compile(r"/\*\s*(SCAN(?:-IN|-OUT|-TXT)?)\s*\n(.*?)\*/", re.DOTALL)


def extract_scans(text: str) -> list[dict]:
    """Return list of {kind, body} for each SCAN block."""
    out = []
    for m in SCAN_RE.finditer(text):
        out.append({"kind": m.group(1), "body": m.group(2)})
    return out


def run_frontend(path: str) -> str:
    """Run cj-frontend, return stderr (diagnostics) only."""
    try:
        r = subprocess.run(
            [FRONTEND, path], capture_output=True, text=True, timeout=15
        )
        return r.stderr
    except subprocess.TimeoutExpired:
        return "<timeout>"


def normalize_expected(body: str) -> list[str]:
    """Lines of the expected block significant for comparison."""
    lines = []
    for line in body.splitlines():
        s = line.rstrip()
        # skip pure separator lines in expectation but keep structure info
        lines.append(s)
    return lines


def compare_one(path: str, verbose: bool = False) -> dict:
    src = Path(path).read_text(encoding="utf-8", errors="replace")
    scans = extract_scans(src)
    if not scans:
        return {"file": path, "scan": False, "matched": 0, "total": 0}
    actual = run_frontend(path)
    # Our output deliberately avoids absolute-path noise? compare backend uses
    # the filename passed; Maple compares against <basename>. We normalize both.
    actual_norm = actual.replace(path, Path(path).name)

    # Normalized actual lines (non-empty)
    act_lines = [l.rstrip() for l in actual_norm.splitlines() if l.strip()]

    matched = 0
    total_lines = 0
    for scan in scans:
        body = scan["body"]
        # Skip files whose SCAN uses named-arg style with `%` substitution etc.
        # We only check exact-line matching of the diagnostic header lines.
        exp_lines = [
            l.rstrip()
            for l in body.splitlines()
            if l.strip() and not l.strip().startswith(("==>", "|", "#"))
        ]
        # also include ==> lines (they carry file:line:col)
        exp_all = [
            l.rstrip()
            for l in body.splitlines()
            if l.strip()
        ]
        total_lines += len(exp_all)
        for el in exp_all:
            if el in act_lines:
                matched += 1
            else:
                if verbose:
                    print(f"    MISS: {el!r}")
    return {
        "file": path,
        "scan": True,
        "matched": matched,
        "total": total_lines,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="+")
    ap.add_argument("--dir", action="store_true", help="treat args as directories")
    ap.add_argument("--verbose", "-v", action="store_true")
    ap.add_argument("--jobs", "-j", type=int, default=0,
                    help="parallel workers (default: min(8, cpu_count))")
    args = ap.parse_args()

    targets: list[str] = []
    if args.dir:
        for d in args.files:
            for p in sorted(Path(d).rglob("*.cj")):
                targets.append(str(p))
    else:
        targets = args.files

    # Parallel execution (subprocess-bound; GIL irrelevant). Deterministic
    # output: results are keyed by path and sorted before printing.
    jobs = args.jobs or min(8, (os.cpu_count() or 1))
    results_map: dict[str, dict] = {}
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        futures = {pool.submit(compare_one, t, args.verbose): t for t in targets}
        for fut in as_completed(futures):
            t = futures[fut]
            try:
                results_map[t] = fut.result()
            except Exception as e:  # noqa: BLE001 - keep going on per-file errors
                print(f"error on {t}: {e}", file=sys.stderr)

    total_match = 0
    total_lines = 0
    scan_files = 0
    results = []
    for t in targets:  # deterministic order
        r = results_map.get(t)
        if not r or not r["scan"]:
            continue
        scan_files += 1
        total_match += r["matched"]
        total_lines += r["total"]
        pct = (r["matched"] / r["total"] * 100) if r["total"] else 100
        results.append((pct, r["file"], r["matched"], r["total"]))

    results.sort()
    print(f"\nSCAN files: {scan_files}, total expected lines: {total_lines}, "
          f"matched: {total_match} ({total_match/total_lines*100:.1f}%)" if total_lines else "no scan lines")
    print("\nWorst 15:")
    for pct, f, m, t in results[:15]:
        print(f"  {pct:5.1f}%  {m:>3}/{t:<3} {f}")
    print("\nBest 5:")
    for pct, f, m, t in results[-5:]:
        print(f"  {pct:5.1f}%  {m:>3}/{t:<3} {f}")


if __name__ == "__main__":
    main()