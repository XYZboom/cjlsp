#!/usr/bin/env python3
"""T65 verifier (final): drive LSPServer over stdio with the proven line-based
LSP reader, run the 5 T65 scenarios at their CORRECT cursor positions, and print
OUR value with PASS/FAIL vs the official expected substring.

Expected values derived from official HoverImpl.cpp + ItemResolverUtil.cpp:
  - defaults always rendered (ProcessSingleParam -> GetFuncNamedParam)
  - comments rendered as \n---\n\n + blocks, each line escaped + hard-broken.
"""
import subprocess, json, time, select

SERVER = "/root/Code/cangjie/cj-lang/target/debug/LSPServer"

def run_hover(src, line, char, uri="file:///proj/main.cj"):
    p = subprocess.Popen([SERVER], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL)
    def send(obj):
        d = json.dumps(obj).encode()
        p.stdin.write(f"Content-Length: {len(d)}\r\n\r\n".encode() + d)
        p.stdin.flush()
    def recv(timeout):
        r, _, _ = select.select([p.stdout], [], [], timeout)
        if not r:
            return ("TIMEOUT", None)
        line_b = p.stdout.readline()
        if not line_b:
            return ("EOF", None)
        n = int(line_b.split(b":", 1)[1].strip())
        while True:
            l = p.stdout.readline()
            if l in (b"\r\n", b"\n"):
                break
        return ("MSG", json.loads(p.stdout.read(n)))
    send({"jsonrpc": "2.0", "id": "0", "method": "initialize",
          "params": {"processId": None, "rootUri": "file:///proj/", "capabilities": {}}})
    recv(3)
    send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
    send({"jsonrpc": "2.0", "method": "textDocument/didOpen",
          "params": {"textDocument": {"uri": uri, "languageId": "Cangjie", "version": 1, "text": src}}})
    time.sleep(0.4)
    send({"jsonrpc": "2.0", "id": "9", "method": "textDocument/hover",
          "params": {"textDocument": {"uri": uri}, "position": {"line": line, "character": char}}})
    out = None
    deadline = time.time() + 8
    while time.time() < deadline:
        tag, m = recv(1.5)
        if tag in ("TIMEOUT", "EOF"):
            continue
        if str(m.get("id")) == "9":
            out = m.get("result")
            break
    try:
        send({"jsonrpc": "2.0", "id": "4", "method": "shutdown", "params": {}})
        recv(0.5)
        send({"jsonrpc": "2.0", "method": "exit", "params": {}})
    except Exception:
        pass
    p.kill()
    return out

def val(out):
    if not out:
        return None
    return (out.get("contents") or {}).get("value")

# (name, src, line, char, expected-substring). Positions are 0-based LSP.
SCENARIOS = [
    ("func-default-param",
     'package default\n\n// a doc line\nfunc add1(a: Int32, b!: Int32 = 1): Int32 { a + b }\n',
     3, 9,
     "```cangjie\ninternal func add1(a: Int32, b!: Int32 = 1): Int32\n```"),
    ("multiline-comment",
     'package default\n\n// 函数返回值类型推断\n//Write return explicitly.\nfunc return_test_1(): Int64 { 3 }\n',
     4, 7,
     "---\n\n函数返回值类型推断  \n\n\nWrite return explicitly.  \n"),
    ("block-comment",
     'package default\n\n/* 块注释 */\nvar LSP_Hover_Comment_Block_001: Int64 = 1\n',
     3, 4,
     "---\n\n块注释  \n"),
    ("doc-comment-param",
     'package default\n\n/**\n * desc\n * @param param1 说明1\n * @param param2 说明2\n * @return Int64\n */\nfunc LSP_Hover_Comment_Doc_001(param1: Int64, param2: Int64): Int64 { 0 }\n',
     8, 7,
     "---\n\ndesc  \n@param param1 说明1  \n@param param2 说明2  \n@return Int64  \n"),
    ("ordered-list-escape",
     'package default\n\n/**\n * 4. Interface 接口定义\n * 3. this is a test\n * 2. cangjie\n */\nvar LSP_Hover_Comment_Block_Ordered_List_001: Int64 = 1\n',
     7, 4,
     "---\n\n4\\. Interface 接口定义  \n3\\. this is a test  \n2\\. cangjie  \n"),
]

all_ok = True
for name, src, line, char, exp_sub in SCENARIOS:
    out = run_hover(src, line, char)
    v = val(out) or ""
    ok = exp_sub in v
    all_ok = all_ok and ok
    print(f"=== {name} === {'PASS' if ok else 'FAIL'}")
    print(f"  OURS: {v!r}")
    if not ok:
        print(f"  want (substr): {exp_sub!r}")
    print()
print("ALL:", "PASS" if all_ok else "FAIL")