# 标准库支持规划（跳转 + 下载 + 二进制符号）— T60 已落地 ①索引跳转/②下载③只读待插件

用户需求（2026-08-29 累积）:
1. **跳转到只读标准库**，且要求是**当前版本**的标准库
2. **官网下载功能**: 未来 IDE 插件管理仓颉版本, 应从官网下载源码到本机
3. **二进制符号解析**: 借鉴仓颉编译器源码
4. ⚠️ **插件不能硬编码本机路径** — 所有标准库路径必须动态解析

## 实现状态（2026-08-30 T60）✅
- ✅ 下载器 `tools/stdlib_download.py`（gitcode API, 递归拉 .cj → ~/.cangjie-lsp/std/<ver>/std/<mod>/）
  - ✅ v1.1.3 已下载: 30 核心模块 + std.cj（std/ 下按模块分子目录, 避免同名文件冲突）
  - ✅ 版本检测: 优先 cjc --version, 回退默认 1.1.3
- ✅ 索引器 `tools/stdlib_index.py` → ~/.cangjie-lsp/std/<ver>/index.json
  - ✅ 1.1.3 index.json: 8000+ symbols, 格式 {"version", "symbols": {name: {file,line,col,kind}}}
- ✅ LSP 跳转落点（hover.rs `definition_at` + server.rs）:
  - ✅ `StdlibIndex` 启动时加载 ~/.cangjie-lsp/std/<最新版本>/index.json（HOME/USERPROFILE 动态解析, 无硬编码）
  - ✅ definition_at 本地/兄弟 miss → 查索引 → 返回 file://<std源码> + range
  - ✅ 修复原有 bug: std.core 内置符号（line=0）definition 不再 underflow 而是查索引
  - ✅ 单测: definition 对 Array 返回 std/core/array.cj 位置（hover.rs tests）
  - ✅ clippy 0 / Windows cross-build 0 error / 全 workspace 测试通过
- ⏳ 只读 std 路径: 插件侧 VSCode 只读配置（后续 T 待办）
- ⏳ 二进制符号解析（C 节）: 未做（SDK .bc 符号表补充）

## 调研现状（已确认）
- 本机 SDK: /root/Code/cangjie/sdk/cangjie-sdk-linux-x64-1.1.3.tar.gz (1.1.3)
  - std 只有预编译模块: modules/linux_x86_64_cjnative/std/libstd.*.bc (LLVM bitcode)
  - **无 .cj 源码** → 跳转目标必须从官网下载的源码包获取
- ✅ **官方标准库源码位置确认**:
  - gitcode.com/Cangjie/cangjie_runtime → `stdlib/libs/std/` 含全部核心模块
    (core 60 .cj 文件: array.cj/string.cj/comparable.cj...; console/collection/
    convert/io/math/net/fs/...)
  - gitcode.com/Cangjie/cangjie_stdx → `src/stdx/` 扩展模块 (网络/安全等)
  - 版本 tag 存在 (v1.2.0, v1.3.0-alpha...; SDK 1.1.3 对应 tag 需映射确认)
- 官网下载页: https://cangjie-lang.cn/download/<version> (JS 渲染, 无直接链接;
  但 gitcode API 可直接拉取)
- 现有 STD_SYMS: hover.rs 硬编码 8 个核心类型符号, 无位置, 无函数 → 无法跳转
  （T60 前 definition 对 Array 等 std 符号返回的是 underflow 错误位置）

## 下载实现（gitcode API, 不硬编码本机路径）
```
1. 版本检测: SDK 包名 (cangjie-sdk-linux-x64-<ver>) 或 cjc --version
2. 下载: gitcode API /api/v5/repos/Cangjie/cangjie_runtime/contents/stdlib/libs/std
   递归拉取 .cj 源码 → ~/.cangjie-lsp/std/<version>/
   (或 git clone --depth 1 --branch <tag> https://gitcode.com/Cangjie/cangjie_runtime.git)
3. 符号索引: 解析 .cj 生成 symbol -> (file, line, col) 存 index.json
4. LSP 加载: ~/.cangjie-lsp/std/<version>/index.json (启动时按 SDK 版本选择)
5. 跳转: definition/hover 本地/兄弟 miss -> 查索引 -> file://<std源码> + range
6. 只读: 插件对 std 路径设置只读
```

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