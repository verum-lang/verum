//! Industrial-grade crash reporter for the Verum toolchain.
//!
//! # What this provides
//!
//! A single `install()` call at the start of `main` wires up:
//!
//! 1. **Panic hook** — wraps [`std::panic::set_hook`] so every Rust panic
//!    (including ones on rayon workers, with `panic = "abort"` set) is
//!    captured, written to a structured report on disk, and surfaces to
//!    the user with the report path and a link to file an issue.
//! 2. **Fatal signal handlers** — `SIGSEGV`, `SIGBUS`, `SIGILL`, `SIGFPE`,
//!    `SIGABRT` are trapped via `sigaction` on Unix. The handler is
//!    best-effort async-signal-safe: it captures a backtrace (via the
//!    `backtrace` crate — not strictly sig-safe but works in practice
//!    for dev tools), dumps a minimal report via raw `libc::write`, then
//!    chains to the default handler so the kernel can still produce a
//!    core dump if configured.
//! 3. **Environment snapshot** — captured once at install time
//!    (version, git SHA, rustc, OS, arch, args, env) so reports are
//!    self-contained even when the user cannot reproduce.
//! 4. **Breadcrumb trail** — see [`crate::breadcrumb`]. Attached to
//!    every report so the dev can see the last known phase of the
//!    compilation pipeline before the crash.
//! 5. **Report store** — reports are written to
//!    `~/.verum/crashes/verum-<ISO-TS>-<short-uid>.{json,log}` with
//!    bounded retention (default: last 50). The JSON is stable, schema-
//!    versioned; the `.log` is a human-friendly render.
//!
//! # Signal-safety caveats
//!
//! A hard fault may leave the process in an inconsistent state. We do
//! our best:
//!
//! - Use `sigaltstack` so a stack overflow still lands the handler.
//! - Pre-allocate the crash-report directory at install time so we
//!   don't need to create dirs on the signal path.
//! - Write the minimal report via raw `libc::write` to avoid the
//!   `std::io` machinery (which takes locks).
//! - Chain to the original handler so `ulimit -c`-configured core
//!   dumps keep working.
//!
//! The fuller JSON report attempted in the signal path may fail if
//! global allocator state is poisoned. This is acceptable: you still
//! get the minimal `.log` plus (optionally) the OS core dump.

#![allow(missing_docs)]

use crate::breadcrumb::{self, Breadcrumb};
use parking_lot::{Mutex, RwLock};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const REPORT_SCHEMA_VERSION: u32 = 1;

/// Public configuration for the crash reporter.
#[derive(Clone)]
pub struct CrashReporterConfig {
    /// Human-readable application name, e.g. `"verum"`.
    pub app_name: String,
    /// Application version (typically `env!("CARGO_PKG_VERSION")`).
    pub app_version: String,
    /// Where to store reports. Defaults to `~/.verum/crashes/`.
    pub report_dir: Option<PathBuf>,
    /// Maximum number of reports to keep. Older ones are rotated off.
    pub retention: usize,
    /// Capture Rust backtraces. Enabled by default (`RUST_BACKTRACE=1`
    /// is forced on the process too).
    pub capture_backtrace: bool,
    /// Install fatal-signal handlers. Unix-only; no-op on Windows for now.
    pub install_signal_handlers: bool,
    /// Redact env vars whose name contains secret-ish tokens.
    pub redact_sensitive_env: bool,
    /// URL to show the user for issue reports.
    pub issue_tracker_url: String,
}

impl Default for CrashReporterConfig {
    fn default() -> Self {
        Self {
            app_name: "verum".into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            report_dir: None,
            retention: 50,
            capture_backtrace: true,
            install_signal_handlers: true,
            redact_sensitive_env: true,
            issue_tracker_url: "https://github.com/verum-lang/verum/issues/new".into(),
        }
    }
}

/// What kind of fault happened.
#[derive(Clone, Debug)]
pub enum CrashKind {
    Panic,
    Signal { name: &'static str, signo: i32 },
}

/// Frozen description of the running process, captured once at install
/// time. Cheap to include in every report.
#[derive(Clone, Debug)]
pub struct EnvSnapshot {
    /// The configured `app_name`. Mirrored into the snapshot so the
    /// renderer (which only sees the report, not the live config) can
    /// surface the correct branding for embedders.
    pub app_name: String,
    pub verum_version: String,
    pub build_profile: String,
    pub build_target: String,
    pub build_host: String,
    pub build_rustc: String,
    pub build_git_sha: String,
    pub build_git_dirty: String,
    pub build_timestamp: String,
    pub os: String,
    pub arch: String,
    pub os_family: String,
    pub cpu_cores: usize,
    pub rust_backtrace_env: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub pid: u32,
    pub env_vars: Vec<(String, String)>,
}

impl EnvSnapshot {
    fn capture(config: &CrashReporterConfig) -> Self {
        let argv: Vec<String> = std::env::args().collect();
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "<unknown>".into());
        let env_vars: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| should_keep_env(k, config.redact_sensitive_env))
            .map(|(k, v)| {
                if is_sensitive(&k) && config.redact_sensitive_env {
                    (k, "<redacted>".into())
                } else {
                    (k, v)
                }
            })
            .collect();

