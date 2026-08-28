// cj-sema: dynamic-library macro expansion via the official Cangjie runtime.
//
// Spec Ch.14 user macros are pre-compiled by the SDK (cjpm) into shared
// libraries that export a C wrapper per macro: `macroCall_c_<Name>_<Pkg>`
// (see src/Sema/Desugar/DesugarMacro.h in the official compiler). This module
// implements the Rust side of that contract, replacing the quote-template
// fallback with a REAL expansion:
//
//   1. Load the runtime + std libs (RTLD_GLOBAL on Linux) and InitCJRuntime.
//   2. Load the compiled macro library (.so on Linux, .dll on Windows).
//   3. Run the macro's global-init via RunCJTask (best-effort), then invoke
//      `macroCall_c_*` on a runtime task with the serialized Tokens argument.
//   4. Parse the returned serialized Tokens.
//
// Token serialization follows the official GetTokensBytes / GetTokensFromBytes
// (src/Macro/TokenSerialization.cpp). Token kind numbers are the official
// Tokens.inc ordinals — our generated TokenKind enum preserves them 1:1.
//
// Every failure path returns Err and the caller falls back to the existing
// template expansion, so a missing SDK or a broken library never destabilizes
// the LSP.

use crate::macro_cache;
use cj_ast::{CodePos, Tokenish};

use std::ffi::c_void;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Cross-platform dynamic library loading (platform layer).
//
// Linux  : libloading::os::unix (dlopen/dlsym, RTLD_NOW | RTLD_GLOBAL)
// Windows: windows-sys (LoadLibraryW + GetProcAddress on kernel32)
//
// Both halves expose the same API shape as libloading — `open(path)` returns a
// handle that unloads on drop, `get::<T>(&handle, symbol)` returns the bare
// function pointer — so the shared runtime/expansion code below reads
// identically on both platforms. Symbol names (`macroCall_c_<Name>_<Pkg>`) and
// the serialized-token byte format (TokenSerialization.cpp) are
// platform-independent.
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
mod imp {
    use libloading::os::unix::Library;
    use std::ffi::c_void;
    use std::path::Path;

    /// dlopen handle (dlclose on drop) — the same type the Linux path always
    /// used.
    pub type Loaded = Library;

    /// dlopen with the flags the Linux path always used (RTLD_NOW|RTLD_GLOBAL).
    pub unsafe fn open(path: &Path) -> Result<Loaded, String> {
        Library::open(Some(path), libc::RTLD_NOW | libc::RTLD_GLOBAL).map_err(|e| e.to_string())
    }

    /// dlsym a symbol; returns the bare function pointer.
    pub unsafe fn get<T: Copy>(lib: &Loaded, symbol: &[u8]) -> Result<T, String> {
        let sym: libloading::os::unix::Symbol<T> = lib.get(symbol).map_err(|e| e.to_string())?;
        Ok(*sym)
    }

    /// free() the buffer a macro wrapper returned (official InvokeMacroFunc
    /// contract).
    pub unsafe fn free(ptr: *mut c_void) {
        libc::free(ptr);
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    /// LoadLibraryW handle; FreeLibrary on drop (mirrors dlclose).
    pub struct Loaded {
        handle: HMODULE,
    }
    // HMODULE is a raw pointer; the Runtime singleton owns exactly one copy, so
    // sharing it across threads (it lives in a OnceLock) is safe.
    unsafe impl Send for Loaded {}
    unsafe impl Sync for Loaded {}

    impl Drop for Loaded {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.handle);
            }
        }
    }

    /// LoadLibraryW a macro .dll by full path.
    pub unsafe fn open(path: &Path) -> Result<Loaded, String> {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = LoadLibraryW(wide.as_ptr());
        if handle.is_null() {
            return Err(format!("LoadLibraryW failed for {}", path.display()));
        }
        Ok(Loaded { handle })
    }

    /// GetProcAddress a symbol; returns the bare function pointer.
    pub unsafe fn get<T: Copy>(lib: &Loaded, symbol: &[u8]) -> Result<T, String> {
        // Symbol names are ASCII (`macroCall_c_<Name>_<Pkg>`); GetProcAddress
        // takes an ANSI name.
        let name = std::ffi::CStr::from_bytes_with_nul(symbol).map_err(|e| e.to_string())?;
        let raw: unsafe extern "system" fn() -> isize =
            GetProcAddress(lib.handle, name.as_ptr() as *const u8)
                .ok_or_else(|| format!("GetProcAddress failed for {:?}", name.to_bytes()))?;
        // FARPROC and every requested T are pointer-sized fn pointers — bit-copy
        // the address into the caller's fn-pointer type.
        Ok(std::mem::transmute_copy(&raw))
    }

    /// free() the buffer a macro wrapper returned. The official compiler
    /// releases it with the C runtime's free() (InvokeMacroFunc); link the
    /// same CRT free here.
    pub unsafe fn free(ptr: *mut c_void) {
        libc::free(ptr);
    }
}

