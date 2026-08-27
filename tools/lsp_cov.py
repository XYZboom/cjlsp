#!/usr/bin/env python3
"""Fast cjlsp diagnostics coverage check.

For each .info in the diagnostics suite, extract expected diagnostics from
Rev# and actual diagnostics from our LSPServer, and report how many expected
message+range pairs we match. Faster than lsp_test.py (no full JSON compare).
"""
import glob
import json
import os
import re
import subprocess
import sys

BASE = "/root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp"
# Resolve the server binary relative to this script so the checker tests the
# checkout it lives in (main repo or a worktree), not a hardcoded path.
HERE = os.path.dirname(os.path.abspath(__file__))
LSPSERVER = os.path.join(os.path.dirname(HERE), "target", "debug", "LSPServer")
CWD = os.path.join(BASE, "sourcecode/cangjieTest")


def frame(obj):
    body = json.dumps(obj, separators=(",", ":"))
    return f"Content-Length: {len(body)}\r\n\r\n{body}"


def run_server(reqs):
    data = "".join(frame(r) for r in reqs)
    p = subprocess.Popen(
        [LSPSERVER, "--test", "--disableAutoImport"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        cwd=CWD,
    )
    out, _ = p.communicate(data.encode(), timeout=30)
    # parse frames
    frames = []
    pos = 0
    raw = out.decode()
    while True:
        hl = raw.find("\r\n\r\n", pos)
        if hl == -1:
            break
        header = raw[pos:hl]
        ln = None
        for line in header.split("\r\n"):
            if line.startswith("Content-Length:"):
                ln = int(line.split(":")[1])
        if ln is None:
            break
        body = raw[hl + 4 : hl + 4 + ln]
        try:
            frames.append(json.loads(body))
        except Exception:
            pass
        pos = hl + 4 + ln
    return frames


def expected_from_info(path):
    content = open(path, encoding="utf-8", errors="replace").read()
    if "Rev#" not in content:
        return [], None
    rev = content.split("Rev#")[1]
    diags = []
    for line in rev.strip().split("\n"):
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except Exception:
            continue
        if obj.get("method") == "textDocument/publishDiagnostics":
            for d in obj["params"]["diagnostics"]:
                rng = d.get("range", {})
                diags.append({
                    "msg": d["message"],
                    "line": rng.get("start", {}).get("line"),
                    "char": rng.get("start", {}).get("character"),
                })
    return diags, content


def main():
    infos = sorted(glob.glob(os.path.join(BASE, "testcases/autotestcase/diagnostics/*.info")))
    total = 0
    matched = 0
    per_case = []
    for info in infos:
        name = os.path.basename(info).replace(".info", "")
        expected, content = expected_from_info(info)
        if not expected:
            continue
        # extract didOpen uri
        m = re.search(r'"textDocument/didOpen"[^}]*?"uri":\s*"([^"]+)"', content)
        if not m:
            continue
        rel_uri = m.group(1)
        # The harness sends a virtual absolute uri (cjlsp/diagnosticsTest/...,
        # which is NOT on disk) plus the real file content in `text`. The file
        # actually lives under sourcecode/cangjieTest/.
        abs_uri = "file:///" + os.path.join(BASE, rel_uri)
        real_file = os.path.join(BASE, "sourcecode", "cangjieTest", rel_uri)
        try:
            text = open(real_file, encoding="utf-8", errors="replace").read()
        except FileNotFoundError:
            text = ""
        reqs = [
            {"jsonrpc": "2.0", "id": "0", "method": "initialize", "params": {"processId": None, "rootUri": os.path.join(BASE, "diagnosticsTest"), "capabilities": {}}},
            {"jsonrpc": "2.0", "method": "initialized", "params": {}},
            {"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {"uri": abs_uri, "languageId": "Cangjie", "version": 1, "text": text}}},
            {"jsonrpc": "2.0", "id": 10, "method": "shutdown", "params": {}},
        ]
        try:
            frames = run_server(reqs)
        except Exception as e:
            per_case.append((name, 0, len(expected), str(e)))
            continue
        actual = []
        for f in frames:
            if f.get("method") == "textDocument/publishDiagnostics":
                for d in f["params"]["diagnostics"]:
                    rng = d.get("range", {})
                    actual.append((d["message"], rng.get("start", {}).get("line"), rng.get("start", {}).get("character")))
        # match: expected msg at expected line+char present in actual
        case_match = 0
        for e in expected:
            for a_msg, a_line, a_char in actual:
                if e["msg"] == a_msg and e["line"] == a_line and e["char"] == a_char:
                    case_match += 1
                    break
        total += len(expected)
        matched += case_match
        per_case.append((name, case_match, len(expected), ""))
        print(f"  {name}: {case_match}/{len(expected)}")

    print(f"\n=== 总计: {matched}/{total} ({matched/total*100:.1f}%) ===")
    # save for later diffing
    with open("/tmp/diag_cov.txt", "w") as f:
        for name, cm, ce, err in per_case:
            f.write(f"{name}\t{cm}/{ce}\t{err}\n")


if __name__ == "__main__":
    main()
