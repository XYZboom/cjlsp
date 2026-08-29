#!/usr/bin/env python3
"""Download the official Cangjie standard library source (current version)
into ~/.cangjie-lsp/std/<version>/ and build a symbol -> (file,line,col)
index for cross-file jump targets.

Source: gitcode.com/Cangjie/cangjie_runtime  ->  stdlib/libs/std/**/*.cj
        (plus cangjie_stdx -> src/stdx/**/*.cj for extension modules)

The plugin / LSP locates the downloaded tree dynamically from the user's
home directory — NO hardcoded machine paths. The version is detected from
the installed SDK when possible, else taken from --version.

Usage:
  python3 tools/stdlib_download.py                  # auto version
  python3 tools/stdlib_download.py --version 1.1.3  # explicit
  python3 tools/stdlib_download.py --list           # show downloaded versions
"""

import argparse
import base64
import json
import os
import re
import sys
import time
import urllib.request

GITCODE_API = "https://gitcode.com/api/v5/repos"
RUNTIME_REPO = "Cangjie/cangjie_runtime"
STDX_REPO = "Cangjie/cangjie_stdx"
# Core stdlib modules under cangjie_runtime/stdlib/libs/std
STD_MODULES = ["core", "argopt", "ast", "binary", "collection", "console",
               "convert", "crypto", "database", "deriving", "env", "fs",
               "interop", "io", "math", "net", "objectpool", "overflow",
               "posix", "process", "random", "ref", "reflect", "regex",
               "runtime", "sort", "sync", "time", "unicode", "unittest"]
UA = {"User-Agent": "Mozilla/5.0"}


def http_get(url: str, retries: int = 4) -> bytes | None:
    for attempt in range(retries):
        req = urllib.request.Request(url, headers=UA)
        try:
            with urllib.request.urlopen(req, timeout=20) as r:
                return r.read()
        except Exception as e:
            code = getattr(e, "code", None)
            if code == 429:  # rate limited — back off and retry
                delay = 1.5 * (attempt + 1)
                print(f"  [warn] rate-limited, retry in {delay:.0f}s...", file=sys.stderr)
                time.sleep(delay)
                continue
            print(f"  [warn] GET {url} failed: {e}", file=sys.stderr)
            return None
    print(f"  [warn] GET {url} gave up after {retries} tries", file=sys.stderr)
    return None


def api_dir(repo: str, path: str) -> list:
    data = http_get(f"{GITCODE_API}/{repo}/contents/{path}")
    if not data:
        return []
    try:
        items = json.loads(data)
        return items if isinstance(items, list) else []
    except Exception:
        return []


def api_file(repo: str, path: str) -> str | None:
    data = http_get(f"{GITCODE_API}/{repo}/contents/{path}")
    if not data:
        return None
    try:
        j = json.loads(data)
        if isinstance(j, dict) and j.get("encoding") == "base64":
            return base64.b64decode(j["content"]).decode("utf-8", "ignore")
    except Exception:
        pass
    return None


def detect_version() -> str:
    """Detect the installed SDK version from common env vars / SDK paths."""
    # cjc --version if on PATH
    import shutil
    cjc = shutil.which("cjc")
    if cjc:
        try:
            out = subprocess_run([cjc, "--version"])
            m = re.search(r"(\d+\.\d+(?:\.\d+)?)", out or "")
            if m:
                return m.group(1)
        except Exception:
            pass
    return "1.1.3"  # fallback default


def subprocess_run(cmd):
    import subprocess
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=15).stdout
    except Exception:
        return None


def download_module(repo: str, rel: str, dest: str) -> int:
    """Recursively download a directory of .cj files. Returns file count."""
    count = 0
    stack = [rel]
    while stack:
        cur = stack.pop()
        for it in api_dir(repo, cur):
            name = it.get("name", "")
            path = it.get("path", "")
            typ = it.get("type", "")
            if typ == "dir":
                stack.append(path)
            elif name.endswith(".cj"):
                content = api_file(repo, path)
                if content is None:
                    continue
                out = os.path.join(dest, os.path.relpath(path, rel))
                os.makedirs(os.path.dirname(out) or dest, exist_ok=True)
                with open(out, "w", encoding="utf-8") as f:
                    f.write(content)
                count += 1
    return count


def main():
    ap = argparse.ArgumentParser(description="Download official Cangjie stdlib source")
    ap.add_argument("--version", default=None, help="SDK version to download (default: detect)")
    ap.add_argument("--list", action="store_true", help="list already-downloaded versions")
    ap.add_argument("--stdlib-only", action="store_true", help="skip stdx extension modules")
    args = ap.parse_args()

    base = os.path.join(os.path.expanduser("~"), ".cangjie-lsp", "std")

    if args.list:
        if os.path.isdir(base):
            print("Downloaded stdlib versions:")
            for v in sorted(os.listdir(base)):
                print("  ", v)
        else:
            print("No stdlib downloaded yet (dir: %s)" % base)
        return

    ver = args.version or detect_version()
    dest = os.path.join(base, ver)
    print(f"Downloading stdlib for SDK version {ver} -> {dest}")

    # Core stdlib from cangjie_runtime/stdlib/libs/std — one subdir per module
    # (std/core/, std/reflect/, ...) so same-named files in different modules
    # (e.g. core/exception.cj vs reflect/exception.cj) never collide. Keeps the
    # index paths stable ("std/core/array.cj") for jump targets.
    os.makedirs(dest, exist_ok=True)
    total = 0
    print("  core modules:")
    for mod in STD_MODULES:
        src = f"stdlib/libs/std/{mod}"
        n = download_module(RUNTIME_REPO, src, os.path.join(dest, "std", mod))
        total += n
        print(f"    {mod}: {n} files")
    # top-level std.cj
    c = api_file(RUNTIME_REPO, "stdlib/libs/std/std.cj")
    if c:
        os.makedirs(os.path.join(dest, "std"), exist_ok=True)
        with open(os.path.join(dest, "std", "std.cj"), "w", encoding="utf-8") as f:
            f.write(c)
        total += 1

    if not args.stdlib_only:
        print("  stdx extensions:")
        n = download_module(STDX_REPO, "src/stdx", os.path.join(dest, "stdx"))
        total += n
        print(f"    stdx: {n} files")

    print(f"\nDone: {total} .cj files in {dest}")
    print("Next: run tools/stdlib_index.py to build the symbol index.")


if __name__ == "__main__":
    main()
