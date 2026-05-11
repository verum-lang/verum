//! High-level Rust intercepts for `core.io.path.{Path, PathBuf}`
//! inherent methods.
//!
//! Sibling to `file_runtime.rs` (which intercepts `core.io.fs.*`
//! free functions). These method-shaped intercepts close the gap
//! left by stub-only registration: when the typechecker sees
//! `pathbuf.as_path()` and codegen emits a `CallM { method_id:
//! "as_path" }`, no real body is loaded into the user module —
//! `emit_missing_stub_descriptors` registers a `RetV` placeholder
//! so dispatch doesn't fail with "method not found", but the
//! placeholder returns Unit and downstream code that treats the
//! result as a `Path` crashes with "field write out of bounds".
//!
//! Each method here re-implements the stdlib body in Rust against
//! the runtime heap layout:
//!   * `Path  { inner: Text }`  — 1-field record, payload 8 bytes
//!   * `PathBuf { path: Path }` — 1-field record, payload 8 bytes
//!
//! # Methods intercepted
//!  * `PathBuf.as_path(&self) -> Path`         — returns the inner `path` field
//!  * `Path.to_path_buf(&self) -> PathBuf`     — wraps the receiver
//!  * `Path.join(&self, &Path) -> PathBuf`     — joins with `/`
//!  * `Path.join_str(&self, &Text) -> PathBuf` — joins with `/`
//!  * `PathBuf.join(&self, &Path) -> PathBuf`  — promoted via as_path
//!  * `PathBuf.join_str(&self, &Text) -> PathBuf`
//!  * `Path.as_str(&self) -> Text`             — returns the inner Text
//!  * `Path.to_str(&self) -> Text`             — alias of as_str
//!  * `PathBuf.as_str(&self) -> Text`          — drills through path.inner
//!  * `PathBuf.to_str(&self) -> Text`
//!  * `Path.parent(&self) -> Maybe<Path>`      — drops trailing component
//!  * `PathBuf.parent(&self) -> Maybe<Path>`

use super::super::super::error::InterpreterResult;
use super::heap_helpers::{alloc_record_n_fields, wrap_in_variant};
use super::string_helpers::{alloc_string_value, extract_string, is_heap_string};
use crate::interpreter::heap;
use crate::interpreter::state::InterpreterState;
use crate::value::Value;

