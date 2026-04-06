//! Asynchronous cell execution with streaming output

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::pipeline::ExecutionPipeline;
use super::context::ExecutionContext;
use crate::playbook::session::CellOutput;

/// Status of an async execution
#[derive(Debug, Clone)]
pub enum ExecutionStatus {
    /// Waiting to start
    Pending,
    /// Currently executing
    Running {
        started_at: Instant,
        progress: Option<f32>,
    },
    /// Completed successfully
    Completed {
        duration: Duration,
        output: CellOutput,
    },
    /// Failed with error
    Failed {
        duration: Duration,
        error: String,
    },
    /// Cancelled by user
    Cancelled,
}

impl PartialEq for ExecutionStatus {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ExecutionStatus::Pending, ExecutionStatus::Pending) => true,
            (ExecutionStatus::Running { .. }, ExecutionStatus::Running { .. }) => true,
            (ExecutionStatus::Completed { .. }, ExecutionStatus::Completed { .. }) => true,
            (ExecutionStatus::Failed { error: e1, .. }, ExecutionStatus::Failed { error: e2, .. }) => e1 == e2,
            (ExecutionStatus::Cancelled, ExecutionStatus::Cancelled) => true,
            _ => false,
        }
    }
}

impl ExecutionStatus {
    /// Check if execution is finished
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            ExecutionStatus::Completed { .. }
                | ExecutionStatus::Failed { .. }
                | ExecutionStatus::Cancelled
        )
    }

    /// Get elapsed time if running
    pub fn elapsed(&self) -> Option<Duration> {
        match self {
            ExecutionStatus::Running { started_at, .. } => Some(started_at.elapsed()),
            ExecutionStatus::Completed { duration, .. } => Some(*duration),
            ExecutionStatus::Failed { duration, .. } => Some(*duration),
            _ => None,
        }
    }
}

/// Message from executor to UI
#[derive(Debug, Clone)]
pub enum ExecutionMessage {
    /// Execution started
    Started,
    /// Progress update (0.0 to 1.0)
    Progress(f32),
    /// Stdout output
    Stdout(String),
    /// Stderr output
    Stderr(String),
    /// Status change
    Status(ExecutionStatus),
    /// Intermediate result (for streaming)
    IntermediateResult(String),
    /// Execution completed
    Completed(CellOutput),
    /// Execution failed
    Failed(String),
}

/// Handle for controlling an async execution
pub struct ExecutionHandle {
    /// Thread handle
    thread: Option<JoinHandle<()>>,
    /// Cancellation signal
    cancel_tx: Sender<()>,
    /// Message receiver
    message_rx: Receiver<ExecutionMessage>,
    /// Current status
    status: Arc<Mutex<ExecutionStatus>>,
}

impl ExecutionHandle {
    /// Check for new messages without blocking
    pub fn poll_messages(&self) -> Vec<ExecutionMessage> {
        let mut messages = Vec::new();
        while let Ok(msg) = self.message_rx.try_recv() {
            messages.push(msg);
        }
        messages
    }

    /// Get current status
    pub fn status(&self) -> ExecutionStatus {
        self.status.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Request cancellation
    pub fn cancel(&self) {
        let _ = self.cancel_tx.send(());
    }

    /// Check if execution is finished
    pub fn is_finished(&self) -> bool {
        self.status.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_finished()
    }

    /// Wait for completion (blocking)
    pub fn wait(mut self) -> ExecutionStatus {
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        self.status.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }
}

/// Async cell executor
pub struct AsyncExecutor {
    /// Execution pipeline
    pipeline: Arc<Mutex<ExecutionPipeline>>,
    /// Execution context
    context: Arc<Mutex<ExecutionContext>>,
}

impl AsyncExecutor {
    /// Create a new async executor
    pub fn new() -> Self {
        Self {
            pipeline: Arc::new(Mutex::new(ExecutionPipeline::new())),
            context: Arc::new(Mutex::new(ExecutionContext::new())),
        }
    }

    /// Create with existing pipeline and context
    pub fn with_state(pipeline: ExecutionPipeline, context: ExecutionContext) -> Self {
        Self {
            pipeline: Arc::new(Mutex::new(pipeline)),
            context: Arc::new(Mutex::new(context)),
        }
    }

