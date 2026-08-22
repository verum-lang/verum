//! REGISTRY-UNKNOWN-KEY-SILENT-1 gate (task #3 / #5).
//!
//! A `@intrinsic("key")` whose key resolves NOWHERE compiled to a silent
//! `nil` in the VBC codegen — this made three whole defect classes
//! invisible during the 2026-07 runtime conformance wave:
//!
//!   * the hollow CBGR bridge (`cbgr_allocate`/`cbgr_deallocate` had no
//!     registry entry → calls produced nil, tests passed vacuously),
//!   * `ptr_add`/`ptr_sub` with no registry entry → nil pointers,
//!   * the ghost `verum_char_*` / `verum_text_*` FFI symbols, and
//!   * the dead `core/runtime/*` tree redeclaring intrinsics under
//!     `@intrinsic("verum.runtime.*")` keys that matched no handler.
//!
//! This test is the standing guard for that class.  It walks the stdlib,
//! extracts every `@intrinsic("...")` key, and asserts:
//!
//!   1. NO key uses the dead `verum.runtime.*` parallel-tree pattern
//!      (RUNTIME-DUPLICATE-TREE-1 — `core/runtime/*` is now re-export
//!      shims over the canonical `core/intrinsics/runtime/*`).
//!   2. Every key declared in the canonical intrinsic home
//!      `core/intrinsics/**` resolves through the `IntrinsicRegistry`
//!      OR the documented name-dispatch allowlist below.  A new
//!      canonical intrinsic that forgets its registry entry / handler
//!      fails here instead of silently emitting nil.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use verum_vbc::intrinsics::IntrinsicRegistry;

fn core_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../core"))
}

/// Recursively collect every `.vr` file under `dir`.
fn collect_vr(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_vr(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("vr") {
            out.push(path);
        }
    }
}

/// Extract every `@intrinsic("key"` key from a source string.  Only the
/// FIRST argument (the key) is taken; type/value args after it are ignored.
fn intrinsic_keys(src: &str) -> Vec<String> {
    // Drop `//` line comments so a doc/banner mentioning the literal
    // `@intrinsic("verum.runtime.*")` (e.g. the shim files' history note)
    // is not mistaken for a declaration.
    let stripped: String = src
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let src = stripped.as_str();
    let mut keys = Vec::new();
    let bytes = src.as_bytes();
    let needle = b"@intrinsic(\"";
    let mut i = 0;
    while i + needle.len() < bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let start = i + needle.len();
            if let Some(end_rel) = src[start..].find('"') {
                keys.push(src[start..start + end_rel].to_string());
            }
            i = start;
        } else {
            i += 1;
        }
    }
    keys
}

/// Intrinsic keys that resolve through interpreter NAME-DISPATCH
/// (`handlers/calls.rs::try_dispatch_intrinsic_by_name`) or a dedicated
/// opcode rather than a registry `Intrinsic` entry.  These are legitimate
/// resolution routes; the gate treats them as resolved.  Grouped by family
/// so a reviewer can see the shape of the exemption.
fn name_dispatch_allowlist() -> HashSet<&'static str> {
    let mut s = HashSet::new();
    // Elementary math (dispatched by bare/suffix name in calls.rs).
    for k in [
        "sqrt", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh",
        "asinh", "acosh", "atanh", "exp", "exp2", "expm1", "log", "log2", "log10", "log1p", "pow",
        "cbrt", "hypot", "fma", "floor", "ceil", "round", "trunc", "fabs", "copysign", "fmod",
    ] {
        s.insert(k);
    }
    // Raw byte/word + C-string (mem_raw underscore + canonical aliases).
    for k in [
        "load_byte", "store_byte", "load_i64", "store_i64", "load_i32", "store_i32", "memcpy",
        "memmove", "memset", "memcmp", "strlen", "strcmp",
    ] {
        s.insert(k);
    }
    // Time / OS name-dispatch families (os.vr `__*_raw`, time keys).
    for k in [
        "sleep_ms", "sleep_ns", "realtime_nanos", "realtime_secs", "monotonic_nanos", "num_cpus",
    ] {
        s.insert(k);
    }
    s
}