/// Try to intercept a Path/PathBuf method call. Returns `Some(value)`
/// when the interception fires, `None` otherwise (caller falls through
/// to normal bytecode dispatch).
pub(in super::super) fn try_intercept_path_method(
    state: &mut InterpreterState,
    bare_method_name: &str,
    receiver: Value,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Fast hot-path miss: only path-shaped methods proceed.
    let is_path_shape = matches!(
        bare_method_name,
        "as_path"
            | "to_path_buf"
            | "join"
            | "join_str"
            | "as_str"
            | "to_str"
            | "parent"
            | "to_path"
    );
    if !is_path_shape {
        return Ok(None);
    }
    if !receiver.is_ptr() || receiver.is_nil() {
        return Ok(None);
    }
    // The receiver must look like a 1-field record (Path or PathBuf).
    // Recognise both shapes by reading field 0:
    //   * Text → receiver is a Path (`{ inner: Text }`)
    //   * Object that itself contains Text in field 0 → receiver is a PathBuf
    //     (`{ path: Path { inner: Text } }`)
    let inner_text_value = extract_path_inner_text(state, &receiver);
    let inner_text = match inner_text_value {
        Some(t) => t,
        None => return Ok(None),
    };
    match bare_method_name {
        "as_path" => {
            // PathBuf.as_path → returns the inner `Path` value.
            // Path.as_path (rare) → identity.
            //
            // Receiver disambiguation against the canonical shapes:
            //   * Path     { inner: Text }       — field 0 is Text
            //   * PathBuf  { path: Path }        — field 0 is an
            //     Object whose own field 0 is Text
            //
            // When field 0 is Text the receiver IS already a Path —
            // return it as-is (identity).  When field 0 is an Object
            // and that Object's field 0 is Text, the receiver is a
            // PathBuf and we return the inner Path.  Anything else
            // falls back to building a fresh Path from the recovered
            // inner text — covers receivers built by malformed
            // intercepts that collapsed the two shapes.
            let f0 = read_field0(&receiver);
            if let Some(v) = f0 {
                if extract_string_if_text(state, &v).is_some() {
                    return Ok(Some(receiver));
                }
                if v.is_ptr() && !v.is_nil() {
                    if let Some(inner) = read_field0(&v) {
                        if extract_string_if_text(state, &inner).is_some() {
                            return Ok(Some(v));
                        }
                    }
                }
            }
            let new_text = alloc_string_value(state, &inner_text)?;
            let path = alloc_record_n_fields(state, "Path", &[new_text])?;
            Ok(Some(path))
        }
        "to_path_buf" | "to_path" => {
            // Path.to_path_buf → PathBuf { path: Path { inner: Text } }
            let new_text = alloc_string_value(state, &inner_text)?;
            let new_path = alloc_record_n_fields(state, "Path", &[new_text])?;
            let pathbuf = alloc_record_n_fields(state, "PathBuf", &[new_path])?;
            Ok(Some(pathbuf))
        }
        "as_str" | "to_str" => {
            let v = alloc_string_value(state, &inner_text)?;
            Ok(Some(v))
        }
        "join" => {
            if arg_count != 1 {
                return Ok(None);
            }
            let other_raw = state
                .registers
                .get(caller_base, crate::instruction::Reg(args_start_reg));
            let other_val = deref_cbgr(state, other_raw);
            let other_text = match extract_path_inner_text(state, &other_val) {
                Some(t) => t,
                None => match extract_string_if_text(state, &other_val) {
                    Some(t) => t,
                    None => return Ok(None),
                },
            };
            let joined = join_paths(&inner_text, &other_text);
            let new_text = alloc_string_value(state, &joined)?;
            let new_path = alloc_record_n_fields(state, "Path", &[new_text])?;
            let pathbuf = alloc_record_n_fields(state, "PathBuf", &[new_path])?;
            Ok(Some(pathbuf))
        }
        "join_str" => {
            if arg_count != 1 {
                return Ok(None);
            }
            let other_raw = state
                .registers
                .get(caller_base, crate::instruction::Reg(args_start_reg));
            let other_val = deref_cbgr(state, other_raw);
            let other_text = match extract_string_if_text(state, &other_val) {
                Some(t) => t,
                None => return Ok(None),
            };
            let joined = join_paths(&inner_text, &other_text);
            let new_text = alloc_string_value(state, &joined)?;
            let new_path = alloc_record_n_fields(state, "Path", &[new_text])?;
            let pathbuf = alloc_record_n_fields(state, "PathBuf", &[new_path])?;
            Ok(Some(pathbuf))
        }
        "parent" => {
            // Maybe<Path>: Some(parent) or None.
            // Strip trailing slash, then drop everything after the
            // last `/`.  No `/` → None.  `/` only → None.
            let trimmed = inner_text.trim_end_matches('/');
            let parent = match trimmed.rfind('/') {
                Some(0) => Some("/".to_string()),
                Some(i) => Some(trimmed[..i].to_string()),
                None => None,
            };
            match parent {
                Some(p) => {
                    let t = alloc_string_value(state, &p)?;
                    let path = alloc_record_n_fields(state, "Path", &[t])?;
                    // Maybe.Some has tag=1 (None=0, Some=1 by stdlib convention).
                    let some = wrap_in_variant(state, "Maybe", 1, &[path])?;
                    Ok(Some(some))
                }
                None => {
                    let none = wrap_in_variant(state, "Maybe", 0, &[])?;
                    Ok(Some(none))
                }
            }
        }
        _ => Ok(None),
    }
}

/// Unwrap any of the three reference encodings that the interpreter
/// uses when passing `&T` arguments across a function boundary:
///
///   * CBGR-ref (negative inline-int with generation>=1) — register
///     handle into the caller's absolute register file.  Used for
///     refs to in-scope locals.
///   * ThinRef (TAG_POINTER + bits 47, 45 set) — heap-side pointer
///     to a stored Value cell.  Used when the referent lives in a
///     different stack frame than the caller.
///   * Plain Value — already unwrapped.
///
/// Pre-fix this only handled CBGR-ref → user code like
/// `paper_dir.join_str(&"paper.tex")` where the `&Text` argument
/// arrived as a ThinRef silently fell out of the intercept (string
/// extraction failed → join_str returned `None` → method dispatch
/// fell through to the `RetV` stub which returns `()`), and
/// downstream `path_exists(&path.as_path())` saw a corrupted small-
/// string-like value rendered as `<value:N>`.
fn deref_cbgr(state: &InterpreterState, v: Value) -> Value {
    if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v.as_i64());
        return state.registers.get_absolute(abs_index);
    }
    if v.is_thin_ref() {
        let tr = v.as_thin_ref();
        if !tr.ptr.is_null() {
            return unsafe { *(tr.ptr as *const Value) };
        }
    }
    v
}

