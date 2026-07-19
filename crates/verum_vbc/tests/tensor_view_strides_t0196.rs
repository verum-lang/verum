//! T0196 TENSOR-VIEW-STRIDES-1 — the flat-index element accessors
//! (`get_element_f64` / `set_element_f64`) must honor a view's `offset`
//! and `strides`, not index the flat buffer from the data start.
//!
//! These pins build non-contiguous / offset views by hand (sharing the
//! base tensor's buffer, non-owning) and assert the accessors read and
//! write the *logically*-correct element. Before the fix every view
//! read raw buffer position, so a transpose/slice/flip silently
//! returned the wrong value.

use verum_vbc::dtype::DType;
use verum_vbc::interpreter::tensor::{TensorFlags, TensorHandle};

/// Contiguous row-major base tensor filled with `vals`.
fn tensor_from(vals: &[f64], shape: &[usize]) -> TensorHandle {
    let mut t = TensorHandle::zeros(shape, DType::F64).expect("zeros");
    for (i, v) in vals.iter().enumerate() {
        assert!(t.set_element_f64(i, *v), "set {}", i);
    }
    t
}

/// Builds a NON-OWNING view over `base`'s buffer with the given logical
/// `shape`, `strides` (elements), `offset`, and contiguity. The view
/// lacks `OWNS_DATA`, so its `Drop` never touches the shared refcount —
/// `base` owns and frees the buffer. `base` must outlive the view.
fn view_over(
    base: &TensorHandle,
    shape: &[usize],
    strides: &[isize],
    offset: usize,
    contiguous: bool,
) -> TensorHandle {
    assert_eq!(shape.len(), strides.len());
    let mut v = TensorHandle::new();
    v.dtype = base.dtype;
    v.ndim = shape.len() as u8;
    v.numel = shape.iter().product();
    for (i, (&d, &s)) in shape.iter().zip(strides.iter()).enumerate() {
        v.shape[i] = d;
        v.strides[i] = s;
    }
    v.offset = offset;
    v.data = base.data; // share the buffer (NonNull is Copy)
    v.flags = if contiguous {
        TensorFlags::IS_VIEW | TensorFlags::CONTIGUOUS
    } else {
        TensorFlags::IS_VIEW
    };
    v
}

fn read_all(t: &TensorHandle) -> Vec<f64> {
    (0..t.numel).map(|i| t.get_element_f64(i).unwrap()).collect()
}

#[test]
fn transpose_view_reads_logical_order() {
    // 2x3 row-major: [[0,1,2],[3,4,5]], strides [3,1].
    let base = tensor_from(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], &[2, 3]);
    // Transpose to 3x2: swap shape and strides -> shape [3,2], strides [1,3].
    let t = view_over(&base, &[3, 2], &[1, 3], 0, false);
    // Row-major flat read of the transpose is the transposed order.
    assert_eq!(read_all(&t), vec![0.0, 3.0, 1.0, 4.0, 2.0, 5.0]);
    // The buggy raw-buffer read would have been the untransposed order.
    assert_ne!(read_all(&t), vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
}

#[test]
fn offset_slice_view_reads_from_offset() {
    // Contiguous 1-D [10,11,12,13,14]; slice [2:] -> offset 2, len 3.
    let base = tensor_from(&[10.0, 11.0, 12.0, 13.0, 14.0], &[5]);
    let s = view_over(&base, &[3], &[1], 2, true); // contiguous fast path
    assert_eq!(read_all(&s), vec![12.0, 13.0, 14.0]);
}

#[test]
fn negative_stride_flip_view_reverses() {
    // Flip a 1-D [10,11,12]: strides [-1], base adjusted to last elem
    // (offset 2) so the physical offset stays in-bounds.
    let base = tensor_from(&[10.0, 11.0, 12.0], &[3]);
    let f = view_over(&base, &[3], &[-1], 2, false);
    assert_eq!(read_all(&f), vec![12.0, 11.0, 10.0]);
}

#[test]
fn set_through_transpose_view_lands_at_strided_position() {
    // Zeroed 2x3 base; write column-major via a 3x2 transpose view,
    // then read the base back to confirm the physical cell is correct.
    let mut base = tensor_from(&[0.0; 6], &[2, 3]);
    {
        let mut t = view_over(&base, &[3, 2], &[1, 3], 0, false);
        // Set logical (2,1) of the transpose -> flat logical index 5 ->
        // physical 2*1 + 1*3 = 5 -> base flat 5 (base[1][2]).
        assert!(t.set_element_f64(5, 99.0));
        // Set logical (0,1) -> flat 1 -> physical 0*1 + 1*3 = 3.
        assert!(t.set_element_f64(1, 42.0));
    }
    // Read the base contiguously; physical positions 3 and 5 changed.
    assert_eq!(
        read_all(&base),
        vec![0.0, 0.0, 0.0, 42.0, 0.0, 99.0],
        "writes must land at strided physical positions"
    );
}

#[test]
fn out_of_range_index_rejected_on_view() {
    let base = tensor_from(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], &[2, 3]);
    let mut t = view_over(&base, &[3, 2], &[1, 3], 0, false);
    assert_eq!(t.get_element_f64(6), None);
    assert!(!t.set_element_f64(6, 1.0));
}

#[test]
fn contiguous_zero_offset_unchanged_regression() {
    // The fix must be byte-identical for the common contiguous,
    // zero-offset tensor: flat read == fill order.
    let base = tensor_from(&[7.0, 8.0, 9.0, 10.0], &[2, 2]);
    assert_eq!(read_all(&base), vec![7.0, 8.0, 9.0, 10.0]);
}
