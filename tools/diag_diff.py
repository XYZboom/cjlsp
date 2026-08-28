import os
#!/usr/bin/env python3
"""Diff expected vs actual diagnostics for specific cases (007, 020)."""
import glob, json, os, re, subprocess, sys

BASE = os.environ.get("CANGJIE_TEST_BASE", "/root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp")
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
    frames = []
    pos = 0
    raw = out.decode()
    while True:
        hl = raw.find("\r\n\r\n", pos)
        if hl == -1: break
        header = raw[pos:hl]
        ln = None
        for line in header.split("\r\n"):
            if line.startswith("Content-Length:"):
                ln = int(line.split(":")[1])
        if ln is None: break
        body = raw[hl + 4: hl + 4 + ln]
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
        if not line: continue
        if line.startswith("Req#"): break
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

cases = sys.argv[1:] if len(sys.argv) > 1 else ["007", "020"]
for case in cases:
    info = os.path.join(BASE, "testcases/autotestcase/diagnostics", f"textDocument_diagnostics_{case}.info")
    expected, content = expected_from_info(info)
    if not expected:
        print(f"{case}: no expected")
        continue
    m = re.search(r'"textDocument/didOpen"[^}]*?"uri":\s*"([^"]+)"', content)
    rel_uri = m.group(1)
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
    ]
    if "Req#" in content:
        req_block = content.split("Req#")[1].split("Rev#")[0]
        for line in req_block.strip().split("\n"):
            line = line.strip()
            if not line: continue
            try:
                obj = json.loads(line)
            except Exception:
                continue
            if obj.get("method") == "textDocument/didChange":
                p = obj.get("params")
                if isinstance(p, dict) and "textDocument" in p:
                    td = dict(p.get("textDocument") or {})
                    td["uri"] = abs_uri
                    p = dict(p)
                    p["textDocument"] = td
                    reqs.append({"jsonrpc": "2.0", "method": "textDocument/didChange", "params": p})
    reqs.append({"jsonrpc": "2.0", "id": 10, "method": "shutdown", "params": {}})
    frames = run_server(reqs)
    actual = []
    for f in frames:
        if f.get("method") == "textDocument/publishDiagnostics":
            for d in f["params"]["diagnostics"]:
                rng = d.get("range", {})
                actual.append((d["message"], rng.get("start", {}).get("line"), rng.get("start", {}).get("character")))
    print(f"=== {case} expected={len(expected)} actual={len(actual)} ===")
    print(f"  source: {rel_uri}")
    for e in expected:
        hit = any(e["msg"] == a[0] and e["line"] == a[1] and e["char"] == a[2] for a in actual)
        mark = "OK " if hit else "MISS"
        print(f"  [{mark}] exp L{e['line']}C{e['char']}: {e['msg']}")
    print("  --- actual ---")
    for a in actual:
        print(f"  act L{a[1]}C{a[2]}: {a[0]}")
    if expected and not any(True for _ in expected):
        pass
    print()