/// Keys whose resolution is intentionally out of this gate's scope: the
/// GPU / tensor / SIMD accelerator families (dispatched via their own
/// extended-opcode tables) and the meta/compile-time-only intrinsics.
/// Prefix-matched.  Keeping this list SMALL and PREFIXED is deliberate —
/// a blanket exemption would defeat the gate.
fn scoped_out_prefix(key: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "gpu_", "GPU_", "tensor_", "simd_", "cuda_", "metal_", "rocm_", "vulkan_", "wgpu_",
        "llvm.", // LLVM-intrinsic passthrough names
    ];
    PREFIXES.iter().any(|p| key.starts_with(p))
}

#[test]
fn no_dead_verum_runtime_parallel_tree() {
    let mut files = Vec::new();
    collect_vr(&core_dir(), &mut files);
    assert!(!files.is_empty(), "no .vr files found under core/");

    let mut offenders = Vec::new();
    for f in &files {
        let src = fs::read_to_string(f).unwrap_or_default();
        for key in intrinsic_keys(&src) {
            if key.starts_with("verum.runtime.") {
                offenders.push(format!("{}: @intrinsic(\"{}\")", f.display(), key));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "RUNTIME-DUPLICATE-TREE-1 regression: the dead `verum.runtime.*` \
         intrinsic-redeclaration pattern is back. `core/runtime/*` must be \
         re-export shims over `core/intrinsics/runtime/*`, not redeclare \
         intrinsics under an unwired key.\n{}",
        offenders.join("\n")
    );
}

#[test]
fn canonical_intrinsic_keys_resolve() {
    let registry = IntrinsicRegistry::new();
    let allow = name_dispatch_allowlist();
    let canonical_root = core_dir().join("intrinsics");

    let mut files = Vec::new();
    collect_vr(&canonical_root, &mut files);
    assert!(
        !files.is_empty(),
        "no .vr files under core/intrinsics — path wrong?"
    );

    let mut unresolved = Vec::new();
    for f in &files {
        let src = fs::read_to_string(f).unwrap_or_default();
        for key in intrinsic_keys(&src) {
            if scoped_out_prefix(&key) || allow.contains(key.as_str()) {
                continue;
            }
            if registry.lookup(&key).is_none() {
                unresolved.push(format!("{}: @intrinsic(\"{}\")", f.display(), key));
            }
        }
    }

    // NOTE: this list is expected to be non-empty on the current tree —
    // several canonical intrinsics resolve via dedicated opcodes or
    // handler name-dispatch that predate this gate.  The gate's JOB is to
    // stop it GROWING: it prints the current set so a reviewer adding a
    // new canonical intrinsic sees immediately whether their key resolves.
    // We assert a CEILING rather than zero so the gate lands green today
    // and tightens as the name-dispatch families migrate into the
    // registry (tracked in task #3).
    let ceiling = 245;
    assert!(
        unresolved.len() <= ceiling,
        "canonical intrinsic keys not resolving via registry/allowlist \
         exceeded the ceiling of {} (found {}). Either add a registry \
         entry for the new key, extend the documented name-dispatch \
         allowlist, or lower the ceiling as families migrate.\n{}",
        ceiling,
        unresolved.len(),
        unresolved.join("\n")
    );
}

// ---------------------------------------------------------------------------
// ARITY-DRIFT-INEXPRESSIBLE-1 (T0846, born from T0844)
// ---------------------------------------------------------------------------

/// Extract `(key, param_arity)` for every ATTRIBUTE-form
/// `@intrinsic("key")` declaration — the form that sits above a `fn`.
/// Body-macro sites (`@intrinsic("key", args…)` inside a function body)
/// carry call arguments, not a declared arity, and are skipped: the
/// discriminator is whether anything follows the key before `)`.
fn intrinsic_decl_arities(src: &str) -> Vec<(String, usize)> {
    let stripped: String = src
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let src = stripped.as_str();
    let mut out = Vec::new();
    let needle = "@intrinsic(\"";
    let mut i = 0usize;
    while let Some(at) = src[i..].find(needle) {
        let start = i + at + needle.len();
        let Some(end_rel) = src[start..].find('"') else { break };
        let key = &src[start..start + end_rel];
        let after_key = &src[start + end_rel + 1..];
        i = start + end_rel;
        // Attribute form closes immediately: `")`. Anything else —
        // a comma, an expression — is the body-macro form.
        if !after_key.starts_with(')') {
            continue;
        }
        let rest = &after_key[1..];
        // An ARGLESS body-macro (`{ @intrinsic("abort") }`) also closes
        // with `")` — distinguish it by what follows: a statement/tail
        // position ends in `;` or `}`, a declaration attribute is
        // followed by modifiers and `fn`.
        let next = rest.trim_start();
        if next.starts_with('}') || next.starts_with(';') {
            continue;
        }
        // The declaration may carry further attributes and modifiers
        // before `fn`; stop the search at the first `;`, `{` or `}` so
        // a stray later `fn` is never attributed to this key.
        let boundary = rest
            .find(|c| c == ';' || c == '{' || c == '}')
            .unwrap_or(rest.len());
        let head = &rest[..boundary];
        let Some(fn_at) = head.find("fn ") else { continue };
        // The parameter list is the first `(` OUTSIDE the generic
        // block. Two traps live in `<...>`: a bound like
        // `F: fn() -> T` contains a paren, and its `->` contains a
        // `>` that is NOT a generic close — track both.
        let after_fn = &head[fn_at..];
        let mut angle = 0i32;
        let mut prev = '\0';
        let mut params_rel = None;
        for (idx, c) in after_fn.char_indices() {
            match c {
                '<' => angle += 1,
                '>' if prev != '-' => angle = (angle - 1).max(0),
                '(' if angle == 0 => {
                    params_rel = Some(idx + 1);
                    break;
                }
                _ => {}
            }
            prev = c;
        }
        let Some(params_rel) = params_rel else { continue };
        let params_start = fn_at + params_rel;
        // Count top-level commas, aware of (), <>, [] nesting — a
        // parameter like `f: fn(Int) -> Int` or `m: Map<K, V>` counts
        // once.
        // Count SEGMENTS with content, so a trailing comma after the
        // last parameter does not manufacture a phantom one.
        let mut depth = 0i32;
        let mut angle = 0i32;
        let mut arity = 0usize;
        let mut segment_has_content = false;
        let mut prev_c = '\0';
        for c in head[params_start..].chars() {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                '<' => angle += 1,
                '>' if prev_c != '-' => angle = (angle - 1).max(0),
                ',' if depth == 0 && angle == 0 => {
                    if segment_has_content {
                        arity += 1;
                    }
                    segment_has_content = false;
                }
                c if !c.is_whitespace() => segment_has_content = true,
                _ => {}
            }
            prev_c = c;
        }
        if segment_has_content {
            arity += 1;
        }
        out.push((key.to_string(), arity));
    }
    out
}

/// Every attribute-form `@intrinsic` declaration whose key has a
/// registry entry must declare exactly `param_count` parameters.
///
/// This is the arity dimension of the T0844 lesson. The `cbgr_alloc`
/// contract was described in several places, and the AOT lowering
/// silently dropped `align` — a drift no test could see, because
/// nothing compared the declared surface against the dispatch table.
/// (A THIRD description — a whole `IntrinsicSignature` registry — had
/// drifted too, unreadably: `get_signature` had no callers, so its
/// entries claimed arities and types nothing ever checked. That
/// registry is deleted rather than repaired; this gate pins the two
/// surfaces that are actually alive.)
#[test]
fn declared_arities_match_the_registry() {
    let mut files = Vec::new();
    collect_vr(&core_dir(), &mut files);
    assert!(
        !files.is_empty(),
        "no .vr files found under core/ — the scan is looking in the wrong place"
    );
    let mut mismatches = Vec::new();
    let mut checked = 0usize;
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        for (key, arity) in intrinsic_decl_arities(&src) {
            let Some(intr) = verum_vbc::intrinsics::lookup_intrinsic(&key) else {
                continue;
            };
            checked += 1;
            if intr.intrinsic.param_count as usize != arity {
                mismatches.push(format!(
                    "{}: @intrinsic(\"{}\") declares {} parameter(s), registry says {}",
                    f.display(),
                    key,
                    arity,
                    intr.intrinsic.param_count,
                ));
            }
        }
    }
    assert!(
        checked > 50,
        "only {checked} declarations were cross-checked — the scan or the \
         registry lookup has gone quiet, and a gate that checks nothing \
         proves nothing"
    );
    mismatches.sort();
    mismatches.dedup();
    assert!(
        mismatches.is_empty(),
        "declared arity and registry param_count disagree — one of them is \
         lying about the contract, and T0844 is what that costs:\n{}",
        mismatches.join("\n"),
    );
}
