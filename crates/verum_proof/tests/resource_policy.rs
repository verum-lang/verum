use verum_proof::{
    admission::{self, Judgment, ReplayOutcome, Report},
    limits::{AdmissionLimits, ReplayLimits},
    proof_checker::{Certificate, Foundation, Term},
};

fn judgment() -> Vec<u8> {
    serde_json::to_vec(&Judgment {
        profile: admission::PROFILE.into(),
        statement: Term::universe(1),
        certificate: Certificate {
            term: Term::universe(0),
            claimed_type: Term::universe(1),
            metadata: Default::default(),
        },
    })
    .unwrap()
}

#[test]
fn exact_byte_node_and_depth_boundaries_do_not_change_the_judgment() {
    let bytes = judgment();
    // Three universe terms and their levels: six visited nodes, depth one.
    let enough = AdmissionLimits {
        max_bytes: bytes.len(),
        max_nodes: 6,
        max_depth: 1,
    };
    for limits in [
        AdmissionLimits {
            max_bytes: bytes.len() - 1,
            ..enough
        },
        AdmissionLimits {
            max_nodes: 5,
            ..enough
        },
        AdmissionLimits {
            max_depth: 0,
            ..enough
        },
    ] {
        let report = admission::replay_with_limits(&bytes, Foundation::Intensional, limits);
        assert_eq!(
            report.outcome,
            ReplayOutcome::ResourceExhausted,
            "{report:?}"
        );
        assert!(!report.accepted);
        assert_eq!(report.limits, limits);
        assert!(report.is_consistent());
        let report = admission::replay_with_limits(&bytes, Foundation::Intensional, enough);
        assert_eq!(report.outcome, ReplayOutcome::Accepted, "{report:?}");
    }
}

#[test]
fn malformed_unsupported_and_exhausted_remain_distinct() {
    let limits = AdmissionLimits::default();
    let malformed = admission::replay_with_limits(b"{]", Foundation::Intensional, limits);
    assert_eq!(malformed.outcome, ReplayOutcome::Rejected);
    let mut data: serde_json::Value = serde_json::from_slice(&judgment()).unwrap();
    data["certificate"]["term"] = serde_json::json!({"Var":0});
    let report = admission::replay(&serde_json::to_vec(&data).unwrap());
    assert_eq!(report.outcome, ReplayOutcome::Rejected);
    assert_eq!(report.diagnostic.unwrap().code, "unbound_variable");
    data["profile"] = "unregistered".into();
    assert_eq!(
        admission::replay(&serde_json::to_vec(&data).unwrap()).outcome,
        ReplayOutcome::Unsupported
    );
    let invalid = AdmissionLimits {
        max_depth: usize::MAX,
        ..limits
    };
    assert_eq!(
        admission::replay_with_limits(&judgment(), Foundation::Intensional, invalid).outcome,
        ReplayOutcome::Inconclusive
    );
}