/// The C signature of the desugared macro wrapper.
/// `macroCall_c_<Name>_<Pkg>(paramPtr: CPointer<UInt8>, paramSize: Int64,
///                           macCall): CPointer<UInt8>`.
type MacroFn = unsafe extern "C" fn(*mut u8, i64, *mut c_void) -> *mut u8;
/// Runtime task callback signature (`TaskFunc` in MacroEvaluationCJNative.cpp).
type TaskFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

// ---------------------------------------------------------------------------
// Runtime configuration (ConfigParam layout from include/cangjie/Macro/InvokeConfig.h)
// ---------------------------------------------------------------------------

#[repr(C)]
struct HeapParam {
    region_size: usize,
    heap_size: usize,
    exemption_threshold: f64,
    heap_utilization: f64,
    heap_growth: f64,
    allocation_rate: f64,
    allocation_wait_time: usize,
}

#[repr(C)]
struct GcParam {
    gc_threshold: usize,
    garbage_threshold: f64,
    gc_interval: u64,
    backup_gc_interval: u64,
    gc_threads: i32,
}

#[repr(C)]
struct LogParam {
    log_level: i32,
}

#[repr(C)]
struct ConcurrencyParam {
    th_stack_size: usize,
    co_stack_size: usize,
    processor_num: u32,
}

#[repr(C)]
struct ConfigParam {
    heap: HeapParam,
    gc: GcParam,
    log: LogParam,
    conc: ConcurrencyParam,
}

impl ConfigParam {
    /// Values mirror InvokeUtilCJNative.cpp `CallRuntime` defaults; coStackSize
    /// is bumped to 64KB (the SDK runtime rejects smaller) and processorNum is
    /// kept low so the LSP process does not spawn 24 scheduler threads.
    fn for_macro_expansion() -> Self {
        ConfigParam {
            heap: HeapParam {
                region_size: 64,
                heap_size: 1024 * 1024,
                exemption_threshold: 0.8,
                heap_utilization: 0.6,
                heap_growth: 0.15,
                allocation_rate: 10240.0,
                allocation_wait_time: 1000,
            },
            gc: GcParam {
                gc_threshold: 20,
                garbage_threshold: 0.5,
                gc_interval: 150,
                backup_gc_interval: 240,
                gc_threads: 8,
            },
            log: LogParam {
                log_level: 6, // RTLOG_FATAL
            },
            conc: ConcurrencyParam {
                th_stack_size: 64 * 1024,
                co_stack_size: 64 * 1024,
                processor_num: 2,
            },
        }
    }
}

/// ResetNotify layout: `{ cFunc, cFuncParam }` — the Cangjie init function calls
/// `cFunc(cFuncParam)` when it finishes (see MacroEvaluationCJNative.cpp).
#[repr(C)]
struct Notify {
    func: unsafe extern "C" fn(*mut c_void),
    arg: *mut c_void,
}

// ---------------------------------------------------------------------------
// Runtime singleton (init once per process; never torn down)
// ---------------------------------------------------------------------------

/// Held runtime handles + entry points. Kept alive for the process lifetime so
/// dlclosing a macro .so never unloads the std libs underneath it.
struct Runtime {
    _runtime_lib: imp::Loaded,
    _std_libs: Vec<(std::path::PathBuf, imp::Loaded)>,
    run_cj_task: unsafe extern "C" fn(TaskFn, *mut c_void) -> *mut c_void,
    release_handle: unsafe extern "C" fn(*mut c_void),
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static INIT_LOCK: Mutex<()> = Mutex::new(());
static INIT_FAILED: AtomicBool = AtomicBool::new(false);

fn ensure_runtime() -> Result<&'static Runtime, String> {
    if INIT_FAILED.load(Ordering::Acquire) {
        return Err("Cangjie runtime init previously failed; using fallback expansion".to_string());
    }
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }
    let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }
    match init_runtime() {
        Ok(rt) => {
            let _ = RUNTIME.set(rt);
            Ok(RUNTIME.get().expect("runtime just set"))
        }
        Err(e) => {
            INIT_FAILED.store(true, Ordering::Release);
            Err(e)
        }
    }
}