fn read_field0(receiver: &Value) -> Option<Value> {
    if !receiver.is_ptr() || receiver.is_nil() {
        return None;
    }
    let ptr = receiver.as_ptr::<u8>();
    if ptr.is_null() {
        return None;
    }
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    if (header.size as usize) < std::mem::size_of::<Value>() {
        return None;
    }
    let v = unsafe { *(ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value) };
    Some(v)
}

fn extract_string_if_text(state: &InterpreterState, v: &Value) -> Option<String> {
    if v.is_small_string() {
        return Some(extract_string(v, state));
    }
    if is_heap_string(v) {
        return Some(extract_string(v, state));
    }
    None
}

/// Try to extract the inner Text content from a Path or PathBuf
/// receiver by drilling field 0 once or twice.
fn extract_path_inner_text(state: &InterpreterState, receiver: &Value) -> Option<String> {
    if let Some(t) = extract_string_if_text(state, receiver) {
        return Some(t);
    }
    let f0 = read_field0(receiver)?;
    if let Some(t) = extract_string_if_text(state, &f0) {
        return Some(t); // Path (field 0 is Text)
    }
    let f00 = read_field0(&f0)?;
    extract_string_if_text(state, &f00) // PathBuf (field 0 is Path, field 0 of Path is Text)
}

/// Concatenate two path components with a single `/` separator.
/// If `other` is absolute (starts with `/`), it replaces `base`.
fn join_paths(base: &str, other: &str) -> String {
    if other.starts_with('/') {
        return other.to_string();
    }
    if base.is_empty() {
        return other.to_string();
    }
    if base.ends_with('/') {
        format!("{}{}", base, other)
    } else {
        format!("{}/{}", base, other)
    }
}

/// Free-function intercept for Path / PathBuf constructors and free
/// helpers.  Sibling to `try_intercept_path_method` — same rationale
/// (stdlib bodies aren't loaded into the user module so RetV stubs
/// return Unit and downstream record-shape consumers crash).  Wired
/// from `calls.rs` next to `try_intercept_file_runtime` etc.
///
/// Functions intercepted (matched on the trailing simple name after
/// `rsplit('.').next()` so the qualified `core.io.path.Path.new` and
/// the bare `Path.new` both fire):
///
///  * `Path.new(s: &Text) -> Path`            — `{ inner: s.clone() }`
///  * `Path.from_str(s: &Text) -> Path`       — synonym for Path.new
///  * `PathBuf.new() -> PathBuf`              — `{ path: { inner: "" } }`
///  * `PathBuf.from(s: &Text) -> PathBuf`     — `{ path: { inner: s.clone() } }`
///  * `PathBuf.from_str(s: &Text) -> PathBuf` — synonym for PathBuf.from
///  * `PathBuf.with_capacity(_) -> PathBuf`   — capacity is a hint;
///    return an empty PathBuf (stdlib semantics: no observable side-effect).
pub(in super::super) fn try_intercept_path_constructor(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Two-level qualified shape (`Type.method`) — strip module path.
    // For `core.io.path.Path.new` we want `Path.new`, not `new`.
    let qualified = func_name.rsplit_terminator('.').take(2).collect::<Vec<_>>();
    let qual_name = if qualified.len() == 2 {
        format!("{}.{}", qualified[1], qualified[0])
    } else {
        func_name.to_string()
    };
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    match (qual_name.as_str(), bare, arg_count) {
        ("Path.new", _, 1)
        | ("Path.from_str", _, 1)
        | ("Path.from", _, 1) => {
            let s = match read_text_arg(state, args_start_reg, caller_base) {
                Some(s) => s,
                None => return Ok(None),
            };
            let text = alloc_string_value(state, &s)?;
            let path = alloc_record_n_fields(state, "Path", &[text])?;
            Ok(Some(path))
        }
        ("PathBuf.new", _, 0) => {
            let text = alloc_string_value(state, "")?;
            let path = alloc_record_n_fields(state, "Path", &[text])?;
            let pathbuf = alloc_record_n_fields(state, "PathBuf", &[path])?;
            Ok(Some(pathbuf))
        }
        ("PathBuf.with_capacity", _, 1) => {
            let text = alloc_string_value(state, "")?;
            let path = alloc_record_n_fields(state, "Path", &[text])?;
            let pathbuf = alloc_record_n_fields(state, "PathBuf", &[path])?;
            Ok(Some(pathbuf))
        }
        ("PathBuf.from", _, 1)
        | ("PathBuf.from_str", _, 1) => {
            let s = match read_text_arg(state, args_start_reg, caller_base) {
                Some(s) => s,
                None => return Ok(None),
            };
            let text = alloc_string_value(state, &s)?;
            let path = alloc_record_n_fields(state, "Path", &[text])?;
            let pathbuf = alloc_record_n_fields(state, "PathBuf", &[path])?;
            Ok(Some(pathbuf))
        }
        _ => {
            // Stdlib free helper `core.io.path.join(base, other)`.
            // Bare-name match avoids requiring the qualified prefix.
            if bare == "join" && arg_count == 2 {
                let base = read_path_arg(state, args_start_reg, caller_base);
                let other = read_path_arg(state, args_start_reg + 1, caller_base);
                if let (Some(b), Some(o)) = (base, other) {
                    let joined = join_paths(&b, &o);
                    let text = alloc_string_value(state, &joined)?;
                    let path = alloc_record_n_fields(state, "Path", &[text])?;
                    let pathbuf = alloc_record_n_fields(state, "PathBuf", &[path])?;
                    return Ok(Some(pathbuf));
                }
            }
            Ok(None)
        }
    }
}