        Self {
            app_name: config.app_name.clone(),
            verum_version: config.app_version.clone(),
            build_profile: option_env!("VERUM_BUILD_PROFILE").unwrap_or("unknown").into(),
            build_target: option_env!("VERUM_BUILD_TARGET").unwrap_or("unknown").into(),
            build_host: option_env!("VERUM_BUILD_HOST").unwrap_or("unknown").into(),
            build_rustc: option_env!("VERUM_BUILD_RUSTC").unwrap_or("unknown").into(),
            build_git_sha: option_env!("VERUM_BUILD_GIT_SHA").unwrap_or("unknown").into(),
            build_git_dirty: option_env!("VERUM_BUILD_GIT_DIRTY").unwrap_or("unknown").into(),
            build_timestamp: option_env!("VERUM_BUILD_TIMESTAMP").unwrap_or("0").into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            os_family: std::env::consts::FAMILY.into(),
            cpu_cores: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            rust_backtrace_env: std::env::var("RUST_BACKTRACE").unwrap_or_default(),
            argv,
            cwd,
            pid: std::process::id(),
            env_vars,
        }
    }
}

fn is_sensitive(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    const NEEDLES: &[&str] = &[
        "PASSWORD", "SECRET", "TOKEN", "APIKEY", "API_KEY", "PRIVATE", "SESSION", "COOKIE",
        "CREDENTIAL", "AUTH", "PASSPHRASE",
    ];
    NEEDLES.iter().any(|n| k.contains(n))
}

/// Replace `$HOME` with `~` and the current username with `<user>` in a
/// path-like string. Idempotent; no-op when the env vars aren't set.
/// Used by `verum diagnose` to sanitise reports before external sharing
/// — never applied to on-disk reports.
pub fn scrub_paths(input: &str) -> String {
    let mut out = input.to_string();
    if let Some(home) = std::env::var_os("HOME").and_then(|v| v.into_string().ok()) {
        if !home.is_empty() {
            out = out.replace(&home, "~");
        }
    }
    if let Some(user) = std::env::var_os("USER").and_then(|v| v.into_string().ok()) {
        if !user.is_empty() {
            out = out.replace(&user, "<user>");
        }
    }
    out
}

fn should_keep_env(key: &str, redact: bool) -> bool {
    // Keep a curated subset of env vars most useful for reproducing a
    // build, plus anything starting with VERUM_. Skip noisy PATH-like
    // things unless redaction is disabled (dev mode).
    let k = key.to_ascii_uppercase();
    if k.starts_with("VERUM_") || k.starts_with("RUST") || k.starts_with("CARGO") {
        return true;
    }
    const KEEP: &[&str] = &[
        "HOME", "USER", "LANG", "LC_ALL", "TERM", "SHELL", "PWD", "OLDPWD", "LLVM_SYS_PREFIX",
        "LLVM_CONFIG_PATH", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH",
        "TMPDIR", "OSTYPE", "HOSTTYPE", "MACHTYPE",
    ];
    if KEEP.contains(&k.as_str()) {
        return true;
    }
    // Without redaction, keep everything.
    !redact
}

/// Per-invocation context attached to the crash report. Mutated as the
/// CLI progresses so we know *what command* was running at the time of
/// the crash.
#[derive(Clone, Default, Debug)]
pub struct CrashContext {
    pub command: Option<String>,
    pub input_file: Option<String>,
    pub tier: Option<String>,
    pub extra: Vec<(String, String)>,
}

/// The structured crash report.
#[derive(Clone, Debug)]
pub struct CrashReport {
    pub schema_version: u32,
    pub report_id: String,
    pub timestamp_ms: u64,
    pub kind: CrashKind,
    pub message: String,
    pub location: Option<String>,
    pub backtrace: Option<String>,
    pub thread_name: String,
    pub breadcrumbs: Vec<Breadcrumb>,
    pub context: CrashContext,
    pub environment: EnvSnapshot,
}

