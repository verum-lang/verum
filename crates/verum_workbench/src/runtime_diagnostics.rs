//! Read observations only from the current operation's bounded report.
use anyhow::{Context, Result, ensure};
use std::{io::Read, path::Path};
use verum_common::{
    List,
    runtime_diagnostics::{MAX_RUNTIME_REPORT_BYTES, RuntimeReport},
};

pub fn read_report(path: &Path, run_id: &str, source_hash: &str) -> Result<RuntimeReport> {
    // O_NOFOLLOW closes the final-component metadata/open race on Unix.
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let meta = std::fs::symlink_metadata(path).context("Runtime report unavailable")?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "Runtime report is not a regular file"
    );
    let file = options.open(path).context("Open runtime report")?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= MAX_RUNTIME_REPORT_BYTES,
        "Runtime report exceeds bounds"
    );
    let mut bytes = List::new();
    let mut reader = file.take(MAX_RUNTIME_REPORT_BYTES + 1);
    let mut chunk = [0u8; 4096];
    loop {
        let length = reader.read(&mut chunk)?;
        if length == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..length]);
    }
    ensure!(
        bytes.len() as u64 <= MAX_RUNTIME_REPORT_BYTES,
        "Runtime report exceeds bounds"
    );
    let report: RuntimeReport =
        serde_json::from_slice(&bytes).context("Malformed runtime report")?;
    report
        .validate_binding(run_id, source_hash)
        .map_err(anyhow::Error::msg)?;
    Ok(report)
}
