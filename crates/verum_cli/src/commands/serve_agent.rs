//! `verum serve --agent` — the Agent Protocol server (T0853 v1).
//!
//! Accepted design: `docs/architecture/agent-protocol.md`. The laws
//! this skeleton already keeps (they are structural, not roadmap):
//!
//! * **Stream ownership**: stdout carries NOTHING but Content-Length
//!   framed JSON-RPC; every narrative line goes to stderr.
//! * **Envelope**: every result is `{protocol_version, method_version,
//!   data, …}`; fields are append-only from this first commit.
//! * **Content addressing / generation law**: every answer about
//!   source CITES the sha256 of the exact content it judged — the
//!   agent verifies the answer is about what it thinks it sent, and
//!   a disk change under an in-flight question is detectable instead
//!   of silently merged.
//! * **Statelessness of truth**: questions carry `content` (full
//!   buffer) or `path` (read once, hashed); each message re-states
//!   the truth — idempotent, replayable.
//! * **Journal**: every request/response pair is recorded with its
//!   hashes; `session.journal` returns the ledger.
//! * **Boundary theorem**: this process READS the tree and never
//!   writes source; the only writes are protocol frames.

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{Read, Write};

const PROTOCOL_VERSION: &str = "1";

/// One journal entry: the ledger of the dialogue (content-addressed).
#[derive(Debug, Serialize)]
struct JournalEntry {
    /// Request id (null for notifications).
    id: Value,
    /// Method name.
    method: String,
    /// sha256 of the content the answer was about, when applicable.
    content_hash: Option<String>,
    /// The verdict-bearing summary of the response (not the full
    /// payload — the journal is a ledger, not a cache).
    outcome: String,
    /// sha256 of the raw request FRAME body (K-4: the two-ledger
    /// judgment compares frame hashes across the seam; notifications
    /// have a request hash and no response hash — the named legal
    /// asymmetry).
    request_frame_sha256: Option<String>,
    /// sha256 of the raw response FRAME body (None for notifications).
    response_frame_sha256: Option<String>,
}

struct AgentServer {
    session_id: String,
    journal: Vec<JournalEntry>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    // The compiler workspace already depends on sha2 transitively;
    // use it directly for the content-address law.
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// Resolve the `{content}|{path}` convention: returns the SOURCE and
/// the sha256 it will be judged under. `content` wins over `path`
/// (the overlay beats the disk, per the generation law).
fn resolve_source(params: &Value) -> Result<(String, String)> {
    if let Some(content) = params.get("content").and_then(Value::as_str) {
        let hash = sha256_hex(content.as_bytes());
        return Ok((content.to_string(), hash));
    }
    if let Some(path) = params.get("path").and_then(Value::as_str) {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
        let hash = sha256_hex(content.as_bytes());
        return Ok((content, hash));
    }
    anyhow::bail!("params must carry `content` or `path`")
}

fn envelope(method_version: &str, data: Value) -> Value {
    json!({
        "protocol_version": PROTOCOL_VERSION,
        "method_version": method_version,
        "data": data,
    })
}

impl AgentServer {
    fn new() -> Self {
        // Session id: content-address of the process identity — stable
        // within a run, unique across runs, no wall-clock in the id.
        let seed = format!("{}-{}", std::process::id(), env!("CARGO_PKG_VERSION"));
        AgentServer {
            session_id: sha256_hex(seed.as_bytes())[..16].to_string(),
            journal: Vec::new(),
        }
    }

    fn record(&mut self, id: &Value, method: &str, hash: Option<&str>, outcome: &str) {
        self.journal.push(JournalEntry {
            id: id.clone(),
            method: method.to_string(),
            content_hash: hash.map(str::to_string),
            outcome: outcome.to_string(),
            request_frame_sha256: None,
            response_frame_sha256: None,
        });
    }

    /// Stamp the LAST journal entry with the seam hashes (K-4). The
    /// dispatch loop calls this once per handled message, after the
    /// response frame bytes exist.
    fn stamp_frames(&mut self, request_body: &[u8], response_body: Option<&[u8]>) {
        if let Some(last) = self.journal.last_mut() {
            last.request_frame_sha256 = Some(sha256_hex(request_body));
            last.response_frame_sha256 = response_body.map(sha256_hex);
        }
    }

