#!/usr/bin/env python3
"""Batch-run official signatureHelp cjlsp cases via lsp_test.py (parallel)."""
import os, sys, glob, json, concurrent.futures
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from run_feature_cases import run_one, RESULTS_ROOT, BASE

CASES = sorted(glob.glob(f"{BASE}/testcases/autotestcase/signatureHelp/*.info"))
print(f"signatureHelp 用例: {len(CASES)} (并行)")

results = []
with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
    futs = {ex.submit(run_one, info): info for info in CASES}
    for i, f in enumerate(concurrent.futures.as_completed(futs)):
        feature, name, status, note = f.result()
        results.append({"case": name, "status": status, "note": note})
        if status == "FAIL" or (i + 1) % 10 == 0:
            print(f"  [{i+1}/{len(CASES)}] {name}: {status} {note[:40]}", flush=True)

passed = sum(1 for r in results if r["status"] == "PASS")
print(f"\n=== signatureHelp 结果: {passed}/{len(CASES)} ({passed/len(CASES)*100:.1f}%) ===")
fails = [r for r in results if r["status"] != "PASS"]
if fails:
    print(f"失败 {len(fails)} 例:")
    for r in fails[:12]:
        print(f"  {r['case']}: {r['status']} {r['note'][:60]}")
else:
    print("全部通过！")
os.makedirs(RESULTS_ROOT, exist_ok=True)
json.dump({"feature": "signatureHelp", "passed": passed, "total": len(CASES), "results": results},
          open(os.path.join(RESULTS_ROOT, "signatureHelp_summary.json"), "w"), indent=2)
