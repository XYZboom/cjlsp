//! cjpm.toml project discovery for the LSP.
//!
//! cjpm is the official Cangjie package manager: a directory containing
//! `cjpm.toml` is the root of a package whose sources live under a
//! configurable `src-dir` (default `src`) or `[[source-set]]` directories,
//! with optional local-path `[dependencies]` for sibling packages. The LSP
//! uses this to (a) resolve the project root from a file's real on-disk
//! location (a `cjpm.toml` ancestor beats cwd-based inference) and (b) scope
//! the cross-file sibling scan to the package's source directories instead
//! of the whole tree (which would waste time on `target/`, `.git/`, ...).
//!
//! The parser is intentionally minimal: cjpm.toml files are machine-written
//! with a fixed shape, so a line-oriented reader suffices for the keys the
//! LSP consumes ([package] name/src-dir, [dependencies] `path`-backed
//! entries, [[source-set]] src-dir). It never fails on unknown keys.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// How many ancestors to walk when hunting for a `cjpm.toml`. Bounded so a
/// stray config far up (e.g. in `$HOME`) can't hijack the project root of a
/// file that belongs to no cjpm package; 16 generously covers deep
/// monorepo layouts while staying deterministic.
const MAX_UP: usize = 16;

/// Walk up from `uri_path`'s directory and return the nearest ancestor that
/// contains a `cjpm.toml` — the root of a cjpm package.
///
/// Only real directories are traversed: the walk stops at the first ancestor
/// that does not exist on disk, so virtual harness URIs (paths that do not
/// exist) fall through to the caller's cwd-based inference instead of
/// unrolling unrelated parents. Returns `None` for non-cjpm projects.
pub fn find_project_root(uri_path: &Path) -> Option<PathBuf> {
    let mut dir = uri_path.parent()?;
    for _ in 0..MAX_UP {
        if dir.join("cjpm.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        if !dir.exists() {
            return None;
        }
        match dir.parent() {
            Some(p) if !p.as_os_str().is_empty() => dir = p,
            _ => return None,
        }
    }
    None
}

/// The source directories to scan for same-package siblings under a project
/// root: the package's `src-dir` (default `src`) plus every
/// `[[source-set]]` `src-dir` (conditional per-platform sources are all part
/// of the same project and should be visible cross-file), plus the sources
/// of local path-backed `[dependencies]`. A missing `src-dir` falls back to
/// the root itself so non-standard layouts still resolve; missing
/// `[[source-set]]` dirs are simply skipped (they are feature-gated).
/// Roots without a `cjpm.toml` scan the root itself (legacy behavior).
pub fn scan_dirs(root: &Path) -> Vec<PathBuf> {
    let Some(sections) = read_cjpm_toml(root) else {
        return vec![root.to_path_buf()];
    };
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |d: PathBuf| {
        let d = normalize(&d);
        if !out.contains(&d) {
            out.push(d);
        }
    };
    // [package] src-dir; empty/absent -> default "src". A missing src dir
    // falls back to the root itself so unusual layouts still resolve.
    let own = section_str(&sections, "package", "src-dir")
        .filter(|s| !s.is_empty())
        .map(|s| root.join(s))
        .unwrap_or_else(|| root.join("src"));
    add(if own.is_dir() {
        own
    } else {
        root.to_path_buf()
    });
    // [[source-set]] entries carry their own src-dir. They are optional
    // (feature-gated per-platform sources), so missing dirs are skipped.
    for (key, value) in sections.get("source-set").into_iter().flatten() {
        if key == "src-dir" && !value.is_empty() {
            let d = root.join(unquote(value));
            if d.is_dir() {
                add(d);
            }
        }
    }
    // Local-path [dependencies] are same-project sibling packages; their
    // sources are scanned too so their decls resolve cross-file.
    for (_, dep_root) in local_dependencies(root) {
        let dep_src = dep_root.join("src");
        add(if dep_src.is_dir() { dep_src } else { dep_root });
    }
    out
}

/// Package names of local `[dependencies]` (path-backed sibling packages).
/// Cross-file scans treat decls from these packages as visible in addition
/// to the same-package filter, so symbols from a dependency's modules
/// resolve too. Remote (version-string) deps contribute nothing.
pub fn visible_packages(root: &Path) -> HashSet<String> {
    local_dependencies(root)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// (name, project root) pairs for every local path-backed `[dependencies]`
/// entry. The package name is read from the dependency's own `cjpm.toml`
/// `[package] name`, falling back to the dependency key when unset.
fn local_dependencies(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Some(sections) = read_cjpm_toml(root) else {
        return out;
    };
    for (name, value) in sections.get("dependencies").into_iter().flatten() {
        let Some(rel) = inline_table_path(value) else {
            continue; // version-string dependency, no local source
        };
        let dep_root = root.join(rel);
        if !dep_root.is_dir() {
            continue;
        }
        let dep_name = read_cjpm_toml(&dep_root)
            .and_then(|s| {
                section_str(&s, "package", "name")
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| name.clone());
        out.push((dep_name, dep_root));
    }
    out
}

/// Strip empty/`.` path components so equal directories compare equal
/// (`/a/./b` == `/a/b`). cjpm `src-dir` values often carry a `./` prefix.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        if !matches!(c, std::path::Component::CurDir) {
            out.push(c.as_os_str());
        }
    }
    out
}