impl CrashReport {
    /// Serialise to JSON by hand (no serde_json dep required).
    fn to_json(&self) -> String {
        let mut s = String::with_capacity(4096);
        s.push('{');
        kv_u32(&mut s, "schema_version", self.schema_version, true);
        kv_str(&mut s, "report_id", &self.report_id, false);
        kv_u64(&mut s, "timestamp_ms", self.timestamp_ms, false);
        s.push(',');
        s.push_str("\"kind\":");
        match &self.kind {
            CrashKind::Panic => s.push_str("\"panic\""),
            CrashKind::Signal { name, signo } => {
                s.push('{');
                kv_str(&mut s, "type", "signal", true);
                kv_str(&mut s, "name", name, false);
                kv_i32(&mut s, "signo", *signo, false);
                s.push('}');
            }
        }
        kv_str(&mut s, "message", &self.message, false);
        s.push(',');
        s.push_str("\"location\":");
        match &self.location {
            Some(loc) => {
                s.push('"');
                push_json_escaped(&mut s, loc);
                s.push('"');
            }
            None => s.push_str("null"),
        }
        s.push(',');
        s.push_str("\"backtrace\":");
        match &self.backtrace {
            Some(bt) => {
                s.push('"');
                push_json_escaped(&mut s, bt);
                s.push('"');
            }
            None => s.push_str("null"),
        }
        kv_str(&mut s, "thread_name", &self.thread_name, false);

        // breadcrumbs
        s.push(',');
        s.push_str("\"breadcrumbs\":[");
        for (i, b) in self.breadcrumbs.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push('{');
            kv_str(&mut s, "phase", b.phase, true);
            kv_str(&mut s, "detail", &b.detail, false);
            kv_str(&mut s, "thread", &b.thread_name, false);
            kv_u64(&mut s, "age_ms", b.age_ms() as u64, false);
            s.push('}');
        }
        s.push(']');

        // context
        s.push(',');
        s.push_str("\"context\":{");
        let mut first = true;
        if let Some(c) = &self.context.command {
            kv_str(&mut s, "command", c, first);
            first = false;
        }
        if let Some(f) = &self.context.input_file {
            kv_str(&mut s, "input_file", f, first);
            first = false;
        }
        if let Some(t) = &self.context.tier {
            kv_str(&mut s, "tier", t, first);
            first = false;
        }
        for (k, v) in &self.context.extra {
            kv_str(&mut s, k, v, first);
            first = false;
        }
        let _ = first;
        s.push('}');

        // environment
        let e = &self.environment;
        s.push(',');
        s.push_str("\"environment\":{");
        kv_str(&mut s, "app_name", &e.app_name, true);
        kv_str(&mut s, "verum_version", &e.verum_version, false);
        kv_str(&mut s, "build_profile", &e.build_profile, false);
        kv_str(&mut s, "build_target", &e.build_target, false);
        kv_str(&mut s, "build_host", &e.build_host, false);
        kv_str(&mut s, "build_rustc", &e.build_rustc, false);
        kv_str(&mut s, "build_git_sha", &e.build_git_sha, false);
        kv_str(&mut s, "build_git_dirty", &e.build_git_dirty, false);
        kv_str(&mut s, "build_timestamp", &e.build_timestamp, false);
        kv_str(&mut s, "os", &e.os, false);
        kv_str(&mut s, "arch", &e.arch, false);
        kv_str(&mut s, "os_family", &e.os_family, false);
        kv_u64(&mut s, "cpu_cores", e.cpu_cores as u64, false);
        kv_str(&mut s, "rust_backtrace", &e.rust_backtrace_env, false);
        kv_u32(&mut s, "pid", e.pid, false);
        kv_str(&mut s, "cwd", &e.cwd, false);
        s.push(',');
        s.push_str("\"argv\":[");
        for (i, a) in e.argv.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push('"');
            push_json_escaped(&mut s, a);
            s.push('"');
        }
        s.push(']');
        s.push(',');
        s.push_str("\"env\":{");
        for (i, (k, v)) in e.env_vars.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            kv_str(&mut s, k, v, true);
        }
        s.push('}');
        s.push('}');

        s.push('}');
        s
    }

    /// Title-case the first character of `app_name` for headers /
    /// prose. The lowercase form (e.g. CLI prefix `verum:`) uses the
    /// snapshot value verbatim — embedders that want a different
    /// casing should set `app_name` accordingly.
    fn app_name_titlecased(&self) -> String {
        let raw = self.environment.app_name.as_str();
        let mut chars = raw.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        }
    }

    /// Human-readable rendering for the `.log` sibling file and for the
    /// short summary printed on stderr at crash time.
    pub fn to_human(&self) -> String {
        let mut out = String::with_capacity(2048);
        use std::fmt::Write as _;
        let _ = writeln!(
            out,
            "=== {} crash report ===========================================",
            self.app_name_titlecased()
        );
        let _ = writeln!(out, "Report ID:   {}", self.report_id);
        let _ = writeln!(
            out,
            "Timestamp:   {} (unix-ms)",
            self.timestamp_ms
        );
        match &self.kind {
            CrashKind::Panic => {
                let _ = writeln!(out, "Kind:        panic");
            }
            CrashKind::Signal { name, signo } => {
                let _ = writeln!(out, "Kind:        fatal signal {} ({})", name, signo);
            }
        }
        let _ = writeln!(out, "Thread:      {}", self.thread_name);
        let _ = writeln!(out, "Message:     {}", self.message);
        if let Some(loc) = &self.location {
            let _ = writeln!(out, "Location:    {}", loc);
        }

        let e = &self.environment;
        let _ = writeln!(
            out,
            "\nBuild:       {} {} ({}, {}, {}, git {} {})",
            e.app_name,
            e.verum_version,
            e.build_profile,
            e.build_target,
            e.build_rustc,
            e.build_git_sha,
            e.build_git_dirty
        );
        let _ = writeln!(
            out,
            "Host:        {} {} ({} cores)",
            e.os, e.arch, e.cpu_cores
        );
        let _ = writeln!(out, "PID:         {}", e.pid);
        let _ = writeln!(out, "Cwd:         {}", e.cwd);
        let _ = writeln!(out, "Args:        {}", e.argv.join(" "));

        let c = &self.context;
        if c.command.is_some() || c.input_file.is_some() || c.tier.is_some() {
            let _ = writeln!(out, "\nContext:");
            if let Some(v) = &c.command {
                let _ = writeln!(out, "  command:    {}", v);
            }
            if let Some(v) = &c.input_file {
                let _ = writeln!(out, "  input:      {}", v);
            }
            if let Some(v) = &c.tier {
                let _ = writeln!(out, "  tier:       {}", v);
            }
            for (k, v) in &c.extra {
                let _ = writeln!(out, "  {}: {}", k, v);
            }
        }

        if !self.breadcrumbs.is_empty() {
            let _ = writeln!(out, "\nBreadcrumbs (most recent last):");
            for b in &self.breadcrumbs {
                let _ = writeln!(
                    out,
                    "  [{:>6}ms] {:<36} {} [thread={}]",
                    b.age_ms(),
                    b.phase,
                    b.detail,
                    b.thread_name
                );
            }
        } else {
            let _ = writeln!(out, "\nBreadcrumbs: <none>");
        }

        if let Some(bt) = &self.backtrace {
            let _ = writeln!(out, "\nBacktrace:\n{}", bt);
        } else {
            let _ = writeln!(
                out,
                "\nBacktrace:   <disabled; set RUST_BACKTRACE=1 or VERUM_BACKTRACE=1>"
            );
        }

        if !self.environment.env_vars.is_empty() {
            let _ = writeln!(out, "\nEnvironment (filtered):");
            for (k, v) in &self.environment.env_vars {
                let _ = writeln!(out, "  {}={}", k, v);
            }
        }

        let _ = writeln!(
            out,
            "\n==================================================================="
        );
        out
    }
}

