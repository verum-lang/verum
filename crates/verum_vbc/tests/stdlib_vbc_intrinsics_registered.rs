//! Ratchet: every `@vbc(NAME, …)` written in the stdlib must resolve to
//! something that emits an instruction.
//!
//! `compile_vbc_intrinsic_call` answers an unknown intrinsic name by emitting
//! `Instruction::LoadNil` and returning `Ok`; the comment in place calls that
//! "not necessarily an error during early development".  The consequence is
//! that a `@vbc` call naming an unregistered intrinsic compiles cleanly,
//! returns nil at runtime, and reports nothing anywhere.
//!
//! That is how `TENSOR_SLICE_ASSIGN` came to be a silent no-op — which is why
//! `KVCache.update()` in `core/math/agent.vr` never writes and every attention
//! pass over a KV cache reads the zeros the cache was allocated with (T0184).
//! `CATCH_UNWIND` in `core/base/panic.vr` is on the same list.
//!
//! This is a RATCHET, not a clean bill of health.  The names below are
//! unregistered TODAY; listing them makes the class visible and stops it
//! growing.  Fixing one means deleting its line here: the second test fails if
//! a listed name becomes registered, so the list can only shrink deliberately.
//!
//! DO NOT "fix" one of these by adding a registry entry without reading its
//! handler first.  A triage of all 72 names against the interpreter's dispatch
//! arms found that NONE of them is one registry entry away from working:
//!
//!   * 68 have no handler at all — they need a real implementation.
//!   * `TENSOR_FLIP` and `TENSOR_POOL` have handlers that read IMMEDIATES from
//!     the bytecode (`read_varint`, then one `u8` per axis).  The generic
//!     emitter writes `[dst][args as registers]`, so registering them would
//!     make the handler read a register byte as a length and then consume
//!     arbitrary bytes as axes — a wire mismatch, which is worse than the nil
//!     they return today because it produces a plausible wrong tensor.
//!   * `EVENT_RECORD`'s handler reads THREE registers (dst, event, stream)
//!     while `core/math/advanced.vr` passes one argument.  Registering it
//!     would leave a `read_reg` consuming the NEXT instruction's bytes and
//!     desynchronise the stream.
//!   * `TENSOR_DEBUG_INFO` resolves to `LogSubOpcode::Info` and
//!     `VALUE_FROM_BOOL` to `TensorDType::Bool` — a different opcode family
//!     and a dtype variant respectively.  Short CamelCase names make that
//!     heuristic lie; check which enum the variant actually belongs to.
//!
//! `SAMPLE_TOP_K` (14a2c1d41) was a genuine registry-only fix, which is what
//! makes this warning necessary: it is the exception, not the pattern.
//!
//! The predicate is `lookup_intrinsic` itself rather than a regex over the
//! registry source.  That matters: while this census was being built, a
//! pattern reading `name: "…"` literals under-reported the registry by four
//! times (242 of 933 entries) and produced seven false accusations.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use verum_vbc::intrinsics::lookup_intrinsic;

fn core_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../core"))
}

/// Handled by `compile_vbc_intrinsic_call` BEFORE it consults the registry,
/// so these legitimately have no entry.  Mirrors the `matches!` arm there.  A
/// name added there but not here surfaces as a new unregistered intrinsic —
/// the safe direction to fail.
const SPECIAL_CASED: &[&str] = &[
    "CELL_SET",
    "LAZYCELL_FORCE",
    "LAZYCELL_INIT",
    "ONCECELL_SET",
    "ONCECELL_TAKE",
    "REFCELL_BORROW",
    "REFCELL_BORROW_MUT",
    "REFCELL_REPLACE",
    "REFCELL_SET",
];