/// Minimal cjpm.toml reader: section header -> ordered (key, value) pairs.
/// Values keep their raw text (quotes included); callers unquote on use.
fn read_cjpm_toml(root: &Path) -> Option<HashMap<String, Vec<(String, String)>>> {
    let content = fs::read_to_string(root.join("cjpm.toml")).ok()?;
    let mut sections: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut cur = String::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Section header: [package], [[source-set]] (array-of-tables collapses
        // into the same bucket as its plain form).
        if line.starts_with('[') {
            let name = line.trim_matches(['[', ']']).trim();
            if !name.is_empty() && !name.contains('=') {
                cur = name.to_string();
                sections.entry(cur.clone()).or_default();
            }
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            sections
                .entry(cur.clone())
                .or_default()
                .push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Some(sections)
}

/// Value of `key` in `section`, unquoted.
fn section_str<'a>(
    sections: &'a HashMap<String, Vec<(String, String)>>,
    section: &str,
    key: &str,
) -> Option<&'a str> {
    sections
        .get(section)?
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| unquote(v))
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    for q in ['"', '\''] {
        if let Some(rest) = v.strip_prefix(q) {
            if let Some(end) = rest.rfind(q) {
                return &rest[..end];
            }
        }
    }
    v
}

/// Extract the quoted `path = "..."` value from a cjpm inline dependency
/// table (`foo = { path = "./bar" }`). Returns `None` for version-string
/// deps (`foo = "1.0.0"`).
fn inline_table_path(value: &str) -> Option<String> {
    for seg in value.split(',') {
        let seg = seg.trim();
        if let Some((k, v)) = seg.split_once('=') {
            if k.trim()
                .trim_start_matches('{')
                .trim()
                .trim_end_matches('}')
                == "path"
            {
                return quoted_value(v).map(str::to_owned);
            }
        }
    }
    None
}

fn quoted_value(s: &str) -> Option<&str> {
    let s = s.trim();
    for q in ['"', '\''] {
        if let Some(rest) = s.strip_prefix(q) {
            if let Some(end) = rest.find(q) {
                return Some(&rest[..end]);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_toml(dir: &Path, body: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("cjpm.toml"), body).unwrap();
    }

    #[test]
    fn finds_nearest_cjpm_root_above_file() {
        let base = std::env::temp_dir().join(format!("cjpm_find_{}", std::process::id()));
        let src = base.join("src").join("sub");
        fs::create_dir_all(&src).unwrap();
        write_toml(&base, "[package]\nname = \"p\"\nsrc-dir = \"src\"\n");
        let file = src.join("a.cj");
        assert_eq!(find_project_root(&file), Some(base.clone()));
        // A file outside the project has no cjpm ancestor.
        let outside = base.join("other").join("b.cj");
        assert_eq!(find_project_root(&outside), None);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn stops_at_first_missing_ancestor() {
        // /nonexistent/... does not exist -> walk stops immediately.
        let virtual_path = Path::new("/nonexistent/x/y/a.cj");
        assert_eq!(find_project_root(virtual_path), None);
    }

    #[test]
    fn scan_dirs_defaults_to_src() {
        let base = std::env::temp_dir().join(format!("cjpm_scan_{}", std::process::id()));
        fs::create_dir_all(base.join("src")).unwrap();
        write_toml(&base, "[package]\nname = \"p\"\nsrc-dir = \"\"\n");
        let dirs = scan_dirs(&base);
        assert_eq!(dirs, vec![base.join("src")]);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn scan_dirs_includes_source_sets() {
        let base = std::env::temp_dir().join(format!("cjpm_srcset_{}", std::process::id()));
        fs::create_dir_all(base.join("src")).unwrap();
        write_toml(
            &base,
            "[package]\nname = \"p\"\nsrc-dir = \"src\"\n\n[[source-set]]\nname = \"linux\"\nsrc-dir = \"./platform\"\nfeatures = []\n",
        );
        // src exists; ./platform is missing (feature-gated, not present) ->
        // it is skipped rather than dragging in the whole tree.
        let dirs = scan_dirs(&base);
        assert_eq!(dirs, vec![base.join("src")]);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn scan_dirs_includes_present_source_sets() {
        let base = std::env::temp_dir().join(format!("cjpm_srcset2_{}", std::process::id()));
        fs::create_dir_all(base.join("src")).unwrap();
        fs::create_dir_all(base.join("platform_linux")).unwrap();
        write_toml(
            &base,
            "[package]\nname = \"p\"\nsrc-dir = \"src\"\n\n[[source-set]]\nname = \"linux\"\nsrc-dir = \"./platform_linux\"\nfeatures = []\n",
        );
        let dirs = scan_dirs(&base);
        assert!(dirs.contains(&base.join("src")));
        assert!(dirs.contains(&base.join("platform_linux")));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn no_cjpm_toml_scans_root() {
        let base = std::env::temp_dir().join(format!("cjpm_none_{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        assert_eq!(scan_dirs(&base), vec![base.clone()]);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn visible_packages_from_path_deps() {
        let base = std::env::temp_dir().join(format!("cjpm_deps_{}", std::process::id()));
        fs::create_dir_all(base.join("test1/src")).unwrap();
        write_toml(
            &base,
            "[dependencies]\ntest1 = {path=\"./test1\"}\npro2 = \"1.0.1\"\n",
        );
        write_toml(&base.join("test1"), "[package]\nname = \"test1\"\n");
        let pkgs = visible_packages(&base);
        assert!(pkgs.contains("test1"), "path dep name visible");
        assert_eq!(pkgs.len(), 1, "version-string dep excluded");
        let _ = fs::remove_dir_all(&base);
    }
}
