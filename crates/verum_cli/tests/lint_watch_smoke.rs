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
    let mut child = Command::new(binary())
        .args(["lint", "--watch", "--threads", "2"])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn verum lint --watch");

    let stdout = child.stdout.take().expect("piped stdout");
    let reader = BufReader::new(stdout);

    // Channel for stdout lines.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in reader.lines().map_while(|r| r.ok()) {
            let _ = tx.send(line);
        }
    });

    // Wait until we see the "Watching for changes" banner — that
    // signals the initial scan has finished.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw_initial = false;
    while Instant::now() < deadline && !saw_initial {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(500)) {
            if line.contains("Watching for changes") {
                saw_initial = true;
            }
        }
    }
    assert!(
        saw_initial,
        "watcher should print 'Watching for changes' after initial scan"
    );

    // Modify the file. Wait briefly so notify reliably picks up
    // the write event on macOS (which uses FSEvents under the hood).
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {\n    let x = Box::new(5);\n}\n",
    )
    .expect("rewrite fixture");

    // Watch for the next "change detected" banner. Allow up to 5s
    // for debounce (300ms) + lint (~milliseconds) + some FS-event
    // latency.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_rerun = false;
    while Instant::now() < deadline && !saw_rerun {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(500)) {
            if line.contains("change detected") {
                saw_rerun = true;
            }
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
