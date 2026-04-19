//! `verum diagnose` — inspect and bundle crash reports.
//!
//! Subcommands:
//!   - `list`     — show recent reports in `~/.verum/crashes/`.
//!   - `show`     — print the latest (or a specified) report.
//!   - `bundle`   — make a `.tar.gz` of recent reports for sharing.
//!   - `env`      — print build/host environment (no crash needed).

use crate::error::{CliError, Result};
use crate::ui;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use verum_error::crash;

#[derive(clap::Subcommand)]
pub enum DiagnoseCommands {
    /// List recent crash reports (newest first).
    List {
        /// Maximum number of reports to show.
        #[clap(long, default_value = "20")]
        limit: usize,
    },
    /// Print a crash report to stdout (defaults to the most recent).
    Show {
        /// Explicit path to a `.log` or `.json` report. Defaults to newest.
        #[clap(value_name = "REPORT")]
        path: Option<String>,
        /// Print the structured JSON instead of the human log.
        #[clap(long)]
        json: bool,
    },
    /// Bundle the latest reports into a `.tar.gz` for sharing on an issue.
    Bundle {
        /// Output path. Defaults to `./verum-crash-bundle-<ts>.tar.gz`.
        #[clap(long, short = 'o')]
        output: Option<String>,
        /// How many most-recent reports to include.
        #[clap(long, default_value = "5")]
        recent: usize,
    },
    /// Print the captured build / host environment snapshot.
    Env {
        /// Emit as JSON.
        #[clap(long)]
        json: bool,
    },
    /// Delete all stored crash reports.
    Clean {
        /// Skip the confirmation prompt.
        #[clap(long)]
        yes: bool,
    },
}

pub fn execute(cmd: DiagnoseCommands) -> Result<()> {
    match cmd {
        DiagnoseCommands::List { limit } => list(limit),
        DiagnoseCommands::Show { path, json } => show(path.as_deref(), json),
        DiagnoseCommands::Bundle { output, recent } => bundle(output.as_deref(), recent),
        DiagnoseCommands::Env { json } => env(json),
        DiagnoseCommands::Clean { yes } => clean(yes),
    }
}

fn report_dir() -> PathBuf {
    crash::report_dir().unwrap_or_else(crash::default_report_dir)
}

fn list(limit: usize) -> Result<()> {
    let dir = report_dir();
    let logs = crash::list_reports()
        .map_err(|e| CliError::Custom(format!("failed to read {}: {}", dir.display(), e)))?;

    if logs.is_empty() {
        ui::note(&format!("No crash reports in {}", dir.display()));
        return Ok(());
    }

    ui::section(&format!("Crash reports in {}", dir.display()));
    for (i, path) in logs.iter().take(limit).enumerate() {
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let meta = first_report_lines(path).unwrap_or_default();
        ui::output(&format!(
            "  {:>2}. {}  ({} B)",
            i + 1,
            path.display(),
            size
        ));
        for line in meta {
            ui::note(&format!("        {}", line));
        }
    }
    let shown = logs.len().min(limit);
    if logs.len() > shown {
        ui::note(&format!(
            "  ... {} more (use --limit to widen)",
            logs.len() - shown
        ));
    }
    Ok(())
}

/// Pull the first ~6 interesting lines out of a `.log` file so
/// `verum diagnose list` is instantly scannable.
fn first_report_lines(path: &Path) -> std::io::Result<Vec<String>> {
    let contents = fs::read_to_string(path)?;
    let picks: Vec<String> = contents
        .lines()
        .filter(|l| {
            l.starts_with("Kind:")
                || l.starts_with("Message:")
                || l.starts_with("Build:")
                || l.starts_with("Args:")
                || l.starts_with("Last known phase:")
        })
        .take(5)
        .map(|s| s.to_string())
        .collect();
    Ok(picks)
}

fn show(path: Option<&str>, want_json: bool) -> Result<()> {
    let log_path: PathBuf = match path {
        Some(p) => PathBuf::from(p),
        None => {
            let logs = crash::list_reports().map_err(|e| CliError::Custom(e.to_string()))?;
            logs.into_iter()
                .next()
                .ok_or_else(|| CliError::Custom("No crash reports found.".into()))?
        }
    };

    // If the user asked for JSON (or passed a .json path), resolve to
    // the json sibling.
    let target = if want_json {
        let base = log_path.with_extension("");
        base.with_extension("json")
    } else if log_path.extension().and_then(|x| x.to_str()) == Some("json") {
        log_path.clone()
    } else {
        log_path.clone()
    };

    if !target.exists() {
        return Err(CliError::Custom(format!(
            "Report not found: {}",
            target.display()
        )));
    }

    let mut f = fs::File::open(&target)
        .map_err(|e| CliError::Custom(format!("open {}: {}", target.display(), e)))?;
    let mut buf = String::new();
    f.read_to_string(&mut buf)
        .map_err(|e| CliError::Custom(format!("read {}: {}", target.display(), e)))?;

    print!("{}", buf);
    if !buf.ends_with('\n') {
        println!();
    }
    Ok(())
}

