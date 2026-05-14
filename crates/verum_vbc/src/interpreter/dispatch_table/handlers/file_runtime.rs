//! High-level Rust intercepts for `core.io.file` and `core.io.fs`
//! operations.
//!

//! Mirrors the architecture of `shell_runtime.rs` (VBC-1): bypass the
//! libSystem FFI chain for open(2)/read(2)/write(2)/stat(2)/mkdir(2)/
//! unlink(2)/rename(2) syscalls and dispatch directly to `std::fs`
//! from the interpreter host process. See `shell_runtime.rs`
//! docstring for the full Tier-0 architectural rationale.
//!

//! # Functions intercepted
//!

//! ## `core.io.file.*` (Text paths)
//!

//!  * `read_to_string(path: &Text) -> IoResult<Text>` —
//!  `std::fs::read_to_string`.
//!  * `read(path: &Text) -> IoResult<List<Byte>>` —
//!  `std::fs::read`.
//!  * `write(path: &Text, contents: &Text) -> IoResult<()>` —
//!  `std::fs::write` with text contents.
//!  * `write_bytes(path: &Text, contents: &[Byte]) -> IoResult<()>` —
//!  `std::fs::write` with byte slice.
//!  * `exists(path: &Text) -> Bool` — `std::path::Path::exists`.
//!

//! ## `core.io.fs.*` (Path-typed paths — `Path` is `{ inner: Text }`)
//!

//!  * `exists(path: &Path) -> Bool`
//!  * `is_file(path: &Path) -> Bool`
//!  * `is_dir(path: &Path) -> Bool`
//!  * `is_symlink(path: &Path) -> Bool`
//!  * `create_dir(path: &Path) -> IoResult<()>` — `std::fs::create_dir`
//!  * `create_dir_all(path: &Path) -> IoResult<()>`
//!  * `remove_file(path: &Path) -> IoResult<()>`
//!  * `remove_dir(path: &Path) -> IoResult<()>`
//!  * `remove_dir_all(path: &Path) -> IoResult<()>`
//!  * `rename(from: &Path, to: &Path) -> IoResult<()>`
//!  * `copy(from: &Path, to: &Path) -> IoResult<Int>` — bytes copied.
//!  * `read(path: &Path) -> IoResult<List<Byte>>`
//!  * `read_to_string(path: &Path) -> IoResult<Text>`
//!  * `write(path: &Path, contents: &[Byte]) -> IoResult<()>`
//!  * `write_str(path: &Path, contents: &Text) -> IoResult<()>`
//!

//! Path-typed args (`&Path`) are unwrapped via [`extract_path_or_text_arg`]
//! which transparently handles BOTH bare `&Text` and `&Path` (the
//! one-field `{ inner: Text }` record produced by `Path.new(s)`).
//! This is what closes the script-mode crash where `fs.exists(&path)`
//! triggered an out-of-bounds register dereference inside the
//! libSystem FFI dispatch chain.
//!

//! # Marshaling
//!

//! `IoResult<T>` = `Result<T, StreamError>` where
//! `StreamError { kind: IoErrorKind, message: Maybe<Text> }`.
//! On Rust-side `std::io::Error`, the kind is mapped from
//! `ErrorKind::NotFound` / `PermissionDenied` / etc. to the
//! corresponding `IoErrorKind` variant; an OS-error message goes
//! into `message` (`Maybe.Some(text)`).
//!

//! # Permission gate
//!

//! Consults `PermissionScope::FileSystem` — a script declaring
//! `permissions = ["time"]` (no `fs`) is denied uniformly with the
//! libSystem open/read FFI gate.

use super::super::super::error::InterpreterResult;
use super::heap_helpers::{
    alloc_byte_list, alloc_record_n_fields, extract_text_arg, wrap_in_variant,
};
use super::string_helpers::{alloc_string_value, extract_string};
use crate::interpreter::heap;
use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::TypeId;
use crate::value::Value;