    /// Execute a cell asynchronously
    pub fn execute_async(&self, cell_id: usize, source: String) -> ExecutionHandle {
        let (cancel_tx, cancel_rx) = mpsc::channel();
        let (message_tx, message_rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(ExecutionStatus::Pending));

        let pipeline = Arc::clone(&self.pipeline);
        let context = Arc::clone(&self.context);
        let status_clone = Arc::clone(&status);

        let thread = thread::spawn(move || {
            // Signal start
            *status_clone.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ExecutionStatus::Running {
                started_at: Instant::now(),
                progress: None,
            };
            let _ = message_tx.send(ExecutionMessage::Started);

            let start_time = Instant::now();

            // Check for cancellation periodically
            let check_cancelled = || cancel_rx.try_recv().is_ok();

            if check_cancelled() {
                *status_clone.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ExecutionStatus::Cancelled;
                return;
            }

            // Execute the cell
            let result = {
                let mut pipeline = pipeline.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let context = context.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

                // Simple execution (in real impl, would stream output)
                pipeline.execute_source(&source, cell_id, &context)
            };

            if check_cancelled() {
                *status_clone.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ExecutionStatus::Cancelled;
                return;
            }

            let duration = start_time.elapsed();

            match result {
                Ok(output) => {
                    *status_clone.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ExecutionStatus::Completed {
                        duration,
                        output: output.clone(),
                    };
                    let _ = message_tx.send(ExecutionMessage::Completed(output));
                }
                Err(e) => {
                    let error = e.to_string();
                    *status_clone.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ExecutionStatus::Failed {
                        duration,
                        error: error.clone(),
                    };
                    let _ = message_tx.send(ExecutionMessage::Failed(error));
                }
            }
        });

        ExecutionHandle {
            thread: Some(thread),
            cancel_tx,
            message_rx,
            status,
        }
    }

    /// Execute multiple cells in sequence
    pub fn execute_all_async(&self, cells: Vec<(usize, String)>) -> ExecutionHandle {
        let (cancel_tx, cancel_rx) = mpsc::channel();
        let (message_tx, message_rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(ExecutionStatus::Pending));

        let pipeline = Arc::clone(&self.pipeline);
        let context = Arc::clone(&self.context);
        let status_clone = Arc::clone(&status);
        let total_cells = cells.len() as f32;

        let thread = thread::spawn(move || {
            let start_time = Instant::now();

            *status_clone.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ExecutionStatus::Running {
                started_at: start_time,
                progress: Some(0.0),
            };
            let _ = message_tx.send(ExecutionMessage::Started);

            let check_cancelled = || cancel_rx.try_recv().is_ok();

            for (i, (cell_id, source)) in cells.into_iter().enumerate() {
                if check_cancelled() {
                    *status_clone.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ExecutionStatus::Cancelled;
                    return;
                }

                // Update progress
                let progress = (i as f32) / total_cells;
                *status_clone.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ExecutionStatus::Running {
                    started_at: start_time,
                    progress: Some(progress),
                };
                let _ = message_tx.send(ExecutionMessage::Progress(progress));

                // Execute cell
                let result = {
                    let mut pipeline = pipeline.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    let context = context.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    pipeline.execute_source(&source, cell_id, &context)
                };

                match result {
                    Ok(output) => {
                        let _ = message_tx.send(ExecutionMessage::IntermediateResult(
                            format!("Cell {}: {:?}", cell_id, output),
                        ));
                    }
                    Err(e) => {
                        let error = e.to_string();
                        *status_clone.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ExecutionStatus::Failed {
                            duration: start_time.elapsed(),
                            error: error.clone(),
                        };
                        let _ = message_tx.send(ExecutionMessage::Failed(error));
                        return;
                    }
                }
            }

            let duration = start_time.elapsed();
            *status_clone.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ExecutionStatus::Completed {
                duration,
                output: CellOutput::Empty,
            };
            let _ = message_tx.send(ExecutionMessage::Progress(1.0));
            let _ = message_tx.send(ExecutionMessage::Completed(CellOutput::Empty));
        });

        ExecutionHandle {
            thread: Some(thread),
            cancel_tx,
            message_rx,
            status,
        }
    }
}

impl Default for AsyncExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Streaming output collector
pub struct StreamingOutput {
    /// Collected stdout
    pub stdout: Vec<String>,
    /// Collected stderr
    pub stderr: Vec<String>,
    /// Output lines with timestamps
    pub timeline: Vec<(Duration, OutputLine)>,
    /// Start time
    start_time: Instant,
}

/// A line of output
#[derive(Debug, Clone)]
pub enum OutputLine {
    Stdout(String),
    Stderr(String),
    Result(String),
}

impl Default for StreamingOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingOutput {
    /// Create a new streaming output collector
    pub fn new() -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            timeline: Vec::new(),
            start_time: Instant::now(),
        }
    }

    /// Add stdout line
    pub fn add_stdout(&mut self, line: String) {
        let elapsed = self.start_time.elapsed();
        self.stdout.push(line.clone());
        self.timeline.push((elapsed, OutputLine::Stdout(line)));
    }

    /// Add stderr line
    pub fn add_stderr(&mut self, line: String) {
        let elapsed = self.start_time.elapsed();
        self.stderr.push(line.clone());
        self.timeline.push((elapsed, OutputLine::Stderr(line)));
    }

    /// Add result line
    pub fn add_result(&mut self, line: String) {
        let elapsed = self.start_time.elapsed();
        self.timeline.push((elapsed, OutputLine::Result(line)));
    }

    /// Get all stdout as single string
    pub fn stdout_str(&self) -> String {
        self.stdout.join("\n")
    }

    /// Get all stderr as single string
    pub fn stderr_str(&self) -> String {
        self.stderr.join("\n")
    }

    /// Get last N lines of output
    pub fn last_lines(&self, n: usize) -> Vec<&OutputLine> {
        self.timeline
            .iter()
            .rev()
            .take(n)
            .map(|(_, line)| line)
            .collect()
    }
}

