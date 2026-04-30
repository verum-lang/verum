//! Generic Extended (`Opcode::Extended` = `0x1F`) opcode handler.
//!
//! Implements #167 Part A — the general-purpose extension-byte scheme.
//! The dispatcher reads a single sub-op byte, then routes to the
//! sub-op handler.  Sub-op `0x00` is reserved as a forward-compat
//! anchor; encoders must never emit it, decoders accept-and-skip it.
//!
//! Future #167 Part B work (and any later first-class instruction
//! that doesn't fit an existing extension namespace) wires its
//! handler here.

use crate::instruction::{ExtendedSubOpcode, Opcode};
use crate::types::{VariantDescriptor, VariantKind};
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

/// Compute the expected payload-field count for a declared variant.
///
/// `MakeVariantTyped` Phase 3b validation cross-checks the bytecode
/// `field_count` operand against this:
///   - `Unit`   → 0    (no payload),
///   - `Tuple`  → `arity` (positional fields),
///   - `Record` → `fields.len()` (named fields).
///
/// Inlined in the dispatcher's hot path; the match is exhaustive so
/// the codegen folds it to a single load + cmov on most targets.
#[inline]
fn expected_field_count(variant: &VariantDescriptor) -> u32 {
    match variant.kind {
        VariantKind::Unit => 0,
        VariantKind::Tuple => variant.arity as u32,
        VariantKind::Record => variant.fields.len() as u32,
    }
}

/// Pure-function validation for `MakeVariantTyped` operands —
/// extracted so the dispatcher's hot path stays branch-light AND
/// the validator is unit-testable without bytecode-stream
/// construction.
///
/// Returns `Ok(())` on success.  On failure the
/// `InterpreterError::LayoutMismatch` payload distinguishes the
/// three rejection classes via `reason`: "unknown type_id",
/// "unknown tag for type", "field_count mismatch".
///
/// Builtin-range type ids (`is_builtin()`, < 0x100) bypass this
/// validation: they're scalar primitives and never carry a
/// `variants` list — codegen never emits MakeVariantTyped for
/// builtin types, and bytecode that does so will be caught by
/// the unknown-type-id branch on the synthetic ids the legacy
/// `MakeVariant` path uses (`0x8000 + tag`, well above the
/// builtin range).
#[inline]
pub(in super::super) fn validate_make_variant_typed(
    module: &crate::module::VbcModule,
    type_id: crate::types::TypeId,
    tag: u32,
    field_count: u32,
) -> Result<(), InterpreterError> {
    if type_id.is_builtin() {
        return Ok(());
    }
    let desc = match module.get_type(type_id) {
        Some(d) => d,
        None => {
            return Err(InterpreterError::LayoutMismatch {
                type_id,
                tag,
                got_field_count: field_count,
                expected_field_count: None,
                reason: "unknown type_id",
            });
        }
    };
    let variant = match desc.variants.iter().find(|v| v.tag == tag) {
        Some(v) => v,
        None => {
            return Err(InterpreterError::LayoutMismatch {
                type_id,
                tag,
                got_field_count: field_count,
                expected_field_count: None,
                reason: "unknown tag for type",
            });
        }
    };
    let expected = expected_field_count(variant);
    if expected != field_count {
        return Err(InterpreterError::LayoutMismatch {
            type_id,
            tag,
            got_field_count: field_count,
            expected_field_count: Some(expected),
            reason: "field_count mismatch",
        });
    }
    Ok(())
}