/// Try to intercept a file-I/O runtime call. Returns `Some(value)`
/// when the interception fires, `None` otherwise (caller falls through
/// to normal bytecode dispatch).
///

/// Hot-path miss: one string-suffix compare + return None.
pub(in super::super) fn try_intercept_file_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    // Qualifier gate — the bare names match too aggressively
    // (`write` / `read` / `len` / `exists` / etc. are widely used
    // method names that surface on `DefaultHasher`, `TextBuilder`,
    // `Channel`, and other unrelated types).  Match only when the
    // bare name is preceded by a known io.fs / io.file qualifier so
    // the intercept fires exclusively on canonical filesystem call
    // sites.  Pre-fix, every `DefaultHasher.write` call landed at
    // the FS `intercept_write_dispatch` and returned garbage —
    // closes task #11 + the hasher cross-tier defect.
    let qualifier_ok = func_name == bare
        || func_name.starts_with("File.")
        || func_name.starts_with("Path.")
        || func_name.starts_with("PathBuf.")
        || func_name.contains(".io.file.")
        || func_name.contains(".io.fs.")
        || func_name.contains(".File.")
        || func_name.contains(".Path.")
        || func_name.contains(".PathBuf.");
    if !qualifier_ok {
        return Ok(None);
    }
    match bare {
        // Reads — both io.file (Text) and io.fs (Path) flavours.
        "read_to_string" if arg_count == 1 => {
            intercept_read_to_string(state, args_start_reg, arg_count, caller_base)
        }
        "read" if arg_count == 1 => {
            intercept_read_bytes(state, args_start_reg, arg_count, caller_base)
        }
        // Writes — io.file uses (path, &Text); io.fs uses (path, &[Byte])
        // for `write` and (path, &Text) for `write_str`. The two-arg
        // signature is preserved; we dispatch on whether the second
        // arg is a Text or a List<Byte> by trying Text extraction
        // first, then byte-list extraction.
        "write" if arg_count == 2 => intercept_write_dispatch(state, args_start_reg, caller_base),
        "write_bytes" if arg_count == 2 => {
            intercept_write_bytes(state, args_start_reg, arg_count, caller_base)
        }
        "write_str" if arg_count == 2 => {
            intercept_write_text(state, args_start_reg, arg_count, caller_base)
        }
        // Existence + type predicates — bool returns, no permission gate
        // (read-only metadata).
        "exists" if arg_count == 1 => {
            intercept_exists(state, args_start_reg, arg_count, caller_base)
        }
        // `path_exists` is core.shell.builtins's wrapper around
        // `fs_metadata(p).is_ok()` — same semantics as fs.exists.
        // Bypasses the metadata-allocation chain that crashes when
        // the inner stub returns Unit.
        "path_exists" if arg_count == 1 => {
            intercept_exists(state, args_start_reg, arg_count, caller_base)
        }
        "is_file" if arg_count == 1 => {
            intercept_metadata_pred(state, args_start_reg, caller_base, MetaPred::File)
        }
        "is_dir" if arg_count == 1 => {
            intercept_metadata_pred(state, args_start_reg, caller_base, MetaPred::Dir)
        }
        "is_symlink" if arg_count == 1 => {
            intercept_metadata_pred(state, args_start_reg, caller_base, MetaPred::Symlink)
        }
        // `core.io.fs.metadata(&Path) -> IoResult<Metadata>`.
        // Allocates a `Metadata { kind: FileType, len: Int, modified:
        // Maybe<Int>, accessed: Maybe<Int>, mtime_ns: Int, atime_ns: Int }`
        // record (6 Value slots).  Stdlib's `Metadata.is_file()` /
        // `is_dir()` / `is_symlink()` reads `kind` (variant tag).
        "metadata" if arg_count == 1 => {
            intercept_metadata(state, args_start_reg, caller_base)
        }
        // `core.io.fs.file_size(&Path) -> IoResult<Int>` — convenience
        // path-shaped wrapper around metadata().len().
        "file_size" if arg_count == 1 => {
            intercept_file_size(state, args_start_reg, caller_base)
        }
        // Mutations — gated on FileSystem write permission.
        "create_dir" if arg_count == 1 => {
            intercept_unit_op(state, args_start_reg, caller_base, FsUnitOp::CreateDir)
        }
        "create_dir_all" if arg_count == 1 => {
            intercept_unit_op(state, args_start_reg, caller_base, FsUnitOp::CreateDirAll)
        }
        // `mkdir_p` and `mkdir` are core.shell.builtins's path-typed
        // mkdir wrappers — semantics: mkdir_p creates all parents
        // (idempotent), mkdir creates one level.
        "mkdir_p" if arg_count == 1 => {
            intercept_unit_op(state, args_start_reg, caller_base, FsUnitOp::CreateDirAll)
        }
        "mkdir" if arg_count == 1 => {
            intercept_unit_op(state, args_start_reg, caller_base, FsUnitOp::CreateDir)
        }
        "remove_file" if arg_count == 1 => {
            intercept_unit_op(state, args_start_reg, caller_base, FsUnitOp::RemoveFile)
        }
        "remove_dir" if arg_count == 1 => {
            intercept_unit_op(state, args_start_reg, caller_base, FsUnitOp::RemoveDir)
        }
        "remove_dir_all" if arg_count == 1 => {
            intercept_unit_op(state, args_start_reg, caller_base, FsUnitOp::RemoveDirAll)
        }
        // `rm` and `rm_rf` are core.shell.builtins's wrappers.
        // `rm` removes a file (or empty dir); `rm_rf` removes a path
        // recursively (file or dir tree).
        "rm" if arg_count == 1 => {
            intercept_rm_smart(state, args_start_reg, caller_base, false)
        }
        "rm_rf" if arg_count == 1 => {
            intercept_rm_smart(state, args_start_reg, caller_base, true)
        }
        "rename" if arg_count == 2 => intercept_rename(state, args_start_reg, caller_base),
        "mv" if arg_count == 2 => intercept_rename(state, args_start_reg, caller_base),
        "copy" if arg_count == 2 => intercept_copy(state, args_start_reg, caller_base),
        "cp" if arg_count == 2 => intercept_copy(state, args_start_reg, caller_base),
        // `which(name: &Text) -> Maybe<Path>` — searches PATH.
        "which" if arg_count == 1 => intercept_which(state, args_start_reg, caller_base),
        // `command_exists(name: &Text) -> Bool` — `which.is_some()`.
        "command_exists" if arg_count == 1 => {
            intercept_command_exists(state, args_start_reg, caller_base)
        }
        _ => Ok(None),
    }
}