/// Intercept `Call { func_id }` for inherent-method names of the
/// form `Path.<method>` / `PathBuf.<method>` (mostly emitted when
/// codegen statically resolved the method-call to a concrete
/// FunctionId because the receiver type was known at compile time
/// — bypassing the `CallM` path that `try_intercept_path_method`
/// covers).  Without this sibling, `p.join_str(&other)` where `p`
/// has static type `&Path` lands at the stub `RetV` body and
/// returns Unit; downstream `joined.as_path()` then operates on
/// Unit and crashes the file-runtime intercept ("path='<value:N>'
/// exists=false").
///
/// Treats `args[0]` as the receiver (mirroring the standard
/// `(self, …)` convention for inherent methods) and forwards to
/// the same per-method bodies as `try_intercept_path_method`.
pub(in super::super) fn try_intercept_path_inherent_call(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Quick gate: must end with one of the known inherent-method
    // names.  Anything else (free constructors, sibling-type
    // methods, etc.) is dispatched elsewhere — fast-path miss.
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    let is_path_inherent = matches!(
        bare,
        "as_path"
            | "to_path_buf"
            | "join"
            | "join_str"
            | "as_str"
            | "to_str"
            | "parent"
            | "to_path"
    );
    if !is_path_inherent {
        return Ok(None);
    }
    // Receiver type prefix MUST be Path or PathBuf — the bare-name
    // gate above is shared with non-Path types (e.g. List.join).
    // Confirm via the qualified prefix.  Accept both fully-qualified
    // (`core.io.path.Path.join_str`) and short (`Path.join_str`).
    let qualified = func_name.rsplit_terminator('.').take(2).collect::<Vec<_>>();
    if qualified.len() != 2 {
        return Ok(None);
    }
    let type_name = qualified[1];
    if type_name != "Path" && type_name != "PathBuf" {
        return Ok(None);
    }
    if arg_count == 0 {
        return Ok(None);
    }
    // Receiver lives at args_start_reg; rebase the inner intercept
    // to args_start_reg+1 with arg_count-1 so it sees the same shape
    // as the CallM pathway.
    let recv_raw = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg));
    let receiver = deref_cbgr(state, recv_raw);
    try_intercept_path_method(
        state,
        bare,
        receiver,
        args_start_reg + 1,
        arg_count - 1,
        caller_base,
    )
}

fn read_text_arg(state: &InterpreterState, reg: u16, caller_base: u32) -> Option<String> {
    let v = state
        .registers
        .get(caller_base, crate::instruction::Reg(reg));
    let v = deref_cbgr(state, v);
    extract_string_if_text(state, &v)
}

fn read_path_arg(state: &InterpreterState, reg: u16, caller_base: u32) -> Option<String> {
    let v = state
        .registers
        .get(caller_base, crate::instruction::Reg(reg));
    let v = deref_cbgr(state, v);
    if let Some(s) = extract_string_if_text(state, &v) {
        return Some(s);
    }
    extract_path_inner_text(state, &v)
}