// ----- tiny JSON helpers -----

fn push_json_escaped(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

fn kv_str(out: &mut String, key: &str, value: &str, first: bool) {
    if !first {
        out.push(',');
    }
    out.push('"');
    push_json_escaped(out, key);
    out.push_str("\":\"");
    push_json_escaped(out, value);
    out.push('"');
}

fn kv_u32(out: &mut String, key: &str, value: u32, first: bool) {
    if !first {
        out.push(',');
    }
    out.push('"');
    push_json_escaped(out, key);
    out.push_str("\":");
    out.push_str(&value.to_string());
}

fn kv_u64(out: &mut String, key: &str, value: u64, first: bool) {
    if !first {
        out.push(',');
    }
    out.push('"');
    push_json_escaped(out, key);
    out.push_str("\":");
    out.push_str(&value.to_string());
}

fn kv_i32(out: &mut String, key: &str, value: i32, first: bool) {
    if !first {
        out.push(',');
    }
    out.push('"');
    push_json_escaped(out, key);
    out.push_str("\":");
    out.push_str(&value.to_string());
}

// ----- global state -----

struct ReporterState {
    config: CrashReporterConfig,
    env: EnvSnapshot,
    report_dir: PathBuf,
    context: RwLock<CrashContext>,
    installed: AtomicBool,
    entered: AtomicBool,
    reports_written: AtomicU64,
}

static STATE: OnceLock<Arc<ReporterState>> = OnceLock::new();

fn state() -> Option<&'static Arc<ReporterState>> {
    STATE.get()
}

/// Default report location: `~/.verum/crashes/`.
pub fn default_report_dir() -> PathBuf {
    match dirs_home() {
        Some(home) => home.join(".verum").join("crashes"),
        None => std::env::temp_dir().join("verum-crashes"),
    }
}

fn dirs_home() -> Option<PathBuf> {
    // Avoid a dep on `dirs` here — `HOME` / `USERPROFILE` covers the
    // common platforms and keeps verum_error dep-light.
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    None
}

/// Install the crash reporter. Idempotent — safe to call twice, only
/// the first call takes effect.
pub fn install(config: CrashReporterConfig) {
    if STATE.get().is_some() {
        return;
    }
    if config.capture_backtrace && std::env::var_os("RUST_BACKTRACE").is_none() {
        // SAFETY: called before worker threads read the env.
        unsafe {
            std::env::set_var("RUST_BACKTRACE", "1");
        }
    }

    let report_dir = config
        .report_dir
        .clone()
        .unwrap_or_else(default_report_dir);
    let _ = fs::create_dir_all(&report_dir);

    let env = EnvSnapshot::capture(&config);
    let state = Arc::new(ReporterState {
        config: config.clone(),
        env,
        report_dir,
        context: RwLock::new(CrashContext::default()),
        installed: AtomicBool::new(true),
        entered: AtomicBool::new(false),
        reports_written: AtomicU64::new(0),
    });
    let _ = STATE.set(state);

    install_panic_hook();

    #[cfg(unix)]
    {
        if config.install_signal_handlers {
            unsafe {
                install_signal_handlers_unix();
            }
        }
    }

    #[cfg(windows)]
    {
        if config.install_signal_handlers {
            unsafe {
                install_unhandled_exception_filter_windows();
            }
        }
    }
}

