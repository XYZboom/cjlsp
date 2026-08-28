# Windows 宏动态加载（LoadLibraryW / GetProcAddress）

本文档说明 Windows 平台上用户宏（spec Ch.14）的动态加载链路：官方 SDK（cjpm）
如何把宏包编译成 `.dll`、LSP 如何用 `LoadLibraryW` + `GetProcAddress` 加载并真正
展开宏，以及本仓库为此做的跨平台改造与验证结果（T38 实现，T48 端到端验证）。

对应代码：

- `crates/cj-sema/src/dylib.rs` —— 动态加载平台层（`imp` 模块）+ 运行时初始化
- `crates/cj-sema/src/macro_cache.rs` —— 宏包编译缓存（跨会话 / 会话内两层）
- `crates/cj-sema/src/expander.rs` —— 展开管线（builtin → .so/.dll → quote 模板回退）

---

## 1. 官方 SDK 的 Windows 宏编译产物格式

结论（已对照官方编译器源码 + 真实 SDK 产物核实）：

| 项 | Linux | Windows |
|---|---|---|
| 宏库文件名 | `lib-macro_<fullPkgName>.so` | `lib-macro_<fullPkgName>.dll` |
| 符号前缀（普通宏） | `macroCall_c_<Ident>_<Pkg>` | 同左 |
| 符号前缀（属性宏） | `macroCall_a_<Ident>_<Pkg>` | 同左 |
| 包名中 `.` / `:` | 映射为 `_` | 同左 |

依据：

- 库后缀是官方编译器的 `LIB_SUFFIX`（`include/cangjie/Macro/MacroCall.h`）：
  Windows 为 `.dll`，macOS 为 `.dylib`，Linux 为 `.so`。
- 宏库命名来自 `Macro/MacroCallResolve.cpp` 的 `FindMacroDefPkg`：
  `"lib-macro_" + ToCjoFileName(fullPackageName) + LIB_SUFFIX`。
- 导出符号来自 `Utils/Utils.cpp` 的 `GetMacroFuncName`：

  ```cpp
  std::string GetMacroFuncName(const std::string& fullPackageName, bool isAttr, const std::string& ident)
  {
      const std::string prefixForAttrMacro = "macroCall_a_";
      const std::string prefixForPlainMacro = "macroCall_c_";
      auto macroFuncName = (isAttr ? prefixForAttrMacro : prefixForPlainMacro) + ident + "_" + fullPackageName;
      std::replace(macroFuncName.begin(), macroFuncName.end(), '.', '_');
      std::replace(macroFuncName.begin(), macroFuncName.end(), ':', '_');
      return macroFuncName;
  }
  ```

`dylib.rs` 的 `macro_symbol_name` 与该约定逐字符一致：

```rust
fn macro_symbol_name(macro_name: &str, pkg: &str) -> String {
    let pkg = pkg.replace(['.', ':'], "_");
    format!("macroCall_c_{macro_name}_{pkg}")
}
```

### 1.1 真实 SDK 样本（本机核实）

本机 Linux SDK 自带完整 Windows 运行时目录：

```
/root/Code/cangjie/sdk/cangjie/runtime/lib/windows_x86_64_cjnative/
  ├── libcangjie-runtime.dll                     # InitCJRuntime / RunCJTask / ReleaseHandle
  ├── libcangjie-std-core.dll                    # _CGPatirHv（std.core 包初始化）
  ├── libcangjie-std-collection.dll              # _CGPacirHv
  ├── libcangjie-std-ast.dll                     # _CGPaxirHv
  ├── libcangjie-std-unittest.testmacro.dll      # 宏样本：21× macroCall_c_ + 16× macroCall_a_
  └── libcangjie-std-unittest.mock.mockmacro.dll # 宏样本：2× macroCall_c_
```

用 `x86_64-w64-mingw32-objdump -p` 核实导出表（均为 PE32+ x86-64 DLL）：

```
$ objdump -p libcangjie-std-unittest.testmacro.dll | grep macroCall_
  [ 51] macroCall_a_AssertThrows_std_unittest_testmacro
  [ 67] macroCall_c_AfterAll_std_unittest_testmacro
  ...
```

关键印证：`macroCall_c_AfterAll_std_unittest_testmacro` =
`"macroCall_c_" + "AfterAll" + "_" + "std.unittest.testmacro"`，包名点号转下划线后与
`GetMacroFuncName` 完全一致 —— LSP 构造的符号名与官方产物一一对应。

### 1.2 token 序列化跨平台一致

宏参数/结果的字节流遵循官方 `Macro/TokenSerialization.cpp` 的
`GetTokensBytes` / `GetTokensFromBytes` 格式，字节序为小端。Linux 与 Windows 均为
x86-64 小端，同一份源码编译，字节布局完全一致，因此 `dylib.rs` 的
`serialize_tokens` / `deserialize_from_ptr` 无需按平台分支。

### 1.3 返回值释放契约

