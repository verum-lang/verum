//! Keep stray writes off the TUI's screen.
//!
//! The playbook draws with ratatui over the process's stdout. Anything
//! else that writes to stdout or stderr while the session is up —
//! a `println!` deep in the compiler, a `tracing` line, a panic
//! message from a worker thread, a runtime warning — lands in the
//! same terminal and shreds the rendered frame. The reader sees
//! smeared borders and half-drawn panels and reasonably calls it "the
//! UI broke".
//!
//! Redirection makes that structurally impossible instead of asking
//! every writer to behave: on entry the real stdout/stderr descriptors
//! are duplicated (the duplicate is what ratatui draws on), the
//! originals are pointed at a pipe, and a reader thread turns whatever
//! arrives into lines the app can display. On exit the descriptors are
//! restored, so a crash after teardown still prints normally.
//!
//! Unix only, by construction: `dup2` is the mechanism. On other
//! platforms the guard is inert and the previous behaviour stands —
//! honest inertness beats a half-working emulation.

use std::io;
use std::sync::{Arc, Mutex};

/// Lines captured while the TUI was up, oldest first.
pub type CapturedLog = Arc<Mutex<Vec<String>>>;

/// A live redirection. Dropping it restores the descriptors.
pub struct TuiCapture {
    /// The writer the TUI must draw on — the duplicated real stdout.
    /// `None` when capture is inert (non-unix, or setup failed).
    screen: Option<std::fs::File>,
    log: CapturedLog,
    #[cfg(unix)]
    saved: Option<SavedFds>,
}

#[cfg(unix)]
struct SavedFds {
    stdout: i32,
    stderr: i32,
}

impl TuiCapture {
    /// Install the redirection. Never fails the session: if anything
    /// in the setup does not work, capture stays inert and the caller
    /// gets `screen() == None`, which means "draw on the ordinary
    /// stdout as before".
    pub fn install() -> Self {
        let log: CapturedLog = Arc::new(Mutex::new(Vec::new()));
        #[cfg(unix)]
        {
            match Self::install_unix(Arc::clone(&log)) {
                Some((screen, saved)) => {
                    return Self {
                        screen: Some(screen),
                        log,
                        saved: Some(saved),
                    };
                }
                None => {
                    return Self {
                        screen: None,
                        log,
                        saved: None,
                    };
                }
            }
        }
        #[cfg(not(unix))]
        {
            Self { screen: None, log }
        }
    }

    #[cfg(unix)]
    fn install_unix(log: CapturedLog) -> Option<(std::fs::File, SavedFds)> {
        use std::os::fd::FromRawFd;

        // SAFETY: all calls below are plain libc fd operations on the
        // process's own descriptors, performed once during single-
        // threaded CLI startup before the TUI or any worker exists.
        unsafe {
            let saved_out = libc::dup(libc::STDOUT_FILENO);
            let saved_err = libc::dup(libc::STDERR_FILENO);
            if saved_out < 0 || saved_err < 0 {
                if saved_out >= 0 {
                    libc::close(saved_out);
                }
                if saved_err >= 0 {
                    libc::close(saved_err);
                }
                return None;
            }

            let mut fds = [0i32; 2];
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                libc::close(saved_out);
                libc::close(saved_err);
                return None;
            }
            let (read_fd, write_fd) = (fds[0], fds[1]);

            if libc::dup2(write_fd, libc::STDOUT_FILENO) < 0
                || libc::dup2(write_fd, libc::STDERR_FILENO) < 0
            {
                libc::close(read_fd);
                libc::close(write_fd);
                libc::dup2(saved_out, libc::STDOUT_FILENO);
                libc::dup2(saved_err, libc::STDERR_FILENO);
                libc::close(saved_out);
                libc::close(saved_err);
                return None;
            }
            libc::close(write_fd);

            let reader = std::fs::File::from_raw_fd(read_fd);
            let sink = Arc::clone(&log);
            std::thread::spawn(move || {
                use std::io::BufRead;
                let buf = io::BufReader::new(reader);
                for line in buf.lines() {
                    let Ok(line) = line else { break };
                    if let Ok(mut guard) = sink.lock() {
                        // Bound the buffer: a runaway writer must not
                        // grow the session's memory without limit.
                        if guard.len() >= MAX_LOG_LINES {
                            guard.remove(0);
                        }
                        guard.push(line);
                    }
                }
            });

            let screen = std::fs::File::from_raw_fd(saved_out);
            Some((
                screen,
                SavedFds {
                    // `screen` owns saved_out now; keep only stderr's
                    // saved descriptor for restoration, and recover
                    // stdout's from the file at drop time.
                    stdout: -1,
                    stderr: saved_err,
                },
            ))
        }
    }

    /// The writer the TUI should draw on, when capture is live.
    pub fn screen(&self) -> Option<&std::fs::File> {
        self.screen.as_ref()
    }

    /// Shared handle to the captured lines — the app displays these.
    pub fn log(&self) -> CapturedLog {
        Arc::clone(&self.log)
    }
}

/// How many captured lines to keep. Enough to hold a compiler's
/// diagnostic burst; small enough that a loop printing forever cannot
/// exhaust memory.
const MAX_LOG_LINES: usize = 2_000;

impl Drop for TuiCapture {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: restoring the descriptors this guard replaced,
            // once, at teardown.
            unsafe {
                if let Some(screen) = self.screen.as_ref() {
                    libc::dup2(screen.as_raw_fd(), libc::STDOUT_FILENO);
                }
                if let Some(saved) = self.saved.as_ref()
                    && saved.stderr >= 0
                {
                    libc::dup2(saved.stderr, libc::STDERR_FILENO);
                    libc::close(saved.stderr);
                }
            }
        }
    }
}