/// Dispatcher for `Opcode::Extended` (0x1F) — #167 Part A.
///
/// Format: `[0x1F] [sub_op:u8] [operands...]`.  The sub-op byte
/// selects the extended-instruction kind from a 256-entry secondary
/// space.  An unknown sub-op surfaces `InterpreterError::NotImplemented`
/// with `opcode = Some(Opcode::Extended)` so the caller can identify
/// the extension family.
pub(in super::super) fn handle_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    match ExtendedSubOpcode::from_byte(sub_op_byte) {
        Some(ExtendedSubOpcode::Reserved) => {
            // Forward-compat anchor.  Encoders MUST NOT emit this
            // sub-op; decoders accept it as a no-op so a future
            // extension that lands here can roll out without breaking
            // older interpreters.
            Ok(DispatchResult::Continue)
        }
        Some(ExtendedSubOpcode::MakeVariantTyped) => {
            // Wire format: `reg:dst + varint:type_id + varint:tag +
            // varint:field_count`.
            //
            // Phase 3b — cross-check the (type_id, tag, field_count)
            // tuple against the global type table BEFORE allocating.
            // Three rejection classes mapped to
            // `InterpreterError::LayoutMismatch`:
            //   - type_id has no registered descriptor;
            //   - tag is not a declared variant of that type;
            //   - field_count disagrees with the declared variant's
            //     arity (Unit=0, Tuple=arity, Record=fields.len()).
            // On success: identical heap footprint to legacy
            // `MakeVariant`, with the real sum-type id stored in the
            // heap header so `format_variant_for_print_depth` can
            // resolve the constructor name in O(N_variants_of_type)
            // (vs scanning every type in the module) — and produces
            // the correct name when distinct sum types share variant
            // tags.
            //
            // Builtin-range type ids (`is_builtin()`, < 0x100) bypass
            // the descriptor lookup: they're scalar primitives and
            // never carry a `variants` list.  Codegen never emits
            // MakeVariantTyped for builtin types, so the only way to
            // hit a builtin id here is hand-rolled / fuzzed
            // bytecode — the synthetic `0x8000 + tag` sentinel id
            // emitted by the legacy `MakeVariant` path is ALSO non-
            // builtin (it's > 0x8000 by construction), so it
            // triggers the `unknown type_id` rejection rather than
            // a silent skip.
            //
            // Performance: O(1) operand read; single descriptor
            // lookup (Vec<TypeDescriptor> indexed by id); branchless
            // tag scan over the variant list (typically ≤ 8 entries
            // — Result/Maybe have 2, user enums rarely exceed 8).
            // Single heap allocation matching `MakeVariant`'s
            // footprint.
            let dst_reg = super::bytecode_io::read_reg(state)?;
            let type_id_raw = super::bytecode_io::read_varint(state)? as u32;
            let tag = super::bytecode_io::read_varint(state)? as u32;
            let field_count = super::bytecode_io::read_varint(state)? as u32;
            let type_id = crate::types::TypeId(type_id_raw);

            validate_make_variant_typed(&state.module, type_id, tag, field_count)?;

            super::pattern_matching::alloc_variant_into_with_type_id(
                state,
                dst_reg,
                tag,
                field_count,
                type_id,
            )?;
            Ok(DispatchResult::Continue)
        }
        Some(ExtendedSubOpcode::ProcessExit) => {
            // Format: `[0x1F][0x10][reg:u16]`. Read the register holding
            // the exit code and raise a `ProcessExit` control-flow
            // signal that the outer driver translates into
            // `std::process::exit` after running post-execution work
            // (cache store, timing flush, telemetry). Calling
            // `process::exit` directly here would short-circuit those
            // steps and force every script to re-pay full compile cost
            // on its next invocation.
            //
            // Stdio flush happens at the driver boundary (just before
            // `process::exit`) so partial-line `print(...)` output is
            // not lost regardless of which path produced the exit.
            //
            // Permission gate: process termination is a script-level
            // resource boundary just like FFI _exit / kill / fork. A
            // script declaring `permissions = ["time"]` (no `run`)
            // shouldn't be able to terminate the process — denying
            // here mirrors the FFI-level enforcement in
            // `ffi_extended.rs::check_ffi_permission`. Plain scripts
            // with no permission policy installed pass the check
            // unconditionally (router default is allow-all).
            let code_reg = super::bytecode_io::read_reg(state)?;
            let code = state.get_reg(code_reg).as_integer_compatible() as i32;
            use crate::interpreter::permission::{PermissionDecision, PermissionScope};
            if state.check_permission(PermissionScope::Process, 0)
                == PermissionDecision::Deny
            {
                use std::io::Write;
                let _ = std::io::stdout().flush();
                let _ = std::io::stderr().flush();
                return Err(InterpreterError::Panic {
                    message: format!(
                        "permission denied: exit({code}) requires Process grant"
                    ),
                });
            }
            Err(InterpreterError::ProcessExit(code))
        }
        None => Err(InterpreterError::NotImplemented {
            feature: "Extended sub-opcode",
            opcode: Some(Opcode::Extended),
        }),
    }
}

#[cfg(test)]
mod make_variant_typed_validation_tests {
    use super::*;
    use crate::module::VbcModule;
    use crate::types::{TypeDescriptor, TypeId, TypeKind, VariantDescriptor, VariantKind};
    use smallvec::smallvec;

    /// Build a module carrying one user-defined sum type with two
    /// variants — one Tuple (arity 2, tag=0) and one Unit (tag=1).
    /// Returns the module + the assigned `TypeId` so tests can pin
    /// the validator against a known shape.
    fn module_with_pair_or_nil() -> (VbcModule, TypeId) {
        let mut module = VbcModule::new("phase3b_validation".to_string());
        let name_id = module.intern_string("PairOrNil");
        let descriptor = TypeDescriptor {
            name: name_id,
            kind: TypeKind::Sum,
            variants: smallvec![
                VariantDescriptor {
                    name: name_id,
                    tag: 0,
                    payload: None,
                    kind: VariantKind::Tuple,
                    arity: 2,
                    fields: smallvec![],
                },
                VariantDescriptor {
                    name: name_id,
                    tag: 1,
                    payload: None,
                    kind: VariantKind::Unit,
                    arity: 0,
                    fields: smallvec![],
                },
            ],
            ..Default::default()
        };
        let mut descriptor = descriptor;
        let assigned = TypeId(module.types.len() as u32 + TypeId::FIRST_USER);
        descriptor.id = assigned;
        module.types.push(descriptor);
        (module, assigned)
    }

