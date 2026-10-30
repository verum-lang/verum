//! An isolated host worker for untrusted native proof terms.
//! Executable, arguments and limits are verifier configuration, never proof input.
use crate::admission::{Diagnostic, ReplayOutcome, Report, checker_identity, profile_for};
use crate::limits::{InstalledLimits, ProcessLimits, ReplayLimits, WorkerReceipt};
use crate::proof_checker::Foundation;
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub const WORKER_ARG: &str = "--internal-native-proof-worker";
pub const EXTENSIONAL_WORKER_ARG: &str = "--internal-native-funext-proof-worker";
pub const POLICY_ARG: &str = "--resource-policy";
pub const WALL_LIMIT: Duration = Duration::from_millis(ProcessLimits::DEFAULT.wall_time_ms);
pub const CPU_SECONDS: u64 = ProcessLimits::DEFAULT.cpu_seconds;
pub const MEMORY_BYTES: u64 = ProcessLimits::DEFAULT.memory_bytes;
pub const OUTPUT_BYTES: u64 = ProcessLimits::DEFAULT.output_bytes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerFailure {
    pub outcome: ReplayOutcome,
    pub diagnostic: Diagnostic,
}

fn failure(outcome: ReplayOutcome, code: &str, message: &str) -> WorkerFailure {
    WorkerFailure {
        outcome,
        diagnostic: Diagnostic {
            code: code.into(),
            stage: "process".into(),
            message: message.into(),
        },
    }
}

fn inconclusive(code: &str, message: &str) -> WorkerFailure {
    failure(ReplayOutcome::Inconclusive, code, message)
}

fn exhausted(code: &str, message: &str) -> WorkerFailure {
    failure(ReplayOutcome::ResourceExhausted, code, message)
}

fn report_failure(foundation: Foundation, limits: ReplayLimits, failure: WorkerFailure) -> Report {
    let mut report = Report::refusal(
        foundation,
        limits.admission,
        failure.outcome,
        &failure.diagnostic.stage,
        &failure.diagnostic.code,
        &failure.diagnostic.message,
    );
    report.worker = Some(WorkerReceipt {
        requested: limits,
        installed: None,
    });
    report
}

/// Compute the actual inherited OS ceilings. The same calculation runs in the
/// parent before launch and in the child before parsing; the receipt must match.
#[cfg(unix)]
fn effective_limits(limits: ProcessLimits) -> std::io::Result<InstalledLimits> {
    use nix::sys::resource::{Resource, getrlimit};
    let bound = |resource: Resource, requested: u64| -> std::io::Result<u64> {
        let (soft, hard) = getrlimit(resource).map_err(std::io::Error::from)?;
        Ok(requested.min(hard).min(soft))
    };
    Ok(InstalledLimits {
        cpu_seconds: bound(Resource::RLIMIT_CPU, limits.cpu_seconds)?,
        #[cfg(not(target_os = "macos"))]
        address_space_bytes: Some(bound(Resource::RLIMIT_AS, limits.memory_bytes)?),
        #[cfg(target_os = "macos")]
        address_space_bytes: None,
        stack_bytes: bound(Resource::RLIMIT_STACK, limits.stack_bytes)?,
        output_bytes: bound(Resource::RLIMIT_FSIZE, limits.output_bytes)?,
    })
}

#[cfg(not(unix))]
fn effective_limits(_limits: ProcessLimits) -> std::io::Result<InstalledLimits> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "native proof worker needs supported process limits",
    ))
}

#[cfg(unix)]
fn install_limits(limits: ProcessLimits) -> std::io::Result<InstalledLimits> {
    use nix::sys::resource::{Resource, setrlimit};
    let installed = effective_limits(limits)?;
    let mut resources = vec![
        (Resource::RLIMIT_CORE, 0),
        (Resource::RLIMIT_CPU, installed.cpu_seconds),
        (Resource::RLIMIT_STACK, installed.stack_bytes),
        (Resource::RLIMIT_FSIZE, installed.output_bytes),
    ];
    if let Some(bytes) = installed.address_space_bytes {
        resources.push((Resource::RLIMIT_AS, bytes));
    }
    for (resource, bound) in resources {
        setrlimit(resource, bound, bound).map_err(std::io::Error::from)?;
    }
    Ok(installed)
}

#[cfg(not(unix))]
fn install_limits(limits: ProcessLimits) -> std::io::Result<InstalledLimits> {
    effective_limits(limits)
}

/// Legacy worker entry point: retain the default OS ceilings.
pub fn install_worker_limits() -> std::io::Result<()> {
    install_limits(ProcessLimits::default()).map(|_| ())
}

