#!/usr/bin/env python3
"""Run the official completion test cases via lsp_test.py and report pass rate."""
import os, subprocess, sys, glob, shutil

BASE = "/root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp"
os.chdir(BASE)

infos = sorted(glob.glob(os.path.join(BASE, "testcases/autotestcase/completion/*.info")))
print(f"completion 用例总数: {len(infos)}")

passed, failed = 0, []
# config says test_number=4, start 1..4 — but run all and report
for info in infos:
    name = os.path.basename(info)
    # clean per-case artifacts
    for f in ("freopen.in", "freopen.out", "compare_result.txt", "log.txt"):
        if os.path.exists(f):
            try: os.remove(f)
            except: pass
    try:
        r = subprocess.run([sys.executable, "lsp_test.py", info],
                           capture_output=True, text=True, timeout=60,
                           env=dict(os.environ, CANGJIE_STDX_PATH="/nonexistent"))
        ok = r.returncode == 0
        if os.path.exists("compare_result.txt"):
            cr = open("compare_result.txt").read()
        else:
            cr = r.stdout
        # lsp_test passes if compare_result.txt has "0" / success markers
        # fallback: returncode + output inspection
        if ok or ("成功" in cr) or ("0" == cr.strip().split()[-1:][0] if cr.strip() else False):
            passed += 1
        else:
            failed.append(name)
    except Exception as e:
        failed.append(f"{name} (err {e})")

print(f"通过: {passed}/{len(infos)}")
if failed:
    print("失败:")
    for f in failed[:20]:
        print("  ", f)