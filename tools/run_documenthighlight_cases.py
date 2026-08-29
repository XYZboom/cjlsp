#!/usr/bin/env python3
"""Batch-run official documentHighlight cjlsp cases via lsp_test.py."""
import os, sys, glob, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from run_feature_cases import run_one, RESULTS_ROOT, BASE, SUMMARY

CASES = sorted(glob.glob(f"{BASE}/testcases/autotestcase/documentHighlight/*.info"))
print(f"documentHighlight 用例: {len(CASES)}")

results = []
for i, info in enumerate(CASES):
    feature, name, status, note = run_one(info)
    results.append({"case": name, "status": status, "note": note})
    if (i + 1) % 20 == 0 or status == "FAIL":
        print(f"  [{i+1}/{len(CASES)}] {name}: {status} {note[:40]}")

passed = sum(1 for r in results if r["status"] == "PASS")
print(f"\n=== documentHighlight 结果: {passed}/{len(CASES)} ({passed/len(CASES)*100:.1f}%) ===")
fails = [r for r in results if r["status"] != "PASS"]
print(f"失败 {len(fails)} 例:")
for r in fails[:10]:
    print(f"  {r['case']}: {r['status']} {r['note'][:60]}")

json.dump({"feature": "documentHighlight", "passed": passed, "total": len(CASES), "results": results},
          open(os.path.join(RESULTS_ROOT, "documentHighlight_summary.json"), "w"), indent=2)
