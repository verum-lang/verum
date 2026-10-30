//! Bounded, cancellable child execution. No shell, and no compiler in the UI host.
use crate::workspace::{SOURCE_LIMIT, digest, private_dir};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{Mutex, watch},
};
use verum_common::{List, Text};

const OUTPUT_LIMIT: usize = 256 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub id: Text,
    pub kind: Text,
    pub path: Text,
    pub source_hash: Text,
    pub status: Text,
    pub stdout: Text,
    pub stderr: Text,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub proof: Option<Value>,
    pub executable: PathBuf,
    pub timeout_ms: u64,
    pub evidence_path: PathBuf,
    pub started_unix_ms: u64,
    pub termination: Text,
    #[serde(default)]
    pub version: u64,
}
pub struct Job {
    pub receipt: Arc<Mutex<Receipt>>,
    cancel: watch::Sender<bool>,
}
pub struct Jobs {
    pub jobs: List<Job>,
    pub verum: PathBuf,
    pub proof: PathBuf,
    pub root: PathBuf,
}
impl Jobs {
    pub fn restore(&mut self) -> Result<()> {
        let root = private_dir(&self.root, &[".verum-studio", "runs"])?;
        let mut receipts = List::new();
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path().join("receipt.json");
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
                continue;
            }
            let Ok(mut receipt) = serde_json::from_slice::<Receipt>(&std::fs::read(path)?) else {
                continue;
            };
            let Ok(source) = std::fs::read(entry.path().join("source.vr")) else {
                continue;
            };
            if source.len() > SOURCE_LIMIT || digest(&source) != receipt.source_hash {
                continue;
            }
            receipt.evidence_path = entry.path();
            if receipt.status == "running" {
                receipt.status = "inconclusive".into();
                receipt.termination = "Studio stopped before receiving a terminal result".into();
            }
            receipts.push(receipt);
        }
        receipts.sort_by_key(|r| r.started_unix_ms);
        for receipt in receipts
            .into_iter()
            .rev()
            .take(126)
            .collect::<List<_>>()
            .into_iter()
            .rev()
        {
            let (cancel, _) = watch::channel(false);
            self.jobs.push(Job {
                receipt: Arc::new(Mutex::new(receipt)),
                cancel,
            });
        }
        Ok(())
    }
    pub fn start(&mut self, params: &Value, source_path: PathBuf) -> Result<Value> {
        let kind = params["kind"].as_str().context("Expected operation kind")?;
        ensure!(
            matches!(kind, "run" | "check" | "proof"),
            "Unknown operation"
        );
        let source = params["source"].as_str().context("Expected source")?;
        ensure!(source.len() <= SOURCE_LIMIT, "Source exceeds limit");
        let timeout = params["timeout_ms"].as_u64().unwrap_or(60000);
        ensure!(
            (1..=300000).contains(&timeout),
            "Timeout must be 1..300000 milliseconds"
        );
        if self.jobs.len() == 128 {
            let index = self
                .jobs
                .iter()
                .position(|j| j.receipt.try_lock().is_ok_and(|r| r.status != "running"))
                .context("Too many active runs")?;
            self.jobs.remove(index);
        }
        // An entry snapshot retains its source directory for relative module resolution.
        let mut snapshot = tempfile::Builder::new()
            .prefix(".verum-workbench-")
            .suffix(".vr")
            .tempfile_in(source_path.parent().context("Source parent")?)?;
        snapshot.write_all(source.as_bytes())?;
        snapshot.as_file().sync_all()?;
        let executable = if kind == "proof" {
            &self.proof
        } else {
            &self.verum
        };
        let id: Text = uuid::Uuid::new_v4().to_string().into();
        let evidence_path = private_dir(&self.root, &[".verum-studio", "runs", &id])?;
        std::fs::write(evidence_path.join("source.vr"), source)?;
        let mut command = Command::new(executable);
        command
            .current_dir(&self.root)
            .env("NO_COLOR", "1")
            .env("CLICOLOR", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        let input = if kind == "proof" {
            let declaration = params["declaration"]
                .as_str()
                .filter(|s| !s.is_empty())
                .context("Choose a theorem declaration")?;
            command.arg("--worker");
            Some(List::from(serde_json::to_vec(
                &json!({"profile":"verum-source-pi-id/1", "source":source,"declaration":declaration}),
            )?))
        } else if kind == "run" {
            // The CLI stdin entry selects script syntax without inventing a
            // wrapper or a second compiler in Studio.
            command.args(["run", "--interp", "-"]);
            Some(source.as_bytes().iter().copied().collect())
        } else {
            command.arg("check").arg(snapshot.path());
            None
        };
        let child = command
            .spawn()
            .with_context(|| format!("Start {}", executable.display()))?;
        let receipt = Arc::new(Mutex::new(Receipt {
            id: id.clone(),
            kind: kind.into(),
            path: params["path"].as_str().unwrap_or("scratch.vr").into(),
            source_hash: digest(source.as_bytes()),
            status: "running".into(),
            stdout: Text::new(),
            stderr: Text::new(),
            exit_code: None,
            elapsed_ms: 0,
            proof: None,
            executable: executable.clone(),
            timeout_ms: timeout,
            evidence_path,
            started_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as u64,
            termination: Text::new(),
            version: 0,
        }));
        let (cancel, receiver) = watch::channel(false);
        self.jobs.push(Job {
            receipt: receipt.clone(),
            cancel,
        });
        tokio::spawn(async move {
            {
                let record = receipt.lock().await;
                let _ = persist(&record);
            }
            execute(
                child,
                input,
                receipt,
                receiver,
                Duration::from_millis(timeout),
            )
            .await;
            drop(snapshot);
        });
        Ok(json!({"id":id}))
    }
    pub async fn list(&self) -> Value {
        self.changes(&Value::Null).await
    }
    pub async fn changes(&self, known: &Value) -> Value {
        let mut values = List::new();
        for job in &self.jobs {
            let receipt = job.receipt.lock().await;
            if known.get(receipt.id.as_str()).and_then(Value::as_u64) != Some(receipt.version) {
                values.push(receipt.clone());
            }
        }
        json!(values)
    }
    pub async fn read(&self, id: &Value) -> Option<Receipt> {
        for job in &self.jobs {
            let receipt = job.receipt.lock().await;
            if id.as_str() == Some(receipt.id.as_str()) {
                return Some(receipt.clone());
            }
        }
        None
    }
    pub async fn cancel(&self, id: &str) -> Result<Value> {
        for job in &self.jobs {
            if job.receipt.lock().await.id.as_str() == id {
                let _ = job.cancel.send(true);
                return Ok(json!({"cancelled":true}));
            }
        }
        anyhow::bail!("Unknown operation")
    }
}
async fn capture(
    mut stream: impl AsyncRead + Unpin,
    receipt: Arc<Mutex<Receipt>>,
    stderr: bool,
) -> Result<()> {
    let mut bytes = List::<u8>::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > OUTPUT_LIMIT {
            return Err(OutputBudget.into());
        }
        bytes.extend_from_slice(&chunk[..n]);
        let text = Text::from_utf8_lossy(&bytes);
        let mut record = receipt.lock().await;
        if stderr {
            record.stderr = text;
        } else {
            record.stdout = text;
        }
        record.version += 1;
    }
    Ok(())
}
async fn execute(
    mut child: tokio::process::Child,
    input: Option<List<u8>>,
    receipt: Arc<Mutex<Receipt>>,
    mut cancel: watch::Receiver<bool>,
    timeout: Duration,
) {
    let group = ProcessGroup(child.id());
    let started = Instant::now();
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out_record = receipt.clone();
    let err_record = receipt.clone();
    let io = async {
        let write = async {
            if let Some(input) = input {
                stdin.write_all(&input).await?;
            }
            drop(stdin);
            Ok::<_, anyhow::Error>(())
        };
        tokio::try_join!(
            write,
            capture(stdout, out_record, false),
            capture(stderr, err_record, true)
        )?;
        Ok::<_, anyhow::Error>(())
    };
    let work = async {
        let (status, ()) =
            tokio::try_join!(async { Ok::<_, anyhow::Error>(child.wait().await?) }, io)?;
        Ok::<_, anyhow::Error>(status)
    };
    let proof_job = receipt.lock().await.kind == "proof";
    let (state, exit, reason) = tokio::select! {
        result = work => match result { Ok(s) => (process_status(s,proof_job), s.code(),Text::from(format!("Process {s}"))), Err(e) => (if e.is::<OutputBudget>() {"exhausted"} else {"inconclusive"},None,Text::from(e.to_string())) },
        _ = tokio::time::sleep(timeout) => ("exhausted",None,Text::from("Wall-time budget exceeded")),
        _ = cancel.changed() => ("cancelled",None,Text::from("Cancelled by user")),
    };
    drop(group);
    let _ = child.kill().await;
    let _ = child.wait().await;
    let mut record = receipt.lock().await;
    record.status = state.into();
    record.exit_code = exit;
    record.elapsed_ms = started.elapsed().as_millis() as u64;
    record.termination = reason;
    record.version += 1;
    if record.kind == "proof" && state == "succeeded" {
        match serde_json::from_str::<Value>(&record.stdout) {
            Ok(proof)
                if proof["source"]["sha256"].as_str() == Some(record.source_hash.as_str()) =>
            {
                record.status = match proof["status"].as_str() {
                    Some("verified") => "succeeded",
                    Some("rejected") => "failed",
                    Some("resource_exhausted") => "exhausted",
                    _ => "inconclusive",
                }
                .into();
                record.proof = Some(proof);
            }
            _ => {
                record.status = "inconclusive".into();
                record
                    .stderr
                    .push_str("\nProof worker did not return evidence for this source.");
            }
        }
    }
    if let Err(error) = persist(&record) {
        record
            .stderr
            .push_str(&format!("\nCould not retain receipt: {error}"));
    }
}
fn process_status(status: std::process::ExitStatus, proof: bool) -> &'static str {
    if status.success() {
        return "succeeded";
    }
    if !proof {
        return "failed";
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if status.signal() == Some(libc::SIGXCPU) {
            return "exhausted";
        }
    }
    // A terminated checker has not rejected a theorem.
    "inconclusive"
}
#[derive(Debug)]
struct OutputBudget;
impl std::fmt::Display for OutputBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Output budget exceeded")
    }
}
impl std::error::Error for OutputBudget {}
fn persist(receipt: &Receipt) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(&receipt.evidence_path)?;
    serde_json::to_writer_pretty(file.as_file_mut(), receipt)?;
    file.as_file().sync_all()?;
    file.persist(receipt.evidence_path.join("receipt.json"))?;
    Ok(())
}
struct ProcessGroup(Option<u32>);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            // SAFETY: the child was spawned into its own process group. This
            // signal addresses that group, not Studio or its caller's shell.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}