pub fn worker() -> std::io::Result<()> {
    worker_with_foundation(Foundation::Intensional)
}

pub fn worker_with_foundation(foundation: Foundation) -> std::io::Result<()> {
    worker_with_limits(foundation, ReplayLimits::default())
}

/// Invoke only with the trusted launch policy, before starting other threads.
pub fn worker_with_limits(foundation: Foundation, limits: ReplayLimits) -> std::io::Result<()> {
    let mut installed = None;
    let mut report = if !limits.is_valid() {
        report_failure(
            foundation,
            limits,
            inconclusive(
                "invalid_limits",
                "The resource policy cannot be represented safely.",
            ),
        )
    } else if let Ok(actual) = install_limits(limits.process) {
        installed = Some(actual);
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(limits.admission.read_limit().unwrap())
            .read_to_end(&mut bytes)?;
        crate::admission::replay_with_limits(&bytes, foundation, limits.admission)
    } else {
        report_failure(
            foundation,
            limits,
            inconclusive(
                "sandbox_unavailable",
                "The worker could not install its required process limits.",
            ),
        )
    };
    report.worker = Some(WorkerReceipt {
        requested: limits,
        installed,
    });
    serde_json::to_writer(std::io::stdout().lock(), &report).map_err(std::io::Error::other)
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn replay(executable: &Path, bytes: &[u8]) -> Report {
    replay_with_foundation(executable, bytes, Foundation::Intensional)
}

/// Compatibility API retains the original lowering-only deadline contract.
/// Use replay_with_limits to select an explicit policy without a default clamp.
pub fn replay_with_deadline(executable: &Path, bytes: &[u8], wall: Duration) -> Report {
    let mut limits = ReplayLimits::default();
    limits.process.wall_time_ms = wall.min(WALL_LIMIT).as_millis() as u64;
    replay_with_limits(executable, bytes, Foundation::Intensional, limits)
}

pub fn replay_with_foundation(executable: &Path, bytes: &[u8], foundation: Foundation) -> Report {
    replay_with_limits(executable, bytes, foundation, ReplayLimits::default())
}

/// Replay using a caller-owned policy. A submitted judgment cannot change it.
pub fn replay_with_limits(
    executable: &Path,
    bytes: &[u8],
    foundation: Foundation,
    limits: ReplayLimits,
) -> Report {
    if !limits.is_valid() {
        return report_failure(
            foundation,
            limits,
            inconclusive(
                "invalid_limits",
                "The resource policy cannot be represented safely.",
            ),
        );
    }
    let expected_installed = match effective_limits(limits.process) {
        Ok(actual) => actual,
        Err(_) => {
            return report_failure(
                foundation,
                limits,
                inconclusive(
                    "sandbox_unavailable",
                    "The supervisor could not determine the worker's inherited limits.",
                ),
            );
        }
    };
    let worker_arg = match foundation {
        Foundation::Intensional => WORKER_ARG,
        Foundation::FunctionExtensionality => EXTENSIONAL_WORKER_ARG,
    };
    let policy =
        serde_json::to_string(&limits).expect("resource policy serialization is infallible");
    let response = match run_worker_command(
        executable,
        &[worker_arg, POLICY_ARG, &policy],
        bytes,
        limits,
    ) {
        Ok(bytes) => bytes,
        Err(error) => return report_failure(foundation, limits, error),
    };
    match validate_worker_report(&response, foundation, limits, expected_installed) {
        Ok(report) => report,
        Err(error) => report_failure(foundation, limits, error),
    }
}

fn validate_worker_report(
    response: &[u8],
    foundation: Foundation,
    limits: ReplayLimits,
    expected_installed: InstalledLimits,
) -> Result<Report, WorkerFailure> {
    let report: Report = serde_json::from_slice(response).map_err(|_| {
        inconclusive(
            "worker_protocol",
            "The worker returned an invalid response.",
        )
    })?;
    let installed_matches = match report.worker {
        Some(receipt) if receipt.requested == limits => match receipt.installed {
            Some(installed) => installed == expected_installed,
            None => {
                report.outcome == ReplayOutcome::Inconclusive
                    && report
                        .diagnostic
                        .as_ref()
                        .is_some_and(|d| d.stage == "process" && d.code == "sandbox_unavailable")
            }
        },
        _ => false,
    };
    if report.profile != profile_for(foundation)
        || report.checker != checker_identity()
        || report.limits != limits.admission
        || !report.is_consistent()
        || !installed_matches
    {
        return Err(inconclusive(
            "worker_protocol",
            "The worker returned a different checker, foundation, resource policy or inconsistent outcome.",
        ));
    }
    Ok(report)
}

/// Generic legacy worker protocol. It must install the default limits itself.
pub fn run_worker(
    executable: &Path,
    trusted_worker_arg: &str,
    bytes: &[u8],
) -> Result<Vec<u8>, Diagnostic> {
    run_worker_with_deadline(executable, trusted_worker_arg, bytes, WALL_LIMIT)
}

pub fn run_worker_with_deadline(
    executable: &Path,
    trusted_worker_arg: &str,
    bytes: &[u8],
    wall: Duration,
) -> Result<Vec<u8>, Diagnostic> {
    let mut limits = ReplayLimits::default();
    limits.process.wall_time_ms = wall.min(WALL_LIMIT).as_millis() as u64;
    run_worker_command(executable, &[trusted_worker_arg], bytes, limits)
        .map_err(|error| error.diagnostic)
}

fn run_worker_command(
    executable: &Path,
    trusted_args: &[&str],
    bytes: &[u8],
    limits: ReplayLimits,
) -> Result<Vec<u8>, WorkerFailure> {
    if bytes.len() > limits.admission.max_bytes {
        return Err(exhausted(
            "byte_limit",
            "Proof input exceeds the input byte budget.",
        ));
    }
    // A zero wall budget does not race a fast worker into acceptance.
    if limits.process.wall_time_ms == 0 {
        return Err(exhausted(
            "wall_limit",
            "The isolated checker exhausted its wall-clock budget.",
        ));
    }
    let run = || -> std::io::Result<Result<Vec<u8>, WorkerFailure>> {
        let mut input = tempfile::tempfile()?;
        input.write_all(bytes)?;
        input.seek(SeekFrom::Start(0))?;
        let mut output = tempfile::tempfile()?;
        let start = Instant::now();
        let mut child = Running(
            Command::new(executable)
                .args(trusted_args)
                .env_clear()
                .stdin(Stdio::from(input))
                .stdout(Stdio::from(output.try_clone()?))
                .stderr(Stdio::null())
                .spawn()?,
        );
        let pid = sysinfo::Pid::from_u32(child.0.id());
        let mut monitor = sysinfo::System::new();
        loop {
            if start.elapsed() >= Duration::from_millis(limits.process.wall_time_ms) {
                return Ok(Err(exhausted(
                    "wall_limit",
                    "The isolated checker exhausted its wall-clock budget.",
                )));
            }
            if let Some(status) = child.0.try_wait()? {
                if !status.success() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        match status.signal() {
                            Some(nix::libc::SIGXCPU) => {
                                return Ok(Err(exhausted(
                                    "cpu_limit",
                                    "The worker exhausted its CPU budget.",
                                )));
                            }
                            Some(nix::libc::SIGXFSZ) => {
                                return Ok(Err(exhausted(
                                    "output_limit",
                                    "The worker exhausted its output budget.",
                                )));
                            }
                            _ => {}
                        }
                    }
                    return Ok(Err(inconclusive(
                        "worker_failed",
                        "The isolated checker exited without a valid result; a resource limit or runtime failure may have interrupted it.",
                    )));
                }
                // The scheduling interval between checking the deadline and
                // reaping a completed child belongs to the same wall budget.
                if start.elapsed() >= Duration::from_millis(limits.process.wall_time_ms) {
                    return Ok(Err(exhausted(
                        "wall_limit",
                        "The isolated checker exhausted its wall-clock budget.",
                    )));
                }
                break;
            }
            // macOS does not enforce RLIMIT_AS; RSS is a termination threshold,
            // not an instantaneous allocation cap.
            monitor.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&[pid]),
                true,
                sysinfo::ProcessRefreshKind::nothing()
                    .with_memory()
                    .without_tasks(),
            );
            if monitor
                .process(pid)
                .is_some_and(|p| p.memory() > limits.process.memory_bytes)
            {
                return Ok(Err(exhausted(
                    "memory_limit",
                    "The isolated checker exceeded its resident-memory termination threshold.",
                )));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        output.seek(SeekFrom::Start(0))?;
        let mut response = Vec::new();
        output
            .take(limits.process.output_bytes + 1)
            .read_to_end(&mut response)?;
        if response.len() as u64 > limits.process.output_bytes {
            return Ok(Err(exhausted(
                "output_limit",
                "The worker exceeded its output budget.",
            )));
        }
        Ok(Ok(response))
    };
    run().unwrap_or_else(|_| {
        Err(inconclusive(
            "worker_unavailable",
            "The isolated checker could not be started or its response could not be read.",
        ))
    })
}

#[cfg(test)]
#[path = "../tests/unit/worker_policy.rs"]
mod worker_policy;
