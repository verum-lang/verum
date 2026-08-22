//! A kernel verdict says how it knows (T0841 follow-up).
//!
//! `holds: true` alone cannot distinguish "this kernel ran the check
//! that decides the question" from "this kernel accepted a published
//! proof's word". Both are legitimate in a proof kernel with a
//! documented trust base — but only if the difference is legible, or
//! the trust base is not documented at all, merely believed.
//!
//! These gates keep the distinction real: a cited verdict must name
//! where its proof lives, and the split between computed and cited
//! must stay visible to anyone counting.

use verum_kernel::intrinsic_dispatch::{
    Evidence, IntrinsicValue, available_intrinsics, dispatch_intrinsic,
};

/// Classify every dispatchable intrinsic that answers without
/// arguments. Argument-taking dispatchers return `None` for an empty
/// argument list, which is correct and not interesting here.
fn nullary_verdicts() -> Vec<(&'static str, bool, Evidence, String)> {
    available_intrinsics()
        .iter()
        .filter_map(|name| match dispatch_intrinsic(name, &[]) {
            Some(IntrinsicValue::Decision {
                holds,
                evidence,
                reason,
            }) => Some((*name, holds, evidence, reason)),
            _ => None,
        })
        .collect()
}

/// A citation must name its source. A cited verdict whose source is
/// empty is indistinguishable from a bare assertion — the precise
/// shape this vocabulary exists to make unwritable.
#[test]
fn every_citation_names_where_the_proof_lives() {
    for (name, _, evidence, _) in nullary_verdicts() {
        if let Evidence::Cited { source } = evidence {
            assert!(
                !source.trim().is_empty(),
                "`{name}` cites a proof without naming it"
            );
            assert!(
                source.len() > 8,
                "`{name}` cites `{source}`, which is too terse to locate the proof by"
            );
        }
    }
}

/// The trust base is legible: some verdicts are computed here and
/// some are taken on citation, and BOTH kinds exist. A kernel where
/// everything claimed to be computed would be hiding its admissions;
/// one where nothing was would have stopped verifying.
#[test]
fn the_trust_base_has_both_kinds_and_is_countable() {
    let verdicts = nullary_verdicts();
    assert!(
        !verdicts.is_empty(),
        "no nullary verdicts found — the classification is measuring nothing"
    );

    let computed = verdicts.iter().filter(|(_, _, e, _)| e.is_computed()).count();
    let cited = verdicts.len() - computed;

    assert!(
        computed > 0,
        "not one verdict is computed — a kernel that only cites is a bibliography"
    );
    assert!(
        cited > 0,
        "not one verdict is cited, yet the kernel admits results from CompCert, \
         Vellvm and the kernel_v0 rule lemmas — an admission recorded as a \
         computation is exactly the dishonesty this vocabulary removes"
    );
}

/// The architectural family is decided here, not cited: every
/// property it claims is one this kernel can execute.
#[test]
fn architectural_verdicts_are_never_cited() {
    for (name, _, evidence, _) in nullary_verdicts() {
        if name.starts_with("kernel_arch_") {
            assert!(
                evidence.is_computed(),
                "`{name}` answered with {evidence:?} — architectural properties are \
                 decidable by the live checkers and must be decided, not cited"
            );
        }
    }
}

/// The tag is stable: audit JSON keys on it.
#[test]
fn evidence_tags_are_stable() {
    assert_eq!(Evidence::Computed.tag(), "computed");
    assert_eq!(
        Evidence::Cited {
            source: "somewhere".to_string()
        }
        .tag(),
        "cited"
    );
}