/// Progress indicator style
#[derive(Debug, Clone, Copy)]
pub enum ProgressStyle {
    /// Simple spinner
    Spinner,
    /// Progress bar
    Bar,
    /// Percentage text
    Percentage,
    /// Elapsed time
    Elapsed,
}

/// Progress display helper
pub struct ProgressDisplay {
    style: ProgressStyle,
    width: usize,
    spinner_chars: Vec<char>,
    spinner_idx: usize,
}

impl Default for ProgressDisplay {
    fn default() -> Self {
        Self::new(ProgressStyle::Bar)
    }
}

impl ProgressDisplay {
    /// Create a new progress display
    pub fn new(style: ProgressStyle) -> Self {
        Self {
            style,
            width: 20,
            spinner_chars: vec!['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'],
            spinner_idx: 0,
        }
    }

    /// Set width for bar display
    pub fn with_width(mut self, width: usize) -> Self {
        self.width = width;
        self
    }

    /// Render progress (0.0 to 1.0)
    pub fn render(&mut self, progress: Option<f32>, elapsed: Duration) -> String {
        match self.style {
            ProgressStyle::Spinner => {
                let char = self.spinner_chars[self.spinner_idx];
                self.spinner_idx = (self.spinner_idx + 1) % self.spinner_chars.len();
                format!("{} Running...", char)
            }
            ProgressStyle::Bar => {
                if let Some(p) = progress {
                    let filled = (p * self.width as f32) as usize;
                    let empty = self.width - filled;
                    format!("[{}{}] {:.0}%", "█".repeat(filled), "░".repeat(empty), p * 100.0)
                } else {
                    let pos = self.spinner_idx % self.width;
                    self.spinner_idx += 1;
                    let mut bar = "░".repeat(self.width);
                    bar.replace_range(pos..pos + 1, "█");
                    format!("[{}]", bar)
                }
            }
            ProgressStyle::Percentage => {
                if let Some(p) = progress {
                    format!("{:.1}%", p * 100.0)
                } else {
                    "...".to_string()
                }
            }
            ProgressStyle::Elapsed => {
                let secs = elapsed.as_secs_f32();
                if secs < 1.0 {
                    format!("{:.0}ms", secs * 1000.0)
                } else if secs < 60.0 {
                    format!("{:.1}s", secs)
                } else {
                    let mins = (secs / 60.0) as u32;
                    let secs = secs % 60.0;
                    format!("{}:{:04.1}", mins, secs)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_status() {
        let status = ExecutionStatus::Pending;
        assert!(!status.is_finished());

        let status = ExecutionStatus::Completed {
            duration: Duration::from_secs(1),
            output: CellOutput::Empty,
        };
        assert!(status.is_finished());
    }

    #[test]
    fn test_streaming_output() {
        let mut output = StreamingOutput::new();
        output.add_stdout("line 1".to_string());
        output.add_stdout("line 2".to_string());
        output.add_stderr("error".to_string());

        assert_eq!(output.stdout.len(), 2);
        assert_eq!(output.stderr.len(), 1);
        assert_eq!(output.timeline.len(), 3);
    }

    #[test]
    fn test_progress_display() {
        let mut display = ProgressDisplay::new(ProgressStyle::Percentage);
        let output = display.render(Some(0.5), Duration::from_secs(1));
        assert!(output.contains("50"));

        let mut display = ProgressDisplay::new(ProgressStyle::Elapsed);
        let output = display.render(None, Duration::from_millis(500));
        assert!(output.contains("ms"));
    }

    #[test]
    fn test_async_executor_creation() {
        let executor = AsyncExecutor::new();
        // Just verify it creates without error
        let _ = executor;
    }
}
