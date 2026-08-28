# vscode-cangjie

仓颉（Cangjie）语言 VSCode 扩展：作为 LSPServer 的客户端，在 VSCode 中提供
仓颉语言服务。

底层是 cj-lang 项目（Rust）构建出的 `LSPServer` 二进制，走标准 LSP over stdio
（JSON-RPC 2.0 + Content-Length 帧），因此本扩展只是很薄的一层语言客户端。

## 提供的功能

- 诊断（diagnostics）：打开 / 修改 `.cj` 文件时，服务端推送语法与语义错误
- 补全（completion）
- 悬停（hover）
- 跳转定义（definition）
- 查找引用（references）

> 注：`LSPServer` 的 initialize 响应里声明了语义高亮 / 文档符号 / 重命名 /
> 文档高亮等能力，但当前 dispatch 尚未实现；扩展已在客户端把这些未实现项
> 屏蔽，避免编辑时出现 "Request ... failed" 报错噪音。以上五项是当前真正可用
> 的功能。

## 环境要求

- VSCode >= 1.85
- Node.js >= 16（仅用于 `npm install` 与打包）

扩展随 VSIX 捆绑了各平台的 `LSPServer` 二进制（`bin/linux`、`bin/win32`、`bin/darwin`），
安装后开箱即用、零配置。仅当你想使用**自己新构建的**服务器（例如 `cargo build -p cj-lsp`
产出的 debug 版本）时，才需要手动指定路径。

## 安装

1. **从 Marketplace 一键安装**（推荐）：在 VSCode 扩展商店搜索 `Cangjie`，点击安装即可，
   无需构建任何东西。
2. **从 VSIX 安装**：`npx @vscode/vsce package` 生成 `vscode-cangjie-0.1.0.vsix`，然后在
   VSCode 中「扩展 → 右上角 `...` → 从 VSIX 安装…」。
3. **开发调试（F5）**：用 VSCode 打开本目录，按 F5（会启动 Extension Development Host，
   见 `.vscode/launch.json`）。
4. **直接复制**：把本目录（含 `node_modules`）复制到 `~/.vscode/extensions/`，重启 VSCode。

打开任意 `.cj` 文件即自动激活。若使用自定义服务器，参考下面的「配置」。

## 配置

在 VSCode 设置（JSON）中（不配置即为捆绑版默认行为）：

```jsonc
{
  // LSPServer 二进制路径（可选；默认使用捆绑的 bin/<平台>/LSPServer，也支持环境变量 CANGJIE_LSPSERVER）
  "cangjie.lsp.serverPath": "<绝对路径>/cj-lang/target/debug/LSPServer",

  // 追加给 LSPServer 的额外启动参数
  "cangjie.lsp.extraArgs": [],

  // 传给 LSPServer 进程的额外环境变量（例如宏运行时所需的 LD_LIBRARY_PATH）
  "cangjie.lsp.env": {
    "LD_LIBRARY_PATH": "<SDK路径>/runtime/lib/linux_x86_64_cjnative"
  }
}
```

## 故障排查

- 启动时提示 `LSPServer binary not found`：确认 `cargo build -p cj-lsp` 已执行，
  或在设置里把 `cangjie.lsp.serverPath` 指向真实二进制路径。
- 想看服务端日志：以 `--enable-log=true` 启动，或在扩展开发调试（F5）模式运行
  （debug 模式会自动追加该参数），日志输出到「输出」面板的
  `Cangjie Language Server` 通道。
- 环境变量 `CANGJIE_LSPSERVER` 优先级高于扩展内置默认路径，低于
  `cangjie.lsp.serverPath` 设置。

## 目录结构

```
vscode-cangjie/
├── package.json              # 扩展清单：语言声明 + 激活事件 + 配置项 + 发布元数据
├── out/extension.js          # 语言客户端（vscode-languageclient），纯 JS 无编译步骤
├── bin/                      # 捆绑的 LSPServer（linux / win32 / darwin，按平台自动选择）
├── icon.png                  # Marketplace 图标
├── language-configuration.json  # 注释 / 括号 / 自动闭合
├── README.md
├── docs/vscode-publish.md    # 发布到 Marketplace 的操作手册
└── .vscode/launch.json       # F5 扩展开发调试
```

## 开发者：跨平台构建与 CI 门禁

仓库根目录的 `tools/ci.sh` 是完整 CI 管道，T45 起含**全平台门禁**：Linux
debug+release 构建、Windows GNU 交叉编译、clippy 双 target（linux &
windows-gnu）、单元测试、LSP 诊断覆盖率、SCAN Parser 对齐、宏 E2E。推送前
跑一次即可全部验证：

```bash
cd <仓库根目录>
./tools/ci.sh            # 完整管道
./tools/ci.sh -j 12      # 并行构建（12 核）
```

跨平台构建命令速查（在仓库根目录执行）：

| 目标平台 | 命令 | 产物 |
|---|---|---|
| Linux（宿主） | `cargo build --release -p cj-lsp -p cj-frontend` | `target/release/LSPServer`、`target/release/cj-frontend` |
| Windows 交叉编译 | `cargo build --release --target x86_64-pc-windows-gnu -p cj-lsp -p cj-frontend` | `target/x86_64-pc-windows-gnu/release/LSPServer.exe`、`…/cj-frontend.exe` |

> 说明：`cj-lsp` 与 `cj-frontend` 两个 crate 会拉入全部依赖（含宏动态加载
> `cj-sema/dylib.rs` 的 `cfg(windows)` 实现），因此交叉编译即覆盖全工作区。
> 如需整工作区校验，可换 `cargo build --target x86_64-pc-windows-gnu`。

Windows 交叉编译前置条件（一次性准备）：

```bash
rustup target add x86_64-pc-windows-gnu          # rustup 目标
sudo apt install gcc-mingw-w64-x86-64            # 链接器（x86_64-w64-mingw32-gcc）
cargo clippy --workspace --target x86_64-pc-windows-gnu -- -D warnings   # Windows target 单独 clippy
```

> 已知良性提示：Windows debug 链接时 mingw ld 可能打印
> `corrupt .drectve at end of def file`（release 链接不出现），是 ld 的
> 误报，不影响产物，ci.sh 中该步骤不因 warning 失败。

## License

MIT
