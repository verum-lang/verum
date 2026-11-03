//! Per-invocation diagnostic output. A fresh file and caller nonce prevent an
//! earlier run's report from being mistaken for this execution's observations.
use crate::error::CliError;
use clap::Args;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
};
use verum_common::{
    Text,
    runtime_diagnostics::{RuntimeExportLoss, RuntimeObservation, RuntimeReport, RuntimeReportSchema},
};
use verum_compiler::Session;

mod transport;

#[derive(Args, Debug, Clone, Default)]
pub struct RuntimeReportArgs {
    /// Write bounded CBGR errors, VBC method measurements and async task outcomes.
    #[arg(long, value_name = "FILE", requires = "interp")]
    pub runtime_report: Option<PathBuf>,
    /// Invocation identity returned in the runtime report (generated if omitted).
    #[arg(long, requires = "runtime_report", value_name = "ID")]
    pub runtime_run_id: Option<String>,
}

pub struct ReportRequest {
    path: PathBuf,
    run_id: Text,
    source_sha256: Text,
    wrapper_lines: u32,
    appended_newline: bool,
}

impl RuntimeReportArgs {
    pub fn request(
        &self,
        source: &[u8],
        synthesize_script: bool,
    ) -> Result<Option<ReportRequest>, CliError> {
        let Some(path) = &self.runtime_report else {
            return Ok(None);
        };
        let run_id = self
            .runtime_run_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if run_id.is_empty()
            || run_id.len() > 128
            || !run_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        {
            return Err(CliError::InvalidArgument(
                "Runtime run ID must contain 1..128 ASCII letters, digits, -, _ or .".into(),
            ));
        }
        Ok(Some(ReportRequest {
            path: path.clone(),
            run_id: run_id.into(),
            source_sha256: hex::encode(Sha256::digest(source)).into(),
            wrapper_lines: u32::from(synthesize_script && !source.starts_with(b"#!")),
            appended_newline: synthesize_script && !source.ends_with(b"\n"),
        }))
    }
}

pub struct ReportFile {
    file: File,
    request: ReportRequest,
    prepared_source_sha256: Text,
}

impl ReportRequest {
    pub fn open(self, source: &[u8]) -> Result<ReportFile, CliError> {
        let requested = if self.wrapper_lines == 1 {
            source
                .splitn(2, |byte| *byte == b'\n')
                .nth(1)
                .ok_or_else(|| CliError::Custom("Missing script wrapper".into()))?
        } else {
            source
        };
        let requested = if self.appended_newline {
            requested
                .strip_suffix(b"\n")
                .ok_or_else(|| CliError::Custom("Missing script newline".into()))?
        } else {
            requested
        };
        if hex::encode(Sha256::digest(requested)) != self.source_sha256.as_str() {
            return Err(CliError::Custom(
                "Source changed before runtime report capture".into(),
            ));
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&self.path)
            .map_err(|error| CliError::Custom(format!("Create fresh runtime report: {error}")))?;
        let mut report = ReportFile {
            file,
            request: self,
            prepared_source_sha256: hex::encode(Sha256::digest(source)).into(),
        };
        report.write(None, None)?;
        Ok(report)
    }
}

impl ReportFile {
    pub fn finish(&mut self, session: &Session) -> Result<(), CliError> {
        let entry_source_path = session.all_sources().iter().find_map(|source| {
            let path = source.path.as_ref()?;
            if path != &session.options().input
                || hex::encode(Sha256::digest(source.source.as_bytes()))
                    != self.prepared_source_sha256.as_str()
            {
                return None;
            }
            let path = path.to_str()?;
            (path.len() <= 2048).then(|| Text::from(path))
        });
        self.write(session.take_runtime_measurements(), entry_source_path)
    }

    fn write(
        &mut self,
        report: Option<verum_compiler::session::RuntimeMeasurements>,
        entry_source_path: Option<Text>,
    ) -> Result<(), CliError> {
        let (cbgr, method_dispatch, async_tasks) = match report {
            Some(report) => (Some(report.cbgr), Some(report.method_dispatch), Some(report.async_tasks)),
            None => (None, None, None),
        };
        let body = RuntimeReport {
            schema: RuntimeReportSchema::V2,
            export_loss: Some(RuntimeExportLoss::default()),
            run_id: self.request.run_id.as_str().into(),
            source_sha256: self.request.source_sha256.as_str().into(),
            prepared_source_sha256: self.prepared_source_sha256.as_str().into(),
            wrapper_lines: self.request.wrapper_lines,
            entry_source_path,
            compiler_version: env!("CARGO_PKG_VERSION").into(),
            observation: if cbgr.is_some() {
                RuntimeObservation::Captured
            } else {
                RuntimeObservation::Unavailable
            },
            cbgr,
            method_dispatch,
            async_tasks,
        };
        let bytes = transport::encode_bounded(body).map_err(CliError::Custom)?;
        // A killed writer leaves an incomplete file, which consumers reject.
        // No old report is reused: creation is exclusive for each invocation.
        self.file
            .seek(SeekFrom::Start(0))
            .and_then(|_| self.file.set_len(0))
            .and_then(|_| self.file.write_all(&bytes))
            .and_then(|_| self.file.sync_all())
            .map_err(|error| CliError::Custom(format!("Write runtime report: {error}")))
    }
}