/// Runtime library subdirectory under `<sdk>/runtime/lib` — the SDK uses a
/// per-platform dir (`linux_x86_64_cjnative` / `windows_x86_64_cjnative`).
#[cfg(target_os = "linux")]
fn runtime_lib_subdir() -> &'static str {
    "linux_x86_64_cjnative"
}
#[cfg(target_os = "windows")]
fn runtime_lib_subdir() -> &'static str {
    "windows_x86_64_cjnative"
}

/// Shared-library extension for the runtime/std libs on this platform
/// (matches the SDK layout: `.so` on Linux, `.dll` on Windows).
#[cfg(target_os = "linux")]
fn lib_ext() -> &'static str {
    "so"
}
#[cfg(target_os = "windows")]
fn lib_ext() -> &'static str {
    "dll"
}

/// Make `<sdk>/runtime/lib/<plat>_x86_64_cjnative` discoverable by the loader
/// so a macro library's dependencies (DT_NEEDED on Linux / import table on
/// Windows) resolve by name:
///   * Linux   — prepend to LD_LIBRARY_PATH. The macro .so's DT_NEEDED entries
///     (`libcangjie-std-ast.so` etc.) have no SONAME, so the loader matches
///     them by filename search through that var; glibc snapshots it at process
///     start, so we set it here exactly as the official envsetup.sh does.
///   * Windows — prepend to PATH. LoadLibraryW resolves a .dll's imported
///     DLLs through the standard search order, which includes PATH.
fn ensure_lib_dir_on_loader_path(lib_dir: &Path) {
    #[cfg(target_os = "linux")]
    {
        let existing = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        let merged = if existing.is_empty() {
            lib_dir.display().to_string()
        } else {
            format!("{}:{}", lib_dir.display(), existing)
        };
        std::env::set_var("LD_LIBRARY_PATH", merged);
    }
    #[cfg(target_os = "windows")]
    {
        let existing = std::env::var("PATH").unwrap_or_default();
        let merged = if existing.is_empty() {
            lib_dir.display().to_string()
        } else {
            format!("{};{}", lib_dir.display(), existing)
        };
        std::env::set_var("PATH", merged);
    }
}

fn init_runtime() -> Result<Runtime, String> {
    let lib_dir = macro_cache::sdk_root()
        .join("runtime/lib")
        .join(runtime_lib_subdir());
    let ext = lib_ext();

    ensure_lib_dir_on_loader_path(&lib_dir);

    let preload = [
        "libboundscheck",
        "libcangjie-runtime",
        "libcangjie-std-core",
        "libcangjie-std-collection",
        "libcangjie-std-ast",
        "libcangjie-std-sort",
    ];
    let mut std_libs = Vec::new();
    for base in preload {
        let name = format!("{base}.{ext}");
        let p = lib_dir.join(&name);
        if p.exists() {
            let lib =
                unsafe { imp::open(&p) }.map_err(|e| format!("dlopen {}: {e}", p.display()))?;
            std_libs.push((p, lib));
        }
    }

    let runtime_path = lib_dir.join(format!("libcangjie-runtime.{ext}"));
    let runtime_lib = unsafe { imp::open(&runtime_path) }
        .map_err(|e| format!("dlopen runtime {}: {e}", runtime_path.display()))?;

    unsafe {
        let init_runtime: unsafe extern "C" fn(*mut ConfigParam) -> i64 =
            imp::get(&runtime_lib, b"InitCJRuntime\0")?;
        let run_cj_task: unsafe extern "C" fn(TaskFn, *mut c_void) -> *mut c_void =
            imp::get(&runtime_lib, b"RunCJTask\0")?;
        let release_handle: unsafe extern "C" fn(*mut c_void) =
            imp::get(&runtime_lib, b"ReleaseHandle\0")?;

        let mut cfg = ConfigParam::for_macro_expansion();
        let rc = init_runtime(&mut cfg);
        if rc != 0 {
            return Err(format!("InitCJRuntime returned {rc}"));
        }

        let rt = Runtime {
            _runtime_lib: runtime_lib,
            _std_libs: std_libs,
            run_cj_task,
            release_handle,
        };

        // Best-effort std package inits in dependency order (std.core →
        // std.collection → std.ast) so static std state is ready. Matches the
        // normal Cangjie startup; failures are ignored — expansion falls back.
        // Same mangled package-init symbols on both platforms (verified against
        // the SDK exports: core → _CGPatirHv, collection → _CGPacirHv, ast →
        // _CGPaxirHv); only the std-lib filename extension differs.
        let std_pkgs: [(&str, &str); 3] = [
            ("libcangjie-std-core", "_CGPatirHv"),
            ("libcangjie-std-collection", "_CGPacirHv"),
            ("libcangjie-std-ast", "_CGPaxirHv"),
        ];
        for (base, sym) in std_pkgs {
            let file = format!("{base}.{ext}");
            let Some(lib) = rt
                ._std_libs
                .iter()
                .find(|(p, _)| p.file_name().map(|f| f == file.as_str()).unwrap_or(false))
            else {
                continue;
            };
            let _ = run_pkg_init(&rt, &lib.1, sym);
        }
        Ok(rt)
    }
}

