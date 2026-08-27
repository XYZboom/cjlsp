#!/usr/bin/env python3
"""Batch-run official cjlsp completion + hover cases and report a pass-rate baseline.

Usage:
  python3 tools/run_feature_cases.py [--workers N] [--feature completion|hover] [--limit N]
  python3 tools/run_feature_cases.py --report-only     # regenerate baseline from last run

- Runs each .info case via lsp_test.py in its own temp dir under the cjlsp dir
  (freopen.in/out, compare_result.txt are per-case, so N workers run in parallel).
- Sets CANGJIE_STDX_PATH to dodge the framework's env_info-None crash (lsp_test.py
  line 124) that fires AFTER "testcase pass!" is written to compare_result.txt.
- Pass/fail is judged by grep 'testcase pass' in compare_result.txt (returncode
  is unreliable because of that same framework bug).
- Failed cases keep their artifacts under <cjlsp>/.batch_results/<feature>/<case>/
  for later debugging; passing cases are cleaned up.
- Per-run pass/fail is persisted to <cjlsp>/.batch_results/run_summary.json so
  --report-only can rebuild tools/feature_baseline.txt without re-running cases.
- Writes a human-readable baseline (stats + failed list + failure-cluster
  analysis) to tools/feature_baseline.txt.
"""
import os
import re
import sys
import json
import glob
import shutil
import argparse
import subprocess
import tempfile
import concurrent.futures
from datetime import datetime

BASE = "/root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp"
LSP_TEST = os.path.join(BASE, "lsp_test.py")
# Allow a worktree-local config override (parallel workers race on the shared
# lsp_config.txt — point CFG at a private copy via CJLSP_CONFIG env).
CFG = os.environ.get("CJLSP_CONFIG", os.path.join(BASE, "lsp_config.txt"))
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # cj-lang repo
BASELINE_OUT = os.path.join(REPO, "tools", "feature_baseline.txt")
RESULTS_ROOT = os.path.join(BASE, ".batch_results")
SUMMARY = os.path.join(RESULTS_ROOT, "run_summary.json")


def run_one(info):
    """Run a single .info case. Returns (feature, case_name, status, note)."""
    name = os.path.basename(info)
    feature = os.path.basename(os.path.dirname(info))
    td = tempfile.mkdtemp(prefix=".batch_", dir=BASE)
    try:
        shutil.copy(CFG, os.path.join(td, "lsp_config.txt"))
        env = dict(os.environ, CANGJIE_STDX_PATH="/nonexistent")
        try:
            r = subprocess.run(
                [sys.executable, LSP_TEST, info],
                capture_output=True, text=True, timeout=90, env=env, cwd=td,
            )
        except subprocess.TimeoutExpired:
            return (feature, name, "FAIL", "timeout")
        except Exception as e:  # noqa: BLE001
            return (feature, name, "FAIL", f"run-error: {e}")
        cr = os.path.join(td, "compare_result.txt")
        if os.path.exists(cr):
            with open(cr, encoding="utf-8", errors="replace") as fh:
                text = fh.read()
        else:
            text = r.stdout or ""
        if "testcase pass" in text:
            shutil.rmtree(td, ignore_errors=True)
            return (feature, name, "PASS", "")
        # keep artifacts of failed cases for later debugging
        dst = os.path.join(RESULTS_ROOT, feature, name)
        if os.path.isdir(dst):
            shutil.rmtree(dst, ignore_errors=True)
        os.makedirs(dst, exist_ok=True)
        for f in ("compare_result.txt", "freopen.out", "freopen.in", "log.txt"):
            src = os.path.join(td, f)
            if os.path.exists(src):
                shutil.copy(src, dst)
        shutil.rmtree(td, ignore_errors=True)
        return (feature, name, "FAIL", "compare mismatch or no pass marker")
    except Exception as e:  # noqa: BLE001
        shutil.rmtree(td, ignore_errors=True)
        return (feature, name, "FAIL", f"outer-error: {e}")


def _recv_json(cr_path):
    """Return (cluster, note) for a failed case's compare_result.txt."""
    with open(cr_path, encoding="utf-8", errors="replace") as fh:
        text = fh.read()
    if "testcase pass" in text:
        return "PASS", ""
    m = re.search(r"receive message:\r?\n(.*?)(?:\r?\n-{20,}|\Z)", text, re.S)
    recv = m.group(1).strip() if m else ""
    if not recv:
        return "no-receive", "server produced no response in compare_result"
    if recv == '""':
        # framework logged write_compare_log(receive_json="") → the expected
        # response id never arrived from the server (no response / crash)
        return "no-response-(expected-id-missing)", "expected response id never received"
    try:
        j = json.loads(recv)
    except Exception:
        try:
            j = json.loads(recv.strip('"'))
        except Exception:
            return "unparseable", "receive message not JSON-parseable"
    if isinstance(j, str):
        try:
            j = json.loads(j)
        except Exception:
            return "unparseable", "receive message not JSON-parseable"
    if not isinstance(j, dict):
        return f"non-dict({type(j).__name__})", "receive is not a JSON object"
    res = j.get("result")
    if res is None:
        return "result-null", "server returned result:null"
    if res == []:
        return "result-empty[]", "server returned an empty list"
    if isinstance(res, list):
        return "result-nonempty-list", "server returned items but mismatch (richness/order)"
    if isinstance(res, dict) and res.get("contents") is None:
        return "result-null-contents", "hover contents null"
    return "result-other", "other mismatch"


