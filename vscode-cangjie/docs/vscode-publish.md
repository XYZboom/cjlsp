# vscode-cangjie 发布到 VSCode Marketplace 操作手册

目标：把 `vscode-cangjie` 发布到 [open-vsx / VSCode Marketplace](https://marketplace.visualstudio.com)，
让用户能在扩展商店里一键安装。

本手册覆盖：注册 Azure DevOps → 创建 Publisher → 生成 PAT → `vsce login` →
本地打包验证 → `vsce publish` → 版本号管理。全部步骤均可从命令行执行
（`vscode-cangjie/` 目录下，Node.js >= 16 + 已 `npm install`）。

---

## 0. 关键事实（先读）

- 发布者（publisher）名 = `cangjie-lang`，已写入 `package.json` 的 `publisher` 字段，
  发布到 Marketplace 后**不可更改**。
- Marketplace 的展示页 = `README.md`（描述）+ `LICENSE` + `icon.png` + （若有）`CHANGELOG.md`。
- 发布凭据是 **Azure DevOps PAT**，不是 GitHub token。
- 流程本质就三步：`vsce login`（一次性）→ `vsce package`（验证）→ `vsce publish`（上传）。

---

## 1. 一次性准备（只需做一次）

### 1.1 微软账号
需要一个 Microsoft 账号（outlook/hotmail/live 或已有的微软账号）。
用该账号登录下面的所有门户。

### 1.2 注册 Azure DevOps 组织
1. 打开 <https://dev.azure.com>，用微软账号登录。
2. 首次进入会引导创建组织（Organization），例如命名为 `cangjie-lang`。
   组织名随意，只影响 PAT 的归属，不影响发布者名。

### 1.3 创建发布者（Publisher）
两种方式任选其一，**推荐方式 B**：

**方式 A —— 网页创建**
1. 打开 <https://marketplace.visualstudio.com/manage>（会自动要求登录微软账号）。
2. 点 `Create Publisher`，填写：
   - Name：`cangjie-lang`（**必须与 package.json 的 publisher 字段一致**，
     且全站唯一、只允许小写字母/数字/连字符）
   - Display name：`Cangjie Language`
   - Description：一行简介，例如 “Cangjie language services for VSCode (LSP)”
   - 建议上传 logo（可用仓库根目录的 `icon.png`）。

**方式 B —— 命令行创建**
```bash
cd vscode-cangjie
npx @vscode/vsce create-publisher cangjie-lang
# 交互式输入 display name 与描述
```

### 1.4 生成 PAT（Personal Access Token）
1. 打开 <https://dev.azure.com/<你的组织>>，点击右上角头像 → **Personal Access Tokens**
   （或直接访问 <https://dev.azure.com/users/me/tokens>）。
2. 点 **+ New Token**，填：
   - Name：`vsce-publish`
   - Organization：`All accessible organizations`（或你的组织）
   - Scopes / 权限：
     - 新 UI：`Marketplace` → 勾选 **Manage**（发布扩展需要 Manage 权限；
       `Acquire` 只够查询/购买，不能 publish）。
     - 老 UI：勾选 `Marketplace` 下的 `Acquire` 和 `Manage` 两项。
   - Expiration：建议按需选择（最长 1 年），到期前记得刷新。
3. 点创建后 **立即复制** PAT 字符串（只显示一次，丢失只能重建）。

> 🔒 安全：PAT 就是发布密码。不要提交进 git；CI 里用 `VSCE_PAT` 环境变量注入。

---

## 2. 登录（一次性）

在 `vscode-cangjie/` 目录下：

```bash
npx @vscode/vsce login cangjie-lang
```

交互式提示输入 PAT，粘贴刚才复制的令牌并回车。成功提示
`Added user '<组织>' to the list of published extension publishers.`

令牌存于 `~/.vsce/` 目录（Linux 无 keytar 时以明文文件保存；请只放在个人机器上）。
如需更换账号：删除 `~/.vsce/publishers/cangjie-lang` 后重新 login。

CI / 非交互场景可免登录，直接用环境变量注入 PAT：

```bash
export VSCE_PAT=<你的PAT>
npx @vscode/vsce publish   # 存在 VSCE_PAT 时跳过 login
```

---

## 3. 本地打包验证（每次发布前必做）

```bash
cd vscode-cangjie
npm install                          # 恢复 vscode-languageclient 依赖
npx @vscode/vsce package             # 生成 vscode-cangjie-<version>.vsix
npx @vscode/vsce ls                  # 预览打进 vsix 的文件清单
```

检查 `vsce ls` 输出必须包含：

| 条目 | 说明 |
|------|------|
| `out/extension.js` | 客户端入口（package.json `main`） |
| `bin/linux/LSPServer` | Linux 捆绑服务器 |
| `bin/win32/LSPServer.exe` | Windows 捆绑服务器 |
| `icon.png` | 商店图标（package.json `icon` 引用） |
| `README.md` / `LICENSE` | Marketplace 展示 + 许可证 |
| `node_modules/...` | production 依赖 `vscode-languageclient`（vsce 自动打包，**必须有**） |

> 注意：`.gitignore` 排除了 `node_modules/`，但 `vsce` 会把 package.json 的
> **production dependencies** 单独打包进 vsix —— 所以扩展在安装后仍能找到
> `vscode-languageclient`。若 `vsce ls` 里没有 node_modules，说明打包有问题，
> 安装后会报 `Cannot find module 'vscode-languageclient'`。

本地验证通过后再发布。

---

## 4. 发布

### 4.1 首次发布
```bash
cd vscode-cangjie
npx @vscode/vsce publish
```
使用当前 `package.json` 的版本（`0.1.0`）上传。成功后输出扩展页地址：
`https://marketplace.visualstudio.com/items?itemName=cangjie-lang.vscode-cangjie`
用户即可在 VSCode 扩展商店搜索 `Cangjie` 一键安装。

### 4.2 后续版本发布
改完代码后，**先 bump 版本再 publish**（Marketplace 不允许两个相同的版本号）：

```bash
# 自动 bump patch 并发布：0.1.0 -> 0.1.1
npx @vscode/vsce publish patch

# 等价于手动改 package.json 的 version 后：
npx @vscode/vsce publish
```

`patch` / `minor` / `major` 三个参数会先改 `package.json` 版本号再上传。
`vsce` 不会提交 git，改完版本号后记得手动提交：

```bash
git add vscode-cangjie/package.json vscode-cangjie/docs ...
git commit -m "chore: bump vscode-cangjie to 0.1.1"
```

---

## 5. 本仓库的版本号管理约定

- `vscode-cangjie/` 是**独立目录**，不进 `cargo` workspace；`package.json`
  版本号独立维护。
- 每次功能性改动 / 修复 / 打包配置调整后，发布前 bump 版本（patch 为主，
  新功能用 minor）。
- 发布的每次版本都要在本地跑通 `vsce package` 验证（第 3 节）。
- 建议同步维护 `CHANGELOG.md`（放在 `vscode-cangjie/` 下），Marketplace
  会自动渲染它。

---

## 6. 常见问题

| 现象 | 原因 / 处理 |
|------|-------------|
| `Publisher 'cangjie-lang' is not known` | 未创建 publisher，或 package.json 的 publisher 名与 1.3 步创建的不一致 |
| `The Personal Access Token ... does not have the Marketplace Manage scope` | PAT 范围不对，回 1.4 重建，勾选 **Marketplace → Manage** |
| `Request ... 401 / AADSTS...` | PAT 过期，重建并重新 login |
| `version '0.1.0' already exists` | 版本号撞车，bump 版本（4.2 节） |
| `/vscode.extensions... ENOENT` | 后台发布服务偶发；重试一次即可 |
| 扩展安装后报 `Cannot find module 'vscode-languageclient'` | vsix 没打进 production 依赖，回 3 节检查 `vsce ls` |

---

## 7. （可选）自动化发布

仓库目前没有 CI。以后接入 GitHub Actions 时可以这样做：

```yaml
# .github/workflows/publish.yml（示例）
on:
  push:
    tags: ['vscode-cangjie-v*']
jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: npm, cache-dependency-path: vscode-cangjie/package-lock.json }
      - run: npm install --prefix vscode-cangjie
      - run: npx @vscode/vsce publish --packagePath vscode-cangjie/vscode-cangjie-*.vsix
        working-directory: vscode-cangjie
        env:
          VSCE_PAT: ${{ secrets.VSCE_PAT }}
```

即在 GitHub 仓库 Settings → Secrets 里加 `VSCE_PAT`（值 = 1.4 节生成的 PAT），
打 tag 即自动发布。

---

## 8. 未做项（有意为之）

- **badges**：包 `badges` 字段未加 —— 商店徽章需要外部托管图片（如 GitHub
  Actions 徽章）。仓库暂无 CI，等接入 CI 后再补。
  ```json
  "badges": [
    { "url": "https://img.shields.io/.../build.svg",
      "href": "https://github.com/XYZboom/cjlsp/actions",
      "description": "CI" }
  ]
  ```
- macOS 捆绑二进制：`bin/darwin/` 目前是占位，未实际构建。在 macOS 上使用
  需手动指定 `cangjie.lsp.serverPath`。
- 上架 open-vsx（VSCodium / 中国市场镜像）：同流程，
  `npx ovsx publish vscode-cangjie-0.1.0.vsix -p <openvsx-token>`，需另外注册
  open-vsx 账号。