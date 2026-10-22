//! A bounded host worker for untrusted native proof terms.
//! The executable and worker argument are deployment configuration, never input.
//! Files rather than pipes keep either direction from blocking the supervisor.
use crate::admission::{Diagnostic, MAX_BYTES, PROFILE, Report, checker_identity};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub const WORKER_ARG: &str = "--internal-native-proof-worker";
pub const WALL_LIMIT: Duration = Duration::from_secs(5);
pub const CPU_SECONDS: u64 = 3;
pub const MEMORY_BYTES: u64 = 512 * 1024 * 1024;
pub const OUTPUT_BYTES: u64 = 64 * 1024;

fn refused(code: &str, message: &str) -> Report {
    Report {
        profile: PROFILE.into(),
        checker: checker_identity(),
        accepted: false,
        diagnostic: Some(Diagnostic {
            code: code.into(),
            stage: "process".into(),
            message: message.into(),
        }),
    }
}

/// Limits must be installed in the single-threaded child, before decoding input.
/// CPU/address-space/file/stack limits are OS limits, not proof-theoretic fuel.
#[cfg(unix)]
fn limits() -> std::io::Result<()> {
    use nix::sys::resource::{Resource, getrlimit, setrlimit};
    for (resource, limit) in [
        (Resource::RLIMIT_CORE, 0),
        (Resource::RLIMIT_CPU, CPU_SECONDS),
        #[cfg(not(target_os = "macos"))]
        (Resource::RLIMIT_AS, MEMORY_BYTES),
        (Resource::RLIMIT_STACK, 16 * 1024 * 1024),
        (Resource::RLIMIT_FSIZE, OUTPUT_BYTES),
    ] {
        let (soft, hard) = getrlimit(resource).map_err(std::io::Error::from)?;
        let bound = (limit as nix::sys::resource::rlim_t).min(hard).min(soft);
        setrlimit(resource, bound, bound).map_err(std::io::Error::from)?;
    }
    Ok(())
}
#[cfg(not(unix))]
fn limits() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "native proof worker needs supported process limits",
    ))
}

/// Call before initializing any async runtime. stdout is one bounded JSON report.
pub fn worker() -> std::io::Result<()> {
    let report = if limits().is_err() {
        refused(
            "sandbox_unavailable",
            "The worker could not install its required process limits.",
        )
    } else {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        crate::admission::replay(&bytes)
    };
    serde_json::to_writer(std::io::stdout().lock(), &report).map_err(std::io::Error::other)
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        // Reap on every error path as well as timeout. No orphan checkers.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Replay in a fresh worker, never in the caller's address space. A crash,
/// timeout or protocol disagreement is inconclusive, not a false theorem.
pub fn replay(executable: &Path, bytes: &[u8]) -> Report {
    replay_with_deadline(executable, bytes, WALL_LIMIT)
}

/// The caller may lower the wall budget but cannot raise the default ceiling.
pub fn replay_with_deadline(executable: &Path, bytes: &[u8], wall: Duration) -> Report {
    if bytes.len() > MAX_BYTES {
        return refused(
            "byte_limit",
            "Native judgment exceeds the input byte budget.",
        );
    }
    let run = || -> std::io::Result<Report> {
        let mut input = tempfile::tempfile()?;
        input.write_all(bytes)?;
        input.seek(SeekFrom::Start(0))?;
        let mut output = tempfile::tempfile()?;
        let mut child = Running(
            Command::new(executable)
                .arg(WORKER_ARG)
                .env_clear()
                .stdin(Stdio::from(input))
                .stdout(Stdio::from(output.try_clone()?))
                .stderr(Stdio::null())
                .spawn()?,
        );
        let start = Instant::now();
        let pid = sysinfo::Pid::from_u32(child.0.id());
        let mut monitor = sysinfo::System::new();
        loop {
            if let Some(status) = child.0.try_wait()? {
                if !status.success() {
                    return Ok(refused(
                        "worker_failed",
                        "The isolated checker exited without a valid proof result; a resource limit or runtime failure may have interrupted it.",
                    ));
                }
                break;
            }
            if start.elapsed() >= wall.min(WALL_LIMIT) {
                return Ok(refused(
                    "wall_limit",
                    "The isolated checker exhausted its wall-clock budget.",
                ));
            }
            // macOS does not enforce RLIMIT_AS. Sample RSS on every platform;
            // this is a termination threshold, not an instantaneous allocation cap.
            monitor.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&[pid]),
                true,
                sysinfo::ProcessRefreshKind::nothing()
                    .with_memory()
                    .without_tasks(),
            );
            if monitor
                .process(pid)
                .is_some_and(|p| p.memory() > MEMORY_BYTES)
            {
                return Ok(refused(
                    "memory_limit",
                    "The isolated checker exceeded its resident-memory termination threshold.",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        output.seek(SeekFrom::Start(0))?;
        let mut response = Vec::new();
        output.take(OUTPUT_BYTES + 1).read_to_end(&mut response)?;
        if response.len() as u64 > OUTPUT_BYTES {
            return Ok(refused(
                "worker_protocol",
                "The worker exceeded its output budget.",
            ));
        }
        let report: Report = serde_json::from_slice(&response).map_err(std::io::Error::other)?;
        if report.profile != PROFILE
            || report.checker != checker_identity()
            || report.accepted == report.diagnostic.is_some()
        {
            return Ok(refused(
                "worker_protocol",
                "The worker returned an inconsistent result or a different checker identity.",
            ));
        }
        Ok(report)
    };
    run().unwrap_or_else(|_| {
        refused(
            "worker_unavailable",
            "The isolated checker could not be started or returned an invalid response.",
        )
    })
}