官方 `Macro/MacroEvaluationCJNative.cpp` 的 `InvokeMacroFunc` 对宏返回的缓冲区调用
C `free()` 释放；`dylib.rs` 的 `imp::free`（`libc::free`）与之一致。测试
`dlopen_and_call_macro_so` 在同一宏库上连续调用两次，证明首次调用的 `free()` 未破坏
运行时堆。

---

## 2. LSP 的 Windows 加载链路（dylib.rs）

### 2.1 平台层（`imp` 模块）

两平台暴露相同的 API 形状（`open` / `get::<T>` / `free`），上层代码零分支：

| 平台 | open | get | 卸载 |
|---|---|---|---|
| Linux | `libloading::os::unix` dlopen `RTLD_NOW\|RTLD_GLOBAL` | dlsym | drop（dlclose） |
| Windows | `LoadLibraryW`（宽字符路径） | `GetProcAddress` | drop（`FreeLibrary`） |

`get` 在 Windows 侧把 `FARPROC` 用 `transmute_copy` 位拷贝到调用方要求的函数指针类型
（两者都是指针尺寸）。`imp::Loaded` 手动 `Send`/`Sync`（运行时单例持有唯一句柄，
`FreeLibrary` 在 drop 时调用）。

### 2.2 运行时初始化（`init_runtime`，T48 跨平台化）

T48 把原先写死的 Linux 路径改为按平台选择（Linux 行为不变）：

- 运行时目录：`<sdk>/runtime/lib/linux_x86_64_cjnative` ⇄
  `windows_x86_64_cjnative`（`runtime_lib_subdir()`）
- 库扩展名：`.so` ⇄ `.dll`（`lib_ext()`）
- 加载器搜索路径：Linux 前置到 `LD_LIBRARY_PATH`；Windows 前置到 `PATH`
  （`LoadLibraryW` 解析 `.dll` 的 import 依赖时按标准搜索序，其中包含 `PATH`）
  —— 对应官方 `envsetup.sh` 的行为。

预加载清单：`libboundscheck`、`libcangjie-runtime`、`libcangjie-std-core`、
`libcangjie-std-collection`、`libcangjie-std-ast`、`libcangjie-std-sort`
（平台层自动补 `.so` / `.dll`）。

运行时入口符号两平台相同（T48 已对 Windows DLL 导出表逐条核实）：

```
libcangjie-runtime.dll : InitCJRuntime / RunCJTask / ReleaseHandle     ✔
libcangjie-std-core.dll: _CGPatirHv（std.core 包 init）                ✔
libcangjie-std-collection.dll: _CGPacirHv                              ✔
libcangjie-std-ast.dll: _CGPaxirHv                                     ✔
```

即：Rust 侧 `GetProcAddress` 要找的每一个符号都真实存在于官方 Windows SDK 的 DLL 中。

### 2.3 展开路径接线（expander.rs）

`expander.rs` 的缓存展开分支以
`#[cfg(any(target_os = "linux", target_os = "windows"))]` 门控：

```rust
if let Some(dir) = ctx.pkg_dir {
    if let Ok((lib, pkg_name)) = ctx.cache.compile_macro_package(dir) {
        let expanded = ctx.cache.expand_cached(&key, || {
            match crate::dylib::expand_macro_call(&lib, name, &pkg_name, args) { ... }
        });
        ...
    }
}
```

任何失败（无 SDK、加载/取符号失败、调用错误）都回退到 quote 模板展开，LSP 永不被
宏库拖垮 —— Windows 与 Linux 同一条回退链。

### 2.4 宏包编译缓存（macro_cache.rs，T48 跨平台化）

- 产物发现：`find_macro_so` 按平台匹配 `lib-macro_*.so` / `lib-macro_*.dll`
  （cjpm 输出仍在 `target/release/<pkg>/` 下，仅扩展名不同）。
- 缓存文件：`<sdk>/../.macro-cache/lib-macro-<srcHash>.so` ⇄ `.dll`。
- cjpm 调用：Linux `bash -c "source <envsetup.sh> && cjpm build"`（原样）；
  Windows 用完整路径定位 `cjpm.exe`（`<sdk>/tools/bin` 或 `<sdk>/bin`），在子进程上
  设 `CANGJIE_HOME` 并把 SDK bin 目录与 `runtime/lib/windows_x86_64_cjnative`
  前置到 `PATH`（镜像 envsetup.bat 语义；Windows 上 PATH 兼作程序与 DLL 搜索路径）。

---

## 3. 在 Windows SDK 编译宏包 + LSP 加载（使用说明）

前置：安装 Windows 版 Cangjie SDK（含 cjpm.exe 与
`runtime/lib/windows_x86_64_cjnative/`），SDK 根目录可用环境变量 `CANGJIE_HOME`
指向，或与 Linux 侧一致放在默认位置（本仓库 `macro_cache::sdk_root` 先查
`CANGJIE_HOME`，再回退默认路径）。

### 3.1 手动编译宏包（推荐，便于排障）

```bat
:: 在宏包根目录（含 cjpm.toml）
call <SDK>\envsetup.bat
cjpm build
:: 产物：target\release\<pkg>\lib-macro_<fullPkgName>.dll
```

宏包源码示例（`src/p/p.cj`）：

