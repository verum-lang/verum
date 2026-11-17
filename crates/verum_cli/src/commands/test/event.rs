use std::process::{ExitStatus, Output};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Default, Deserialize, Serialize)]
pub(super) struct ProcessDiagnostics {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) stderr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) signal: Option<i32>,
}

pub(super) fn process_signal(status: ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

mod milliseconds {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        value: &Duration,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(
            value
                .as_millis()
                .try_into()
                .map_err(serde::ser::Error::custom)?,
        )
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(deserializer)?))
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "outcome")]
pub(super) enum TestResult {
    #[serde(rename = "ok")]
    Pass {
        #[serde(rename = "duration_ms", with = "milliseconds")]
        duration: Duration,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        stdout: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        stderr: String,
    },
    #[serde(rename = "failed")]
    Fail {
        #[serde(rename = "duration_ms", with = "milliseconds")]
        duration: Duration,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        stdout: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        stderr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signal: Option<i32>,
        error: String,
    },
    #[serde(rename = "compile-error")]
    CompileError {
        #[serde(rename = "duration_ms", with = "milliseconds")]
        duration: Duration,
        error: String,
        #[serde(flatten)]
        process: ProcessDiagnostics,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub(super) enum TestEvent<N, R> {
    Test {
        #[serde(default)]
        name: N,
        #[serde(flatten)]
        result: R,
    },
    #[serde(other)]
    Other,
}

/// Suite and summary records share the stream with named test results. Decode
/// the named envelope first so that an invalid outcome still belongs to its test.
pub(super) fn reported_tests(
    stdout: &[u8],
) -> impl Iterator<Item = (Option<String>, serde_json::Result<TestResult>)> + '_ {
    stdout.split(|byte| *byte == b'\n').filter_map(|line| {
        match serde_json::from_slice::<TestEvent<Value, Value>>(line).ok()? {
            TestEvent::Test { name, result } => Some((
                name.as_str().map(str::to_owned),
                serde_json::from_value(result),
            )),
            TestEvent::Other => None,
        }
    })
}

pub(super) fn worker_failure(output: &Output, duration: Duration, error: String) -> TestResult {
    TestResult::Fail {
        duration,
        error: format!("{error} ({})", output.status),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code(),
        signal: process_signal(output.status),
    }
}

pub(super) fn single_worker_result(name: &str, output: Output, elapsed: Duration) -> TestResult {
    let mut records = reported_tests(&output.stdout);
    let Some((reported, result)) = records.next() else {
        return worker_failure(
            &output,
            elapsed,
            format!("worker reported no result for `{name}`"),
        );
    };
    let reported = reported.as_deref().unwrap_or("<unnamed>");
    if reported != name {
        return worker_failure(
            &output,
            elapsed,
            format!("worker reported `{reported}` while running `{name}`"),
        );
    }
    if let Some((extra, _)) = records.next() {
        let extra = extra.as_deref().unwrap_or("<unnamed>");
        return worker_failure(
            &output,
            elapsed,
            format!("worker reported an extra result for `{extra}` while running `{name}`"),
        );
    }
    let mut result = match result {
        Ok(result) => result,
        Err(err) => {
            return worker_failure(
                &output,
                elapsed,
                format!("invalid worker result for `{name}`: {err}"),
            );
        }
    };
    if matches!(result, TestResult::Pass { .. }) && !output.status.success() {
        return worker_failure(
            &output,
            elapsed,
            format!("worker reported success for `{name}` then terminated unsuccessfully"),
        );
    }
    let (stderr, termination) = match &mut result {
        TestResult::Pass { stderr, .. } => (stderr, None),
        TestResult::Fail {
            stderr,
            exit_code,
            signal,
            ..
        } => (stderr, Some((exit_code, signal))),
        TestResult::CompileError { process, .. } => (
            &mut process.stderr,
            Some((&mut process.exit_code, &mut process.signal)),
        ),
    };
    // The native test and its reporting worker can exit with different codes.
    // A compilation refusal has only the worker's process status.
    if let Some((exit_code, signal)) = termination
        && exit_code.is_none()
        && signal.is_none()
    {
        *exit_code = output.status.code();
        *signal = process_signal(output.status);
    }
    let captured = String::from_utf8_lossy(&output.stderr);
    if !captured.is_empty() && !stderr.contains(captured.as_ref()) {
        if captured.contains(stderr.as_str()) {
            *stderr = captured.into_owned();
        } else {
            if !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(&captured);
        }
    }
    result
}
