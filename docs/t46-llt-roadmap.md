T46 LLT 提升路线图（从 blocked worker 日志抢救的分析成果）
============================================================
来源: hermes kanban log t_e157d2b0（150 轮耗尽前定位的修复点，代码未落盘）
基线: success=4640 (86.7%), fail=711, crash=0
目标: 89% = ≥4763 success, 需再修 123
预期: 修复约翻转 30-35 文件 → success ≈4725± (88.3-88.4%)

已定位修复点（按优先级）:
1. crates/cj-parser/src/expr.rs —— 两处:
   - `f()` 声明 (ok_func_nest、ok_is_as、box_in_array): parse_lambda 的 body
     在 paramLists.size()>1 时报 parse_expected_left_brace（官方
     ParseDecl.cpp:1923/1994 证据）
2. crates/cj-parser/src/decl.rs —— 两处:
   - FUNC 分支的 name 匹配需允许 LPAREN 后接成员名
     (parse_support_keyword_context/decl/class.cj 第 8 行 public(…))
   - FOREIGN 后置 LCURL 处理（decl.rs 加 FOREIGN 后置 LCURL 分支）
3. import 花括号内 {X as alias} 已在 WIP parser.rs
4. 其他 82 例 fixable 分布见 tools/t42_classify.py（FIXABLE=145 大类）

注意: 3-way 应用到新 master 基线（86.7% 是 T48 合并后的值）
验证: python3 tools/llt_baseline.py --jobs 8 --out /tmp/llt_t46b.txt
      crash=0 必须保持; Windows 交叉编译 0 error