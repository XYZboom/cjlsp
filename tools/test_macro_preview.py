#!/usr/bin/env python3
"""E2E (T14): macro-expansion preview note through the LSP.

A locally-defined `Wrap` macro expands to `print ( 42` via the quote-template
fallback (no SDK needed). Its call site `@Wrap(42)!` has a trailing `!` whose
parser diagnostic lands *inside* the expansion's source span (one column past
the closing paren). The server must attach the official cjc note:

  note: the code after the macro is expanded as follows
      /* 5.1 */print ( 42

exposed as LSP `relatedInformation` on that diagnostic.
"""
import json, os, subprocess, re, sys

SERVER = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "LSPServer"
)
SRC = """package p

public macro Wrap(x: Tokens): Tokens { quote(print(x)) }

let x = @Wrap(42)!
"""
HDR = "the code after the macro is expanded as follows"


def frame(obj):
    body = json.dumps(obj, separators=(",", ":"))
    return f"Content-Length: {len(body)}\r\n\r\n{body}"


msgs = [
    {"jsonrpc": "2.0", "id": "0", "method": "initialize", "params": {
        "processId": None, "rootUri": "file:///tmp", "capabilities": {}}},
    {"jsonrpc": "2.0", "method": "initialized", "params": {}},
    {"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
        "textDocument": {"uri": "file:///tmp/macro_preview.cj",
                         "languageId": "Cangjie", "version": 1, "text": SRC}}},
]
req = "".join(frame(m) for m in msgs).encode()
p = subprocess.run([SERVER, "--test", "--disableAutoImport"], input=req,
                   capture_output=True, cwd="/tmp",
                   env=dict(os.environ, CANGJIE_STDX_PATH="/nonexistent"))
out = p.stdout.decode("utf-8", "replace")

# Split on Content-Length frames, parse each JSON body.
diags = []
for body in re.split(r"Content-Length: \d+\r\n\r\n", out):
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
    r = d["range"]["start"]
    n = d.get("relatedInformation") or []
    print(f"  L{r['line']+1}:{r['character']+1} [{d.get('severity')}] {d['message'][:70]}")
    for it in n:
        print(f"      related: {it['message'][:80]}")

# The trailing `!` after the macro call must carry the expansion preview note.
carrier = [d for d in diags if d["message"].startswith("expected declaration, found '!'")]
if not carrier:
    print("FAIL: no 'expected declaration, found '!'' diagnostic at the macro call")
    sys.exit(1)

related = []
for d in carrier:
    related.extend((it.get("message") or "") for it in (d.get("relatedInformation") or []))
if HDR not in related:
    print(f"FAIL: expansion preview note header {HDR!r} missing")
    sys.exit(1)
preview = [m for m in related if m.startswith("/* ")]
if not preview or "print ( 42" not in preview[0]:
    print(f"FAIL: expansion preview code missing, got {preview!r}")
    sys.exit(1)
print(f"PASS: diagnostic carries expansion preview note -> {preview[0]}")
sys.exit(0)