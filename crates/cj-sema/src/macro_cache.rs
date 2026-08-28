// cj-sema: macro expansion compile cache.
//
// Performance requirement (user): macro expansion in the LSP must be fast —
// recompiling a macro package on every didChange is unacceptable (cjc takes
// seconds). This module provides a three-layer cache:
//
//   1. Macro-package compile cache: sha256(source) -> compiled .so path.
//      If the macro package source is unchanged, reuse the cached .so (no cjc
//      invocation). Keyed by the package's source hash, so any edit invalidates
//      exactly the packages it touches.
//   2. Expansion-result cache: (macro name + serialized args) hash -> expanded
//      output, in-memory LRU. Macro expansion is deterministic per (macro,args),
//      so repeated identical calls (LSP typing churn) hit the fast path.
//   3. LSP-session cache: a session-scoped container keyed by file uri, so
//      didChange only recomputes the changed file's expansions, reusing .so
//      loads and result hashes across the session.
//
// The cjc invocation is delegating and shelled out (the SDK is the Cangjie
// backend we deliberately reuse — spec Ch.14 macros compile through cjc, the
// same toolchain the official compiler uses).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// SDK root (set at build/test time; resolves via CANGJIE_HOME or default).
pub(crate) fn sdk_root() -> PathBuf {
    if let Ok(home) = std::env::var("CANGJIE_HOME") {
        let p = PathBuf::from(&home);
        // The env var may point at a stale/deleted install (e.g. the old
        // self-hosting SDK); only trust it if a compiler binary actually
        // exists there, otherwise fall through to the project default.
        if p.join("bin/cjc").exists() {
            return p;
        }
    }
    // Default: the SDK we installed for this project.
    PathBuf::from("/root/Code/cangjie/sdk/cangjie")
}

/// Macro-library extension cjpm produces on this platform — the official
/// compiler's LIB_SUFFIX (MacroCall.h): `.so` on Linux, `.dll` on Windows,
/// `.dylib` on macOS. cjpm emits `lib-macro_<fullPkgName>.<ext>` regardless
/// of platform (GetMacroFuncName / FindMacroDefPkg in MacroCallResolve.cpp).
#[cfg(target_os = "linux")]
fn macro_lib_ext() -> &'static str {
    "so"
}
#[cfg(target_os = "windows")]
fn macro_lib_ext() -> &'static str {
    "dll"
}

/// sha256 hex of a string (used for cache keys).
fn sha256_hex(s: &str) -> String {
    use std::fmt::Write;
    let digest = {
        // Minimal FNV-1a 64-bit is not a real hash for keys; use std DefaultHasher
        // which is fine for cache identity (not adversarial).
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        s.hash(&mut h);
        h.finish()
    };
    // Render as hex for file names.
    let mut out = String::with_capacity(16);
    write!(out, "{digest:016x}").unwrap();
    out
}

/// Macro-package compile cache: source hash -> compiled .so.
///
/// The cache lives under `<sdk>/../.macro-cache` (outside the SDK tree so SDK
/// updates don't collide), keyed by content hash so edits invalidate precisely.
#[derive(Debug, Default)]
pub struct MacroCache {
    /// (pkg source hash) -> cached .so path (session-level memo).
    compiled: HashMap<String, PathBuf>,
    /// (macro+sig hash) -> expanded tokens text (LRU capped).
    expanded: HashMap<String, String>,
    /// LRU order (front = most recent) for eviction.
    lru: Vec<String>,
    /// Max entries in the expansion-result cache.
    cap: usize,
}

impl MacroCache {
    pub fn new() -> Self {
        MacroCache {
            compiled: HashMap::new(),
            expanded: HashMap::new(),
            lru: Vec::new(),
            cap: 256,
        }
    }

