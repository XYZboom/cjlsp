#!/usr/bin/env python3
path = "/root/Code/cangjie/cj-lang/.worktrees/t_ca649751/crates/cj-lsp/src/hover.rs"
with open(path, "r", encoding="utf-8") as f:
    lines = f.readlines()

# (1-based line of `self.types.insert(...)`, kind)
kinds = [(340, "class"), (394, "interface"), (440, "struct"), (479, "enum")]
for ln, kind in sorted(kinds, reverse=True):
    assert "self.types.insert(name.clone(), self.all.len() - 1);" in lines[ln - 1], lines[ln-1]
    insert = (
        "                self.type_kind.insert(name.clone(), %r.to_string());\n"
        "                self.collect_generic_params(type_params, &label);\n"
    ) % kind
    lines.insert(ln, insert)  # insert AFTER line ln

with open(path, "w", encoding="utf-8") as f:
    f.writelines(lines)
print("done")