    #[test]
    fn validate_accepts_known_tuple_variant_with_matching_arity() {
        // Pin: well-formed `(type_id, tag=0, field_count=2)` —
        // matches the Tuple variant's declared arity. Validator
        // returns Ok(()).
        let (module, type_id) = module_with_pair_or_nil();
        validate_make_variant_typed(&module, type_id, 0, 2).expect("valid");
    }

    #[test]
    fn validate_accepts_known_unit_variant_with_zero_fields() {
        // Pin: Unit variants (no payload) are valid only when
        // field_count=0.
        let (module, type_id) = module_with_pair_or_nil();
        validate_make_variant_typed(&module, type_id, 1, 0).expect("valid");
    }

    #[test]
    fn validate_rejects_unknown_type_id() {
        // Pin: type_id has no registered descriptor → reason
        // "unknown type_id" with `expected_field_count: None`.
        let (module, _type_id) = module_with_pair_or_nil();
        let bogus = TypeId(99_999);
        let err = validate_make_variant_typed(&module, bogus, 0, 2).unwrap_err();
        match err {
            InterpreterError::LayoutMismatch {
                type_id,
                expected_field_count,
                reason,
                ..
            } => {
                assert_eq!(type_id, bogus);
                assert_eq!(expected_field_count, None);
                assert_eq!(reason, "unknown type_id");
            }
            other => panic!("expected LayoutMismatch, got {:?}", other),
        }
    }

    #[test]
    fn validate_rejects_unknown_tag_for_known_type() {
        // Pin: type_id is registered but the tag is not in the
        // variant list → reason "unknown tag for type" with
        // `expected_field_count: None`.
        let (module, type_id) = module_with_pair_or_nil();
        let err = validate_make_variant_typed(&module, type_id, 99, 0).unwrap_err();
        match err {
            InterpreterError::LayoutMismatch {
                tag,
                expected_field_count,
                reason,
                ..
            } => {
                assert_eq!(tag, 99);
                assert_eq!(expected_field_count, None);
                assert_eq!(reason, "unknown tag for type");
            }
            other => panic!("expected LayoutMismatch, got {:?}", other),
        }
    }

    #[test]
    fn validate_rejects_field_count_mismatch_tuple_variant() {
        // Pin: type_id + tag are valid but field_count disagrees
        // with the declared arity → reason "field_count mismatch"
        // with `expected_field_count: Some(declared_arity)`.
        let (module, type_id) = module_with_pair_or_nil();
        let err = validate_make_variant_typed(&module, type_id, 0, 5).unwrap_err();
        match err {
            InterpreterError::LayoutMismatch {
                got_field_count,
                expected_field_count,
                reason,
                ..
            } => {
                assert_eq!(got_field_count, 5);
                assert_eq!(expected_field_count, Some(2));
                assert_eq!(reason, "field_count mismatch");
            }
            other => panic!("expected LayoutMismatch, got {:?}", other),
        }
    }

    #[test]
    fn validate_rejects_field_count_mismatch_unit_variant() {
        // Pin: Unit variant declared with arity=0 — supplying
        // any field_count > 0 must trip the same mismatch path.
        let (module, type_id) = module_with_pair_or_nil();
        let err = validate_make_variant_typed(&module, type_id, 1, 1).unwrap_err();
        match err {
            InterpreterError::LayoutMismatch {
                expected_field_count,
                reason,
                ..
            } => {
                assert_eq!(expected_field_count, Some(0));
                assert_eq!(reason, "field_count mismatch");
            }
            other => panic!("expected LayoutMismatch, got {:?}", other),
        }
    }

    #[test]
    fn validate_bypasses_builtin_type_ids() {
        // Pin: builtin-range type ids (Bool, Int, Float, ...) skip
        // descriptor lookup and accept any (tag, field_count) pair.
        // Codegen never emits MakeVariantTyped for builtin types, so
        // this branch is unreachable from clean compilation but
        // prevents a panic on hand-rolled / fuzzed bytecode.
        let module = VbcModule::new("builtin_bypass".to_string());
        validate_make_variant_typed(&module, TypeId::INT, 999, 999).expect("builtin bypass");
        validate_make_variant_typed(&module, TypeId::BOOL, 0, 0).expect("builtin bypass");
    }
}
