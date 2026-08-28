#!/usr/bin/env python3
import sys

path = "/root/Code/cangjie/cj-lang/.worktrees/t_ca649751/crates/cj-lsp/src/hover.rs"
with open(path, "r", encoding="utf-8") as f:
    lines = f.readlines()

# (1-based line of `is_type: ...`, replacement for the line AFTER it)
# Generic sites: insert `param_tys: Vec::new(),` after the is_type line.
# Special sites get a real expression.
generic = [213, 326, 379, 424, 462, 489, 521, 582, 645, 675, 690, 769, 793, 857, 920, 956, 1130]
special = {
    274: '                    param_tys: params.iter().map(|p| render_type(&p.ty)).collect(),\n',
    612: '                    param_tys: params.iter().map(|p| render_type(&p.ty)).collect(),\n',
    726: '                        param_tys: params.iter().map(|p| render_type(&p.ty)).collect(),\n',
}

# apply in descending line order so indices stay valid
targets = sorted(generic + list(special.keys()), reverse=True)
for ln in targets:
    repl = special.get(ln, '                    param_tys: Vec::new(),\n')
    # verify the line is an is_type line
    assert "is_type:" in lines[ln - 1], f"line {ln} is not is_type: {lines[ln-1]!r}"
    lines.insert(ln, repl)  # insert after line ln (1-based)

with open(path, "w", encoding="utf-8") as f:
    f.writelines(lines)
print("done, inserted", len(targets))
