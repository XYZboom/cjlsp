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
- 已构建好的 `LSPServer`（`cd /root/Code/cangjie/cj-lang && cargo build -p cj-lsp`）

## 安装

1. 构建语言服务器：

   ```bash
   cd /root/Code/cangjie/cj-lang
   cargo build -p cj-lsp
   # 产物：target/debug/LSPServer（默认配置即指向它）
   ```

2. 安装依赖：

   ```bash
   cd vscode-cangjie
   npm install
   ```

3. 三种加载方式任选其一：

   - **开发调试（F5）**：用 VSCode 打开本目录，按 F5（会启动 Extension
     Development Host，见 `.vscode/launch.json`）。
   - **打包为 VSIX**：`npx @vscode/vsce package`，生成 `vscode-cangjie-0.1.0.vsix`，
     然后在 VSCode 中「扩展 → 右上角 `...` → 从 VSIX 安装…」。
   - **直接复制**：把本目录（含 `node_modules`）复制到 `~/.vscode/extensions/`，
     重启 VSCode。

4. 打开任意 `.cj` 文件即自动激活。

## 配置

在 VSCode 设置（JSON）中：

```jsonc
{
  // LSPServer 二进制路径（默认已指向本地构建产物；也支持环境变量 CANGJIE_LSPSERVER）
  "cangjie.lsp.serverPath": "/root/Code/cangjie/cj-lang/target/debug/LSPServer",

  // 追加给 LSPServer 的额外启动参数
  "cangjie.lsp.extraArgs": [],

  // 传给 LSPServer 进程的额外环境变量（例如宏运行时所需的 LD_LIBRARY_PATH）
  "cangjie.lsp.env": {
    "LD_LIBRARY_PATH": "/root/Code/cangjie/sdk/cangjie/runtime/lib/linux_x86_64_cjnative"
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
├── package.json              # 扩展清单：语言声明 + 激活事件 + 配置项
├── out/extension.js          # 语言客户端（vscode-languageclient），纯 JS 无编译步骤
├── language-configuration.json  # 注释 / 括号 / 自动闭合
├── README.md
└── .vscode/launch.json       # F5 扩展开发调试
```

## License

MIT
