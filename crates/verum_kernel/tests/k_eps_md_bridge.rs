//! ε-invariant ↔ md-ω bridge tests ().
//!
//! Per defect 3: the Actic ε-invariant ordinal
//! arithmetic (Diakrisis 12-actic/03-epsilon-invariant.md) and
//! the kernel's Cantor-normal-form OrdinalDepth (used by
//! `m_depth_omega` for K-Refine-omega) are different
//! arithmetics. `convert_eps_to_md_omega` is the canonical
//! order-preserving embedding from the former into the latter.

use verum_kernel::{EpsInvariant, OrdinalDepth, convert_eps_to_md_omega};

#[test]
fn zero_maps_to_finite_zero() {
    assert_eq!(
        convert_eps_to_md_omega(&EpsInvariant::Zero),
        OrdinalDepth::finite(0),
    );
}

#[test]
fn finite_n_preserves_value() {
    assert_eq!(
        convert_eps_to_md_omega(&EpsInvariant::Finite(0)),
        OrdinalDepth::finite(0),
    );
    assert_eq!(
        convert_eps_to_md_omega(&EpsInvariant::Finite(1)),
        OrdinalDepth::finite(1),
    );
    assert_eq!(
        convert_eps_to_md_omega(&EpsInvariant::Finite(42)),
        OrdinalDepth::finite(42),
    );
}

#[test]
fn omega_maps_to_kernel_omega() {
    assert_eq!(
        convert_eps_to_md_omega(&EpsInvariant::Omega),
        OrdinalDepth::omega(),
    );
}

#[test]
fn omega_plus_n_maps_to_one_omega_plus_n() {
    assert_eq!(
        convert_eps_to_md_omega(&EpsInvariant::OmegaPlus(1)),
        OrdinalDepth { omega_coeff: 1, finite_offset: 1 },
    );
    assert_eq!(
        convert_eps_to_md_omega(&EpsInvariant::OmegaPlus(7)),
        OrdinalDepth { omega_coeff: 1, finite_offset: 7 },
    );
}

#[test]
fn omega_times_n_maps_to_n_omega() {
    assert_eq!(
        convert_eps_to_md_omega(&EpsInvariant::OmegaTimes {
            coeff: 2,
            offset: 0,
        }),
        OrdinalDepth { omega_coeff: 2, finite_offset: 0 },
    );
    assert_eq!(
        convert_eps_to_md_omega(&EpsInvariant::OmegaTimes {
            coeff: 3,
            offset: 5,
        }),
        OrdinalDepth { omega_coeff: 3, finite_offset: 5 },
    );
}

#[test]
fn omega_zero_via_omega_times_equals_omega_zero() {
    // ε_omega and ε_omega_times{coeff=1, offset=0} both encode ω.
    let direct = convert_eps_to_md_omega(&EpsInvariant::Omega);
    let via_times = convert_eps_to_md_omega(&EpsInvariant::OmegaTimes {
        coeff: 1,
        offset: 0,
    });
    assert_eq!(direct, via_times);
}

#[test]
fn omega_plus_zero_equals_omega() {
    // ε_omega and ε_omega_plus(0) both encode ω.
    let direct = convert_eps_to_md_omega(&EpsInvariant::Omega);
    let via_plus = convert_eps_to_md_omega(&EpsInvariant::OmegaPlus(0));
    assert_eq!(direct, via_plus);
}

// =============================================================================
// Monotonicity: Actic ε-order implies kernel lex-order.
// =============================================================================

#[test]
fn finite_zero_lt_finite_one() {
    let a = convert_eps_to_md_omega(&EpsInvariant::Finite(0));
    let b = convert_eps_to_md_omega(&EpsInvariant::Finite(1));
    assert!(a.lt(&b));
}

#[test]
fn finite_n_lt_omega() {
    for n in [0u32, 1, 5, 100, u32::MAX - 1] {
        let a = convert_eps_to_md_omega(&EpsInvariant::Finite(n));
        let b = convert_eps_to_md_omega(&EpsInvariant::Omega);
        assert!(
            a.lt(&b),
            "Finite({}) must be lt Omega in lex ordering",
            n,
        );
    }
}

#[test]
fn omega_lt_omega_plus_one() {
    let a = convert_eps_to_md_omega(&EpsInvariant::Omega);
    let b = convert_eps_to_md_omega(&EpsInvariant::OmegaPlus(1));
    assert!(a.lt(&b));
}

#[test]
fn omega_plus_n_lt_omega_times_two() {
    let a = convert_eps_to_md_omega(&EpsInvariant::OmegaPlus(99));
    let b = convert_eps_to_md_omega(&EpsInvariant::OmegaTimes {
        coeff: 2,
        offset: 0,
    });
    assert!(a.lt(&b));
}

#[test]
fn omega_times_n_lt_omega_times_n_plus_one() {
    let a = convert_eps_to_md_omega(&EpsInvariant::OmegaTimes {
        coeff: 2,
        offset: 99,
    });
    let b = convert_eps_to_md_omega(&EpsInvariant::OmegaTimes {
        coeff: 3,
        offset: 0,
    });
    assert!(a.lt(&b));
}

#[test]
fn zero_is_canonical_minimum() {
    let zero = convert_eps_to_md_omega(&EpsInvariant::Zero);
    for eps in [
        EpsInvariant::Finite(1),
        EpsInvariant::Omega,
        EpsInvariant::OmegaPlus(0),
        EpsInvariant::OmegaTimes { coeff: 1, offset: 0 },
        EpsInvariant::OmegaTimes { coeff: u32::MAX, offset: u32::MAX },
    ] {
        let v = convert_eps_to_md_omega(&eps);
        assert!(
            zero == v || zero.lt(&v),
            "Zero must be ≤ {:?}, got {:?} vs {:?}",
            eps,
            zero,
            v,
        );
    }
}
