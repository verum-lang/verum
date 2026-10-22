#![cfg(feature = "process")]
use std::{
    path::Path,
    time::{Duration, Instant},
};
use verum_proof::{
    admission::{Judgment, PROFILE},
    process,
    proof_checker::{Certificate, Level, Term},
};
fn judgment() -> Vec<u8> {
    let ty = Term::Pi(
        Box::new(Term::Universe(Level::Concrete(0))),
        Box::new(Term::Pi(Box::new(Term::Var(0)), Box::new(Term::Var(1)))),
    );
    serde_json::to_vec(&Judgment {
        profile: PROFILE.into(),
        statement: ty.clone(),
        certificate: Certificate {
            term: Term::Lam(
                Box::new(Term::Universe(Level::Concrete(0))),
                Box::new(Term::Lam(Box::new(Term::Var(0)), Box::new(Term::Var(0)))),
            ),
            claimed_type: ty,
            metadata: Default::default(),
        },
    })
    .unwrap()
}
#[test]
fn actual_worker_replays_under_limits_and_rejects_bad_terms() {
    let exe = Path::new(env!("CARGO_BIN_EXE_verum-proof"));
    let report = process::replay(exe, &judgment());
    assert!(report.accepted, "{report:?}");
    let report = process::replay(exe, b"{\"accepted\":true}");
    assert!(!report.accepted);
    assert_eq!(report.diagnostic.unwrap().code, "invalid_certificate");
}
#[test]
fn missing_worker_and_exhausted_budget_do_not_become_proof_rejection() {
    let report = process::replay(Path::new("/nonexistent/verum-proof"), &judgment());
    assert_eq!(report.diagnostic.unwrap().code, "worker_unavailable");
    let start = Instant::now();
    let report = process::replay_with_deadline(
        Path::new(env!("CARGO_BIN_EXE_verum-proof")),
        &judgment(),
        Duration::ZERO,
    );
    assert!(!report.accepted);
    assert_eq!(report.diagnostic.unwrap().code, "wall_limit");
    assert!(start.elapsed() < Duration::from_secs(2));
}