#[derive(Copy, Clone)]
enum MetaPred {
    File,
    Dir,
    Symlink,
}

#[derive(Copy, Clone)]
enum FsUnitOp {
    CreateDir,
    CreateDirAll,
    RemoveFile,
    RemoveDir,
    RemoveDirAll,
}

// ============================================================================
// Per-function intercepts
// ============================================================================

fn intercept_read_to_string(
    state: &mut InterpreterState,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if arg_count == 0 {
        return Ok(None);
    }
    let path = extract_path_arg(state, args_start_reg, caller_base);
    if let Some(denied) = check_fs_permission(state, "read", &path) {
        return Ok(Some(denied));
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            let text = alloc_string_value(state, &s)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[text])?))
        }
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

fn intercept_read_bytes(
    state: &mut InterpreterState,
    args_start_reg: u16,
    _arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    if let Some(denied) = check_fs_permission(state, "read", &path) {
        return Ok(Some(denied));
    }
    match std::fs::read(&path) {
        Ok(bytes) => {
            let list = alloc_byte_list(state, &bytes)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[list])?))
        }
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

fn intercept_write_text(
    state: &mut InterpreterState,
    args_start_reg: u16,
    _arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    let contents = extract_text_arg(state, args_start_reg + 1, caller_base);
    if let Some(denied) = check_fs_permission(state, "write", &path) {
        return Ok(Some(denied));
    }
    match std::fs::write(&path, contents.as_bytes()) {
        Ok(()) => {
            let unit = Value::unit();
            Ok(Some(wrap_in_variant(state, "Result", 0, &[unit])?))
        }
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

fn intercept_write_bytes(
    state: &mut InterpreterState,
    args_start_reg: u16,
    _arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    let bytes = extract_byte_list_arg(state, args_start_reg + 1, caller_base);
    if let Some(denied) = check_fs_permission(state, "write", &path) {
        return Ok(Some(denied));
    }
    match std::fs::write(&path, &bytes) {
        Ok(()) => {
            let unit = Value::unit();
            Ok(Some(wrap_in_variant(state, "Result", 0, &[unit])?))
        }
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

fn intercept_exists(
    state: &mut InterpreterState,
    args_start_reg: u16,
    _arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    let exists = std::path::Path::new(&path).exists();
    if std::env::var("VERUM_DEBUG_FS").is_ok() {
        eprintln!("[path_exists] path='{}' exists={}", path, exists);
    }
    Ok(Some(Value::from_bool(exists)))
}

fn intercept_metadata_pred(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
    which: MetaPred,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    let p = std::path::Path::new(&path);
    let result = match which {
        MetaPred::File => p.is_file(),
        MetaPred::Dir => p.is_dir(),
        MetaPred::Symlink => std::fs::symlink_metadata(p)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false),
    };
    Ok(Some(Value::from_bool(result)))
}

fn intercept_unit_op(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
    op: FsUnitOp,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    if let Some(denied) = check_fs_permission(state, "write", &path) {
        return Ok(Some(denied));
    }
    let p = std::path::Path::new(&path);
    let result = match op {
        FsUnitOp::CreateDir => std::fs::create_dir(p),
        FsUnitOp::CreateDirAll => std::fs::create_dir_all(p),
        FsUnitOp::RemoveFile => std::fs::remove_file(p),
        FsUnitOp::RemoveDir => std::fs::remove_dir(p),
        FsUnitOp::RemoveDirAll => std::fs::remove_dir_all(p),
    };
    match result {
        Ok(()) => Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::unit()])?)),
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

fn intercept_rename(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let from = extract_path_arg(state, args_start_reg, caller_base);
    let to = extract_path_arg(state, args_start_reg + 1, caller_base);
    if let Some(denied) = check_fs_permission(state, "write", &from) {
        return Ok(Some(denied));
    }
    if let Some(denied) = check_fs_permission(state, "write", &to) {
        return Ok(Some(denied));
    }
    match std::fs::rename(&from, &to) {
        Ok(()) => Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::unit()])?)),
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

