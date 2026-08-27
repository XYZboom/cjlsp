#!/usr/bin/env python3
"""E2E: verify macro expansion diagnostics through our LSPServer."""
import json, os, subprocess, re, sys

SERVER = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "LSPServer")
SRC = 'let x = @sourceLine()\nlet y = @NoSuchMacro(1)\nmain() {}\n'

def frame(obj):
    body = json.dumps(obj, separators=(",", ":"))
    return f"Content-Length: {len(body)}\r\n\r\n{body}"

msgs = [
    {"jsonrpc": "2.0", "id": "0", "method": "initialize", "params": {"processId": None, "rootUri": "file:///tmp", "capabilities": {}}},
    {"jsonrpc": "2.0", "method": "initialized", "params": {}},
    {"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {"uri": "file:///tmp/macro_test.cj", "languageId": "Cangjie", "version": 1, "text": SRC}}},
]
req = "".join(frame(m) for m in msgs).encode()
p = subprocess.run([SERVER], input=req, capture_output=True, cwd="/tmp", env=dict(os.environ, CANGJIE_STDX_PATH="/nonexistent"))
out = p.stdout.decode("utf-8", "replace")

# Split on Content-Length frames, parse each JSON body.
diags = []
frames = re.split(r'Content-Length: \d+\r\n\r\n', out)
for body in frames:
    body = body.strip()
    if not body:
        continue
    try:
        obj = json.loads(body)
    except Exception:
        continue
    if obj.get("method") == "textDocument/publishDiagnostics":
        diags.extend(obj["params"]["diagnostics"])

print(f"=== 收到 {len(diags)} 条诊断 ===")
for d in diags:
    print(f"  L{d['range']['start']['line']+1}:{d['range']['start']['character']+1} [{d.get('severity')}] {d['message'][:100]}")

# Assertions: unresolved macro MUST be reported (the point of the test).
unresolved = [d for d in diags if "unresolved macro" in d.get("message", "")]
if unresolved:
    print("PASS: unresolved macro reported")
    sys.exit(0)
else:
    print("FAIL: unresolved macro not reported")
    sys.exit(1)