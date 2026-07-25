//! Pins that the f32 float-classification intrinsics reach the F32 sub-ops.
//!
//! `is_nan_f32`, `is_infinite_f32` and `is_finite_f32` share an
//! `InlineSequenceId` with their f64 twins, so the seq_id alone cannot tell
//! the emitter which width it is lowering. The operand width is the only
//! discriminator, and it survives only if the registry entry carries it via
//! `InlineSequenceWithWidth`. Registered as a plain `InlineSequence`, the
//! emitter instead receives `ptr_elem_stride` — 8 for every non-pointer
//! intrinsic — so an f32 entry silently lowers to the F64 sub-op and the F32
//! interpreter arms become unreachable. That was the state before T0422.
//!
//! This is not a cosmetic distinction for two of the three. Narrowing a
//! finite f64 whose magnitude exceeds f32 range yields infinity, so
//! `is_infinite` and `is_finite` genuinely disagree between the widths:
//! `1e300` is finite as f64 and infinite as f32. Only `is_nan` is
//! width-invariant, because narrowing never manufactures a NaN.

use verum_vbc::intrinsics::registry::{CodegenStrategy, InlineSequenceId, IntrinsicRegistry};

/// The width that marks an f32 operand. A `ptr_elem_stride` is only ever 8
/// (the non-pointer default) or 1 (byte buffers), so 4 is unambiguous.
const F32_WIDTH: u8 = 4;

fn assert_f32_classification(name: &str, expected: InlineSequenceId) {
    let registry = IntrinsicRegistry::new();
    let intrinsic = registry
        .lookup(name)
        .unwrap_or_else(|| panic!("`{name}` is not registered — the F32 sub-op is unreachable"));

    match &intrinsic.strategy {
        CodegenStrategy::InlineSequenceWithWidth(seq_id, width) => {
            let (seq_id, width) = (*seq_id, *width);
            assert_eq!(
                seq_id, expected,
                "`{name}` lowers through the wrong inline sequence"
            );
            assert_eq!(
                width, F32_WIDTH,
                "`{name}` must carry width {F32_WIDTH} so the emitter selects the F32 sub-op; \
                 width {width} would select the F64 one"
            );
        }
        other => panic!(
            "`{name}` must register as InlineSequenceWithWidth(.., {F32_WIDTH}) — a plain \
             InlineSequence hands the emitter ptr_elem_stride (8), which silently selects the \
             F64 sub-op. Got {other:?}"
        ),
    }
}

#[test]
fn is_nan_f32_reaches_the_f32_sub_op() {
    assert_f32_classification("is_nan_f32", InlineSequenceId::IsNan);
}

#[test]
fn is_infinite_f32_reaches_the_f32_sub_op() {
    assert_f32_classification("is_infinite_f32", InlineSequenceId::IsInf);
}

#[test]
fn is_finite_f32_reaches_the_f32_sub_op() {
    assert_f32_classification("is_finite_f32", InlineSequenceId::IsFinite);
}

#[test]
fn f64_classification_stays_width_free() {
    // The f64 entries deliberately do NOT carry a width: they are the default
    // the emitter falls back to. Pinning this keeps the discriminator
    // meaningful — if these ever gained width 4 the f32 selection would fire
    // for f64 operands.
    let registry = IntrinsicRegistry::new();
    for name in ["is_nan_f64", "is_infinite_f64", "is_finite_f64"] {
        let Some(intrinsic) = registry.lookup(name) else {
            continue; // not all three are registered under an _f64 name
        };
        if let CodegenStrategy::InlineSequenceWithWidth(_, width) = intrinsic.strategy {
            assert_ne!(
                width, F32_WIDTH,
                "`{name}` is an f64 intrinsic but carries the f32 width marker"
            );
        }
    }
}
