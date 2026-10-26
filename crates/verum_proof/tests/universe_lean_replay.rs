//! Compact universe arithmetic must have the same meaning in both checkers.
use std::process::Command;
use verum_proof::proof_checker::{Certificate, Level, Term};

fn offset(base: Level, by: u32) -> Level {
    Level::Offset(Box::new(base), by)
}
fn maximum(a: Level, b: Level) -> Level {
    Level::Max(Box::new(a), Box::new(b))
}
fn successor(level: Level) -> Level {
    Level::Succ(Box::new(level))
}
fn cases() -> Vec<(&'static str, Certificate, bool)> {
    let u = Level::var("u");
    let v = Level::var("v");
    let big = u32::MAX;
    let pairs = [
        ("zero-offset", offset(u.clone(), 0), u.clone(), true),
        (
            "successor",
            offset(u.clone(), 1),
            successor(u.clone()),
            true,
        ),
        (
            "small-composition",
            offset(offset(u.clone(), 2), 3),
            offset(u.clone(), 5),
            true,
        ),
        (
            "large-offset",
            offset(u.clone(), big),
            offset(u.clone(), big),
            true,
        ),
        (
            "common-large-prefix",
            maximum(offset(u.clone(), big), offset(v.clone(), big)),
            offset(maximum(u.clone(), v.clone()), big),
            true,
        ),
        (
            "nested-wide-prefix",
            maximum(
                offset(offset(u.clone(), big), big),
                offset(offset(v.clone(), big), big),
            ),
            offset(offset(maximum(u.clone(), v.clone()), big), big),
            true,
        ),
        (
            "concrete-prefix",
            maximum(Level::Concrete(big), offset(u.clone(), big)),
            offset(u.clone(), big),
            true,
        ),
        (
            "closed-beyond-wire-width",
            offset(Level::Concrete(big), 1),
            successor(Level::Concrete(big)),
            true,
        ),
        (
            "concrete-boundary",
            offset(Level::Concrete(big - 3), 1),
            Level::Concrete(big - 2),
            true,
        ),
        (
            "max-order",
            maximum(maximum(v.clone(), u.clone()), Level::var("w")),
            maximum(Level::var("w"), maximum(u.clone(), v.clone())),
            true,
        ),
        (
            "wrong-large-offset",
            offset(u.clone(), big),
            offset(u.clone(), big - 1),
            false,
        ),
        ("wrong-variable", offset(u.clone(), 2), offset(v, 2), false),
        // Native equality is deliberately conservative on symbolic dominance.
        (
            "no-invented-dominance",
            maximum(u.clone(), successor(u.clone())),
            successor(u),
            false,
        ),
        (
            "concrete-overflow",
            offset(Level::Concrete(big - 1), 1),
            Level::Concrete(big),
            false,
        ),
    ];
    pairs
        .into_iter()
        .map(|(id, left, right, accepted)| {
            let domain = Term::universe_at(right);
            (
                id,
                Certificate {
                    term: Term::lam(Term::universe_at(left), Term::var(0)),
                    claimed_type: Term::pi(domain.clone(), domain),
                    metadata: Default::default(),
                },
                accepted,
            )
        })
        .collect()
}

#[test]
fn native_compact_universe_controls() {
    for (id, certificate, accepted) in cases() {
        assert_eq!(certificate.verify().is_ok(), accepted, "substitution: {id}");
        assert_eq!(
            verum_proof::proof_checker_nbe::verify_certificate(&certificate).is_ok(),
            accepted,
            "NbE: {id}"
        );
    }
}

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the independent checker"]
fn independent_compact_universes_and_wire_bounds() {
    let replay = std::env::var_os("VERUM_LEAN_REPLAY").expect("build verum_replay_checker first");
    let cases = cases();
    let rows: Vec<_> = cases.iter().map(|(id, certificate, _)| {
        serde_json::json!({"id":id,"term":certificate.term,"claimed_type":certificate.claimed_type})
    }).collect();
    let path =
        std::env::temp_dir().join(format!("verum-universe-replay-{}.json", std::process::id()));
    let battery = serde_json::json!({"schema_version":1,"certificates":rows});
    std::fs::write(&path, serde_json::to_vec(&battery).unwrap()).unwrap();
    let output = Command::new(&replay).arg(&path).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let decoded: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let verdicts = decoded["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), cases.len());
    for (verdict, (id, _, expected)) in verdicts.iter().zip(cases) {
        assert_eq!(verdict["id"], id);
        assert_eq!(verdict["ok"], expected, "{id}: {verdict}");
    }
    for invalid in [
        serde_json::json!(4294967296_u64),
        serde_json::json!({"Concrete":4294967296_u64}),
        serde_json::json!({"Offset":[{"Var":"u"},4294967296_u64]}),
        serde_json::json!({"Offset":[{"Var":"u"},-1]}),
        serde_json::json!({"Offset":[{"Var":"u"},2,3]}),
        serde_json::json!({"Var":"u","Concrete":0}),
    ] {
        assert!(serde_json::from_value::<Level>(invalid.clone()).is_err());
        let mut malformed = battery.clone();
        malformed["certificates"][0]["term"] = serde_json::json!({"Universe":invalid});
        std::fs::write(&path, serde_json::to_vec(&malformed).unwrap()).unwrap();
        let rejected = Command::new(&replay).arg(&path).output().unwrap();
        assert_eq!(rejected.status.code(), Some(4), "invalid level: {invalid}");
    }
    std::fs::remove_file(path).unwrap();
}