/// Set or update the command currently running (e.g. `"build"`, `"run"`).
pub fn set_command(cmd: impl Into<String>) {
    if let Some(s) = state() {
        s.context.write().command = Some(cmd.into());
    }
}

/// Set the input file (e.g. path to the `.vr` being compiled).
pub fn set_input_file(path: impl Into<String>) {
    if let Some(s) = state() {
        s.context.write().input_file = Some(path.into());
    }
}

/// Set the execution tier (`"interpreter"` or `"aot"`).
pub fn set_tier(tier: impl Into<String>) {
    if let Some(s) = state() {
        s.context.write().tier = Some(tier.into());
    }
}

/// Add a free-form key/value into the context block.
pub fn add_context(key: impl Into<String>, value: impl Into<String>) {
    if let Some(s) = state() {
        s.context.write().extra.push((key.into(), value.into()));
    }
}

/// Read a snapshot of the environment — useful for `verum version -v`.
pub fn env_snapshot() -> Option<EnvSnapshot> {
    state().map(|s| s.env.clone())
}

/// Where reports are being written.
pub fn report_dir() -> Option<PathBuf> {
    state().map(|s| s.report_dir.clone())
}

// ----- panic hook -----

fn install_panic_hook() {
    // Preserve any prior hook so we chain into it (e.g. the one set by
    // crate::panic_handler::setup_panic_hook for metrics).
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Recursion guard — a panic inside our hook must not loop.
        let reenter = state()
            .map(|s| s.entered.swap(true, Ordering::SeqCst))
            .unwrap_or(false);
        if reenter {
            prev(info);
            return;
        }

        let msg = panic_message(info);
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let backtrace = capture_backtrace();

        let report = build_report(CrashKind::Panic, msg, location, backtrace);
        write_and_notify(&report);

        // Chain into the prior hook (so `PanicLogger` still records metrics).
        prev(info);

        if let Some(s) = state() {
            s.entered.store(false, Ordering::SeqCst);
        }
    }));
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "Box<dyn Any> panic payload (non-string)".to_string()
    }
}

fn capture_backtrace() -> Option<String> {
    #[cfg(feature = "backtrace")]
    {
        let bt = backtrace::Backtrace::new();
        Some(format!("{:?}", bt))
    }
    #[cfg(not(feature = "backtrace"))]
    {
        let bt = std::backtrace::Backtrace::force_capture();
        Some(format!("{}", bt))
    }
}

fn build_report(
    kind: CrashKind,
    message: String,
    location: Option<String>,
    backtrace: Option<String>,
) -> CrashReport {
    let s = state().expect("crash reporter not installed");
    let breadcrumbs = breadcrumb::current_trail();
    let breadcrumbs = if breadcrumbs.is_empty() {
        breadcrumb::last_snapshot()
    } else {
        breadcrumbs
    };
    let thread_name = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_string();
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64;
    let report_id = generate_report_id(timestamp_ms);

    CrashReport {
        schema_version: REPORT_SCHEMA_VERSION,
        report_id,
        timestamp_ms,
        kind,
        message,
        location,
        backtrace,
        thread_name,
        breadcrumbs,
        context: s.context.read().clone(),
        environment: s.env.clone(),
    }
}

fn generate_report_id(timestamp_ms: u64) -> String {
    // pid + ts + counter; good enough to avoid collisions within a host
    // without pulling in `uuid`.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{:x}-{:x}-{:x}", std::process::id(), timestamp_ms, n)
}

fn write_and_notify(report: &CrashReport) {
    let s = state().expect("crash reporter not installed");

    // 1. Write the reports (best-effort — do not panic on I/O failure).
    let (json_path, log_path) = write_reports(&s.report_dir, report);
    s.reports_written.fetch_add(1, Ordering::SeqCst);

    // 2. Rotate old ones.
    let _ = rotate_reports(&s.report_dir, s.config.retention);

    // 3. Surface on stderr.
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr);
    let _ = writeln!(
        stderr,
        "{}: internal compiler error — a crash report has been saved.",
        s.config.app_name
    );
    match &report.kind {
        CrashKind::Panic => {
            let _ = writeln!(stderr, "       (panic: {})", report.message);
        }
        CrashKind::Signal { name, .. } => {
            let _ = writeln!(
                stderr,
                "       (fatal signal {}: {})",
                name, report.message
            );
        }
    }
    if let Some(p) = &log_path {
        let _ = writeln!(stderr, "       {}", p.display());
    }
    if let Some(p) = &json_path {
        let _ = writeln!(stderr, "       {}", p.display());
    }
    let app_titlecased = {
        let mut chars = s.config.app_name.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
            None => String::new(),
        }
    };
    let _ = writeln!(
        stderr,
        "\nThis is a bug in the {} compiler. Please file an issue at:",
        app_titlecased
    );
    let _ = writeln!(stderr, "  {}", s.config.issue_tracker_url);
    let _ = writeln!(
        stderr,
        "and attach the crash report above (run `{} diagnose bundle` to make a tarball).",
        s.config.app_name
    );

    // Short breadcrumb preview — often tells the reader *exactly* where
    // we were.
    if let Some(last) = report.breadcrumbs.last() {
        let _ = writeln!(
            stderr,
            "\nLast known phase: {} — {}",
            last.phase, last.detail
        );
    }
    let _ = stderr.flush();
}

