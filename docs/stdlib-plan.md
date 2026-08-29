# 标准库支持规划（跳转 + 下载 + 二进制符号）— 第 15 轮待拆

用户需求（2026-08-29 累积）:
1. **跳转到只读标准库**，且要求是**当前版本**的标准库
2. **官网下载功能**: 未来 IDE 插件管理仓颉版本, 应从官网下载源码到本机
3. **二进制符号解析**: 借鉴仓颉编译器源码
4. ⚠️ **插件不能硬编码本机路径** — 所有标准库路径必须动态解析

## 调研现状
- 本机 SDK: /root/Code/cangjie/sdk/cangjie-sdk-linux-x64-1.1.3.tar.gz (1.1.3)
  - std 只有预编译模块: modules/linux_x86_64_cjnative/std/libstd.*.bc (LLVM bitcode)
  - **无 .cj 源码** → 跳转目标必须从官网下载的源码包获取
- 本机 CangjieCorpus/libs/std/core 只有 core_package_api 文档/corpus 样本, 无完整源码
- 官网: https://cangjie-lang.cn (下载页 /zh-CN/download 等, 需确认具体 URL 结构)
- 现有 STD_SYMS: hover.rs 硬编码 8 个核心类型符号 (String/Array/VArray/...),
  无位置信息 (line 0), 无函数符号 (print 等) → 无法跳转

## 实现方案（草稿）

### A. 标准库源码下载器（tools/ 或插件侧）
- 官网下载 URL 结构待确认（可能 https://cangjie-lang.cn/zh-CN/download 或 GitHub releases）
- 下载指定版本源码包 → 解压到用户目录 (~/.cangjie-lsp/std/<version>/)
- 插件提供命令 "Cangjie: Download Standard Library <version>" + 版本检测（SDK 中读版本）
- **当前版本检测**: SDK 包名里带版本 (cangjie-sdk-linux-x64-1.1.3) / cjc --version

### B. 标准库符号表 → 源码位置映射（跳转核心）
- 从下载的标准库源码解析出符号 → (file, line, col) 索引（AST 扫描一次生成索引文件）
- 索引存 ~/.cangjie-lsp/std/<version>/index.json，启动时加载
- definition_at: 本地/兄弟文件 miss 后查该索引 → 返回 file://<std源码路径> + range
- **只读**: 返回的 std 文件路径设为只读（插件 markdown 或 VSCode readonly 配置）
- 替代方案（更快）: 官网有 API 文档 JSON 的话直接生成符号→位置表

### C. 二进制符号解析（借鉴 cangjie_compiler）
- SDK modules 里是 LLVM bitcode (.bc)，官方 cjc 链接时用 llvm ir 解析符号
- 参考: cangjie_compiler 源码里 SymbolTable / ReadableCallInfo / 二进制导入逻辑
- 产出: 对标准库 .bc 导出符号 (名称+签名), 补充 A/B 的符号表覆盖（编译期可用的符号）
- 详见 cangjie_compiler: include/cangjie/SymbolTable.h 等

### D. 高亮联动
- semantic.rs 用 std 符号表: 标准库类型 → type, 标准库函数 → function（替代现在 Array/print 显示 variable）

## 验收
1. Ctrl+点击 `Array`/`String`/`print` → 打开官网对应版本标准库源码文件（只读）
2. 插件无任何硬编码本机路径（用环境变量/用户目录/下载器）
3. 版本可切换（SDK 1.1.3 就用 1.1.3 源码）
4. Windows/Linux 双平台工作