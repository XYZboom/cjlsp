#!/usr/bin/env python3
"""T31 fast probe: drive ONE LSPServer and fire a completion at a given
position in a source snippet. Prints the returned result list.

Usage:
  python3 tools/t31_probe.py 098          # replay case 098's didOpen+didChange+completion
  python3 tools/t31_probe.py 098 --raw    # also show raw source + method calls
  python3 tools/t31_probe.py --src 'func f(){ let x:X() = x.' --line 0 --char 20
"""
import json, os, re, subprocess, sys, threading, time

BIN = "/root/Code/cangjie/cj-lang/.worktrees/t_1f379622/target/debug/LSPServer"
BASE = "/root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp"


class LSP:
    def __init__(self):
        self.proc = subprocess.Popen([BIN, "--test", "--disableAutoImport", "--enable-log=true"],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.buf = b""
        self.counter = 0
        self.recv = {}          # id -> response

    def send(self, obj):
        data = json.dumps(obj).encode()
        self.proc.stdin.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
        self.proc.stdin.flush()

    def read_frame(self):
        while b"\r\n\r\n" not in self.buf:
            b = self.proc.stdout.read(1)
            if not b:
                return None
            self.buf += b
        head, _, rest = self.buf.partition(b"\r\n\r\n")
        for line in head.split(b"\r\n"):
            if line.lower().startswith(b"content-length:"):
                n = int(line.split(b":")[1].strip())
                break
        else:
            raise RuntimeError("no content-length")
        while len(rest) < n:
            b = self.proc.stdout.read(1)
            if not b:
                return None
            rest += b
        self.buf = rest[n:]
        return json.loads(rest[:n])

    def request(self, method, params):
        self.counter += 1
        i = self.counter
        self.send({"jsonrpc": "2.0", "id": i, "method": method, "params": params})
        while True:
            m = self.read_frame()
            if m is None:
                return None
            if m.get("id") == i:
                return m.get("result")

    def notify(self, method, params):
        self.send({"jsonrpc": "2.0", "method": method, "params": params})

    def close(self):
        self.proc.kill()


def from_case(n, raw=False):
    info = f"{BASE}/testcases/autotestcase/completion/textDocument_completion_{n:03d}.info"
    txt = open(info, encoding="utf-8").read()
    req = txt.split("Rev#")[0]
    didopen = None
    changes = []
    comp = None
    for l in req.splitlines():
        l = l.strip()
        if not l.startswith("{"):
            continue
        try:
            j = json.loads(l)
        except Exception:
            continue
        m = j.get("method")
        if m == "textDocument/didOpen":
            didopen = j["params"]
        elif m == "textDocument/didChange":
            changes.append(j["params"])
        elif m == "textDocument/completion":
            comp = j["params"]
    if raw:
        print("== didOpen params ==")
        print(json.dumps(didopen)[:200])
        print("== completion params ==")
        print(json.dumps(comp))
    return didopen, changes, comp


def main():
    args = sys.argv[1:]
    lsp = LSP()
    lsp.request("initialize", {"processId": None, "rootPath": "cangjiesource", "rootUri": "cangjiesource",
                               "capabilities": {"textDocument": {"completion": {"completionItem": {"snippetSupport": True}}}}})
    lsp.notify("initialized", {})
    try:
        if args and args[0].startswith("--src"):
            src = args[args.index("--src") + 1]
            line = int(args[args.index("--line") + 1]) if "--line" in args else 0
            ch = int(args[args.index("--char") + 1]) if "--char" in args else len(src.split("\n")[line])
            uri = "file:///probe.cj"
            lsp.notify("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "Cangjie", "version": 1, "text": src}})
            params = {"textDocument": {"uri": uri}, "position": {"line": line, "character": ch},
                      "context": {"triggerKind": 2, "triggerCharacter": "."}}
            res = lsp.request("textDocument/completion", params)
            print(json.dumps(res, indent=1))
        else:
            n = int(args[0])
            raw = "--raw" in args
            didopen, changes, comp = from_case(n, raw)
            uri = "file:///probe.cj"
            d = dict(didopen); d["textDocument"] = dict(didopen["textDocument"], uri=uri)
            lsp.notify("textDocument/didOpen", d)
            for chg in changes:
                c = dict(chg); c["textDocument"] = dict(chg["textDocument"], uri=uri)
                lsp.notify("textDocument/didChange", c)
            p = dict(comp); p["textDocument"] = dict(comp["textDocument"], uri=uri)
            res = lsp.request("textDocument/completion", p)
            if raw:
                print("== result ==")
            if isinstance(res, list):
                for it in res:
                    print(f"  {it['label']!r} k{it['kind']} d={it.get('detail','')!r} i={it.get('insertText','')!r} f{it.get('insertTextFormat')} filt={it.get('filterText','')!r}")
            else:
                print(res)
    finally:
        lsp.close()

if __name__ == "__main__":
    main()