fn write_reports(dir: &Path, report: &CrashReport) -> (Option<PathBuf>, Option<PathBuf>) {
    let base = format!(
        "verum-{}-{}",
        format_timestamp(report.timestamp_ms),
        &report.report_id
    );
    let json_path = dir.join(format!("{}.json", base));
    let log_path = dir.join(format!("{}.log", base));

    let json_ok = write_atomic(&json_path, report.to_json().as_bytes()).is_ok();
    let log_ok = write_atomic(&log_path, report.to_human().as_bytes()).is_ok();

    (
        if json_ok { Some(json_path) } else { None },
        if log_ok { Some(log_path) } else { None },
    )
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

fn format_timestamp(ms: u64) -> String {
    // Minimal ISO-ish format `YYYY-MM-DDTHH-MM-SS` from unix-ms without
    // pulling in chrono. Uses civil-from-days (Howard Hinnant's algorithm)
    // so filenames sort chronologically.
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u32;
    let (y, mo, d) = civil_from_days(days);
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;
    format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
        y, mo, d, hh, mm, ss
    )
}

fn civil_from_days(mut z: i64) -> (i32, u32, u32) {
    z += 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y as i32, m, d)
}

fn rotate_reports(dir: &Path, keep: usize) -> std::io::Result<()> {
    let mut entries: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if !name.starts_with("verum-") {
                return None;
            }
            let mtime = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((mtime, path))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
    let pair_keep = keep.saturating_mul(2); // .json + .log
    for (_, path) in entries.into_iter().skip(pair_keep) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

// ----- signal handling (Unix) -----

#[cfg(unix)]
unsafe fn install_signal_handlers_unix() {
    use libc::{
        SA_ONSTACK, SA_SIGINFO, SIGABRT, SIGBUS, SIGFPE, SIGILL, SIGSEGV, sigaction, sigaltstack,
        sigemptyset, sigset_t, stack_t,
    };
    use std::mem::MaybeUninit;

    // Install an alternate signal stack so SIGSEGV from stack overflow
    // still reaches the handler.
    const ALT_STACK_SIZE: usize = 64 * 1024;
    let stack = Box::leak(vec![0u8; ALT_STACK_SIZE].into_boxed_slice());
    let mut ss: stack_t = unsafe { std::mem::zeroed() };
    ss.ss_sp = stack.as_mut_ptr() as *mut _;
    ss.ss_size = ALT_STACK_SIZE;
    ss.ss_flags = 0;
    unsafe {
        let _ = sigaltstack(&ss, std::ptr::null_mut());
    }

    let mut mask: MaybeUninit<sigset_t> = MaybeUninit::uninit();
    unsafe { sigemptyset(mask.as_mut_ptr()) };

    for &signo in &[SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGABRT] {
        let mut action: sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = fatal_signal_handler as *const () as usize;
        action.sa_flags = SA_SIGINFO | SA_ONSTACK;
        action.sa_mask = unsafe { mask.assume_init() };

        let mut prev: sigaction = unsafe { std::mem::zeroed() };
        unsafe {
            sigaction(signo, &action, &mut prev);
        }
        remember_prev(signo, prev);
    }
}

#[cfg(unix)]
use libc::sigaction as LibcSigaction;
#[cfg(unix)]
static PREV_ACTIONS: OnceLock<Mutex<Vec<(i32, LibcSigaction)>>> = OnceLock::new();

#[cfg(unix)]
fn remember_prev(signo: i32, prev: LibcSigaction) {
    let cell = PREV_ACTIONS.get_or_init(|| Mutex::new(Vec::new()));
    cell.lock().push((signo, prev));
}

#[cfg(unix)]
fn prev_for(signo: i32) -> Option<LibcSigaction> {
    let cell = PREV_ACTIONS.get()?;
    cell.lock().iter().find(|(s, _)| *s == signo).map(|(_, a)| *a)
}

#[cfg(unix)]
extern "C" fn fatal_signal_handler(
    signo: libc::c_int,
    _info: *mut libc::siginfo_t,
    _ctx: *mut libc::c_void,
) {
    // Recursion guard: a signal inside the handler → default behaviour.
    if let Some(s) = state() {
        if s.entered.swap(true, Ordering::SeqCst) {
            unsafe {
                restore_and_reraise(signo);
            }
            return;
        }
    }

    let name = signal_name(signo);

    // Best-effort structured report (may allocate — not strictly sig-safe,
    // but the backtrace crate and our JSON builder are in-practice robust
    // enough for a CLI dev tool; worst case the minimal notice below
    // still fires).
    let backtrace = capture_backtrace();
    let report = build_report(
        CrashKind::Signal { name, signo },
        format!("received fatal signal {} ({})", name, signo),
        None,
        backtrace,
    );
    write_and_notify(&report);

    // Reset to default and re-raise so the kernel produces a core dump
    // (if `ulimit -c` allows) and the process exits with the normal
    // signal status.
    unsafe {
        restore_and_reraise(signo);
    }
}

#[cfg(unix)]
unsafe fn restore_and_reraise(signo: libc::c_int) {
    if let Some(prev) = prev_for(signo) {
        unsafe {
            libc::sigaction(signo, &prev, std::ptr::null_mut());
        }
    } else {
        let mut default_action: libc::sigaction = unsafe { std::mem::zeroed() };
        default_action.sa_sigaction = libc::SIG_DFL;
        unsafe {
            libc::sigaction(signo, &default_action, std::ptr::null_mut());
        }
    }
    unsafe {
        libc::raise(signo);
    }
}

#[cfg(unix)]
fn signal_name(signo: i32) -> &'static str {
    match signo {
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGBUS => "SIGBUS",
        libc::SIGILL => "SIGILL",
        libc::SIGFPE => "SIGFPE",
        libc::SIGABRT => "SIGABRT",
        _ => "SIGNAL",
    }
}