def analyze_clusters(feature):
    """Count failure clusters from preserved artifacts. Returns {cluster: [names]}."""
    clusters = {}
    for d in sorted(glob.glob(os.path.join(RESULTS_ROOT, feature, "*"))):
        name = os.path.basename(d)
        cr = os.path.join(d, "compare_result.txt")
        if not os.path.exists(cr) or os.path.getsize(cr) == 0:
            # compare_result.txt empty: framework crashed before writing. Check the
            # .info's expected Rev# — official expects `result: null` but our server
            # returns `[]`, and json_compare's `len(None)` raises TypeError.
            info = os.path.join(BASE, "testcases", "autotestcase", feature, name)
            expects_null = False
            try:
                txt = open(info, encoding="utf-8").read()
                m = re.search(r"Rev#\r?\n(.*)", txt, re.S)
                if m:
                    for line in m.group(1).splitlines():
                        line = line.strip()
                        if line.startswith("{"):
                            j = json.loads(line)
                            if j.get("result") is None:
                                expects_null = True
            except Exception:  # noqa: BLE001
                pass
            c = ("expected-null-vs-[]-harness-crash" if expects_null
                 else "no-compare-file")
        else:
            c, _ = _recv_json(cr)
        clusters.setdefault(c, []).append(name)
    return clusters


def build_baseline(results, commit):
    lines = []
    lines.append("=" * 78)
    lines.append("cj-lang LSP feature baseline (T16)")
    lines.append(f"generated: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    lines.append(f"LSPServer: {os.path.join(REPO, 'target', 'debug', 'LSPServer')}")
    lines.append(f"commit:    {commit}")
    lines.append("=" * 78)
    for feat in ("completion", "hover"):
        fr = [r for r in results if r[0] == feat]
        if not fr:
            continue
        npass = sum(1 for r in fr if r[2] == "PASS")
        lines.append("")
        lines.append(f"[{feat}]  pass={npass}  fail={len(fr) - npass}  total={len(fr)}  "
                     f"rate={npass / len(fr) * 100:.1f}%")
        fails = [r for r in fr if r[2] != "PASS"]
        if fails:
            lines.append("  failed:")
            for r in fails:
                lines.append(f"    {r[1]}  ({r[3]})")
            clusters = analyze_clusters(feat)
            if clusters:
                lines.append("  failure clusters (from preserved artifacts):")
                for c, names in sorted(clusters.items(), key=lambda kv: -len(kv[1])):
                    lines.append(f"    {c}: {len(names)}  e.g. {', '.join(names[:4])}")
    lines.append("")
    lines.append("note: judgment = 'testcase pass' marker in compare_result.txt "
                 "(framework env_info-None crash after writing is ignored).")
    lines.append("note: failed-case artifacts kept under cjlsp/.batch_results/.")
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--feature", choices=["completion", "hover"], default=None,
                    help="only run one feature (default: both)")
    ap.add_argument("--limit", type=int, default=None, help="max cases per feature (smoke test)")
    ap.add_argument("--report-only", action="store_true",
                    help="skip running cases; rebuild baseline from run_summary.json + artifacts")
    args = ap.parse_args()

    commit = subprocess.run(
        ["git", "-C", REPO, "rev-parse", "--short", "HEAD"],
        capture_output=True, text=True).stdout.strip()

    if args.report_only:
        with open(SUMMARY, encoding="utf-8") as fh:
            results = [tuple(r) for r in json.load(fh)]
        results.sort(key=lambda r: (r[0], r[1]))
        text = build_baseline(results, commit)
        with open(BASELINE_OUT, "w", encoding="utf-8") as fh:
            fh.write(text)
        print(text)
        print(f"baseline written to {BASELINE_OUT}")
        return

    features = ["completion", "hover"] if args.feature is None else [args.feature]
    infos = []
    for feat in features:
        hits = sorted(glob.glob(os.path.join(BASE, "testcases", "autotestcase", feat, "*.info")))
        if args.limit:
            hits = hits[: args.limit]
        infos.extend(hits)

    total = len(infos)
    print(f"total cases to run: {total}  (workers={args.workers})")
    results = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as ex:
        futs = {ex.submit(run_one, i): i for i in infos}
        done = 0
        for fut in concurrent.futures.as_completed(futs):
            done += 1
            r = fut.result()
            results.append(r)
            if r[2] == "PASS":
                print(f"[{done}/{total}] PASS  {r[0]}/{r[1]}")
            else:
                print(f"[{done}/{total}] FAIL  {r[0]}/{r[1]}  ({r[3]})", flush=True)

    results.sort(key=lambda r: (r[0], r[1]))
    os.makedirs(RESULTS_ROOT, exist_ok=True)
    with open(SUMMARY, "w", encoding="utf-8") as fh:
        json.dump(results, fh)

    text = build_baseline(results, commit)
    with open(BASELINE_OUT, "w", encoding="utf-8") as fh:
        fh.write(text)
    print()
    print(text)
    print(f"baseline written to {BASELINE_OUT}")


if __name__ == "__main__":
    main()