/// Unregistered today: each compiles to a silent `LoadNil` at every call site.
/// The trailing comment on each line is the stdlib module that calls it.
const KNOWN_UNREGISTERED: &[&str] = &[
    "AUTODIFF_CHECKPOINTED_SEQ",         // math/autodiff.vr
    "AUTODIFF_HVP",                      // math/autodiff.vr
    "BACKEND_GET_CAPS",                  // math/internal.vr
    "CATCH_UNWIND",                      // base/panic.vr
    "DEBUG_DISASSEMBLE",                 // math/internal.vr
    "DEBUG_INSTRUCTION_COUNT",           // math/internal.vr
    "DEBUG_RESET_INSTRUCTION_COUNT",     // math/internal.vr
    "EVENT_RECORD",                      // math/advanced.vr
    "EVENT_WAIT",                        // math/advanced.vr
    "FFI_CALL_RAW",                      // math/internal.vr
    "FFI_GET_SYMBOL",                    // math/internal.vr
    "GRAD_GET",                          // math/internal.vr
    "GRAD_RECORD",                       // math/advanced.vr
    "GRAD_REGISTER_BACKWARD",            // math/advanced.vr
    "GRAD_REGISTER_VJP",                 // math/advanced.vr
    "GRAD_SET",                          // math/internal.vr
    "KERNEL_INVOKE_DIRECT",              // math/internal.vr
    "KERNEL_REGISTER",                   // math/advanced.vr
    "MEM_ALLOC_ALIGNED",                 // math/advanced.vr
    "MEM_CMP_RAW",                       // math/internal.vr
    "MEM_COPY_RAW",                      // math/internal.vr
    "MEM_DEALLOC",                       // math/advanced.vr
    "MEM_SET_RAW",                       // math/internal.vr
    "PTR_ALIGNMENT",                     // math/internal.vr
    "STREAM_POP",                        // math/advanced.vr
    "STREAM_PUSH",                       // math/advanced.vr
    "STREAM_SYNC",                       // math/advanced.vr
    "TAPE_PUSH_OP",                      // math/internal.vr
    "TENSOR_ADD_INPLACE",                // math/advanced.vr
    "TENSOR_CHUNK",                      // math/tensor.vr
    "TENSOR_CONV1D",                     // math/nn.vr
    "TENSOR_DATA_PTR",                   // math/internal.vr
    "TENSOR_DATA_PTR_CONST",             // math/internal.vr
    "TENSOR_DEBUG_INFO",                 // math/internal.vr
    "TENSOR_FLIP",                       // math/tensor.vr
    "TENSOR_FROM_BUFFER",                // math/advanced.vr
    "TENSOR_FROM_HANDLE",                // math/internal.vr
    "TENSOR_GET_HANDLE",                 // math/internal.vr
    "TENSOR_GET_ID",                     // math/internal.vr
    "TENSOR_MATMUL_INTO",                // math/advanced.vr
    "TENSOR_MATMUL_PRECISE",             // math/advanced.vr
    "TENSOR_MATMUL_TILED",               // math/advanced.vr
    "TENSOR_MAX",                        // math/tensor.vr
    "TENSOR_MIN",                        // math/tensor.vr
    "TENSOR_MUL_INPLACE",                // math/advanced.vr
    "TENSOR_NORM_AXIS",                  // math/tensor.vr
    "TENSOR_POOL",                       // math/tensor.vr
    "TENSOR_SCALE_INPLACE",              // math/advanced.vr
    "TENSOR_SCATTER_ADD",                // math/tensor.vr
    "TENSOR_SET",                        // math/nn.vr
    "TENSOR_SLICE_ASSIGN",               // math/tensor.vr
    "TENSOR_SOFTMAX_INTO",               // math/advanced.vr
    "TENSOR_TRANSPOSE_LAST2",            // math/nn.vr
    "TYPE_ALIGN",                        // math/internal.vr
    "TYPE_GET_ID",                       // math/internal.vr
    "TYPE_SIZE",                         // math/internal.vr
    "VALUE_FROM_BOOL",                   // math/internal.vr
    "VALUE_FROM_FLOAT",                  // math/internal.vr
    "VALUE_FROM_INT",                    // math/internal.vr
    "VALUE_FROM_PTR",                    // math/internal.vr
    "VALUE_GET_TAG",                     // math/internal.vr
    "VALUE_TO_FLOAT_UNCHECKED",          // math/internal.vr
    "VALUE_TO_INT_UNCHECKED",            // math/internal.vr
];

