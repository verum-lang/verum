//! K-Refine-omega modal-depth integration tests (VFE-7 V1, Theorem 136.T).
//!
//! Diakrisis Definition 136.D1: transfinite modal language L^ω_α with
//! ordinal-valued modal-depth `md^ω`. The K-Refine-omega rule extends
//! K-Refine with the second invariant
//!
//!     md^ω(P) < md^ω(A) + 1
//!
//! blocking every modal-paradox witness up to depth κ_2.
//!
//! This file exercises the V1 m_depth_omega computation on the four
//! canonical modal-rank shapes: atomic (md^ω = 0), single-box
//! (md^ω = 1), nested-box (md^ω = 2), and big-and (sup over
//! components).

use verum_common::{Heap, List, Text};
use verum_kernel::{CoreTerm, OrdinalDepth, UniverseLevel, m_depth_omega};

fn tvar(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}

fn box_(phi: CoreTerm) -> CoreTerm {
    CoreTerm::ModalBox(Heap::new(phi))
}

fn diamond(phi: CoreTerm) -> CoreTerm {
    CoreTerm::ModalDiamond(Heap::new(phi))
}

fn big_and(phis: Vec<CoreTerm>) -> CoreTerm {
    let mut args: List<Heap<CoreTerm>> = List::new();
    for p in phis {
        args.push(Heap::new(p));
    }
    CoreTerm::ModalBigAnd(args)
}

#[test]
fn atomic_term_has_md_omega_zero() {
    let phi = tvar("p");
    assert_eq!(m_depth_omega(&phi), OrdinalDepth::finite(0));
}

#[test]
fn universe_term_has_md_omega_zero() {
    let u = CoreTerm::Universe(UniverseLevel::Prop);
    assert_eq!(m_depth_omega(&u), OrdinalDepth::finite(0));
}

#[test]
fn single_box_has_md_omega_one() {
    let phi = box_(tvar("p"));
    assert_eq!(m_depth_omega(&phi), OrdinalDepth::finite(1));
}

#[test]
fn single_diamond_has_md_omega_one() {
    // ◇ has the same md^ω as □ (both bump by 1 per Def 136.D1).
    let phi = diamond(tvar("p"));
    assert_eq!(m_depth_omega(&phi), OrdinalDepth::finite(1));
}

#[test]
fn nested_box_has_md_omega_two() {
    // □□p — modal depth 2.
    let phi = box_(box_(tvar("p")));
    assert_eq!(m_depth_omega(&phi), OrdinalDepth::finite(2));
}

#[test]
fn box_diamond_alternation_has_md_omega_three() {
    // □◇□p — modal depth 3.
    let phi = box_(diamond(box_(tvar("p"))));
    assert_eq!(m_depth_omega(&phi), OrdinalDepth::finite(3));
}

#[test]
fn big_and_takes_supremum_of_components() {
    // ⋀ {p, □p, □□p} has md^ω = 2 (max of 0, 1, 2).
    let phi = big_and(vec![
        tvar("p"),
        box_(tvar("p")),
        box_(box_(tvar("p"))),
    ]);
    assert_eq!(m_depth_omega(&phi), OrdinalDepth::finite(2));
}

#[test]
fn empty_big_and_has_md_omega_zero() {
    // ⋀ ∅ — identity of conjunction, md^ω = 0.
    let phi = big_and(vec![]);
    assert_eq!(m_depth_omega(&phi), OrdinalDepth::finite(0));
}

#[test]
fn box_of_big_and_succ_of_supremum() {
    // □(⋀ {p, □p}) — md^ω = succ(max(0, 1)) = 2.
    let phi = box_(big_and(vec![tvar("p"), box_(tvar("p"))]));
    assert_eq!(m_depth_omega(&phi), OrdinalDepth::finite(2));
}

#[test]
fn ordinal_depth_lex_ordering_holds() {
    // Cantor-normal-form lex ordering: finite(k) < finite(k+1) <
    // ω·1 < ω·1 + k < ω·2.
    assert!(OrdinalDepth::finite(0).lt(&OrdinalDepth::finite(1)));
    assert!(OrdinalDepth::finite(5).lt(&OrdinalDepth::omega()));
    assert!(OrdinalDepth::omega().lt(&OrdinalDepth { omega_coeff: 1, finite_offset: 1 }));
    assert!(OrdinalDepth { omega_coeff: 1, finite_offset: 99 }
        .lt(&OrdinalDepth { omega_coeff: 2, finite_offset: 0 }));
}

#[test]
fn ordinal_depth_succ_increments_finite_component() {
    let d = OrdinalDepth::omega();
    let s = d.succ();
    assert_eq!(s, OrdinalDepth { omega_coeff: 1, finite_offset: 1 });
}