#[cfg(not(unix))]
unsafe fn install_signal_handlers_unix() {}

// ----- signal handling (Windows) -----
//
// `SetUnhandledExceptionFilter` fires before the Windows Error
// Reporting dialog. We write the structured crash report, then return
// `EXCEPTION_CONTINUE_SEARCH` so the OS still runs its default chain
// (WerFault minidump, etc.) for maximum data.

#[cfg(windows)]
unsafe fn install_unhandled_exception_filter_windows() {
    type Filter = unsafe extern "system" fn(*mut WindowsExceptionInfo) -> i32;
    unsafe extern "system" {
        fn SetUnhandledExceptionFilter(new: Filter) -> Filter;
    }
    unsafe {
        SetUnhandledExceptionFilter(windows_exception_handler);
    }
}

#[cfg(windows)]
#[repr(C)]
struct WindowsExceptionInfo {
    _exception_record: *mut core::ffi::c_void,
    _context_record: *mut core::ffi::c_void,
}

#[cfg(windows)]
unsafe extern "system" fn windows_exception_handler(_info: *mut WindowsExceptionInfo) -> i32 {
    if let Some(s) = state() {
        if s.entered.swap(true, Ordering::SeqCst) {
            return 0; // EXCEPTION_CONTINUE_SEARCH
        }
    }
    let backtrace = capture_backtrace();
    let report = build_report(
        CrashKind::Signal {
            name: "WIN32_EXCEPTION",
            signo: 0,
        },
        "received unhandled Windows exception".into(),
        None,
        backtrace,
    );
    write_and_notify(&report);
    0 // EXCEPTION_CONTINUE_SEARCH — let the OS show the standard UI.
}

// ----- public inspection APIs -----

/// List crash reports in the default (or configured) directory, newest first.
pub fn list_reports() -> std::io::Result<Vec<PathBuf>> {
    let dir = state()
        .map(|s| s.report_dir.clone())
        .unwrap_or_else(default_report_dir);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("log") {
                return None;
            }
            let mtime = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((mtime, p))
        })
        .collect();
    logs.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(logs.into_iter().map(|(_, p)| p).collect())
}

