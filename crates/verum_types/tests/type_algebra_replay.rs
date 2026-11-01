use std::{path::Path, process::Command};

use verum_types::{
    type_algebra::{
        AtomId, Certificate, ExpressionDag, ExpressionId, Limits, Outcome, Rule, prove,
    },
    type_algebra_replay::{ReplayError, export_lean},
};

fn certificate(dag: &ExpressionDag, a: ExpressionId, b: ExpressionId) -> Certificate {
    let Outcome::Proven(proof) = prove(dag, a, b, Limits::default()) else {
        panic!("expected a lattice derivation");
    };
    proof
}

#[test]
fn export_refuses_stale_queries_and_excessive_output() {
    let mut dag = ExpressionDag::new(8);
    let a = dag.atom(AtomId(17)).unwrap();
    let b = dag.atom(AtomId(18)).unwrap();
    let ab = dag.union(a, b).unwrap();
    let proof = certificate(&dag, a, ab);
    assert!(matches!(
        export_lean(&dag, a, b, &proof, Limits::default(), 100_000),
        Err(ReplayError::InvalidCertificate(_))
    ));
    assert_eq!(
        export_lean(&dag, a, ab, &proof, Limits::default(), 1),
        Err(ReplayError::OutputLimit)
    );
    let text = export_lean(&dag, a, ab, &proof, Limits::default(), 100_000).unwrap();
    assert!(text.contains(".atom 17"));
    assert!(text.contains(".atom 18"));
    assert!(text.contains("theorem verifiedQuery"));
}

fn lean(path: &Path) -> std::process::Output {
    Command::new(std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into()))
        .arg(path)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification/external/lean"))
        .output()
        .expect("Lean must be installed for this explicitly requested independent replay")
}

#[test]
#[ignore = "requires the pinned Lean toolchain; run explicitly with --ignored"]
fn independent_lean_replays_every_rule_and_rejects_changed_conclusions() {
    let mut dag = ExpressionDag::new(32);
    let a = dag.atom(AtomId(17)).unwrap();
    let b = dag.atom(AtomId(18)).unwrap();
    let bottom = dag.bottom().unwrap();
    let top = dag.top().unwrap();
    let ab = dag.union(a, b).unwrap();
    let ba = dag.union(b, a).unwrap();
    let meet = dag.intersection(a, b).unwrap();
    let top_ab = dag.intersection(top, ab).unwrap();
    let bottom_ba = dag.union(bottom, ba).unwrap();
    let meet_unions = dag.intersection(ba, ab).unwrap();
    let cases = [
        (a, ab),
        (meet, a),
        (ab, ba),
        (bottom, a),
        (a, top),
        (top_ab, ba),
        (ab, bottom_ba),
        (ab, meet_unions),
    ];
    let directory = tempfile::tempdir().unwrap();
    let mut rules = [false; 9];
    for (index, (source, target)) in cases.into_iter().enumerate() {
        let proof = certificate(&dag, source, target);
        for step in &proof.steps {
            rules[match step.rule {
                Rule::Identity(_) => 0,
                Rule::BottomLeft(_) => 1,
                Rule::TopRight(_) => 2,
                Rule::IntersectionLeft(..) => 3,
                Rule::UnionRight(..) => 4,
                Rule::TopLeft(..) => 5,
                Rule::BottomRight(..) => 6,
                Rule::UnionLeft(..) => 7,
                Rule::IntersectionRight(..) => 8,
            }] = true;
        }
        let source_text =
            export_lean(&dag, source, target, &proof, Limits::default(), 1_000_000).unwrap();
        let path = directory.path().join(format!("Replay{index}.lean"));
        // Inline the canonical module in this isolated fixture so no stale
        // compiled Lean library can satisfy the import during the test.
        let source_text = verum_common::Text::from(format!(
            "{}\n{}",
            include_str!(
                "../../../verification/external/lean/VerumExternalReplay/TypeAlgebra.lean"
            ),
            source_text
                .strip_prefix("import VerumExternalReplay.TypeAlgebra\n")
                .unwrap()
        ));
        std::fs::write(&path, source_text.as_bytes()).unwrap();
        let result = lean(&path);
        let stdout = std::string::String::from_utf8_lossy(&result.stdout);
        let stderr = std::string::String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success(), "{index}: {stdout}\n{stderr}");
        assert!(
            stdout.contains("Verum.TypeAlgebraReplay.verifiedQuery"),
            "{stdout}"
        );
        for forbidden in ["sorryAx", "Lean.ofReduceBool", "warning:", "error:"] {
            assert!(
                !stdout.contains(forbidden) && !stderr.contains(forbidden),
                "{stdout}\n{stderr}"
            );
        }
        if index == 0 {
            // Change only the final theorem. Lean must reject A -> B even
            // though every original proof step and its native receipt remain.
            let corrupted = source_text.replace(
                &format!(
                    "denote valuation e{} → denote valuation e{}",
                    source.0, target.0
                ),
                &format!("denote valuation e{} → denote valuation e{}", source.0, b.0),
            );
            std::fs::write(&path, corrupted.as_bytes()).unwrap();
            let rejected = lean(&path);
            assert!(
                !rejected.status.success(),
                "Lean accepted a changed conclusion"
            );
            let diagnostic = std::string::String::from_utf8_lossy(&rejected.stdout);
            assert!(diagnostic.contains("mismatch"), "{diagnostic}");
        }
    }
    assert!(
        rules.iter().all(|seen| *seen),
        "missing rule coverage: {rules:?}"
    );
}
