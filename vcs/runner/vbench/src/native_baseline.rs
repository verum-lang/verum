//! End-to-end measurements of two prebuilt executables on one checked workload.
use std::fs::File;
use std::io::{self, Read, Seek};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use tempfile::NamedTempFile;
use vbench::{BenchmarkCategory, BenchmarkReport, BenchmarkResult, ReportMetadata, Statistics};
use wait_timeout::ChildExt;

use super::BaselineArg;

#[derive(Args, Debug)]
pub(super) struct Programs {
    /// Compiled Verum AOT executable.
    #[arg(long)]
    candidate: PathBuf,
    /// Compiled reference executable implementing the same workload.
    #[arg(long)]
    reference: PathBuf,
    /// File supplied on stdin to both executables on every invocation.
    #[arg(long)]
    input: PathBuf,
    /// File containing the expected stdout bytes, including final newlines.
    #[arg(long)]
    expected_stdout: PathBuf,
    /// Expected process exit code, checked together with stdout.
    #[arg(long, default_value_t = 0)]
    expected_exit: i32,
    /// Measured runs per executable after a warm-up pair.
    #[arg(long, default_value = "9")]
    samples: NonZeroU32,
    /// Maximum seconds allowed for each invocation.
    #[arg(long, default_value = "60")]
    timeout_secs: NonZeroU32,
}

impl Programs {
    pub(super) fn measure(&self, language: BaselineArg) -> Result<BenchmarkReport> {
        let paths = [
            self.candidate.canonicalize()?,
            self.reference.canonicalize()?,
        ];
        ensure!(
            paths[0] != paths[1],
            "candidate and reference must be separate executables"
        );
        let mut input = NamedTempFile::new()?;
        io::copy(&mut File::open(&self.input)?, input.as_file_mut())?;
        let expected = std::fs::read(&self.expected_stdout)?;
        let identities = [digest(&paths[0])?, digest(&paths[1])?];
        let mut times = [Vec::new(), Vec::new()];
        for round in 0..=self.samples.get() {
            // Reverse the order on alternate pairs to distribute startup bias.
            let first = (round % 2) as usize;
            for index in [first, 1 - first] {
                let elapsed = self
                    .run_checked(&paths[index], &input, &expected)
                    .with_context(|| format!("checking {}", paths[index].display()))?;
                if round > 0 {
                    times[index].push(elapsed);
                }
            }
        }

        let reference_name = format!("{language:?}");
        let mut rows = Vec::new();
        for (index, name) in ["Verum AOT", &reference_name].into_iter().enumerate() {
            ensure!(
                digest(&paths[index])? == identities[index],
                "executable changed during measurement"
            );
            let stats =
                Statistics::from_durations(&times[index]).context("empty native measurement")?;
            rows.push(
                BenchmarkResult::new(name.into(), BenchmarkCategory::Baseline, stats, None)
                    .with_metadata("executable", paths[index].display().to_string())
                    .with_metadata("executable_blake3", identities[index].to_hex().to_string()),
            );
        }
        let speedup = rows[1].statistics.median_ns / rows[0].statistics.median_ns;
        let title = format!(
            "Verum AOT vs {reference_name}: {speedup:.3}× reference speed (median process time)"
        );
        let metadata = ReportMetadata::new(title, "prebuilt executable")
            .with_tag("input", self.input.display().to_string())
            .with_tag("input_blake3", digest(input.path())?.to_hex().to_string())
            .with_tag(
                "expected_stdout_blake3",
                blake3::hash(&expected).to_hex().to_string(),
            )
            .with_tag(
                "expected_stdout",
                self.expected_stdout.display().to_string(),
            )
            .with_tag("expected_exit", self.expected_exit.to_string());
        Ok(BenchmarkReport::new(metadata, rows, vec![], vec![]))
    }

    fn run_checked(
        &self,
        program: &Path,
        input: &NamedTempFile,
        expected: &[u8],
    ) -> Result<Duration> {
        let mut output = tempfile::tempfile()?;
        let mut command = Command::new(program);
        command
            .stdin(input.reopen()?)
            .stdout(output.try_clone()?)
            .stderr(Stdio::inherit());
        let started = Instant::now();
        let mut child = command.spawn()?;
        let Some(status) =
            child.wait_timeout(Duration::from_secs(self.timeout_secs.get().into()))?
        else {
            let _ = child.kill();
            child.wait()?;
            bail!("execution exceeded {} seconds", self.timeout_secs);
        };
        let elapsed = started.elapsed();
        ensure!(
            status.code() == Some(self.expected_exit),
            "unexpected process status {status}"
        );
        ensure!(
            output.metadata()?.len() == expected.len() as u64,
            "stdout length differs from expected"
        );
        output.rewind()?;
        let mut actual = Vec::with_capacity(expected.len());
        output.read_to_end(&mut actual)?;
        ensure!(actual == expected, "stdout bytes differ from expected");
        Ok(elapsed)
    }
}

fn digest(path: &Path) -> Result<blake3::Hash> {
    Ok(blake3::Hasher::new()
        .update_reader(File::open(path)?)?
        .finalize())
}