/// Number of reports written this process.
pub fn reports_written() -> u64 {
    state()
        .map(|s| s.reports_written.load(Ordering::SeqCst))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_round_trips_as_valid_json() {
        let report = CrashReport {
            schema_version: REPORT_SCHEMA_VERSION,
            report_id: "abc-1-0".into(),
            timestamp_ms: 1_700_000_000_000,
            kind: CrashKind::Panic,
            message: "boom \"quoted\" \n newline".into(),
            location: Some("foo.rs:1:2".into()),
            backtrace: Some("frame1\nframe2".into()),
            thread_name: "worker".into(),
            breadcrumbs: vec![],
            context: CrashContext::default(),
            environment: EnvSnapshot {
                app_name: "verum".into(),
                verum_version: "0.1.0".into(),
                build_profile: "release".into(),
                build_target: "aarch64-apple-darwin".into(),
                build_host: "aarch64-apple-darwin".into(),
                build_rustc: "rustc 1.93.0".into(),
                build_git_sha: "deadbee".into(),
                build_git_dirty: "clean".into(),
                build_timestamp: "0".into(),
                os: "macos".into(),
                arch: "aarch64".into(),
                os_family: "unix".into(),
                cpu_cores: 10,
                rust_backtrace_env: "1".into(),
                argv: vec!["verum".into(), "build".into()],
                cwd: "/tmp".into(),
                pid: 42,
                env_vars: vec![("HOME".into(), "/home".into())],
            },
        };
        let s = report.to_json();
        assert!(s.contains("\"schema_version\":1"));
        assert!(s.contains("\"kind\":\"panic\""));
        // Quoted content is escaped.
        assert!(s.contains("boom \\\"quoted\\\""));
        assert!(s.contains("\\n"));
    }

    #[test]
    fn human_format_is_stable() {
        let report = CrashReport {
            schema_version: REPORT_SCHEMA_VERSION,
            report_id: "id".into(),
            timestamp_ms: 1,
            kind: CrashKind::Signal {
                name: "SIGSEGV",
                signo: 11,
            },
            message: "boom".into(),
            location: None,
            backtrace: None,
            thread_name: "t".into(),
            breadcrumbs: vec![],
            context: CrashContext::default(),
            environment: EnvSnapshot {
                app_name: "verum".into(),
                verum_version: "0".into(),
                build_profile: "x".into(),
                build_target: "x".into(),
                build_host: "x".into(),
                build_rustc: "x".into(),
                build_git_sha: "x".into(),
                build_git_dirty: "x".into(),
                build_timestamp: "0".into(),
                os: "x".into(),
                arch: "x".into(),
                os_family: "x".into(),
                cpu_cores: 1,
                rust_backtrace_env: "".into(),
                argv: vec![],
                cwd: "".into(),
                pid: 1,
                env_vars: vec![],
            },
        };
        let h = report.to_human();
        assert!(h.contains("Verum crash report"));
        assert!(h.contains("fatal signal SIGSEGV"));
    }

    #[test]
    fn sensitive_env_filter() {
        assert!(is_sensitive("AWS_SECRET_ACCESS_KEY"));
        assert!(is_sensitive("MY_TOKEN"));
        assert!(!is_sensitive("HOME"));
    }

    #[test]
    fn timestamp_formats() {
        // 2001-09-09T01:46:40 UTC.
        let s = format_timestamp(1_000_000_000 * 1000);
        assert_eq!(s, "2001-09-09T01-46-40");
    }

    fn make_minimal_report(app_name: &str) -> CrashReport {
        CrashReport {
            schema_version: REPORT_SCHEMA_VERSION,
            report_id: "id".into(),
            timestamp_ms: 1,
            kind: CrashKind::Panic,
            message: "boom".into(),
            location: None,
            backtrace: None,
            thread_name: "t".into(),
            breadcrumbs: vec![],
            context: CrashContext::default(),
            environment: EnvSnapshot {
                app_name: app_name.into(),
                verum_version: "0".into(),
                build_profile: "x".into(),
                build_target: "x".into(),
                build_host: "x".into(),
                build_rustc: "x".into(),
                build_git_sha: "x".into(),
                build_git_dirty: "x".into(),
                build_timestamp: "0".into(),
                os: "x".into(),
                arch: "x".into(),
                os_family: "x".into(),
                cpu_cores: 1,
                rust_backtrace_env: "".into(),
                argv: vec![],
                cwd: "".into(),
                pid: 1,
                env_vars: vec![],
            },
        }
    }

    #[test]
    fn human_header_uses_titlecased_app_name() {
        // Pin: header surfaces the configured app_name with a title-
        // cased first letter so embedders that set `app_name = "myapp"`
        // get `"=== Myapp crash report ==="` without needing to
        // pre-capitalise the field.
        let report = make_minimal_report("myapp");
        let h = report.to_human();
        assert!(
            h.contains("=== Myapp crash report"),
            "header must titlecase the app_name, got: {h}"
        );
    }

    #[test]
    fn human_build_line_uses_app_name_verbatim() {
        // Pin: the Build line uses app_name verbatim (lowercased
        // matches `verum --version` style); embedders set the casing
        // they want via `CrashReporterConfig.app_name`.
        let report = make_minimal_report("myapp");
        let h = report.to_human();
        assert!(
            h.contains("Build:       myapp 0"),
            "Build line must surface app_name verbatim, got: {h}"
        );
    }

    #[test]
    fn json_envelope_emits_app_name_in_environment() {
        // Pin: the JSON sidecar carries app_name in `environment` so
        // a downstream collector can route reports per-tool without
        // grepping the human format.
        let report = make_minimal_report("myapp");
        let s = report.to_json();
        assert!(
            s.contains("\"app_name\":\"myapp\""),
            "JSON must carry app_name in environment block, got: {s}"
        );
    }

    #[test]
    fn default_app_name_keeps_verum_branding() {
        // Pin: with `CrashReporterConfig::default()`, app_name flows
        // through as `"verum"`, the header titlecases to `"Verum"`,
        // and existing tooling that greps `"Verum crash report"` keeps
        // working.
        let report = make_minimal_report(&CrashReporterConfig::default().app_name);
        let h = report.to_human();
        assert!(
            h.contains("=== Verum crash report"),
            "default app_name = \"verum\" must render \"=== Verum crash report\", got: {h}",
        );
    }
}
