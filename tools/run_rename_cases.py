#!/usr/bin/env python3
"""Batch-run official rename cjlsp cases via lsp_test.py (parallel)."""
import os, sys, glob, json, concurrent.futures
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from run_feature_cases import run_one, RESULTS_ROOT, BASE

CASES = sorted(glob.glob(f"{BASE}/testcases/autotestcase/rename/*.info"))
print(f"rename 用例: {len(CASES)} (并行)")

results = []
with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
    futs = {ex.submit(run_one, info): info for info in CASES}
    for i, f in enumerate(concurrent.futures.as_completed(futs)):
        feature, name, status, note = f.result()
        results.append({"case": name, "status": status, "note": note})
        if status == "FAIL" or (i + 1) % 20 == 0:
            print(f"  [{i+1}/{len(CASES)}] {name}: {status} {note[:40]}", flush=True)

passed = sum(1 for r in results if r["status"] == "PASS")
print(f"\n=== rename 结果: {passed}/{len(CASES)} ({passed/len(CASES)*100:.1f}%) ===")
fails = [r for r in results if r["status"] != "PASS"]
print(f"失败 {len(fails)} 例:")
for r in fails[:12]:
    print(f"  {r['case']}: {r['status']} {r['note'][:60]}")
json.dump({"feature": "rename", "passed": passed, "total": len(CASES), "results": results},
          open(os.path.join(RESULTS_ROOT, "rename_summary.json"), "w"), indent=2)