#[test]
fn proof_metadata_cannot_select_resources_and_report_roundtrips() {
    let mut data: serde_json::Value = serde_json::from_slice(&judgment()).unwrap();
    data["certificate"]["metadata"]["limits"] = "unlimited".into();
    let limits = AdmissionLimits {
        max_nodes: 0,
        ..AdmissionLimits::default()
    };
    let report = admission::replay_with_limits(
        &serde_json::to_vec(&data).unwrap(),
        Foundation::Intensional,
        limits,
    );
    assert_eq!(report.outcome, ReplayOutcome::ResourceExhausted);
    let decoded: Report = serde_json::from_slice(&serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(decoded, report);
    data["limits"] = serde_json::to_value(AdmissionLimits::default()).unwrap();
    assert_eq!(
        admission::replay(&serde_json::to_vec(&data).unwrap()).outcome,
        ReplayOutcome::Rejected
    );
    let limits = ReplayLimits::default();
    assert_eq!(
        serde_json::from_slice::<ReplayLimits>(&serde_json::to_vec(&limits).unwrap()).unwrap(),
        limits
    );
}

#[cfg(feature = "process")]
#[test]
fn worker_installs_and_echoes_selected_policy_and_distinguishes_failures() {
    use std::path::Path;
    use verum_proof::process;
    let exe = Path::new(env!("CARGO_BIN_EXE_verum-proof"));
    let mut limits = ReplayLimits::default();
    limits.admission.max_nodes = 6;
    limits.admission.max_depth = 1;
    limits.process.wall_time_ms = 2000;
    limits.process.cpu_seconds = 2;
    limits.process.memory_bytes /= 2;
    limits.process.stack_bytes /= 2;
    limits.process.output_bytes /= 2;
    let good = process::replay_with_limits(exe, &judgment(), Foundation::Intensional, limits);
    assert_eq!(good.outcome, ReplayOutcome::Accepted, "{good:?}");
    let receipt = good.worker.unwrap();
    assert_eq!(receipt.requested, limits);
    let installed = receipt.installed.unwrap();
    assert!(installed.cpu_seconds <= limits.process.cpu_seconds);
    assert!(installed.stack_bytes <= limits.process.stack_bytes);
    assert!(installed.output_bytes <= limits.process.output_bytes);
    assert!(
        installed
            .address_space_bytes
            .is_none_or(|bytes| bytes <= limits.process.memory_bytes)
    );
    let roundtrip: Report = serde_json::from_slice(&serde_json::to_vec(&good).unwrap()).unwrap();
    assert_eq!(roundtrip, good);
    for admission in [
        AdmissionLimits {
            max_nodes: 5,
            ..limits.admission
        },
        AdmissionLimits {
            max_depth: 0,
            ..limits.admission
        },
        AdmissionLimits {
            max_bytes: judgment().len() - 1,
            ..limits.admission
        },
    ] {
        let limits = ReplayLimits {
            admission,
            ..limits
        };
        let report = process::replay_with_limits(exe, &judgment(), Foundation::Intensional, limits);
        assert_eq!(
            report.outcome,
            ReplayOutcome::ResourceExhausted,
            "{report:?}"
        );
        assert_eq!(report.worker.unwrap().requested, limits);
    }
    let missing = process::replay_with_limits(
        Path::new("/nonexistent/verum-proof"),
        &judgment(),
        Foundation::Intensional,
        limits,
    );
    assert_eq!(missing.outcome, ReplayOutcome::Inconclusive);
    assert_eq!(missing.worker.unwrap().requested, limits);
    limits.process.wall_time_ms = 0;
    let timeout = process::replay_with_limits(exe, &judgment(), Foundation::Intensional, limits);
    assert_eq!(timeout.outcome, ReplayOutcome::ResourceExhausted);
    assert_eq!(timeout.diagnostic.unwrap().code, "wall_limit");
    assert_eq!(timeout.worker.unwrap().requested, limits);
}

#[cfg(feature = "process")]
#[test]
fn cli_reports_resource_exhaustion_with_a_distinct_exit_code() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    for (args, outcome, status) in [
        (
            vec!["--max-nodes", "6", "--max-depth", "1"],
            ReplayOutcome::Accepted,
            0,
        ),
        (
            vec!["--max-nodes", "5"],
            ReplayOutcome::ResourceExhausted,
            3,
        ),
        (
            vec!["--max-depth", "0"],
            ReplayOutcome::ResourceExhausted,
            3,
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_verum-proof"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&judgment()).unwrap();
        let output = child.wait_with_output().unwrap();
        let report: Report = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report.outcome, outcome, "{report:?}");
        assert_eq!(output.status.code(), Some(status));
    }
}

#[test]
fn explicit_policy_admits_a_valid_proof_beyond_the_default_depth() {
    // Unused transparent definitions do not change U0 : U1. This certificate
    // crosses the old fixed depth boundary without expensive normalization.
    let mut term = Term::universe(0);
    for _ in 0..129 {
        term = Term::Let {
            value: Box::new(Term::universe(0)),
            body: Box::new(term),
        };
    }
    let bytes = serde_json::to_vec(&Judgment {
        profile: admission::PROFILE.into(),
        statement: Term::universe(1),
        certificate: Certificate {
            term,
            claimed_type: Term::universe(1),
            metadata: Default::default(),
        },
    })
    .unwrap();
    let default = admission::replay(&bytes);
    assert_eq!(default.outcome, ReplayOutcome::ResourceExhausted);
    assert_eq!(default.diagnostic.unwrap().code, "structural_limit");
    let admission = AdmissionLimits {
        max_depth: 160,
        ..AdmissionLimits::default()
    };
    let expanded = admission::replay_with_limits(&bytes, Foundation::Intensional, admission);
    assert_eq!(expanded.outcome, ReplayOutcome::Accepted, "{expanded:?}");
    #[cfg(feature = "process")]
    {
        let limits = ReplayLimits {
            admission,
            ..ReplayLimits::default()
        };
        let report = verum_proof::process::replay_with_limits(
            std::path::Path::new(env!("CARGO_BIN_EXE_verum-proof")),
            &bytes,
            Foundation::Intensional,
            limits,
        );
        assert_eq!(report.outcome, ReplayOutcome::Accepted, "{report:?}");
        assert_eq!(report.worker.unwrap().requested, limits);
    }
}

#[cfg(feature = "process")]
#[test]
fn explicit_wall_budget_is_not_silently_clamped_to_the_default() {
    let mut limits = ReplayLimits::default();
    limits.process.wall_time_ms += 1;
    let report = verum_proof::process::replay_with_limits(
        std::path::Path::new(env!("CARGO_BIN_EXE_verum-proof")),
        &judgment(),
        Foundation::Intensional,
        limits,
    );
    assert_eq!(report.outcome, ReplayOutcome::Accepted, "{report:?}");
    assert_eq!(
        report.worker.unwrap().requested.process.wall_time_ms,
        limits.process.wall_time_ms
    );
}
