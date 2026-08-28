import os
#!/usr/bin/env python3
"""Debug: for failing cases, show expected vs actual diagnostics side by side."""
import json, os, re, subprocess, sys

BASE = os.environ.get("CANGJIE_TEST_BASE", "/root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp")
HERE = os.path.dirname(os.path.abspath(__file__))
LSPSERVER = os.path.join(os.path.dirname(HERE), "target", "debug", "LSPServer")
CWD = os.path.join(BASE, "sourcecode/cangjieTest")
# LSPSERVER = "/root/Code/cangjie/cj-lang/target/debug/LSPServer"

def frame(obj):
    body = json.dumps(obj, separators=(",", ":"))
    return f"Content-Length: {len(body)}\r\n\r\n{body}"

def run_server(reqs):
    data = "".join(frame(r) for r in reqs)
    p = subprocess.Popen([LSPSERVER, "--test", "--disableAutoImport"],
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=CWD)
    out, err = p.communicate(data.encode(), timeout=30)
    frames = []
    raw = out.decode()
    pos = 0
    while True:
        hl = raw.find("\r\n\r\n", pos)
        if hl == -1: break
        header = raw[pos:hl]
        ln = None
        for line in header.split("\r\n"):
            if line.startswith("Content-Length:"):
                ln = int(line.split(":")[1])
        if ln is None: break
        body = raw[hl+4:hl+4+ln]
        try: frames.append(json.loads(body))
        except Exception: pass
        pos = hl + 4 + ln
    return frames, err.decode()

def expected_from_info(path):
    content = open(path, encoding="utf-8", errors="replace").read()
    rev = content.split("Rev#")[1]
    diags = []
    for line in rev.strip().split("\n"):
        line = line.strip()
        if not line: continue
        try: obj = json.loads(line)
        except Exception: continue
        if obj.get("method") == "textDocument/publishDiagnostics":
            for d in obj["params"]["diagnostics"]:
                rng = d.get("range", {})
                diags.append({
                    "msg": d["message"],
                    "line": rng.get("start", {}).get("line"),
                    "char": rng.get("start", {}).get("character"),
                    "sev": d.get("severity"),
                })
    return diags, content

cases = sys.argv[1:] if len(sys.argv) > 1 else ["007", "008", "012", "014", "018", "020"]
for num in cases:
    info = os.path.join(BASE, "testcases/autotestcase/diagnostics", f"textDocument_diagnostics_{num}.info")
    if not os.path.exists(info): continue
    expected, content = expected_from_info(info)
    m = re.search(r'"textDocument/didOpen"[^}]*?"uri":\s*"([^"]+)"', content)
    rel_uri = m.group(1)
    abs_uri = "file:///" + os.path.join(BASE, rel_uri)
    real_file = os.path.join(BASE, "sourcecode", "cangjieTest", rel_uri)
    try: text = open(real_file, encoding="utf-8", errors="replace").read()
    except FileNotFoundError: text = ""
    print(f"\n##### {num} :: {rel_uri}")
    for i, line in enumerate(text.split("\n"), 1):
        print(f"{i:3}| {line}")
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
            try: obj = json.loads(line)
            except Exception: continue
            if obj.get("method") == "textDocument/didChange":
                p = obj.get("params")
                if isinstance(p, dict) and "textDocument" in p:
                    td = dict(p.get("textDocument") or {}); td["uri"] = abs_uri
                    p = dict(p); p["textDocument"] = td
                    reqs.append({"jsonrpc": "2.0", "method": "textDocument/didChange", "params": p})
    reqs.append({"jsonrpc": "2.0", "id": 10, "method": "shutdown", "params": {}})
    frames, err = run_server(reqs)
    actual = []
    for f in frames:
        if f.get("method") == "textDocument/publishDiagnostics":
            for d in f["params"]["diagnostics"]:
                rng = d.get("range", {})
                actual.append((d["message"], rng.get("start", {}).get("line"), rng.get("start", {}).get("character")))
    print(f"--- expected ({len(expected)}) vs actual ({len(actual)})")
    for e in expected:
        tag = "OK "
        ln, ch = e["line"], e["char"]
        for a_msg, a_line, a_char in actual:
            if e["msg"] == a_msg and e["line"] == a_line and e["char"] == a_char:
                tag = "HIT"
                break
        print(f"[{tag}] L{ln}:{ch} sev={e['sev']} {e['msg']}")
    print("  ACTUAL:")
    for a_msg, a_line, a_char in actual:
        print(f"      L{a_line}:{a_char} {a_msg}")
    if err.strip():
        print("  STDERR:", err.strip()[:500])