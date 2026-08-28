#!/usr/bin/env python3
"""LSP cross-file scan scalability benchmark (T33).

Builds a synthetic same-package repo of N real LLT `.cj` files, drives the
real LSPServer over stdio and times:
  - cold:  the didOpen-triggered publishDiagnostics (first request on a fresh
           server — this is where the full project scan+parse happens once),
  - hot:   repeated textDocument/completion requests with no file change (must
           be an mtime diff — read_dir walk only, no re-parse),
  - hot+1: one sibling file touched -> only that file re-parsed.

Usage:
  python3 tools/lsp_scan_bench.py [--files 400] [--iters 3] [--no-cleanup]

Setup rules (from references/lsp-scalability-measurement.md):
  - cwd MUST be the project root (<repo>), NOT <repo>/pkg — with the wrong cwd
    resolve_project_root resolves to a FILE, the scan silently no-ops and you
    measure a fake-fast ~1.9ms.
  - didOpen must carry the REAL file text (never rely on the uri path).
  - the didOpen uri must point at a file that EXISTS under the cwd.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
LSPSERVER = os.path.join(os.path.dirname(HERE), "target", "debug", "LSPServer")
LLT_SEMA = "/root/Code/cangjie/cangjie_test/testsuites/LLT/compiler/Sema"
PKG = "bench_pkg"


def frame(obj):
    body = json.dumps(obj, separators=(",", ":"))
    return f"Content-Length: {len(body)}\r\n\r\n{body}"


def read_frame(f):
    headers = {}
    while True:
        line = f.readline()
        if line in (b"\r\n", b"\n", b""):
            break
        k, _, v = line.decode().partition(":")
        headers[k.strip().lower()] = v.strip()
    n = int(headers["content-length"])
    return f.read(n)


def rewrite_package(src, pkg):
    lines = src.split("\n")
    for i, line in enumerate(lines):
        s = line.strip()
        if s.startswith("package"):
            indent = line[: len(line) - len(line.lstrip())]
            lines[i] = f"{indent}package {pkg}"
            return "\n".join(lines)
    return f"package {pkg}\n" + src


def all_llt_cj_files():
    files = []
    for dirpath, _dirs, names in os.walk(LLT_SEMA):
        for name in sorted(names):
            if name.endswith(".cj"):
                files.append(os.path.join(dirpath, name))
    return sorted(files)


def build_repo(n, tmp):
    pkg_dir = os.path.join(tmp, "pkg")
    os.makedirs(pkg_dir, exist_ok=True)
    files = all_llt_cj_files()
    if not files:
        print(f"no LLT samples under {LLT_SEMA}", file=sys.stderr)
        sys.exit(1)
    for i in range(n):
        fname = files[i % len(files)]
        with open(os.path.join(LLT_SEMA, fname), encoding="utf-8",
                  errors="replace") as fh:
            src = fh.read()
        # Every sibling carries the same sentinel decl, so the "Sent" prefix
        # in cur.cj deterministically hits cross-file candidates (proving the
        # scan/cache ran, whatever the LLT content).
        body = rewrite_package(src, PKG) + "\nclass Sent {}\n"
        with open(os.path.join(pkg_dir, f"f{i:03d}.cj"), "w",
                  encoding="utf-8") as fh:
            fh.write(body)
    # cur.cj: cursor sits after "Sent" (a sentinel class injected into every
    # sibling) so the response provably contains cross-file candidates.
    cur = f"package {PKG}\nmain() {{\n    let t = Sent\n}}"
    cur_path = os.path.join(pkg_dir, "cur.cj")
    with open(cur_path, "w", encoding="utf-8") as fh:
        fh.write(cur)
    return tmp


def run_repo(repo, n, iters):
    uri = f"file://{repo}/pkg/cur.cj"
    p = subprocess.Popen(
        [LSPSERVER, "--test", "--disableAutoImport"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, cwd=repo,
    )
    try:
        # initialize
        p.stdin.write(frame({"jsonrpc": "2.0", "id": 1,
                             "method": "initialize",
                             "params": {"rootUri": f"file://{repo}/",
                                        "capabilities": {}}}).encode() + b"\n")
        p.stdin.flush()
        read_frame(p.stdout)

        # didOpen with REAL text -> cold scan happens here.
        with open(os.path.join(repo, "pkg", "cur.cj"), encoding="utf-8") as fh:
            text = fh.read()
        didopen = {"jsonrpc": "2.0", "method": "textDocument/didOpen",
                   "params": {"textDocument": {"uri": uri, "languageId": "cangjie",
                                               "version": 1, "text": text}}}
        t0 = time.perf_counter()
        p.stdin.write(frame(didopen).encode() + b"\n")
        p.stdin.flush()
        read_frame(p.stdout)  # publishDiagnostics
        cold = (time.perf_counter() - t0) * 1000

        # completion request (0-based cursor: line 2, after "Sent")
        comp = {"jsonrpc": "2.0", "id": 100, "method": "textDocument/completion",
                "params": {"textDocument": {"uri": uri},
                           "position": {"line": 2, "character": 14}}}
        hots = []
        resp = b"{}"
        for _ in range(iters):
            t0 = time.perf_counter()
            p.stdin.write(frame(comp).encode() + b"\n")
            p.stdin.flush()
            resp = read_frame(p.stdout)
            hots.append((time.perf_counter() - t0) * 1000)
        items = len(json.loads(resp).get("result") or [])

        # touch one sibling -> only that file re-parsed
        os.utime(os.path.join(repo, "pkg", "f000.cj"))
        t0 = time.perf_counter()
        p.stdin.write(frame(comp).encode() + b"\n")
        p.stdin.flush()
        read_frame(p.stdout)
        hot_delta = (time.perf_counter() - t0) * 1000
    finally:
        p.stdin.close()
        p.wait(timeout=10)
    return cold, hots, hot_delta, items


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--files", type=int, default=400)
    ap.add_argument("--iters", type=int, default=3)
    ap.add_argument("--no-cleanup", action="store_true")
    args = ap.parse_args()

    tmp = tempfile.mkdtemp(prefix="t33_scan_bench_")
    try:
        build_repo(args.files, tmp)
        cold, hots, hot_delta, items = run_repo(tmp, args.files, args.iters)
    finally:
        if not args.no_cleanup:
            shutil.rmtree(tmp, ignore_errors=True)

    print(f"files={args.files}  server={LSPSERVER}")
    print(f"cold (didOpen+full scan):        {cold:7.2f} ms")
    print(f"hot completions (n={len(hots)}): "
          + ", ".join(f"{h:6.2f} ms" for h in hots))
    print(f"hot after touching 1 file:       {hot_delta:7.2f} ms")
    print(f"completion items: {items} (sentinel 'Sent' proves cross-file scan ran)")


if __name__ == "__main__":
    main()