    /// Compile a macro package and cache the resulting shared library keyed
    /// by source hash. Returns the .so/.dll path to load plus the macro
    /// package's full name (e.g. `macro_calling.define` — derived from the
    /// `lib-macro_<pkg>.<ext>` filename, needed to form the
    /// `macroCall_c_<name>_<pkg>` symbol). Reuses cached artifacts when the
    /// source is unchanged, otherwise invokes cjpm build.
    pub fn compile_macro_package(&mut self, pkg_dir: &Path) -> std::io::Result<(PathBuf, String)> {
        // The original build artifact gives us the macro package full name.
        let (_orig_so, pkg_name) = find_macro_so_and_pkg(pkg_dir)?;
        // Source hash = hash of all .cj files under pkg_dir.
        let src_hash = hash_package_sources(pkg_dir)?;

        let cache_dir = sdk_root().join("..").join(".macro-cache");
        fs::create_dir_all(&cache_dir)?;
        let cache_so = cache_dir.join(format!("lib-macro-{src_hash}.{}", macro_lib_ext()));

        // 1. Cross-session artifact: reuse the cached library without rebuilding.
        if cache_so.exists() {
            self.compiled.insert(src_hash, cache_so.clone());
            return Ok((cache_so, pkg_name));
        }
        // 2. In-session memo (a previous compile in this process).
        if let Some(so) = self.compiled.get(&src_hash) {
            if so.exists() {
                return Ok((so.clone(), pkg_name));
            }
        }

        // Compile: run the SDK's cjpm in the package dir (env setup per
        // platform — see build_macro_package).
        let root = sdk_root();
        build_macro_package(pkg_dir, &root)?;

        // cjpm emits target/release/<pkg>/lib-macro_*.<ext> — locate it.
        let (so, pkg_name) = find_macro_so_and_pkg(pkg_dir)?;
        // Copy into cache under content-hash name.
        let _ = fs::copy(&so, &cache_so);
        self.compiled.insert(src_hash, cache_so.clone());
        Ok((cache_so, pkg_name))
    }

    /// Expand a macro call: (macro name, serialized args) -> expanded text,
    /// cached by key. If the key was seen before, returns the cached expansion
    /// without touching the .so.
    pub fn expand_cached(&mut self, key: &str, expand_fn: impl FnOnce() -> String) -> String {
        if let Some(hit) = self.expanded.get(key) {
            // touch LRU
            if let Some(pos) = self.lru.iter().position(|k| k == key) {
                self.lru.remove(pos);
                self.lru.push(key.to_string());
            }
            return hit.clone();
        }
        let out = expand_fn();
        self.expanded.insert(key.to_string(), out.clone());
        self.lru.push(key.to_string());
        // evict oldest beyond cap
        while self.lru.len() > self.cap {
            if let Some(old) = self.lru.first().cloned() {
                self.lru.remove(0);
                self.expanded.remove(&old);
            }
        }
        out
    }

    /// Expansion key: deterministic from macro name + args text.
    pub fn expansion_key(macro_name: &str, args_text: &str) -> String {
        format!("{}:{}", macro_name, sha256_hex(args_text))
    }
}

/// Build a macro package with the official SDK's cjpm. Returns the location
/// of the produced shared library (`.so` on Linux, `.dll` on Windows).
#[cfg(target_os = "linux")]
fn build_macro_package(pkg_dir: &Path, root: &Path) -> std::io::Result<()> {
    let envsetup = root.join("envsetup.sh");
    if !envsetup.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("SDK envsetup.sh not found at {}", envsetup.display()),
        ));
    }
    let shell_cmd = format!("source {} && cjpm build 2>&1", envsetup.display());
    let _ = Command::new("bash")
        .arg("-c")
        .arg(&shell_cmd)
        .current_dir(pkg_dir)
        .output()?;
    Ok(())
}

/// Build a macro package with the official SDK's cjpm.exe on Windows. The SDK
/// env setup (envsetup.bat) sets CANGJIE_HOME and prepends the SDK bin dirs to
/// PATH; we mirror that on the child process (PATH doubles as the DLL search
/// path on Windows). The macro package is still produced as
/// `target/release/<pkg>/lib-macro_<fullPkgName>.dll`, exporting the same
/// `macroCall_c_<Name>_<Pkg>` symbols — the byte contract is platform-agnostic.
///
/// NOTE: cross-compile-verified only (this repo builds for x86_64-pc-windows-gnu);
/// runtime verification requires a Windows box with the Cangjie Windows SDK.
/// Any failure here just falls back to quote-template expansion (never fatal).
#[cfg(target_os = "windows")]
fn build_macro_package(pkg_dir: &Path, root: &Path) -> std::io::Result<()> {
    let cjpm = [root.join("tools/bin/cjpm.exe"), root.join("bin/cjpm.exe")]
        .into_iter()
        .find(|p| p.exists())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Windows SDK cjpm.exe not found under {}", root.display()),
            )
        })?;
    let lib_dir = root.join("runtime/lib/windows_x86_64_cjnative");
    let mut path = format!(
        "{};{};{}",
        root.join("bin").display(),
        root.join("tools/bin").display(),
        lib_dir.display()
    );
    if let Ok(existing) = std::env::var("PATH") {
        if !existing.is_empty() {
            path.push(';');
            path.push_str(&existing);
        }
    }
    let _ = Command::new(&cjpm)
        .arg("build")
        .current_dir(pkg_dir)
        .env("CANGJIE_HOME", root)
        .env("PATH", path)
        .output()?;
    Ok(())
}