fn intercept_copy(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let from = extract_path_arg(state, args_start_reg, caller_base);
    let to = extract_path_arg(state, args_start_reg + 1, caller_base);
    if let Some(denied) = check_fs_permission(state, "read", &from) {
        return Ok(Some(denied));
    }
    if let Some(denied) = check_fs_permission(state, "write", &to) {
        return Ok(Some(denied));
    }
    match std::fs::copy(&from, &to) {
        Ok(n) => Ok(Some(wrap_in_variant(
            state,
            "Result",
            0,
            &[Value::from_i64(n as i64)],
        )?)),
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

/// `core.io.fs.metadata(&Path) -> IoResult<Metadata>`.  Allocates the
/// canonical stdlib shape:
///
/// ```text
/// Metadata { raw: FsMetadataRaw }
/// FsMetadataRaw {
///     size: Int,
///     modified_secs: Int,
///     modified_nanos: Int,
///     accessed_secs: Int,
///     accessed_nanos: Int,
///     created_secs: Int,
///     created_nanos: Int,
///     mode: Int,
///     is_dir: Int,
///     is_file: Int,
///     is_symlink: Int,
/// }
/// ```
///
/// Field count: Metadata=1, FsMetadataRaw=11.  Stdlib's
/// `metadata.is_file()` / `is_dir()` / `is_symlink()` / `len()` / etc.
/// reads the raw fields directly.
fn intercept_metadata(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    match std::fs::symlink_metadata(&path) {
        Ok(m) => {
            let ft = m.file_type();
            let (msec, mnsec) = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| (d.as_secs() as i64, d.subsec_nanos() as i64))
                .unwrap_or((0, 0));
            let (asec, ansec) = m
                .accessed()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| (d.as_secs() as i64, d.subsec_nanos() as i64))
                .unwrap_or((0, 0));
            let (csec, cnsec) = m
                .created()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| (d.as_secs() as i64, d.subsec_nanos() as i64))
                .unwrap_or((0, 0));
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() as i64
            };
            #[cfg(not(unix))]
            let mode = if m.permissions().readonly() { 0o444 } else { 0o644 };
            let raw = alloc_record_n_fields(
                state,
                "FsMetadataRaw",
                &[
                    Value::from_i64(m.len() as i64),
                    Value::from_i64(msec),
                    Value::from_i64(mnsec),
                    Value::from_i64(asec),
                    Value::from_i64(ansec),
                    Value::from_i64(csec),
                    Value::from_i64(cnsec),
                    Value::from_i64(mode),
                    Value::from_i64(if ft.is_dir() { 1 } else { 0 }),
                    Value::from_i64(if ft.is_file() { 1 } else { 0 }),
                    Value::from_i64(if ft.is_symlink() { 1 } else { 0 }),
                ],
            )?;
            let metadata = alloc_record_n_fields(state, "Metadata", &[raw])?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[metadata])?))
        }
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