    fn handle(&mut self, id: &Value, method: &str, params: &Value) -> Result<Value> {
        match method {
            "session.open" => {
                self.record(id, method, None, "opened");
                Ok(envelope(
                    "1",
                    json!({
                        "session_id": self.session_id,
                        "tool_version": env!("CARGO_PKG_VERSION"),
                        "methods": [
                            "session.open", "session.journal", "parse.check",
                            "arch.query", "test.run", "tiers.diff", "shutdown",
                        ],
                    }),
                ))
            }
            "session.journal" => {
                let entries = serde_json::to_value(&self.journal)?;
                self.record(id, method, None, "read");
                Ok(envelope("1", json!({ "entries": entries })))
            }
            "parse.check" => {
                let (content, hash) = resolve_source(params)?;
                let parser = verum_fast_parser::FastParser::new();
                let outcome = match parser
                    .parse_module_str(&content, verum_common::FileId::new(0))
                {
                    Ok(_) => json!({ "ok": true, "diagnostics": [] }),
                    Err(e) => json!({
                        "ok": false,
                        "diagnostics": [format!("{e:?}")],
                    }),
                };
                let ok = outcome["ok"].as_bool().unwrap_or(false);
                self.record(id, method, Some(&hash), if ok { "ok" } else { "diagnostics" });
                let mut env = envelope("1", outcome);
                env["content_hash"] = json!(hash);
                Ok(env)
            }
            "arch.query" => {
                let (content, hash) = resolve_source(params)?;
                let report = verum_compiler::arch_query::arch_query_source(&content)?;
                let verdict = match (&report.escalations, &report.dead_rights) {
                    (Some(e), Some(d)) if e.is_empty() && d.is_empty() => "clean",
                    (Some(_), _) => "judged",
                    _ => "derived-only",
                };
                self.record(id, method, Some(&hash), verdict);
                let mut env = envelope("1", serde_json::to_value(&report)?);
                env["content_hash"] = json!(hash);
                // Provenance law: what this verdict was computed from.
                env["provenance"] = json!({
                    "computed_from": { "content_sha256": hash },
                    "tool_version": env!("CARGO_PKG_VERSION"),
                    "evidence": "Computed",
                });
                Ok(env)
            }
            "test.run" => {
                // The stand's oracle: run a program (its asserts ARE
                // the test) under the interpreter tier with an
                // explicit budget. MISSED-honesty: exceeding the
                // budget is a named verdict, never a hang and never a
                // fake failure.
                let path = params
                    .get("path")
                    .and_then(Value::as_str)
                    .context("test.run requires `path`")?;
                let budget_s = params
                    .get("budget_s")
                    .and_then(Value::as_u64)
                    .unwrap_or(60);
                let bytes =
                    std::fs::read(path).with_context(|| format!("reading {path}"))?;
                let hash = sha256_hex(&bytes);
                let exe = std::env::current_exe()?;
                let mut child = std::process::Command::new(&exe)
                    .args(["run", "--tier", "interpret", path])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()?;
                let deadline = std::time::Instant::now()
                    + std::time::Duration::from_secs(budget_s);
                let verdict = loop {
                    match child.try_wait()? {
                        Some(status) => {
                            break if status.success() { "green" } else { "red" };
                        }
                        None if std::time::Instant::now() >= deadline => {
                            let _ = child.kill();
                            let _ = child.wait();
                            break "MISSED";
                        }
                        None => std::thread::sleep(
                            std::time::Duration::from_millis(25),
                        ),
                    }
                };
                self.record(id, method, Some(&hash), verdict);
                let mut env = envelope(
                    "1",
                    json!({ "verdict": verdict, "budget_s": budget_s }),
                );
                env["content_hash"] = json!(hash);
                env["provenance"] = json!({
                    "computed_from": { "content_sha256": hash },
                    "tool_version": env!("CARGO_PKG_VERSION"),
                    "evidence": "Computed",
                });
                Ok(env)
            }
            "tiers.diff" => {
                // Tier judgment needs a real file (the AOT tier builds
                // it); `path` is required — content-only buffers are a
                // named limitation of v1, not a silent fallback.
                let path = params
                    .get("path")
                    .and_then(Value::as_str)
                    .context("tiers.diff requires `path` (v1 judges files on disk)")?;
                let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
                let hash = sha256_hex(&bytes);
                let report = super::diff_tiers::judge(std::path::Path::new(path))?;
                self.record(id, method, Some(&hash), &report.verdict.clone());
                let mut env = envelope("1", serde_json::to_value(&report)?);
                env["content_hash"] = json!(hash);
                env["provenance"] = json!({
                    "computed_from": { "content_sha256": hash },
                    "tool_version": env!("CARGO_PKG_VERSION"),
                    "evidence": "Computed",
                });
                Ok(env)
            }
            other => anyhow::bail!("unknown method: {other}"),
        }
    }
}

fn read_frame(stdin: &mut impl Read) -> Result<Option<Value>> {
    // LSP-style: `Content-Length: N\r\n\r\n<body>`. A torn header is
    // an ERROR, never a resync guess.
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stdin.read(&mut byte) {
            Ok(0) => return Ok(None), // clean EOF between frames
            Ok(_) => {
                header.push(byte[0]);
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
                if header.len() > 4096 {
                    anyhow::bail!("frame header exceeds 4096 bytes — torn stream");
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    let text = String::from_utf8_lossy(&header);
    let len: usize = text
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length:"))
        .context("missing Content-Length")?
        .trim()
        .parse()
        .context("bad Content-Length")?;
    let mut body = vec![0u8; len];
    stdin.read_exact(&mut body).context("truncated frame body")?;
    Ok(Some(serde_json::from_slice(&body)?))
}

fn write_frame(stdout: &mut impl Write, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(stdout, "Content-Length: {}\r\n\r\n", body.len())?;
    stdout.write_all(&body)?;
    stdout.flush()?;
    Ok(())
}

pub fn execute() -> Result<()> {
    let mut server = AgentServer::new();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();

    eprintln!(
        "verum agent protocol v{PROTOCOL_VERSION} — session {} (stdout is frames-only)",
        server.session_id
    );

    while let Some(msg) = read_frame(&mut input)? {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

        if method == "shutdown" {
            let resp = json!({ "jsonrpc": "2.0", "id": id, "result": envelope("1", json!({"ok": true})) });
            write_frame(&mut output, &resp)?;
            break;
        }
        let request_body = serde_json::to_vec(&msg)?;
        if method == "$/cancel" {
            // v1 is a sequential queue: by the time a cancel arrives,
            // the request it names has completed. Journal it honestly
            // — request frame hashed, response hash ABSENT (the named
            // legal asymmetry of the K-4 seam).
            server.record(&id, &method, None, "no-op (sequential v1)");
            server.stamp_frames(&request_body, None);
            continue; // notification — no response
        }

        let response = match server.handle(&id, &method, &params) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": e.to_string() },
            }),
        };
        let response_body = serde_json::to_vec(&response)?;
        server.stamp_frames(&request_body, Some(&response_body));
        write_frame(&mut output, &response)?;
    }
    eprintln!("agent session {} closed", server.session_id);
    Ok(())
}
