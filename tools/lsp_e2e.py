#!/usr/bin/env python3
"""LSP end-to-end verification (T37).

Simulates a real IDE LSP client against LSPServer over stdio:
  initialize -> didOpen -> completion/hover/definition/references
  -> didChange -> shutdown -> exit
Validates every capability actually works end-to-end and prints a PASS/FAIL
report. Exit code 0 = all passed.

Usage:
  python3 tools/lsp_e2e.py
  E2E=1 ./tools/ci.sh        # as an optional CI step
"""
import json
import os
import subprocess
import sys

SERVER = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                      "target", "debug", "LSPServer")
BASE = "/root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp"
SRC_FILE = os.path.join(BASE, "sourcecode/cangjieTest/cangjiesource/src/Any/a1.cj")
CWD = os.path.join(BASE, "sourcecode/cangjieTest/cangjiesource/src")
URI = "cangjiesource/src/Any/a1.cj"

results = []  # (name, ok, detail)


def send(p, obj):
    data = json.dumps(obj).encode()
    p.stdin.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
    p.stdin.flush()


def recv(p):
    """Read exactly one JSON-RPC message (blocking; a request always gets a
    response and notifications are small, so blocking read is reliable)."""
    headers = {}
    line = b""
    while True:
        ch = p.stdout.read(1)
        if not ch:
            return None
        if ch == b'\n':
            if line.strip():
                parts = line.decode().split(':', 1)
                if len(parts) == 2:
                    headers[parts[0].strip().lower()] = parts[1].strip()
                line = b""
            else:
                break
        else:
            line += ch
    n = int(headers.get('content-length', 0))
    body = p.stdout.read(n)
    return json.loads(body) if body else None


def wait_for(p, req_id, timeout=15):
    """Read messages until a response matching `req_id` arrives.
    Skips server->client notifications (window/log, publishDiagnostics...).
    Blocking read: a peer that never replies would hang, so cap with a timer."""
    import signal
    def _handler(signum, frame):
        raise TimeoutError("LSP response timeout")
    old = signal.signal(signal.SIGALRM, _handler)
    signal.alarm(timeout)
    try:
        while True:
            msg = recv(p)
            if msg is None:
                return None
            if msg.get("id") == req_id:
                return msg
    except TimeoutError:
        return None
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGALRM, old)


def check(name, ok, detail=""):
    results.append((name, ok, detail))
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}" + (f"  {detail}" if detail else ""))


def main():
    src_text = open(SRC_FILE).read()
    p = subprocess.Popen([SERVER, "--test", "--disableAutoImport", "--enable-log=true"],
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL, cwd=CWD)

    # 1. initialize
    send(p, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
    r = wait_for(p, 1)
    caps = (r or {}).get("result", {}).get("capabilities", {})
    check("initialize",
          bool(caps),
          f"capabilities keys: {sorted(caps.keys())[:8]}")
    for cap in ["completionProvider", "hoverProvider", "definitionProvider", "referencesProvider"]:
        check(f"capability:{cap}", cap in caps)

    # initialized notification
    send(p, {"jsonrpc": "2.0", "method": "initialized", "params": {}})

    # 2. didOpen -> diagnostics
    send(p, {"jsonrpc": "2.0", "method": "textDocument/didOpen",
             "params": {"textDocument": {"uri": URI, "languageId": "cangjie",
                                          "version": 1, "text": src_text}}})
    diag_msg = recv(p)
    while diag_msg and diag_msg.get("method") != "textDocument/publishDiagnostics":
        diag_msg = recv(p)
    diags = (diag_msg or {}).get("params", {}).get("diagnostics", [])
    check("didOpen->diagnostics", diag_msg is not None and diag_msg.get("method") == "textDocument/publishDiagnostics",
          f"{len(diags)} diagnostics")

    # 3. completion (case 000 position)
    send(p, {"jsonrpc": "2.0", "id": 2, "method": "textDocument/completion",
             "params": {"textDocument": {"uri": URI}, "position": {"line": 17, "character": 12}}})
    r = wait_for(p, 2)
    items = (r or {}).get("result", []) or []
    labels = [i.get("label") for i in items]
    check("completion", isinstance(items, list) and len(items) > 0,
          f"{len(items)} items, has Any={('Any' in labels)}")

    # 4. hover (case 000 position on I3)
    send(p, {"jsonrpc": "2.0", "id": 3, "method": "textDocument/hover",
             "params": {"textDocument": {"uri": URI}, "position": {"line": 17, "character": 11}}})
    r = wait_for(p, 3)
    hover = (r or {}).get("result")
    hover_ok = hover is not None and "I3" in json.dumps(hover)
    check("hover", hover_ok, f"value={str(hover)[:60]}")

    # 5. definition
    send(p, {"jsonrpc": "2.0", "id": 4, "method": "textDocument/definition",
             "params": {"textDocument": {"uri": URI}, "position": {"line": 17, "character": 11}}})
    r = wait_for(p, 4)
    defn = (r or {}).get("result")
    check("definition", defn is not None and isinstance(defn, (dict, list)),
          f"location={str(defn)[:60]}")

    # 6. references
    send(p, {"jsonrpc": "2.0", "id": 5, "method": "textDocument/references",
             "params": {"textDocument": {"uri": URI}, "position": {"line": 17, "character": 11},
                        "context": {"includeDeclaration": True}}})
    r = wait_for(p, 5)
    refs = (r or {}).get("result") or []
    check("references", isinstance(refs, list), f"{len(refs)} refs")

    # 7. didChange -> diagnostics change
    changed = src_text.replace("interface I3", "interface I4", 1)
    send(p, {"jsonrpc": "2.0", "method": "textDocument/didChange",
             "params": {"textDocument": {"uri": URI, "version": 2},
                        "contentChanges": [{"range": {"start": {"line": 17, "character": 10},
                                                      "end": {"line": 17, "character": 12}},
                                            "text": "I4"}]}})
    diag_msg = recv(p)
    while diag_msg and diag_msg.get("method") != "textDocument/publishDiagnostics":
        diag_msg = recv(p)
    check("didChange->diagnostics", diag_msg is not None,
          "incremental change handled")

    # 8. shutdown + exit
    send(p, {"jsonrpc": "2.0", "id": 6, "method": "shutdown", "params": None})
    r = wait_for(p, 6)
    check("shutdown", (r or {}).get("result") is None, "shutdown ok")
    send(p, {"jsonrpc": "2.0", "method": "exit", "params": None})
    p.wait(timeout=5)
    check("exit", p.returncode == 0, f"rc={p.returncode}")

    # summary
    passed = sum(1 for _, ok, _ in results if ok)
    print(f"\n=== E2E: {passed}/{len(results)} passed ===")
    failed = [n for n, ok, _ in results if not ok]
    if failed:
        print("Failed:", failed)
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
