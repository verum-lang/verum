//! Audit execution receipts are diagnostics, never proof certificates.
use std::path::Path;

/// Capture only the result of this invocation. Legacy report files are not read:
/// they lack run/input binding and cannot be evidence for the current execution.
pub fn capture<E: std::fmt::Display>(
    run_id: &str,
    gate: &str,
    invoke: impl FnOnce() -> Result<(), E>,
) -> serde_json::Value {
    let result = invoke();
    serde_json::json!({
        "run_id": run_id, "gate": gate,
        "execution_status": if result.is_ok() { "completed" } else { "failed" },
        "error": result.err().map(|e| e.to_string()),
        "proof_status": "no_bound_proof_evidence",
        "load_bearing": false,
    })
}

/// Publish a complete JSON value atomically; I/O failure fails the caller.
pub fn publish(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
    serde_json::to_writer(file.as_file_mut(), value)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
