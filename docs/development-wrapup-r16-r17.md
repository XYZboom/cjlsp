# cjlsp 开发收尾记录（第 16–17 轮）

> 暂停日期: 2026-08-30 · 用户计划暂停 cjlsp 开发一段时间，此为完整开发记录与交接文档。

## 一、GitHub Issues（已全部处理并关闭，0 open）

| Issue | 问题 | 处理方案 | 关闭方式 |
|---|---|---|---|
| #1 | vscode 插件加载缓慢 | 日志分析（主体为 VSCode 内置 Git 扩展噪音）+ `strip = true` 优化二进制：Linux 2.27→1.86MB、Windows 3.25→2.38MB | commit 带 `Closes #1` 自动关闭 |
| #2 | 误上传 default_AST 非源码文件 | `git rm` 移除 5 个 AST 转储 + 加入 `.git/info/exclude`（本地 exclude，不入库） | commit 带 `Closes #2` 自动关闭 |

## 二、第 16 轮：documentHighlight 从零到 39/124（31.5%）

### 2.1 核心改进链（按提交顺序）
1. **AST 修复**：`Decl::Struct`/`Decl::Enum` 新增 `parents: Vec<Type>` 字段——parser 原代码注释承认丢弃 `<:Any` 父类型，现完整保留（039 处 match 全带 `..` 通配，仅 parser 构造 1 处需同步）
2. **Type 遍历全覆盖**：Class/Interface/Struct/Enum/Extend/TypeAlias/Func/Prop/Var 的 parents/ret/param/bounds/case-payload 类型位置
3. **Pattern::Var 支持**：`var j:Any` 的类型注解与局部变量名命中
4. **官方边界语义**：命中判定含右边界（`character <= e0`）——光标在 `j:` 冒号上也能命中 `j`
5. **kind=1 官方语义**：官方 300 处期望高亮全部 `kind:1 (Text)`（连声明都是）——从 kind=3 修正为统一 kind=1
6. **references walker 扩展**：`expr_name_at` + `expr_name_refs` 覆盖 `Subscript`/`Assign`/`Member`/`LetPatternDestructor`（数组索引 `dabc[0]`、赋值、接收者、局部变量声明均命中）
7. **name_at 全语句遍历**：不再只看 `decl_first_expr`（首条语句），改为 `expr_name_at_anywhere` 遍历所有 body 语句

### 2.2 关键验证
- 官方用例 001/002/003/007/008/011/012 逐一手动精确匹配
- 18→29 tests、clippy 0、Windows 0、hover 官方 12/12 无回归
- LLT 88.2%（crash=0）无回归

## 三、第 17 轮：并行 4 任务（T62–T65）

| 任务 | 功能 | 提交 | 结果 |
|---|---|---|---|
| T62 | textDocument/rename + prepareRename | `8eeafe2` | 复用 documentHighlight 引用收集 → WorkspaceEdit（changes 全量替换），官方 rename 用例 29/111（基线 21） |
| T63 | textDocument/codeLens | `287cb4a` + `73839c2` | 顶层声明上方 "N references" 透镜，`editor.action.showReferences` 直跳；5 单测 + 性能优化 O(file) 单遍 |
| T64 | documentHighlight 嵌套成员方案 | `2f7d71c` | **容器作用域感知**（只高亮光标所在容器内的同名成员，不高亮其他类同名方法）——documentHighlight 42→54/124 |
| T65 | hover 参数文档对齐 | `db8af10` | func 签名默认值 + 多行/块/文档注释支持 |

### 3.1 T64 技术要点（嵌套成员作用域感知）
- 早期尝试：递归收集所有类的同名成员 → **过度匹配**（008 期望 1 处但收集到 2 处），27→7 回归，已回退
- 正确方案：`type_or_expr_name_at` 返回命中容器范围，收集时按容器过滤——**只高亮本类成员**，008/009 修复且 39 例不回归

## 四、最终指标

- **documentHighlight**：0 → 54/124（43.5%，T64 提交声明值，最终验证中）
- **LLT**：88.2%（crash=0）
- **单元测试**：29 通过
- **构建**：clippy 0、Windows GNU 0 error
- **master**：`2f7d71c`（T64）为第 17 轮收尾提交

## 五、经验教训（供后续恢复开发）

1. **worker 共享 repo 竞争**：多 worker 直接写主工作区——重要代码在 /tmp 留独立备份（`.bak2`/`FINAL.bak`），验证最终文件含全部修复后才提交
2. **嵌套成员收集需作用域感知**：否则过度匹配（008 案例）
3. **AST 改字段前验证 match 兼容性**：39 处 match 带 `..` 通配安全，仅 parser 构造需同步
4. **官方 documentHighlight 全 kind=1**：连声明都用 Text，非标准 Write(3)
5. **kanban 派发**：任务落在当前 cwd 的 board，`dispatch --board cj-lang` 启动；worker 会自主提交到 master
6. **回退策略**：无净收益改动（如 decl_name_at_recursive 单独无收益）及时回退保持基线干净

## 六、恢复开发时的建议起点

- 文档高亮剩余 70 例：多为跨文件符号/复杂泛型场景
- completion/hover 官方用例可继续提升
- 新 capability：官方 20 项清单中 `callHierarchy`/`semanticTokens` 详情等
- 系统盘 89%（3.5G），target 2.8G 可清理
