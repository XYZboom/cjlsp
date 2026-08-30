#!/usr/bin/env python3
"""Analyze official hover .info cases: find decls with doc comments above them,
and check whether any use multi-line `//`, block `/* */`, or `/** */` doc comments
with @param — the scenarios T65 asks about."""
import json, re, glob, sys

base = "/root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp/testcases/autotestcase/hover"
files = sorted(glob.glob(base + "/textDocument_hover_*.info"))
print(f"total cases: {len(files)}")

doc_cases = []
for f in files:
    data = open(f, encoding="utf-8", errors="replace").read()
    # extract didOpen source
    m = re.search(r'"method"\s*:\s*"textDocument/didOpen".*?"text"\s*:\s*"((?:[^"\\]|\\.)*)"', data, re.S)
    if not m:
        continue
    try:
        src = json.loads('"' + m.group(1) + '"')
    except Exception:
        continue
    lines = src.split("\n")
    # note decl lines (rough: lines containing func/class/struct/enum/interface/type declarations)
    # find lines above decls that are comments
    for i, line in enumerate(lines):
        # a declaration line (crude heuristic for variable/type/func)
        if re.match(r"^\s*(public\s+|protected\s+|internal\s+|private\s+)?(func|class|struct|enum|interface|type|var|let|static\s+func|open\s+class)", line):
            # comments directly above (skip blank lines, collect contiguous comment block)
            j = i - 1
            while j >= 0 and lines[j].strip() == "":
                j -= 1
            if j >= 0 and ("//" in lines[j] or "/*" in lines[j]):
                # collect the comment block
                block = []
                k = j
                while k >= 0 and (lines[k].lstrip().startswith("//") or "/*" in lines[k] or "*" in lines[k].lstrip()):
                    block.insert(0, lines[k])
                    k -= 1
                # check for multi-line or block comments
                joined = "\n".join(block)
                is_multi = len(block) > 1
                is_block = "/**" in joined or ("/*" in joined and len(block) > 1)
                has_param = "@param" in joined or "@brief" in joined
                if is_multi or is_block or has_param:
                    doc_cases.append((f.split("/")[-1], i, block, line.strip()[:50]))
                break  # one decl per case is enough for this scan

print(f"\n=== cases with MULTI-LINE or BLOCK or @param doc comments: {len(doc_cases)} ===")
for name, lineno, block, decl in doc_cases[:20]:
    print(f"\n-- {name} (decl line {lineno+1}: {decl})")
    for b in block:
        print(f"   | {b}")