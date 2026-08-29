#!/usr/bin/env python3
"""T60 e2e: Ctrl+Click definition on a std.core symbol (Array) must return a
Location pointing into the locally downloaded stdlib source
(~/.cangjie-lsp/std/<ver>/std/core/array.cj). Drives the real LSPServer over
stdio and checks the definition response uri+range."""
import json
import os
import subprocess
import sys

SERVER = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                      "target", "debug", "LSPServer")
SRC = "/tmp/t60_main.cj"
os.makedirs("/tmp/t60_proj", exist_ok=True)
SRC = "/tmp/t60_proj/main.cj"
# Array used as a type on line 1; cursor lands on the name token.
src_text = "func main() {\n    let a: Array<Int64> = Array<Int64>([])\n}\n"
with open(SRC, "w") as f:
    f.write(src_text)
URI = "file://" + SRC
CWD = "/tmp/t60_proj"

def send(p, obj):
    data = json.dumps(obj).encode()
    p.stdin.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
    p.stdin.flush()

def recv(p):
    headers = {}
    line = b""
    while not line.endswith(b"\r\n"):
        c = p.stdout.read(1)
        if not c:
            return None
        line += c
    key, _, value = line.decode().partition(":")
    headers[key.strip().lower()] = value.strip()
    while True:
        line = p.stdout.readline()
        if line == b"\r\n":
            break
        k, _, v = line.decode().partition(":")
        headers[k.strip().lower()] = v.strip()
    n = int(headers["content-length"])
    return json.loads(p.stdout.read(n))

def wait_for(p, rid):
    while True:
        m = recv(p)
        if m is None:
            return None
        if m.get("id") == rid:
            return m

p = subprocess.Popen([SERVER, "--test"],
                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.DEVNULL, cwd=CWD)

send(p, {"jsonrpc": "2.0", "id": 1, "method": "initialize",
         "params": {"rootUri": "file://" + CWD, "capabilities": {}}})
print("init:", wait_for(p, 1)["result"]["serverInfo"])
send(p, {"jsonrpc": "2.0", "method": "initialized", "params": {}})
send(p, {"jsonrpc": "2.0", "method": "textDocument/didOpen",
         "params": {"textDocument": {"uri": URI, "languageId": "cangjie",
                                     "version": 1, "text": src_text}}})

# definition on "Array" — line 1 (0-based), col 12 (0-based)
send(p, {"jsonrpc": "2.0", "id": 2, "method": "textDocument/definition",
         "params": {"textDocument": {"uri": URI}, "position": {"line": 1, "character": 12}}})
r = wait_for(p, 2)
defn = (r or {}).get("result")
print("definition result:", json.dumps(defn, indent=1)[:400])

ok = isinstance(defn, dict) and defn.get("uri", "").endswith("std/core/array.cj")
print("PASS" if ok else "FAIL", "-> uri ends with std/core/array.cj")
if ok:
    uri = defn["uri"]
    # the file must actually exist on disk and be the real downloaded source
    fs = uri.replace("file://", "")
    print("  on-disk:", fs, "exists:", os.path.isfile(fs))
    print("  range:", defn["range"])
    with open(fs) as f:
        lines = f.read().splitlines()
    line = defn["range"]["start"]["line"]
    print("  decl line:", repr(lines[line][:60]))

send(p, {"jsonrpc": "2.0", "id": 9, "method": "shutdown", "params": {}})
wait_for(p, 9)
send(p, {"jsonrpc": "2.0", "method": "exit", "params": {}})
p.wait(timeout=5)
sys.exit(0 if ok else 1)
