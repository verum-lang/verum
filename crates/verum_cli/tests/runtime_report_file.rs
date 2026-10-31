//! Observation files bind the exact input before execution begins.
use sha2::{Digest, Sha256};
use verum_cli::runtime_diagnostics::RuntimeReportArgs;
use verum_common::runtime_diagnostics::{RuntimeObservation, RuntimeReport};

#[test]
fn report_binding_survives_actual_script_preparation_and_rejects_source_drift() {
    let dir = tempfile::tempdir().unwrap();
    for (index, source) in [
        "",
        "print(42);",
        "print(42);\n",
        "#!/usr/bin/env verum\nprint(42);",
    ]
    .into_iter()
    .enumerate()
    {
        let path = dir.path().join(format!("{index}.json"));
        let args = RuntimeReportArgs {
            runtime_report: Some(path.clone()),
            runtime_run_id: Some("prepared".into()),
        };
        let prepared =
            verum_cli::commands::file::synthesize_script_temp(source, "report-binding").unwrap();
        let bytes = std::fs::read(prepared.path()).unwrap();
        let request = args.request(source.as_bytes(), true).unwrap().unwrap();
        let report_file = request.open(&bytes).unwrap();
        let report: RuntimeReport = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        report
            .validate_binding("prepared", &hex::encode(Sha256::digest(source.as_bytes())))
            .unwrap();
        assert_eq!(
            report.prepared_source_sha256.as_str(),
            hex::encode(Sha256::digest(&bytes))
        );
        assert_eq!(report.observation, RuntimeObservation::Unavailable);
        assert!(report.cbgr.is_none());
        drop(report_file);
        std::fs::remove_file(&path).unwrap();

        let mut changed = bytes;
        changed.extend_from_slice(b"// changed\n");
        assert!(
            args.request(source.as_bytes(), true)
                .unwrap()
                .unwrap()
                .open(&changed)
                .is_err()
        );
        assert!(
            !path.exists(),
            "source drift must fail before publishing an observation"
        );
    }
}

#[cfg(unix)]
#[test]
fn report_creation_refuses_symlinks_and_keeps_observations_private() {
    use std::os::unix::{fs::PermissionsExt, fs::symlink};
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("retained.json");
    std::fs::write(&target, b"retained observation").unwrap();
    let path = dir.path().join("report.json");
    symlink(&target, &path).unwrap();
    let args = RuntimeReportArgs {
        runtime_report: Some(path.clone()),
        runtime_run_id: Some("private".into()),
    };
    assert!(
        args.request(b"", false)
            .unwrap()
            .unwrap()
            .open(b"")
            .is_err()
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"retained observation");
    std::fs::remove_file(&path).unwrap();
    let _report_file = args
        .request(b"", false)
        .unwrap()
        .unwrap()
        .open(b"")
        .unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
