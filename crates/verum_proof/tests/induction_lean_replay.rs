#[path = "support/inductives.rs"]
mod fixtures;

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the independent built checker"]
fn independent_lean_replays_exact_induction_certificates_and_mutations() {
    let replay = std::env::var_os("VERUM_LEAN_REPLAY").expect("build verum_replay_checker first");
    let rows = fixtures::battery();
    let certs: Vec<_>=rows.iter().map(|(id,cert,_)| serde_json::json!({"id":id,"term":cert.term,"claimed_type":cert.claimed_type})).collect();
    let path = std::env::temp_dir().join(format!(
        "verum-induction-replay-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"certificates":certs})).unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new(replay)
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let verdicts = result["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), rows.len());
    for ((name, cert, expected), verdict) in rows.iter().zip(verdicts) {
        assert_eq!(verdict["id"], *name);
        assert_eq!(verdict["ok"], *expected, "Lean: {name}: {verdict}");
        assert_eq!(cert.verify().is_ok(), *expected, "Rust: {name}");
        assert_eq!(
            verum_proof::proof_checker_nbe::verify_certificate(cert).is_ok(),
            *expected,
            "NbE: {name}"
        );
    }
}