/// `core.io.fs.file_size(&Path) -> IoResult<Int>` — convenience wrapper.
fn intercept_file_size(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    match std::fs::metadata(&path) {
        Ok(m) => Ok(Some(wrap_in_variant(
            state,
            "Result",
            0,
            &[Value::from_i64(m.len() as i64)],
        )?)),
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

/// `rm` (recursive=false) and `rm_rf` (recursive=true).  When the target
/// is a directory and `recursive` is true, removes the whole tree;
/// when false, falls through to file-level remove (which fails on dirs
/// per the OS — same semantics as the stdlib wrapper).
fn intercept_rm_smart(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
    recursive: bool,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    if let Some(denied) = check_fs_permission(state, "write", &path) {
        return Ok(Some(denied));
    }
    let p = std::path::Path::new(&path);
    let result = if recursive {
        match std::fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_dir() => std::fs::remove_dir_all(p),
            Ok(_) => std::fs::remove_file(p),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        std::fs::remove_file(p)
    };
    match result {
        Ok(()) => Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::unit()])?)),
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

/// `core.shell.builtins.which(name: &Text) -> Maybe<Path>` —
/// resolves a program name in `$PATH`.
fn intercept_which(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let name = extract_text_arg(state, args_start_reg, caller_base);
    match resolve_in_path(&name) {
        Some(p) => {
            let text = alloc_string_value(state, &p)?;
            let path = alloc_record_n_fields(state, "Path", &[text])?;
            Ok(Some(wrap_in_variant(state, "Maybe", 1, &[path])?))
        }
        None => Ok(Some(wrap_in_variant(state, "Maybe", 0, &[])?)),
    }
}

/// `core.shell.builtins.command_exists(name: &Text) -> Bool` —
/// `which(name).is_some()` short-circuit.
fn intercept_command_exists(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let name = extract_text_arg(state, args_start_reg, caller_base);
    Ok(Some(Value::from_bool(resolve_in_path(&name).is_some())))
}

