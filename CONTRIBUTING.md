# Cangjie Frontend & LSP 贡献指南 (CONTRIBUTING)

欢迎参与 `cj-lang`（仓颉编译器前端与 LSP 服务端）的开发与贡献！本文档规定了诊断系统的设计规范、新增与增强诊断的闭环流程，以及本地与 CI 性能门禁规范。

---

## 1. 诊断错误码规范 (Diagnostic Code Conventions)

### 1.1 命名规范与分类
- **命名规则**：
  - 采用小写蛇形命名（snake_case）字符串，对应官方定义中的稳定标识符。
  - Rust 枚举 `DiagId` 采用全大写下划线标识符（例如 `DiagId::PARSE_EXPECTED_RIGHT_DELIMITER`），对应稳定的 JSON `code` 字段为 `"parse_expected_right_delimiter"`。
  - 严禁在外部输出、JSON 或 LSP 通信中使用枚举序号（ordinal）或临时数值；必须输出稳定的字面量字符串 `code`。
- **Category 生产者分类**：
  - `parser`：语法与文法解析阶段产生的错误。
  - `lexer`：词法分词阶段产生的错误。
  - `sema`：语义分析、类型检查、符号解析与作用域检查阶段产生的错误。
  - `chir`：轻量级控制流或高级语义阶段产生的错误。

### 1.2 官方定义与生成流程 (`tools/gen_diags.py`)
- 诊断模板源自官方编译器的定义文件（`cangjie_compiler/include/cangjie/Basic/DiagRefactor/Diagnostic*.def`）。
- 生成工具：`tools/gen_diags.py`
  - 该脚本自动定位官方仓定义，解析 `ERROR(...)` 和 `WARNING(...)` 宏定义；
  - 生成 `crates/cj-diag/src/templates.rs` 中的 `DiagId` 枚举、`template(id: DiagId) -> DiagTemplate`、`code()` 及 `category()` 映射。
- **重新生成命令**：
  ```bash
  python3 tools/gen_diags.py > crates/cj-diag/src/templates.rs
  ```
- **注意**：切勿直接手动修改 `templates.rs`。若官方 `.def` 更新或添加新定义，应通过该脚本重新生成。

---

## 2. 诊断增强的三步闭环 (3-Step Diagnostic Enhancement Workflow)

当为编译器前端增加新的诊断检查或增强现有错误信息的结构化字段时，必须严格执行三步闭环：

### 步骤一：添加 in-tree 黄金样例 (Add In-Tree Golden Fixtures)
- 在 `tests/diagnostics/<case-name>/` 下创建测试样例目录，命名贴合错误特性（例如 `tests/diagnostics/type-mismatch/`）。
- 目录内包含两个核心文件：
  1. `input.cj`：触发该诊断的最小可复现 Cangjie 源码。
  2. `expected.json`：期望的前端输出诊断 JSON，必须包含 `schemaVersion`（当前为 `1`）以及具体的诊断项（包含 `severity`、`code`、`category`、`location`、`message`、`notes` 等字段）。
- 集成测试 `crates/cj-frontend/tests/diagnostic_fixtures.rs` 会自动遍历 `tests/diagnostics/` 下的所有样例，执行深比较校验。

### 步骤二：实现与接入结构化字段 (Implement and Connect Structured Fields)
- 在对应的分析模块（`cj-parser`、`cj-sema` 等）中实现或补充诊断逻辑：
  - 使用 `Diag::error(...)` 或 `Diag::warning(...)` 构建诊断。
  - 调用 `.with_id(DiagId::...)` 关联官方错误码与分类。
  - 根据官方格式提供精确的 span（`.with_span(...)`）、错误指引（`.with_here(...)`）、补充说明（`.with_note(...)`）。
  - 如有需要，填写结构化诊断字段：`expected`、`actual`、`candidates`、`related_locations`、`suggestions`。
- 确保 `cj-frontend --diagnostic-format=json <input.cj>` 输出的 JSON 与期望严格匹配。

### 步骤三：本地验证与性能门禁 (Local Verification & Gates)
- 执行单元测试与集成测试验证：
  ```bash
  cargo test -p cj-frontend
  cargo test --workspace
  ```
- 检查代码风格与 Clippy 警告：
  ```bash
  cargo clippy --workspace -- -D warnings
  ```
- 运行性能门禁，确保新诊断不会引入非预期的性能衰退：
  ```bash
  python3 -m unittest tools/test_perf_gate.py
  python3 tools/perf_gate.py --baseline tools/bench_baseline.txt --head tools/bench_baseline.txt
  ```

---

## 3. 性能门禁指南 (Performance Gate Guide)

### 3.1 Criterion 1.05x 门禁原理
- 诊断管线（词法分析 + 语法解析 + 本地语义分析 + 诊断格式化）处于编译器前端热路径上。为了防止新特性的加入导致解析耗时劣化，项目引入了 **1.05x（最大允许变慢 5%）** 的性能门禁。
- 采用中位数（Median）比对：
  $$\text{ratio} = \frac{\text{median}(\text{head samples})}{\text{median}(\text{baseline samples})} \le 1.05$$
  使用中位数而非单次测量平均值，能够有效消除同机系统抖动、缓存预热差异等噪音干扰。

### 3.2 使用 `tools/perf_gate.py` 进行同机基线比对
`tools/perf_gate.py` 支持多种比对方式：

1. **同机已有基线文件与当前运行比对**：
   ```bash
   # 1. 运行当前分支的 Criterion benchmark
   cargo bench -p cj-lsp --bench bench_frontend -- 'pipeline/full_diagnostics' --noplot > target/perf-head.txt

   # 2. 与同机基线（例如 tools/bench_baseline.txt）比对
   python3 tools/perf_gate.py --baseline tools/bench_baseline.txt --head target/perf-head.txt --max-ratio 1.05
   ```

2. **自检基线一致性（验证门禁脚本自身）**：
   ```bash
   python3 tools/perf_gate.py --baseline tools/bench_baseline.txt --head tools/bench_baseline.txt
   ```

3. **双命令同机动态比对**：
   ```bash
   python3 tools/perf_gate.py \
     --baseline-command "git stash && cargo bench -p cj-lsp --bench bench_frontend -- 'pipeline/full_diagnostics' --noplot && git stash pop" \
     --head-command "cargo bench -p cj-lsp --bench bench_frontend -- 'pipeline/full_diagnostics' --noplot" \
     --max-ratio 1.05
   ```

4. **CI 环境下的集成**：
   - 在 `tools/ci.sh` 中，可通过设置环境变量 `PERF_BASELINE_FILE=tools/bench_baseline.txt` 或 `PERF_BASELINE_COMMAND` / `PERF_HEAD_COMMAND` 触发性能门禁检查。

---

## 4. 提交与 PR 规范 (Commit & PR Guidelines)

- 提交信息遵循 Conventional Commits 规范，例如：
  - `feat(parser): add diagnostics for high-frequency expression errors`
  - `docs(contrib): add diagnostics contributing guide and in-tree regression fixtures`
- 仅添加修改相关的文件，严禁盲目执行 `git add -A`。提交前务必执行：
  ```bash
  git diff --cached --stat
  ```
- 保持工作区整洁，不提交临时测试文件、缓存与无关构建产物。
