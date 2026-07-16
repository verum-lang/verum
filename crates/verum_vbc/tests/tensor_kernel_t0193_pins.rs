//! Kernel-level pins for the T0193/T0199/T0200 tensor campaign —
//! exercise the interpreter tensor kernels directly, below the
//! bytecode wire, so wire defects and kernel defects triage apart.

use verum_vbc::dtype::DType;
use verum_vbc::interpreter::tensor::{
    TensorHandle, tensor_argmin, tensor_broadcast, tensor_det, tensor_norm, tensor_trace,
};

fn tensor_from(vals: &[f64], shape: &[usize]) -> TensorHandle {
    let mut t = TensorHandle::zeros(shape, DType::F64).expect("zeros");
    for (i, v) in vals.iter().enumerate() {
        assert!(t.set_element_f64(i, *v), "set {}", i);
    }
    t
}

#[test]
fn det_of_diagonal_is_product() {
    let d = tensor_from(&[2.0, 0.0, 0.0, 3.0], &[2, 2]);
    assert_eq!(tensor_det(&d), Some(6.0));
}

#[test]
fn trace_of_diagonal_is_sum() {
    let d = tensor_from(&[2.0, 0.0, 0.0, 3.0], &[2, 2]);
    assert_eq!(tensor_trace(&d), Some(5.0));
}

#[test]
fn argmin_finds_smallest() {
    let t = tensor_from(&[3.0, 1.0, 2.0], &[3]);
    let (idx, val) = tensor_argmin(&t, None).expect("argmin");
    assert_eq!(idx, 1);
    assert_eq!(val, 1.0);
}

#[test]
fn l2_norm_pythagorean() {
    let t = tensor_from(&[3.0, 4.0], &[2]);
    let n = tensor_norm(&t, 2).expect("norm");
    assert!((n - 5.0).abs() < 1e-9, "n={}", n);
}

#[test]
fn broadcast_identity_preserves_values() {
    let t = tensor_from(&[1.0, 2.0], &[1, 2]);
    let b = tensor_broadcast(&t, &[1, 2]).expect("identity broadcast");
    assert_eq!(b.get_element_f64(0), Some(1.0));
    assert_eq!(b.get_element_f64(1), Some(2.0));
}

#[test]
fn broadcast_row_tile() {
    let t = tensor_from(&[1.0, 2.0], &[1, 2]);
    let b = tensor_broadcast(&t, &[2, 2]).expect("row tile");
    assert_eq!(b.get_element_f64(0), Some(1.0));
    assert_eq!(b.get_element_f64(1), Some(2.0));
    assert_eq!(b.get_element_f64(2), Some(1.0));
    assert_eq!(b.get_element_f64(3), Some(2.0));
}
