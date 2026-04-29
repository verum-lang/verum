//! AOT lowering coverage guard.
//!
//! Locks in the #77 / #80–#88 audit: every `Instruction::*` variant
//! defined in `crates/verum_vbc/src/instruction.rs` must have a
//! corresponding match arm in `crates/verum_codegen/src/llvm/instruction.rs`.
//!
//! Approach: scan both files for variant names and diff. Failure
//! means a new VBC opcode was added without an LLVM lowering — the
//! programmer should either add the arm or, if the opcode is genuinely
//! unreachable at AOT, mark it via the explicit allow-list below.
//!
//! The check is intentionally text-grep based rather than reflective:
//!
//!   * It catches additions that haven't been wired in yet, even if
//!     the match arm exists with a stale signature.
//!   * It runs in `cargo test` without LLVM context setup or any of
//!     the inkwell scaffolding needed by the integration tests.
//!   * It complements (does NOT replace) the runtime catch-all that
//!     errors with `Unimplemented VBC instruction in AOT lowering`
//!     — that path remains the last line of defence at runtime.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

/// Variants present in the VBC instruction definition but exempt from
/// requiring a Tier-1 lowering arm. Document the reason inline so the
/// next person to read this list can decide whether to remove an entry.
const ALLOW_LIST: &[&str] = &[
    // PermissionCheckWire — historical name retained in the
    // Instruction enum for forward-compat; no live emitter, no
    // dispatch handler. Kept here while the variant is awaiting
    // either deletion or a documented use case.
    "PermissionCheckWire",
];

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/verum_codegen → crates → workspace root
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("workspace root")
}

fn read_file(path: PathBuf) -> String {
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {}", path.display(), e))
}

/// Extract `Instruction::<Name>` variant names from the VBC enum
/// definition. The enum lives inside `pub enum Instruction { ... }`
/// and each variant is a brace-form `Name { ... }` (we ignore tuple
/// variants — there are none currently).
fn parse_vbc_variants(src: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let mut in_instruction_enum = false;
    // depth = 0 outside the enum, 1 just inside the enum body, 2 inside
    // a variant body, 3+ deeper (e.g. nested generics in field types).
    let mut depth = 0_i32;
    for line in src.lines() {
        let trimmed = line.trim_start();
        if !in_instruction_enum {
            if trimmed.starts_with("pub enum Instruction") {
                in_instruction_enum = true;
                // The `{` on this line will be consumed below, taking
                // us from 0 → 1 (just-inside the enum body).
            } else {
                continue;
            }
        }
        let depth_at_line_start = depth;
        for ch in line.chars() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 && in_instruction_enum {
                        return set;
                    }
                }
                _ => {}
            }
        }
        // Variant declarations live at depth 1 — i.e. the line started
        // at depth 1 (just-inside the enum body, before any '{' of a
        // variant is seen). For brace-form variants the line then opens
        // the variant body; for tuple/unit variants the depth stays at
        // 1.
        if depth_at_line_start == 1 {
            if let Some(name) = leading_pascal(trimmed) {
                if !name.is_empty() {
                    set.insert(name.to_string());
                }
            }
        }
    }
    set
}

/// Extract `Instruction::<Name>` references from the LLVM lowering
/// match arms. We intentionally use the qualified form so doc-comment
/// PascalCase words don't accidentally match.
fn parse_lowering_arms(src: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let needle = "Instruction::";
    let mut start = 0;
    while let Some(idx) = src[start..].find(needle) {
        let pos = start + idx + needle.len();
        let rest = &src[pos..];
        let end = rest
            .char_indices()
            .find(|(_, c)| !c.is_ascii_alphanumeric() && *c != '_')
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if !name.is_empty()
            && name.chars().next().unwrap().is_ascii_uppercase()
        {
            set.insert(name.to_string());
        }
        start = pos + end;
    }
    set
}

/// True if the leading token of a line is a PascalCase identifier
/// followed by `{`, `(`, or `,` (variant declaration shape).
fn leading_pascal(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    if !bytes[0].is_ascii_uppercase() {
        return None;
    }
    let end = line
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphanumeric() && *c != '_')
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    let tail = line[end..].trim_start();
    if tail.starts_with('{') || tail.starts_with('(') || tail.starts_with(',') {
        Some(&line[..end])
    } else {
        None
    }
}

#[test]
fn aot_lowering_covers_every_vbc_variant() {
    let root = workspace_root();
    let vbc = read_file(root.join("crates/verum_vbc/src/instruction.rs"));
    let llvm = read_file(root.join("crates/verum_codegen/src/llvm/instruction.rs"));

    let variants = parse_vbc_variants(&vbc);
    let arms = parse_lowering_arms(&llvm);

    let allow: BTreeSet<String> = ALLOW_LIST.iter().map(|s| s.to_string()).collect();

    let missing: Vec<&String> = variants
        .iter()
        .filter(|v| !arms.contains(*v) && !allow.contains(*v))
        .collect();

    if !missing.is_empty() {
        let summary: String = missing
            .iter()
            .map(|s| format!("  - Instruction::{s}"))
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "{} VBC opcode(s) lack an AOT lowering arm:\n{summary}\n\n\
             Either add a `Instruction::<Name> {{ ... }}` arm to \
             `crates/verum_codegen/src/llvm/instruction.rs` (function \
             `lower_instruction`), or — if the opcode is genuinely \
             unreachable at Tier-1 — add it to ALLOW_LIST in \
             `crates/verum_codegen/tests/aot_lowering_coverage.rs` with \
             a one-line comment explaining why.",
            missing.len()
        );
    }
}

#[test]
fn parser_finds_known_variants() {
    // Sanity: the parsers themselves work on a known-good case.
    let sample = "pub enum Instruction {\n    Mov { dst: Reg, src: Reg },\n    LoadK { dst: Reg, const_id: u32 },\n}\n";
    let variants = parse_vbc_variants(sample);
    assert!(variants.contains("Mov"));
    assert!(variants.contains("LoadK"));
    assert_eq!(variants.len(), 2);

    let arms = parse_lowering_arms("match x { Instruction::Mov { dst, src } => {} Instruction::LoadK { dst, const_id } => {} _ => {} }");
    assert!(arms.contains("Mov"));
    assert!(arms.contains("LoadK"));
}
