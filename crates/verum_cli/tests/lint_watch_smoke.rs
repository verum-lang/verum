//! Smoke test for `verum lint --watch`.
//!
//! Spawns the watch process, modifies a fixture file, and verifies
//! that the watcher prints a re-run banner before timeout. We can't
//! easily assert on the *contents* of the second run's output
//! without relying on tty behaviour, so the contract here is just
//! "the watcher reacts to a file edit".

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_lint_watch_test_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("write manifest");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {}\n",
    )
    .expect("write main.vr");
    dir
}

#[test]
fn watch_responds_to_file_change() {
    let dir = make_fixture("watch_smoke");
    // The watch banners go through `ui::step` → stderr (Cargo
    // convention). Lint diagnostics go to stdout. We need both, so
    // we pipe each stream and tee the lines into one channel.
    let mut child = Command::new(binary())
        .args(["lint", "--watch", "--threads", "2"])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn verum lint --watch");

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let tx_err = tx.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(|r| r.ok()) {
            let _ = tx.send(line);
        }
    });
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(|r| r.ok()) {
            let _ = tx_err.send(line);
        }
    });

    // Wait until we see the "Watching for changes" banner — that
    // signals the initial scan has finished and the watcher is set
    // up. Allow 30s — initial scan compiles a fresh fixture.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut saw_initial = false;
    while Instant::now() < deadline && !saw_initial {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(500))
            && line.contains("Watching for changes") {
                saw_initial = true;
            }
    }
    assert!(
        saw_initial,
        "watcher should print 'Watching for changes' after initial scan"
    );

    // Modify the file. The 500ms wait gives the FS-events backend
    // (FSEvents on macOS, inotify on Linux) time to register before
    // we touch the file — these backends drop pre-registration writes.
    std::thread::sleep(Duration::from_millis(500));
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {\n    let x = Box::new(5);\n}\n",
    )
    .expect("rewrite fixture");

    // Watch for the next "change detected" banner. 10s budget for
    // debounce (300ms) + relint (compile cost) + FSEvents latency.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_rerun = false;
    while Instant::now() < deadline && !saw_rerun {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(500))
            && line.contains("change detected") {
                saw_rerun = true;
            }
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        saw_rerun,
        "watcher should re-run after the fixture file was rewritten"
    );
}
