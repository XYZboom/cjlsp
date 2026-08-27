#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
# 从索引移除 .worktrees（worker 的 git 仓库不应被主仓库追踪）
git rm --cached -r .worktrees 2>&1 | tail -2
# 确保 .gitignore 排除它
grep -q "^\.worktrees/" .gitignore 2>/dev/null || echo ".worktrees/" >> .gitignore
git add .gitignore
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "fix: untrack .worktrees/ from master index (worker git repos)" 2>&1 | tail -1
git status --short | head -5
echo "=== 确认干净 ==="
git ls-files | grep -c worktrees || echo "0 worktree files tracked"