/// Every `@vbc(NAME` bareword under `core/`, with the file that spells it.
fn collect_vbc_names(dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_vbc_names(&path, out);
        } else if path.extension().is_some_and(|e| e == "vr") {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            // Strip `//` comments before scanning. Prose that SPELLS an
            // intrinsic — "this previously called `@vbc(REGEX_FIND_ALL, ..)`"
            // — is not a call site, and counting it made this gate fail on a
            // comment explaining a fix it had just prompted. A `//` inside a
            // string literal on the same line as a real `@vbc(` would hide
            // that call; no such line exists, and the alternative is a gate
            // that forbids documenting an intrinsic by name.
            let code: String = text
                .lines()
                .map(|l| l.split_once("//").map_or(l, |(before, _)| before))
                .collect::<Vec<_>>()
                .join("\n");
            let mut rest = code.as_str();
            while let Some(at) = rest.find("@vbc(") {
                rest = &rest[at + 5..];
                let name: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    out.push((name, path.display().to_string()));
                }
            }
        }
    }
}

#[test]
fn no_new_unregistered_vbc_intrinsics_in_the_stdlib() {
    let mut names = Vec::new();
    collect_vbc_names(&core_dir(), &mut names);
    assert!(
        names.len() > 100,
        "found only {} `@vbc` call sites under {} — the walk did not run, and \
         an empty walk would pass this gate vacuously",
        names.len(),
        core_dir().display()
    );

    let mut unexpected: Vec<String> = names
        .iter()
        .filter(|(n, _)| {
            lookup_intrinsic(n).is_none()
                && !SPECIAL_CASED.contains(&n.as_str())
                && !KNOWN_UNREGISTERED.contains(&n.as_str())
        })
        .map(|(n, f)| format!("{n} ({f})"))
        .collect();
    unexpected.sort();
    unexpected.dedup();

    assert!(
        unexpected.is_empty(),
        "{} new `@vbc` name(s) resolve to nothing and will compile to a silent \
         LoadNil:\n  {}\n\nRegister them, or add them to KNOWN_UNREGISTERED \
         with the module that calls them.",
        unexpected.len(),
        unexpected.join("\n  ")
    );
}

#[test]
fn the_known_unregistered_list_has_no_stale_entries() {
    let fixed: Vec<&str> = KNOWN_UNREGISTERED
        .iter()
        .copied()
        .filter(|n| lookup_intrinsic(n).is_some())
        .collect();
    assert!(
        fixed.is_empty(),
        "{} name(s) are now registered and must be deleted from \
         KNOWN_UNREGISTERED so the ratchet keeps tightening: {:?}",
        fixed.len(),
        fixed
    );

    let mut seen = BTreeSet::new();
    for n in KNOWN_UNREGISTERED {
        assert!(seen.insert(*n), "duplicate entry in KNOWN_UNREGISTERED: {n}");
    }
}

#[test]
fn the_known_unregistered_list_has_no_unused_entries() {
    let mut names = Vec::new();
    collect_vbc_names(&core_dir(), &mut names);
    let used: BTreeSet<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    assert!(!used.is_empty(), "the walk did not run");

    let unused: Vec<&str> = KNOWN_UNREGISTERED
        .iter()
        .copied()
        .filter(|n| !used.contains(n))
        .collect();
    assert!(
        unused.is_empty(),
        "{} name(s) are no longer called from the stdlib and must be deleted \
         from KNOWN_UNREGISTERED: {:?}\n\nThe list tracks live silent-nil call \
         sites; a stale entry overstates the remaining debt and hides the fact \
         that one was retired.",
        unused.len(),
        unused
    );
}
