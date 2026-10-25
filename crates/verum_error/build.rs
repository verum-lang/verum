// Capture build-time metadata for the crash reporter.
//

// These `cargo:rustc-env` variables are available inside the crate as
// `option_env!(...)` and flow into `CrashReport::environment` without
// needing the user's dev environment to replicate them.

use std::process::Command;

fn main() {
    emit(
        "VERUM_BUILD_PROFILE",
        std::env::var("PROFILE").unwrap_or_else(|_| "unknown".into()),
    );
    emit(
        "VERUM_BUILD_TARGET",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".into()),
    );
    emit(
        "VERUM_BUILD_HOST",
        std::env::var("HOST").unwrap_or_else(|_| "unknown".into()),
    );
    emit("VERUM_BUILD_TIMESTAMP", iso_timestamp());
    emit("VERUM_BUILD_GIT_SHA", git_sha());
    emit("VERUM_BUILD_GIT_DIRTY", git_dirty());
    emit("VERUM_BUILD_RUSTC", rustc_version());

    // Invalidate on HEAD changes so the git SHA stays fresh.
    println!("cargo:rerun-if-changed=build.rs");
    watch_git_inputs();
    println!("cargo:rerun-if-env-changed=PROFILE");
}

fn watch_git_inputs() {
    // Git resolves worktree indirection and packed/loose branch references.
    // Source archives have no Git metadata: watching a missing path makes
    // Cargo rerun this timestamp-producing script on every invocation.
    let branch = run("git", &["symbolic-ref", "--quiet", "HEAD"]);
    for input in [Some("HEAD".to_owned()), Some("packed-refs".to_owned()), branch.clone()]
        .into_iter().flatten()
    {
        let Some(path) = run("git", &["rev-parse", "--git-path", &input]) else { continue; };
        let path = std::path::Path::new(&path);
        if path.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
        } else if branch.as_deref() == Some(input.as_str()) {
            // A packed branch can acquire a loose ref on its next commit.
            if let Some(parent) = path.ancestors().skip(1).find(|p| p.is_dir()) {
                println!("cargo:rerun-if-changed={}", parent.display());
            }
        }
    }
}

fn emit(key: &str, value: String) {
    println!("cargo:rustc-env={}={}", key, sanitize(&value));
}

fn sanitize(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn git_sha() -> String {
    run("git", &["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".into())
}

fn git_dirty() -> String {
    match run("git", &["status", "--porcelain"]) {
        Some(out) if !out.trim().is_empty() => "dirty".into(),
        Some(_) => "clean".into(),
        None => "unknown".into(),
    }
}

fn rustc_version() -> String {
    run("rustc", &["--version"]).unwrap_or_else(|| "unknown".into())
}

fn iso_timestamp() -> String {
    // SystemTime::now in build script — formatted as seconds-since-epoch.
    // Cheaper than pulling `chrono` into build-deps.
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => "0".into(),
    }
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}