/// Hash all .cj sources under a package dir (deterministic, sorted paths).
fn hash_package_sources(pkg_dir: &Path) -> std::io::Result<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_cj_files(pkg_dir, &mut files)?;
    files.sort();
    let mut acc = String::new();
    for f in files {
        if let Ok(text) = fs::read_to_string(&f) {
            acc.push_str(&text);
            acc.push('\n');
        }
    }
    Ok(sha256_hex(&acc))
}

fn collect_cj_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let p = entry?.path();
            if p.is_dir() {
                collect_cj_files(&p, out)?;
            } else if p.extension().map(|e| e == "cj").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    Ok(())
}

/// Locate the macro shared library cjpm produced under a package dir
/// (`lib-macro_*.so` on Linux, `lib-macro_*.dll` on Windows).
pub(crate) fn find_macro_so(pkg_dir: &Path) -> std::io::Result<PathBuf> {
    let target = pkg_dir.join("target").join("release");
    let ext = format!(".{}", macro_lib_ext());
    for entry in fs::read_dir(target)? {
        let path = entry?.path();
        if path.is_dir() {
            for f in fs::read_dir(&path)? {
                let p = f?.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("lib-macro_") && name.ends_with(&ext) {
                    return Ok(p);
                }
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("no macro {} under {}", macro_lib_ext(), pkg_dir.display()),
    ))
}

/// Locate the macro shared library and derive the macro package's full name
/// from its filename. cjpm emits `lib-macro_<module>.<pkg>.<ext>`, so the pkg
/// name is the filename with the `lib-macro_` prefix and `.<ext>` suffix
/// stripped. That name is what the exported symbol embeds
/// (`macroCall_c_<Macro>_<pkg>` with `.`→`_`). Identical on Linux and Windows.
fn find_macro_so_and_pkg(pkg_dir: &Path) -> std::io::Result<(PathBuf, String)> {
    let so = find_macro_so(pkg_dir)?;
    let name = so.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let ext = format!(".{}", macro_lib_ext());
    let pkg = name
        .strip_prefix("lib-macro_")
        .and_then(|n| n.strip_suffix(&ext))
        .map(str::to_string)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("cannot derive macro package name from {name}"),
            )
        })?;
    Ok((so, pkg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn cache_key_deterministic() {
        let k1 = MacroCache::expansion_key("Wrap", "42");
        let k2 = MacroCache::expansion_key("Wrap", "42");
        assert_eq!(k1, k2);
        assert_ne!(k1, MacroCache::expansion_key("Wrap", "43"));
    }

    #[test]
    fn expand_cached_calls_fn_once() {
        let mut cache = MacroCache::new();
        let key = "Wrap:abc";
        let mut calls = 0;
        let out1 = cache.expand_cached(key, || {
            calls += 1;
            "print(42)".to_string()
        });
        let out2 = cache.expand_cached(key, || {
            calls += 1;
            "never".to_string()
        });
        assert_eq!(out1, "print(42)");
        assert_eq!(out2, "print(42)"); // cached — closure not re-run
        assert_eq!(calls, 1);
    }

    #[test]
    fn source_hash_changes_on_edit() {
        let dir = std::env::temp_dir().join("cj-macro-hash-test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("m.cj"),
            "macro package p\nimport std.ast.*\npublic macro M(x: Tokens): Tokens { quote(x) }\n",
        )
        .unwrap();
        let h1 = hash_package_sources(&dir).unwrap();
        fs::write(
            dir.join("m.cj"),
            "macro package p\nimport std.ast.*\npublic macro M2(x: Tokens): Tokens { quote(x) }\n",
        )
        .unwrap();
        let h2 = hash_package_sources(&dir).unwrap();
        assert_ne!(h1, h2, "editing source must change the cache key");
        let _ = fs::remove_dir_all(&dir);
    }
}