fn bundle(output: Option<&str>, recent: usize) -> Result<()> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    let logs = crash::list_reports().map_err(|e| CliError::Custom(e.to_string()))?;
    if logs.is_empty() {
        return Err(CliError::Custom("No crash reports to bundle.".into()));
    }

    let out_path = match output {
        Some(p) => PathBuf::from(p),
        None => {
            let ts = now_secs();
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(format!("verum-crash-bundle-{}.tar.gz", ts))
        }
    };

    let file =
        fs::File::create(&out_path).map_err(|e| CliError::Custom(format!("create bundle: {}", e)))?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = tar::Builder::new(enc);

    let mut included = 0usize;
    for log_path in logs.into_iter().take(recent) {
        // Pair the .log with its .json sibling.
        let stem = log_path.with_extension("");
        let json_path = stem.with_extension("json");
        add_to_tar(&mut tar, &log_path)?;
        if json_path.exists() {
            add_to_tar(&mut tar, &json_path)?;
        }
        included += 1;
    }

    // Include a README that explains how to submit.
    let readme = b"This bundle is a snapshot of recent Verum crash reports.\n\
Each .log is a human-readable render; each .json is the structured form.\n\
\n\
Please attach the whole archive to an issue at:\n\
  https://github.com/verum-lang/verum/issues/new\n";
    let mut header = tar::Header::new_gnu();
    header.set_path("README.txt").ok();
    header.set_size(readme.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append(&header, readme.as_ref())
        .map_err(|e| CliError::Custom(format!("write tar: {}", e)))?;

    let enc = tar
        .into_inner()
        .map_err(|e| CliError::Custom(format!("finish tar: {}", e)))?;
    enc.finish()
        .map_err(|e| CliError::Custom(format!("finish gzip: {}", e)))?;

    ui::success(&format!(
        "Bundled {} report(s) → {}",
        included,
        out_path.display()
    ));
    Ok(())
}

fn add_to_tar<W: Write>(tar: &mut tar::Builder<W>, path: &Path) -> Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "report.bin".into());
    tar.append_path_with_name(path, name)
        .map_err(|e| CliError::Custom(format!("add {}: {}", path.display(), e)))
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn env(json: bool) -> Result<()> {
    // The installer runs in main(), so env_snapshot is always Some here.
    let snap = match crash::env_snapshot() {
        Some(s) => s,
        None => {
            return Err(CliError::Custom(
                "crash reporter not installed — cannot read environment snapshot".into(),
            ));
        }
    };

    if json {
        // Render via the human format's env section or re-use CrashReport's
        // JSON. Simpler: hand-format a tiny envelope.
        let mut out = String::new();
        out.push('{');
        push_field(&mut out, "verum_version", &snap.verum_version, true);
        push_field(&mut out, "build_profile", &snap.build_profile, false);
        push_field(&mut out, "build_target", &snap.build_target, false);
        push_field(&mut out, "build_host", &snap.build_host, false);
        push_field(&mut out, "build_rustc", &snap.build_rustc, false);
        push_field(&mut out, "build_git_sha", &snap.build_git_sha, false);
        push_field(&mut out, "build_git_dirty", &snap.build_git_dirty, false);
        push_field(&mut out, "os", &snap.os, false);
        push_field(&mut out, "arch", &snap.arch, false);
        out.push_str(",\"cpu_cores\":");
        out.push_str(&snap.cpu_cores.to_string());
        out.push('}');
        println!("{}", out);
    } else {
        ui::section("Verum build / host environment");
        ui::detail("verum", &snap.verum_version);
        ui::detail("profile", &snap.build_profile);
        ui::detail("target", &snap.build_target);
        ui::detail("host", &snap.build_host);
        ui::detail("rustc", &snap.build_rustc);
        ui::detail(
            "git",
            &format!("{} ({})", snap.build_git_sha, snap.build_git_dirty),
        );
        ui::detail(
            "os/arch",
            &format!("{} {} ({} cores)", snap.os, snap.arch, snap.cpu_cores),
        );
    }
    Ok(())
}

fn push_field(out: &mut String, key: &str, value: &str, first: bool) {
    if !first {
        out.push(',');
    }
    out.push('"');
    for ch in key.chars() {
        if ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push_str("\":\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn clean(skip_confirm: bool) -> Result<()> {
    let dir = report_dir();
    if !dir.exists() {
        ui::note(&format!("Nothing to clean — {} does not exist", dir.display()));
        return Ok(());
    }
    if !skip_confirm {
        ui::warn(&format!(
            "About to delete all crash reports in {}. Re-run with --yes to confirm.",
            dir.display()
        ));
        return Ok(());
    }
    let mut removed = 0;
    for entry in fs::read_dir(&dir)
        .map_err(|e| CliError::Custom(format!("read {}: {}", dir.display(), e)))?
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name.starts_with("verum-") {
            if fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    ui::success(&format!("Removed {} crash report file(s)", removed));
    Ok(())
}