unsafe extern "C" fn notify_cb(arg: *mut c_void) {
    let done = &*(arg as *const AtomicBool);
    done.store(true, Ordering::Release);
}

/// Run a raw Cangjie function (e.g. a package `..._global_init$_reset`) on a
/// runtime task and wait for its notify callback. Best-effort.
fn run_pkg_init(rt: &Runtime, lib: &imp::Loaded, sym: &str) -> Result<(), String> {
    unsafe {
        let init_fn: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
            imp::get(lib, sym.as_bytes())?;
        let done = Box::new(AtomicBool::new(false));
        let notify = Box::new(Notify {
            func: notify_cb,
            arg: &*done as *const AtomicBool as *mut c_void,
        });
        let arg = &*notify as *const Notify as *mut c_void;
        let handle = (rt.run_cj_task)(init_fn, arg);
        wait_flag(&done, Duration::from_secs(5));
        (rt.release_handle)(handle);
        Ok(())
    }
}

fn wait_flag(flag: &AtomicBool, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if flag.load(Ordering::Acquire) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ---------------------------------------------------------------------------
// The macro invocation
// ---------------------------------------------------------------------------

/// Ctx passed to the runtime task. Boxed so a write racing a timeout stays
/// within live memory.
struct TaskData {
    macro_fn: MacroFn,
    input: Vec<u8>,
    fake_node: *mut c_void,
    out: AtomicPtr<u8>,
}

unsafe extern "C" fn task_entry(arg: *mut c_void) -> *mut c_void {
    let data = &mut *(arg as *mut TaskData);
    let ret = (data.macro_fn)(
        data.input.as_ptr() as *mut u8,
        data.input.len() as i64,
        data.fake_node,
    );
    data.out.store(ret, Ordering::Release);
    ptr::null_mut()
}

/// dlopen a macro .so and actually expand `macro_name` over `args` by calling
/// its `macroCall_c_*` wrapper on the Cangjie runtime. Returns the resulting
/// token sequence; callers render it to text for previews/diagnostics.
///
/// `pkg_name` is the macro package's full name (`module.pkg`, dots retained) —
/// the symbol and the .so filename both embed it.
pub fn expand_macro_call(
    so_path: &Path,
    macro_name: &str,
    pkg_name: &str,
    args: &[Tokenish],
) -> Result<Vec<Tokenish>, String> {
    let rt = ensure_runtime()?;
    let so =
        unsafe { imp::open(so_path) }.map_err(|e| format!("dlopen {}: {e}", so_path.display()))?;
    let symbol = macro_symbol_name(macro_name, pkg_name);
    let macro_fn: MacroFn =
        unsafe { imp::get(&so, symbol.as_bytes()) }.map_err(|e| format!("dlsym {symbol}: {e}"))?;

    // Serialized Tokens input (official GetTokensBytes format).
    let input = serialize_tokens(args);

    // The runtime's `getMacroPosition` (std.ast) reads the macro-call position
    // from the node the N2C stub stashed in TLS. The official compiler passes a
    // real MacroCall*; we pass a minimal stand-in with position fields at the
    // offsets CJ_GetMacroPosition reads (line/col/fileID at 0x1a8/0x1ac/0x1b0).
    let mut fake_node = Box::new([0u8; 0x200]);
    fake_node[0x1a8..0x1ac].copy_from_slice(&1i32.to_le_bytes()); // line
    fake_node[0x1ac..0x1b0].copy_from_slice(&1i32.to_le_bytes()); // column
    fake_node[0x1b0..0x1b4].copy_from_slice(&0i32.to_le_bytes()); // fileID

    let data = Box::new(TaskData {
        macro_fn,
        input,
        fake_node: fake_node.as_mut_ptr() as *mut c_void,
        out: AtomicPtr::new(ptr::null_mut()),
    });
    let data_ptr = Box::into_raw(data) as *mut c_void;

    let handle = unsafe { (rt.run_cj_task)(task_entry, data_ptr) };
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = ptr::null_mut();
    while Instant::now() < deadline {
        out = unsafe { (*(data_ptr as *const TaskData)).out.load(Ordering::Acquire) };
        if !out.is_null() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    unsafe { (rt.release_handle)(handle) };

    if out.is_null() {
        // Timed out: the task may still be running and write into our boxed
        // context, so intentionally leak it (Box::into_raw above left it on
        // the heap) rather than freeing memory a late write could touch.
        return Err(format!("macro '{macro_name}' expansion timed out"));
    }

    let tokens = unsafe { deserialize_from_ptr(out) }?;
    // The wrapper returns a malloc'd buffer (the official compiler free()s it
    // in InvokeMacroFunc) — release it, matching the contract.
    unsafe { imp::free(out as *mut c_void) };
    // Task finished: reclaim the boxed context.
    unsafe {
        drop(Box::from_raw(data_ptr as *mut TaskData));
    }
    Ok(tokens)
}

/// Render an expansion token sequence to text (preview style: space-separated).
pub fn tokens_to_text(tokens: &[Tokenish]) -> String {
    tokens
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// The exported wrapper symbol: `macroCall_c_<Name>_<Pkg>` with `.`/`:` mapped
/// to `_` (see Utils::GetMacroFuncName).
fn macro_symbol_name(macro_name: &str, pkg: &str) -> String {
    let pkg = pkg.replace(['.', ':'], "_");
    format!("macroCall_c_{macro_name}_{pkg}")
}

// ---------------------------------------------------------------------------
// Tokens serialization (official TokenSerialization.cpp)
// ---------------------------------------------------------------------------

/// Try to guess the official token kind for an argument by re-lexing its text.
fn token_kind_of(text: &str) -> u16 {
    let mut lexer = cj_lexer::Lexer::new(text);
    let toks = lexer.tokenize();
    match toks.first() {
        Some(t) => t.kind as u16,
        None => cj_lexer::TokenKind::IDENTIFIER as u16,
    }
}

/// Encode tokens as the official GetTokensBytes byte format:
///   [u32 count][per token: u16 kind, u32 len, bytes, u32 fileID, i32 line,
///    i32 col, u16 isSingleQuote][+u16 delimiterNum for MULTILINE_RAW_STRING]
/// Empty input encodes to an empty buffer (matches GetTokensBytes `{}`).
fn serialize_tokens(args: &[Tokenish]) -> Vec<u8> {
    if args.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    out.extend_from_slice(&(args.len() as u32).to_le_bytes());
    for a in args {
        let kind = token_kind_of(&a.text);
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(a.text.len() as u32).to_le_bytes());
        out.extend_from_slice(a.text.as_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // fileID
        out.extend_from_slice(&0i32.to_le_bytes()); // line
        out.extend_from_slice(&0i32.to_le_bytes()); // column
        out.extend_from_slice(&0u16.to_le_bytes()); // isSingleQuote
    }
    out
}

/// Decode serialized token bytes, bounds-checked against a hard cap.
/// All multi-byte reads go through `read_unaligned` — the source buffer may
/// come from the macro .so (malloc'd, aligned) OR from test-owned byte slices
/// (alignment not guaranteed), so we must not require 4/2-byte alignment.
unsafe fn deserialize_from_ptr(ptr: *const u8) -> Result<Vec<Tokenish>, String> {
    const CAP: usize = 1 << 20;
    if ptr.is_null() {
        return Err("macro returned a null token buffer".to_string());
    }
    let count = ptr::read_unaligned(ptr as *const u32);
    if count > 65536 {
        return Err(format!("suspicious output token count {count}"));
    }
    let mut off = 4usize;
    let mut toks = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if off + 2 + 4 > CAP {
            return Err("macro output buffer overflow".to_string());
        }
        let kind = ptr::read_unaligned(ptr.add(off) as *const u16);
        off += 2;
        let len = ptr::read_unaligned(ptr.add(off) as *const u32);
        off += 4;
        if off + len as usize > CAP {
            return Err("macro output buffer overflow".to_string());
        }
        let bytes = std::slice::from_raw_parts(ptr.add(off), len as usize);
        let text = String::from_utf8_lossy(bytes).into_owned();
        off += len as usize;
        off += 4 + 4 + 4; // fileID, line, column
        if off + 2 > CAP {
            return Err("macro output buffer overflow".to_string());
        }
        let _is_single = ptr::read_unaligned(ptr.add(off) as *const u16);
        off += 2;
        if kind == cj_lexer::TokenKind::MULTILINE_RAW_STRING as u16 {
            off += 2; // delimiterNum
        }
        toks.push(Tokenish {
            text,
            pos: CodePos::new(0, 0, 0, 0, 0, 0),
        });
    }
    Ok(toks)
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use cj_ast::CodePos;

    fn tok(text: &str) -> Tokenish {
        Tokenish {
            text: text.to_string(),
            pos: CodePos::new(1, 1, 0, 1, 1, 0),
        }
    }

    #[test]
    fn symbol_name_matches_official() {
        // Verified against cjpm output: lib-macro_macro_calling.define.so exports
        // `macroCall_c_Wrap_macro_calling_define` (Utils::GetMacroFuncName).
        assert_eq!(
            macro_symbol_name("Wrap", "macro_calling.define"),
            "macroCall_c_Wrap_macro_calling_define"
        );
        assert_eq!(
            macro_symbol_name("Say", "macro_calling2.say"),
            "macroCall_c_Say_macro_calling2_say"
        );
    }

    #[test]
    fn token_kind_numbers_match_official() {
        // Official Tokens.inc ordinals (verified against cjc's own serialized
        // quote: quote(0) emits kind 134+5 = 139 for integer literals, and the
        // live .so decode returned 139/137/2/3 for `print ( 42 )`).
        assert_eq!(cj_lexer::TokenKind::INTEGER_LITERAL as u16, 139);
        assert_eq!(cj_lexer::TokenKind::IDENTIFIER as u16, 137);
        assert_eq!(cj_lexer::TokenKind::STRING_LITERAL as u16, 147);
        assert_eq!(cj_lexer::TokenKind::LPAREN as u16, 2);
    }

    #[test]
    fn serialize_empty_is_empty() {
        assert!(serialize_tokens(&[]).is_empty());
    }

    #[test]
    fn serialize_format() {
        // One integer arg "42": count=1, kind=139, len=2, "42", zeroed pos.
        let input = serialize_tokens(&[tok("42")]);
        assert_eq!(&input[0..4], &1u32.to_le_bytes());
        assert_eq!(&input[4..6], &139u16.to_le_bytes());
        assert_eq!(&input[6..10], &2u32.to_le_bytes());
        assert_eq!(&input[10..12], b"42");
        assert_eq!(input.len(), 4 + 2 + 4 + 2 + 4 + 4 + 4 + 2);
    }

    #[test]
    fn deserialize_roundtrip() {
        let text = "42";
        let input = serialize_tokens(&[tok(text)]);
        let buf = Vec::leak(input);
        let toks = unsafe { deserialize_from_ptr(buf.as_ptr()) }.unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].text, "42");
    }

    #[test]
    fn tokens_to_text_joins() {
        let toks = vec![tok("print"), tok("("), tok("42"), tok(")")];
        assert_eq!(tokens_to_text(&toks), "print ( 42 )");
    }

    // ---- integration: build a macro package with cjpm, dlopen it, CALL it ----
    fn sdk_available() -> bool {
        macro_cache::sdk_root().join("bin/cjc").exists()
    }

    /// True when the SDK runtime lib dir is on the loader path. glibc snapshots
    /// LD_LIBRARY_PATH at process START (mid-process setenv is ignored), and
    /// the macro .so's DT_NEEDED entries (`libcangjie-std-ast.so`, no SONAME)
    /// resolve ONLY via that path — same contract the official cjc has via
    /// envsetup.sh. The test re-execs itself with the env set when missing.
    fn loader_has_sdk_dir() -> bool {
        let dir = macro_cache::sdk_root().join("runtime/lib/linux_x86_64_cjnative");
        let want = dir.to_string_lossy();
        std::env::var("LD_LIBRARY_PATH")
            .map(|p| p.split(':').any(|d| d == want.as_ref()))
            .unwrap_or(false)
    }

    fn build_macro_pkg(dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
        use std::fs;
        use std::process::Command;
        let _ = fs::create_dir_all(dir.join("src/p"));
        let cjpm_toml = "[dependencies]\n\n[package]\n  cjc-version = \"1.1.3\"\n  name = \"pkg\"\n  output-type = \"executable\"\n  src-dir = \"src\"\n  version = \"1.0.0\"\n  package-configuration = {}\n";
        fs::write(dir.join("cjpm.toml"), cjpm_toml).map_err(|e| e.to_string())?;
        fs::write(
            dir.join("src/p/p.cj"),
            "macro package pkg.p\n\nimport std.ast.*\n\npublic macro Wrap(x: Tokens): Tokens {\n    quote(print($(x)))\n}\n",
        )
        .map_err(|e| e.to_string())?;
        fs::write(
            dir.join("src/main.cj"),
            "package pkg\n\nimport pkg.p.*\n\nmain(): Int64 {\n    @Wrap(42)\n    return 0\n}\n",
        )
        .map_err(|e| e.to_string())?;
        let root = macro_cache::sdk_root();
        let shell = format!(
            "source {} && cjpm build 2>&1",
            root.join("envsetup.sh").display()
        );
        let out = Command::new("bash")
            .arg("-c")
            .arg(&shell)
            .current_dir(dir)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "cjpm build failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let so = crate::macro_cache::find_macro_so(dir).map_err(|e| e.to_string())?;
        Ok(so)
    }

    #[test]
    fn dlopen_and_call_macro_so() {
        // Requires the official SDK (cjc/cjpm). Skip (pass) elsewhere.
        if !sdk_available() {
            eprintln!("SKIP: Cangjie SDK not present; cannot build macro .so");
            return;
        }

        // The macro .so's DT_NEEDED entries resolve only through LD_LIBRARY_PATH
        // as seen at process start. If our loader lacks the SDK dir, re-exec
        // this same test in a child process with the env var set.
        if !loader_has_sdk_dir() {
            let exe = std::env::current_exe().expect("test binary path");
            let mut cmd = std::process::Command::new(exe);
            cmd.arg("--exact")
                .arg("dylib::tests::dlopen_and_call_macro_so")
                .arg("--nocapture");
            let dir = macro_cache::sdk_root().join("runtime/lib/linux_x86_64_cjnative");
            let ldp = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
            let merged = if ldp.is_empty() {
                dir.display().to_string()
            } else {
                format!("{}:{}", dir.display(), ldp)
            };
            cmd.env("LD_LIBRARY_PATH", merged);
            let status = cmd.status().expect("re-exec test with LD_LIBRARY_PATH");
            assert!(
                status.success(),
                "child expansion test failed (exit {status})"
            );
            return;
        }

        let dir = std::env::temp_dir().join(format!("cj-macro-dylib-test-{}", std::process::id()));
        let so = match build_macro_pkg(&dir) {
            Ok(so) => so,
            Err(e) => {
                eprintln!("SKIP: could not build macro package: {e}");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
        };
        let res = expand_macro_call(&so, "Wrap", "pkg.p", &[tok("42")]);
        let _ = std::fs::remove_dir_all(&dir);
        match res {
            Ok(tokens) => {
                let text = tokens_to_text(&tokens);
                assert!(
                    text.contains("42"),
                    "expansion should contain the spliced arg, got: {text:?}"
                );
                assert!(text.contains("print"));
                // Call a second time — proves the first call's output-buffer
                // free() did not corrupt the runtime heap.
                let _ = expand_macro_call(&so, "Wrap", "pkg.p", &[tok("42")]);
            }
            Err(e) => panic!("macro dylib expansion failed: {e}"),
        }
    }
}
