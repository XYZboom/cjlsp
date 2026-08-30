#!/usr/bin/env python3
"""T62: rename + prepareRename end-to-end verification (textDocument/rename).

Drives LSPServer over stdio like a real LSP client:
  initialize -> didOpen -> prepareRename -> rename -> shutdown -> exit

Validates:
  1. prepareRename returns the symbol's range + placeholder (not null).
  2. rename returns a WorkspaceEdit whose TextEdits cover EVERY occurrence
     of the symbol in the current file (declaration + usages).
  3. Applying the edits to the source yields the expected renamed text.
  4. Renaming a symbol named `bara` replaces all bara positions (acceptance).

Usage: python3 tools/test_rename.py
"""
import json
import os
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SERVER = os.path.join(REPO, "target", "debug", "LSPServer")
CWD = os.path.dirname(SERVER)

results = []


def send(p, obj):
    data = json.dumps(obj).encode()
    p.stdin.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
    p.stdin.flush()


def recv(p):
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


def apply_edits(text, edits):
    """Apply a list of TextEdits (sorted descending by position) to text.
    Edits are [{range:{start:{line,character},end:{line,character}}, newText}]."""
    lines = text.split('\n')
    for e in sorted(edits, key=lambda x: (-x['range']['start']['line'],
                                          -x['range']['start']['character'])):
        s = e['range']['start']
        en = e['range']['end']
        start_idx = sum(len(l) + 1 for l in lines[:s['line']]) + s['character']
        end_idx = sum(len(l) + 1 for l in lines[:en['line']]) + en['character']
        text = text[:start_idx] + e['newText'] + text[end_idx:]
    return text


def run_case(p, uri, src, sym, positions, new_name):
    """Rename `sym` at each of `positions` (list of (line,char)) to new_name
    and verify the WorkspaceEdit covers all occurrences of sym."""
    send(p, {"jsonrpc": "2.0", "method": "textDocument/didOpen",
             "params": {"textDocument": {"uri": uri, "languageId": "cangjie",
                                          "version": 1, "text": src}}})
    # Drain the publishDiagnostics notification from didOpen (exactly one).
    recv(p)

    # ---- prepareRename at the first position ----
    send(p, {"jsonrpc": "2.0", "id": 100, "method": "textDocument/prepareRename",
             "params": {"textDocument": {"uri": uri},
                        "position": {"line": positions[0][0], "character": positions[0][1]}}})
    r = wait_for(p, 100)
    prep = (r or {}).get("result")
    prep_ok = prep is not None and isinstance(prep, dict) and "range" in prep
    check(f"prepareRename[{sym}]", prep_ok, f"result={json.dumps(prep)}")

    # ---- rename at every given position ----
    all_ok = True
    for i, (line, char) in enumerate(positions):
        send(p, {"jsonrpc": "2.0", "id": 200 + i, "method": "textDocument/rename",
                 "params": {"textDocument": {"uri": uri},
                            "position": {"line": line, "character": char},
                            "newName": new_name}})
        r = wait_for(p, 200 + i)
        res = (r or {}).get("result")
        if res is None:
            check(f"rename[{sym}]@{line}:{char}", False, "result is null")
            all_ok = False
            continue
        # WorkspaceEdit may use `changes` or `documentChanges`; official cjlsp
        # suite uses documentChanges.
        edits = []
        if "documentChanges" in (res or {}):
            for dc in res.get("documentChanges", []):
                if dc.get("textDocument", {}).get("uri") == uri:
                    edits.extend(dc.get("edits", []))
        else:
            edits = (res.get("changes") or {}).get(uri, [])
        # Expected: all sym occurrences renamed.
        out = apply_edits(src, edits)
        expected = src.replace(sym, new_name)
        ok = out == expected
        if not ok:
            print(f"      edits={json.dumps(edits)}")
            print(f"      expected: {expected!r}")
            print(f"      actual:   {out!r}")
        check(f"rename[{sym}]@{line}:{char} all occurrences", ok,
              f"{len(edits)} edits")
        all_ok = all_ok and ok
    return all_ok


def find_positions(src, sym):
    """Return [(line, char)] for every occurrence of `sym` as a whole word."""
    positions = []
    for i, line in enumerate(src.split('\n')):
        start = 0
        while True:
            idx = line.find(sym, start)
            if idx == -1:
                break
            # whole-word: prev/next char must not be an identifier char
            prev_ok = idx == 0 or not (line[idx - 1].isalnum() or line[idx - 1] == '_')
            after = idx + len(sym)
            next_ok = after >= len(line) or not (line[after].isalnum() or line[after] == '_')
            if prev_ok and next_ok:
                positions.append((i, idx))
            start = idx + len(sym)
    return positions


def main():
    p = subprocess.Popen([SERVER, "--test", "--disableAutoImport", "--enable-log=true"],
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL, cwd=CWD)

    send(p, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
    r = wait_for(p, 1)
    caps = (r or {}).get("result", {}).get("capabilities", {})
    check("capability:renameProvider", caps.get("renameProvider") is not None,
          f"renameProvider={json.dumps(caps.get('renameProvider'))}")
    send(p, {"jsonrpc": "2.0", "method": "initialized", "params": {}})

    # Case 1: /tmp/simple.cj — rename `add` (decl on line0, call in main line7).
    simple = open("/tmp/simple.cj").read()
    run_case(p, "file:///tmp/simple.cj", simple, "add",
             find_positions(simple, "add"), "bara")

    # Case 2: symbol named `bara` used in multiple positions (acceptance).
    bara_src = "func bara(x: Int64): Int64 { return x + 1 }\nmain() { let y = bara(2) }\n"
    run_case(p, "file:///tmp/bara_test.cj", bara_src, "bara",
             find_positions(bara_src, "bara"), "renamed")

    # Case 3: renaming a type symbol (class Foo) also covers type usages.
    foo_src = "class Foo {}\nfunc make(): Foo { return Foo() }\n"
    run_case(p, "file:///tmp/foo_test.cj", foo_src, "Foo",
             find_positions(foo_src, "Foo"), "Bar")

    # Case 4: rename on a non-symbol position returns null.
    send(p, {"jsonrpc": "2.0", "id": 300, "method": "textDocument/rename",
             "params": {"textDocument": {"uri": "file:///tmp/simple.cj"},
                        "position": {"line": 3, "character": 2},
                        "newName": "x"}})
    r = wait_for(p, 300)
    check("rename[non-symbol]->null", (r or {}).get("result") is None)

    send(p, {"jsonrpc": "2.0", "id": 301, "method": "textDocument/prepareRename",
             "params": {"textDocument": {"uri": "file:///tmp/simple.cj"},
                        "position": {"line": 3, "character": 2}}})
    r = wait_for(p, 301)
    check("prepareRename[non-symbol]->null", (r or {}).get("result") is None)

    send(p, {"jsonrpc": "2.0", "id": 6, "method": "shutdown", "params": None})
    wait_for(p, 6)
    send(p, {"jsonrpc": "2.0", "method": "exit", "params": None})
    p.wait(timeout=5)
    check("exit", p.returncode == 0, f"rc={p.returncode}")

    passed = sum(1 for _, ok, _ in results if ok)
    print(f"\n=== rename E2E: {passed}/{len(results)} passed ===")
    failed = [n for n, ok, _ in results if not ok]
    if failed:
        print("Failed:", failed)
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
