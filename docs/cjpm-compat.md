# LSP 与官方 cjpm 兼容性验证

实测项目: /tmp/cjpmproj (mainpkg 依赖 lib, [dependencies] path 本地依赖, 官方 cjpm 1.1.3 build 通过)

验证通过:
1. 项目根识别: cjpm.toml 祖先目录优先于 cwd 推断 (cjpm.rs find_project_root)
2. 源码目录扫描: src-dir + [[source-set]] + 本地依赖包 (cjpm.rs scan_dirs)
3. 包可见性: 同包 + import + path 依赖 (cjpm.rs visible_packages)
4. 跨包跳转: main.cj 点击 Helper/msg -> 跳到 lib/src/helper.cj (definition)
5. uri 规范化: 返回 file:///tmp/cjpmproj/lib/src/helper.cj (无 ../)

官方 cjpm.toml 关键字段: [package] name/version/cjc-version/src-dir/output-type + [dependencies] path