```cangjie
macro package demo.p

import std.ast.*

public macro Wrap(x: Tokens): Tokens {
    quote(print($(x)))
}
```

在 `main.cj` 中 `import demo.p.*` 并调用 `@Wrap(42)`。LSP 打开该项目时，若
`pkg_dir` 能定位到该项目根（含 `cjpm.toml`），会走缓存展开路径：

1. `compile_macro_package` 找到/触发构建 `lib-macro_demo.p.dll`；
2. `expand_macro_call` 用 `LoadLibraryW` 打开该 DLL，`GetProcAddress` 取
   `macroCall_c_Wrap_demo_p`；
3. 在 Cangjie 运行时任务上调用该符号，`GetTokensFromBytes` 解析返回的 token 序列；
4. 渲染为文本预览挂到诊断上（“报错带完整宏展开预览”）。

### 3.2 失败时的行为

任一环节失败（未装 SDK、DLL 缺失、符号缺失、调用超时）→ `expand_macro_call` 返回
`Err` → 回退到文件内 quote 模板展开；若宏无定义则报告 `unresolved macro`。LSP
不会崩溃。

---

## 4. 验证结果（T48）

| 门禁 | 命令 | 结果 |
|---|---|---|
| Windows debug 交叉编译 | `cargo build --target x86_64-pc-windows-gnu -p cj-lsp` | 0 error（仅 mingw ld 已知误报 `corrupt .drectve`，T45 已文档化，release 链接不出现） |
| Windows release 交叉编译 | `cargo build --release --target x86_64-pc-windows-gnu -p cj-lsp` | 0 error / 0 warning |
| Windows target Clippy | `cargo clippy --workspace --target x86_64-pc-windows-gnu --all-targets -- -D warnings` | 0 |
| Windows 测试目标编译 | `cargo test --no-run --target x86_64-pc-windows-gnu -p cj-sema` | 0 error |
| Linux 回归 | `cargo test --workspace` | 112 passed, 0 failed |
| Linux Clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 0 |
| Linux fmt | `cargo fmt --all --check` | clean |
| Linux 宏真机 E2E | `cargo test -p cj-sema dylib::tests::dlopen_and_call_macro_so` | ok（真实 .so 构建 + dlopen + 展开 ×2） |

契约级验证（不需要 Windows 真机即可核实）：

- 官方 `GetMacroFuncName` / `LIB_SUFFIX` / `FindMacroDefPkg` 与 `dylib.rs` 符号构造
  逐字符一致；
- 官方 Windows SDK DLL 的导出表（`macroCall_c_*`、`InitCJRuntime`、`RunCJTask`、
  `ReleaseHandle`、`_CGPatirHv` / `_CGPacirHv` / `_CGPaxirHv`）已用
  `x86_64-w64-mingw32-objdump` 逐条核实；
- token 字节格式跨平台一致（同一 `TokenSerialization.cpp`，双端小端）。

### 4.1 已知限制 / 待真机验证

以下项依赖 Windows 真机（本仓库只在 Linux host 上交叉编译），T48 未覆盖：

1. **Windows 运行时真机初始化**：`init_runtime` 的 Windows 分支（PATH 前置 +
   `LoadLibraryW` 预加载 + `RunCJTask` 包初始化）只做了编译期验证，需在 Windows +
   Cangjie Windows SDK 上跑一次 `dlopen_and_call_macro_so` 等价用例。
2. **Windows cjpm 构建宏包**：`build_macro_package` 的 Windows 分支按
   envsetup.bat 语义实现（完整路径定位 `cjpm.exe` + 子进程 env），仅交叉编译通过；
   需确认 SDK 中 `cjpm.exe` 的实际位置（`tools/bin` 或 `bin`）。
3. **跨 CRT 的 `free()`**：Windows 官方宏 DLL 用其自带 CRT 分配返回缓冲，本侧用
   `libc::free` 释放（与官方 `InvokeMacroFunc` 相同）。mingw-w64 与 MSVC CRT 混用
   时 `free` 的边界需真机验证。
4. **MSVC vs GNU 工具链**：本仓库 CI 用 `x86_64-pc-windows-gnu` 交叉编译；Windows
   本机用 MSVC 工具链时属另一构建配置，需单独验证。

---

## 5. 相关文件

- `crates/cj-sema/src/dylib.rs` —— 加载平台层 + 运行时初始化 + token 序列化
- `crates/cj-sema/src/macro_cache.rs` —— 宏包编译缓存（平台相关产物发现/构建）
- `crates/cj-sema/src/expander.rs` —— 展开管线与回退
- `tools/ci.sh` —— 全平台门禁（第 8c–8e 步为 Windows 交叉编译 + clippy）
- 官方参考：`cangjie_compiler/src/Utils/Utils.cpp`（GetMacroFuncName）、
  `include/cangjie/Macro/MacroCall.h`（LIB_SUFFIX）、
  `src/Macro/MacroCallResolve.cpp`（FindMacroDefPkg）、
  `src/Macro/MacroEvaluationCJNative.cpp`（InvokeMacroFunc）
