#!/usr/bin/env python3
"""Fast single-case completion probe against LSPServer.

Replays a completion .info file's requests directly over stdio (no 1s sleeps,
no lsp_test.py framework), compares the completion response to the expected
Rev# result using the same ignore/sort rules, and prints a detailed diff.

Usage:
  python3 tools/t35_probe.py <case.info> [--server /abs/LSPServer] [--no-symlink]
"""
import os
import re
import sys
import json
import time
import shutil
import argparse
import subprocess
import tempfile

BASE = "/root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp"
DEFAULT_SERVER = "/root/Code/cangjie/cj-lang/.worktrees/t_5ed69d1f/target/debug/LSPServer"

IGNORE_KEY = {"jsonrpc", "sortText", "symbolId", "category", "code", "codeActions"}
IGNORE_SORT_KEY = {
    "range", "data", "to", "fromRanges", "end", "start", "selectionRange",
    "children", "additionalTextEdits", "edits", "textDocument", "parameters",
    "location", "sortText", "containerName", "symbolId", "codeActions",
}


def load_info(path):
    with open(path, encoding="utf-8") as fh:
        text = fh.read()
    parts = re.split(r"Req#\n|Rev#\n", text)
    reqs = [l for l in parts[1].split("\n") if l.strip()]
    resps = [l for l in parts[2].split("\n") if l.strip()]
    return reqs, resps


def rewrite(json_str, cwd):
    """Mirror lsp_test.py's uri/rootPath rewriting for completion cases."""
    def abspath(p):
        return os.path.abspath(os.path.join(cwd, p)) if not os.path.isabs(p) else p
    obj = json.loads(json_str)
    # find uri / rootPath / rootUri recursively
    def walk(o):
        if isinstance(o, dict):
            for k, v in list(o.items()):
                if k == "uri" and isinstance(v, str) and not v.startswith("file://"):
                    o[k] = "file://" + urllib_quote(abspath(v))
                elif k in ("rootPath", "rootUri") and isinstance(v, str) and not v.startswith("file://"):
                    o[k] = abspath(v)
                else:
                    walk(v)
        elif isinstance(o, list):
            for i in o:
                walk(i)
    walk(obj)
    return json.dumps(obj, separators=(",", ":"))


def urllib_quote(p):
    import urllib.parse
    q = urllib.parse.quote(p.replace("\\", "/"))
    return q[0].lower() + q[1:]


def read_message(stream):
    """Read one Content-Length framed message. Skips leading blank lines."""
    headers = {}
    while True:
        raw = stream.readline()
        if not raw:
            return None
        line = raw.decode("utf-8", errors="replace").strip()
        if line == "":
            if "content-length" in headers:
                break
            continue
        if ":" in line:
            k, v = line.split(":", 1)
            headers[k.strip().lower()] = v.strip()
    n = int(headers.get("content-length", 0))
    if n <= 0:
        return None
    return stream.read(n)


def send(proc, payload):
    body = payload.encode("utf-8")
    proc.stdin.write(f"Content-Length: {len(body)}\r\n\r\n".encode())
    proc.stdin.write(body)
    proc.stdin.flush()


def comparable(item):
    """Sort key over the comparable part (ignoring IGNORE_SORT_KEY)."""
    d = {k: v for k, v in item.items() if k not in IGNORE_SORT_KEY}
    return json.dumps(d, sort_keys=True, ensure_ascii=False)


def diff_lists(recv, expected):
    """Return (missing, extra) by label after matching comparable parts."""
    rmap = {}
    for it in recv:
        rmap.setdefault(comparable(it), []).append(it)
    emap = {}
    for it in expected:
        emap.setdefault(comparable(it), []).append(it)
    missing = []
    extra = []
    for k, items in emap.items():
        for it in items:
            if k in rmap and rmap[k]:
                rmap[k].pop()
            else:
                missing.append(it)
    for k, items in rmap.items():
        extra.extend(items)
    return missing, extra


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("info")
    ap.add_argument("--server", default=DEFAULT_SERVER)
    ap.add_argument("--cwd", default=None, help="dir to run server in (default: temp dir)")
    ap.add_argument("--symlink", action="store_true", help="symlink cangjiesource into cwd")
    args = ap.parse_args()

    reqs, resps = load_info(args.info)
    cwd = args.cwd or tempfile.mkdtemp(prefix=".probe_", dir=BASE)
    if args.symlink:
        src = os.path.join(BASE, "sourcecode", "cangjieTest", "cangjiesource")
        link = os.path.join(cwd, "cangjiesource")
        if not os.path.exists(link) and os.path.isdir(src):
            os.symlink(src, link)

    proc = subprocess.Popen(
        [args.server, "--test", "--disableAutoImport", "--enable-log=true"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        cwd=cwd, bufsize=0,
    )

    expected_by_id = {}
    for r in resps:
        obj = json.loads(r)
        if "id" in obj:
            expected_by_id[str(obj["id"])] = obj
    target_ids = set(expected_by_id.keys())

    got = {}
    # Send all requests first (no inter-request 1s sleeps).
    rewritten = [rewrite(r, cwd) for r in reqs]
    for r in rewritten:
        send(proc, r)
        time.sleep(0.05)

    # Then read responses with a select-based timeout until all targets seen.
    import select
    deadline = time.time() + 15
    while time.time() < deadline and len(got) < len(target_ids):
        rlist, _, _ = select.select([proc.stdout], [], [], 0.5)
        if not rlist:
            continue
        msg = read_message(proc.stdout)
        if msg is None:
            break
        try:
            obj = json.loads(msg.decode("utf-8"))
        except Exception:
            continue
        if "id" in obj and str(obj["id"]) in target_ids:
            got.setdefault(str(obj["id"]), obj)

    try:
        proc.stdin.close()
    except Exception:
        pass
    try:
        proc.wait(timeout=3)
    except Exception:
        proc.kill()

    all_ok = True
    for cid, expected in sorted(expected_by_id.items()):
        if cid not in got:
            print(f"[{cid}] NO RESPONSE (expected id never received)")
            all_ok = False
            continue
        recv = got[cid].get("result")
        exp = expected.get("result")
        if isinstance(exp, list) and isinstance(recv, list):
            if len(recv) != len(exp):
                print(f"[{cid}] LENGTH: recv={len(recv)} exp={len(exp)}")
                all_ok = False
            missing, extra = diff_lists(recv, exp)
            for m in missing:
                print(f"  MISSING {m.get('label')!r} kind={m.get('kind')} detail={m.get('detail')!r}")
            for e in extra:
                print(f"  EXTRA   {e.get('label')!r} kind={e.get('kind')} detail={e.get('detail')!r}")
            # detail mismatches on same label
            rlab = {i.get("label"): i for i in recv}
            elab = {i.get("label"): i for i in exp}
            for lab in set(rlab) & set(elab):
                ri, ei = rlab[lab], elab[lab]
                for k in ("kind", "detail", "filterText", "insertText", "documentation", "deprecated"):
                    if ri.get(k) != ei.get(k):
                        print(f"  DIFF {lab!r} {k}: recv={ri.get(k)!r} exp={ei.get(k)!r}")
                        all_ok = False
            if not missing and not extra and len(recv) == len(exp):
                print(f"[{cid}] MATCH (label/kind/detail identical, {len(recv)} items)")
        else:
            print(f"[{cid}] TYPE: recv={type(recv).__name__}={recv!r} exp={type(exp).__name__}")
            all_ok = False
    print("RESULT:", "PASS" if all_ok else "FAIL")
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
