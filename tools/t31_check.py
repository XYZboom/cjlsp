#!/usr/bin/env python3
"""T31 fast iteration: run a list of completion cases and report PASS/FAIL.

Usage: python3 tools/t31_check.py 098 099 021 ...
Runs each case via lsp_test.py (like run_feature_cases.py) but for a chosen
subset, against the worktree binary via a private config.
"""
import os, sys, shutil, subprocess, tempfile

BASE = "/root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp"
LSP_TEST = os.path.join(BASE, "lsp_test.py")
CFG = "/root/Code/cangjie/cj-lang/.worktrees/t_1f379622/tools/lsp_config_worktree.txt"
TDIR = "/tmp/t31_check"

def run_one(case):
    name = f"textDocument_completion_{case}.info"
    info = os.path.join(BASE, "testcases", "autotestcase", "completion", name)
    td = tempfile.mkdtemp(prefix=".t31_", dir=TDIR)
    try:
        shutil.copy(CFG, os.path.join(td, "lsp_config.txt"))
        env = dict(os.environ, CANGJIE_STDX_PATH="/nonexistent")
        r = subprocess.run([sys.executable, LSP_TEST, info],
                           capture_output=True, text=True, timeout=90, env=env, cwd=td)
        cr = os.path.join(td, "compare_result.txt")
        text = open(cr, encoding="utf-8", errors="replace").read() if os.path.exists(cr) else (r.stdout or "")
        ok = "testcase pass" in text
        # keep failing artifacts for diffing
        dst = os.path.join(TDIR, name)
        shutil.rmtree(dst, ignore_errors=True)
        if not ok:
            shutil.copytree(td, dst)
        shutil.rmtree(td, ignore_errors=True)
        return case, ok
    except Exception as e:
        shutil.rmtree(td, ignore_errors=True)
        return case, f"ERR {e}"

if __name__ == "__main__":
    cases = sys.argv[1:] or ["098", "099", "021"]
    os.makedirs(TDIR, exist_ok=True)
    # clear stale artifacts only for the requested cases
    results = []
    for c in cases:
        case, ok = run_one(c)
        results.append((case, ok))
        print(f"{'PASS' if ok is True else 'FAIL'}  {case}" + ("" if ok is True else f"  ({ok})"), flush=True)
    np = sum(1 for _, ok in results if ok is True)
    print(f"\n{np}/{len(results)} pass")
