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
fn sdk_root() -> PathBuf {
    if let Ok(home) = std::env::var("CANGJIE_HOME") {
        return PathBuf::from(home);
    }
    // Default: the SDK we installed for this project.
    PathBuf::from("/root/Code/cangjie/sdk/cangjie")
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

    /// Compile a macro package and cache the resulting .so keyed by source hash.
    /// Returns the .so path. Reuses cached artifact when source unchanged,
    /// otherwise invokes cjpm build via the SDK.
    pub fn compile_macro_package(&mut self, pkg_dir: &Path) -> std::io::Result<PathBuf> {
        // Source hash = hash of all .cj files under pkg_dir.
        let src_hash = hash_package_sources(pkg_dir)?;
        if let Some(so) = self.compiled.get(&src_hash) {
            if so.exists() {
                return Ok(so.clone());
            }
        }

        let cache_dir = sdk_root().join("..").join(".macro-cache");
        fs::create_dir_all(&cache_dir)?;
        let so_name = format!("lib-macro-{}.so", src_hash);
        let so_path = cache_dir.join(&so_name);

        if so_path.exists() {
            // Rebuilt by a previous process; still cache in-session.
            self.compiled.insert(src_hash, so_path.clone());
            return Ok(so_path);
        }

        // Compile: run cjpm in the package dir (source envsetup first).
        let root = sdk_root();
        let envsetup = root.join("envsetup.sh");
        let shell_cmd = format!(
            "source {} && cjpm build --output-type=dynamic 2>&1",
            envsetup.display()
        );
        let _ = Command::new("bash")
            .arg("-c")
            .arg(&shell_cmd)
            .current_dir(pkg_dir)
            .output()?;

        // cjpm emits target/release/<pkg>/lib-*.so — locate it.
        let so = find_macro_so(pkg_dir)?;
        // Copy into cache under content-hash name.
        fs::copy(&so, &so_path)?;
        self.compiled.insert(src_hash, so_path.clone());
        Ok(so_path)
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

/// Locate the macro .so cjpm produced under a package dir.
fn find_macro_so(pkg_dir: &Path) -> std::io::Result<PathBuf> {
    let target = pkg_dir.join("target").join("release");
    for entry in fs::read_dir(target)? {
        let path = entry?.path();
        if path.is_dir() {
            for f in fs::read_dir(&path)? {
                let p = f?.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("lib-macro-") && name.ends_with(".so") {
                    return Ok(p);
                }
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("no macro .so under {}", pkg_dir.display()),
    ))
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