/// Walk the user's `PATH` env var looking for an executable named `name`.
/// Returns the absolute path of the first match.  If `name` itself
/// contains a `/`, treats it as a literal path (no PATH lookup).
fn resolve_in_path(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if name.contains('/') {
        let p = std::path::Path::new(name);
        if p.is_file() && is_executable(p) {
            return Some(name.to_string());
        }
        return None;
    }
    let path_env = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_env) {
        let candidate = dir.join(name);
        if candidate.is_file() && is_executable(&candidate) {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_p: &std::path::Path) -> bool {
    true
}

/// Two-arg `write(path, contents)` — choose between text and byte
/// payload by inspecting the second arg's heap shape. TEXT-typed
/// values flow through `extract_text_arg`; List-typed values flow
/// through `extract_byte_list_arg`. This unifies `core.io.file.write`
/// (text) and `core.io.fs.write` (bytes) under one bare-name match.
fn intercept_write_dispatch(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    if let Some(denied) = check_fs_permission(state, "write", &path) {
        return Ok(Some(denied));
    }
    let contents_v = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg + 1));
    let unwrapped = if super::cbgr_helpers::is_cbgr_ref(&contents_v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(contents_v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        contents_v
    };
    let result = if value_is_text(&unwrapped) {
        let text = extract_string(&unwrapped, state);
        std::fs::write(&path, text.as_bytes())
    } else {
        let bytes = extract_byte_list_arg(state, args_start_reg + 1, caller_base);
        std::fs::write(&path, &bytes)
    };
    match result {
        Ok(()) => Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::unit()])?)),
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

/// Quick shape probe — does this Value carry text payload (TEXT type
/// id, the 0x0001 concat layout, or a small string)? Used to
/// dispatch between text-write and byte-write at `write(path, ...)`.
fn value_is_text(v: &Value) -> bool {
    if v.is_small_string() {
        return true;
    }
    if !v.is_ptr() || v.is_nil() {
        return false;
    }
    let ptr = v.as_ptr::<u8>();
    match unsafe { heap::ObjectHeader::try_type_id(ptr) } {
        Some(TypeId::TEXT) => true,
        Some(t) if t == TypeId(0x0001) => true,
        _ => false,
    }
}

// ============================================================================
// Argument extraction
// ============================================================================

/// Extract a path argument, transparently unwrapping THREE shapes:
///

///  1. Bare `&Text` (`extract_text_arg` handles small + heap strings).
///  2. `&Path` — Verum's `Path is { inner: Text }` record. We peek
///  the first field of the heap record and try Text extraction on
///  it; on success the path is the inner Text.
///  3. CBGR-encoded references on top of either of the above.
///

/// Falls back to the empty string when the value is none of the
/// above — the caller's `std::fs::*` invocation will then surface a
/// `NotFound` error which the script can match on.
fn extract_path_arg(state: &InterpreterState, reg: u16, caller_base: u32) -> String {
    let v = state
        .registers
        .get(caller_base, crate::instruction::Reg(reg));
    // Unwrap CBGR-ref AND ThinRef.  Pre-fix only CBGR was unwrapped;
    // ThinRef-encoded `&path` arguments fell into the fast-text
    // branch as a non-Text value, hit the field-drill fallback with
    // garbage, and bottomed out in the `<value:bits>` debug
    // rendering — surfacing as `NotFound` against a path the user
    // never typed.
    let mut unwrapped = if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    };
    if unwrapped.is_thin_ref() {
        let tr = unwrapped.as_thin_ref();
        if !tr.ptr.is_null() {
            unwrapped = unsafe { *(tr.ptr as *const Value) };
        }
    }
    // Fast path: it's already a Text.
    if value_is_text(&unwrapped) {
        return extract_string(&unwrapped, state);
    }
    // Slow path: drill through the canonical 1- or 2-field record
    // shapes carrying a Text payload.
    //
    //   * `Path    { inner: Text }`        — 1 level: peek field 0
    //   * `PathBuf { path: Path { … } }`   — 2 levels: peek field 0
    //     of field 0
    //
    // Pre-fix this only drilled one level, so a `PathBuf` argument
    // (typical when user code calls `path_exists(&pathbuf.as_path())`
    // and as_path is intercepted by `path_ops_runtime`) returned the
    // inner Path-record pointer formatted as a debug string, and the
    // `std::path::Path::new(&path).exists()` host call then asked
    // about a path like `<ptr@0x600003a8c000>` which never exists.
    if unwrapped.is_ptr() && !unwrapped.is_nil() {
        let ptr = unwrapped.as_ptr::<u8>();
        if let Some(header) = unsafe { heap::ObjectHeader::try_from_ptr(ptr) }
            && (header.size as usize) >= std::mem::size_of::<Value>()
        {
            let field0 = unsafe { *(ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value) };
            if value_is_text(&field0) {
                return extract_string(&field0, state);
            }
            // Second level (PathBuf-shaped): if field0 is itself a
            // 1-field record whose own field 0 is a Text, return
            // that Text.
            if field0.is_ptr() && !field0.is_nil() {
                let inner_ptr = field0.as_ptr::<u8>();
                if let Some(inner_header) =
                    unsafe { heap::ObjectHeader::try_from_ptr(inner_ptr) }
                    && (inner_header.size as usize) >= std::mem::size_of::<Value>()
                {
                    let inner_field0 = unsafe {
                        *(inner_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value)
                    };
                    if value_is_text(&inner_field0) {
                        return extract_string(&inner_field0, state);
                    }
                }
            }
        }
    }
    // Last resort — let extract_string do its `<value:...>` debug
    // rendering rather than silently returning empty, so any future
    // bug surfaces in the path string and not as a baffling NotFound.
    extract_string(&unwrapped, state)
}

/// Extract a `&[Byte]` (or `List<Byte>`) argument from a register
/// into an owned `Vec<u8>`. Reads the List header `[len, cap,
/// backing_ptr]` and copies the byte payload.
fn extract_byte_list_arg(state: &InterpreterState, reg: u16, caller_base: u32) -> Vec<u8> {
    let v = state
        .registers
        .get(caller_base, crate::instruction::Reg(reg));
    let unwrapped = if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    };
    if !unwrapped.is_ptr() || unwrapped.is_nil() {
        return Vec::new();
    }
    let base = unwrapped.as_ptr::<u8>();
    if base.is_null() {
        return Vec::new();
    }
    unsafe {
        let header = match heap::ObjectHeader::try_from_ptr(base) {
            Some(h) => h,
            None => return Vec::new(),
        };
        if header.type_id != TypeId::LIST {
            return Vec::new();
        }
        let data_ptr = base.add(heap::OBJECT_HEADER_SIZE) as *const Value;
        let len = (*data_ptr).as_i64() as usize;
        let backing_val = *data_ptr.add(2);
        if !backing_val.is_ptr() || backing_val.is_nil() {
            return Vec::new();
        }
        let backing_ptr = backing_val.as_ptr::<u8>();
        if backing_ptr.is_null() {
            return Vec::new();
        }
        // Backing is one Value-slot per element (List<Byte> stores
        // each byte as a NaN-boxed integer); unpack by truncating
        // each slot to u8. Mirrors `alloc_byte_list`.
        let backing_data = backing_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value;
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            out.push((*backing_data.add(i)).as_i64() as u8);
        }
        out
    }
}

// ============================================================================
// Permission gate
// ============================================================================

/// Check that the script has FileSystem permission for the given
/// access kind (`"read"` or `"write"`) and target path.  Returns
/// `Some(denied_err)` when blocked — the caller substitutes the
/// value into `dst` and short-circuits.  Returns `None` when
/// allowed.
///
/// **VBC-PERM-1 — granular target_id**: passes a stable
/// `target_id_for(path)` hash so a script frontmatter
/// `permissions = ["fs:read=./data"]` grants only that path
/// (the policy registry pre-populates `(FileSystem,
/// target_id_for("./data")) → Allow`).  The router falls through
/// to its `WILDCARD_TARGET_ID` fallback when the script's policy
/// has any wildcard fs grant — preserving backward compatibility
/// for scripts that grant `"fs:read"` without a target.
fn check_fs_permission(state: &mut InterpreterState, _kind: &str, target_path: &str) -> Option<Value> {
    use crate::interpreter::permission::{target_id_for, WILDCARD_TARGET_ID};
    let target_id = target_id_for(target_path);
    // Granular check first.  If the policy doesn't have a
    // path-specific entry, fall through to the wildcard check that
    // matches the legacy "any fs grant allows" behaviour.
    if state.check_permission(PermissionScope::FileSystem, target_id) == PermissionDecision::Allow {
        return None;
    }
    if state.check_permission(PermissionScope::FileSystem, WILDCARD_TARGET_ID) != PermissionDecision::Deny {
        return None;
    }
    // Build an Err(PermissionDenied) result.
    let kind_variant = build_io_error_kind(state, "PermissionDenied", 1).ok()?;
    let msg = format!("permission denied: filesystem access to {}", target_path);
    let msg_text = alloc_string_value(state, &msg).ok()?;
    let msg_some = wrap_in_variant(state, "Maybe", 1, &[msg_text]).ok()?;
    let stream_err =
        alloc_record_n_fields(state, "StreamError", &[kind_variant, msg_some]).ok()?;
    wrap_in_variant(state, "Result", 1, &[stream_err]).ok()
}

// ============================================================================
// Result/StreamError construction
// ============================================================================

fn build_io_err(state: &mut InterpreterState, e: &std::io::Error) -> InterpreterResult<Value> {
    use std::io::ErrorKind as K;
    let (kind_name, kind_tag) = match e.kind() {
        K::NotFound => ("NotFound", 0u32),
        K::PermissionDenied => ("PermissionDenied", 1),
        K::ConnectionRefused => ("ConnectionRefused", 2),
        K::ConnectionReset => ("ConnectionReset", 3),
        K::ConnectionAborted => ("ConnectionAborted", 4),
        K::NotConnected => ("NotConnected", 5),
        K::AddrInUse => ("AddrInUse", 6),
        K::AddrNotAvailable => ("AddrNotAvailable", 7),
        K::BrokenPipe => ("BrokenPipe", 8),
        K::AlreadyExists => ("AlreadyExists", 9),
        K::WouldBlock => ("WouldBlock", 10),
        K::InvalidInput => ("InvalidInput", 11),
        K::InvalidData => ("InvalidData", 12),
        K::TimedOut => ("TimedOut", 13),
        K::WriteZero => ("WriteZero", 14),
        K::Interrupted => ("Interrupted", 15),
        K::UnexpectedEof => ("UnexpectedEof", 16),
        K::OutOfMemory => ("OutOfMemory", 17),
        K::Unsupported => ("Unsupported", 18),
        _ => ("Other", 19),
    };
    let kind_variant = build_io_error_kind(state, kind_name, kind_tag)?;
    let msg_text = alloc_string_value(state, &format!("{}", e))?;
    let msg_some = wrap_in_variant(state, "Maybe", 1, &[msg_text])?;
    let stream_err = alloc_record_n_fields(state, "StreamError", &[kind_variant, msg_some])?;
    wrap_in_variant(state, "Result", 1, &[stream_err])
}

fn build_io_error_kind(
    state: &mut InterpreterState,
    _name: &str,
    tag: u32,
) -> InterpreterResult<Value> {
    // IoErrorKind is a unit-only sum type — variant has no payload.
    wrap_in_variant(state, "IoErrorKind", tag, &[])
}

// Heap helpers (`alloc_byte_list`, `alloc_record_n_fields`,
// `wrap_in_variant`, `lookup_type_id_by_name`) live in
// `super::heap_helpers` — single canonical source for all six
// Tier-0 intercept modules. See VBC-HEAP-1.
