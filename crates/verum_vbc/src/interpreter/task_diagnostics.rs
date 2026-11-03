//! Optional bounded provenance, with outcomes read from the scheduler's TaskQueue.
//! No task handles, closure values, results or modules are retained by telemetry.
use super::state::{TaskId, TaskQueue, TaskStatus};
use crate::module::{FunctionId, VbcModule};
use std::{
    collections::VecDeque,
    fmt::{self, Write},
};
use verum_common::{Text, runtime_diagnostics::*};

#[derive(Clone, Debug)]
pub(super) struct TaskCapture {
    capacity: usize,
    omitted: u64,
    active: Option<TaskId>,
    recent: VecDeque<TaskOrigin>,
}

#[derive(Clone, Debug)]
struct TaskOrigin {
    task_id: u64,
    function_id: Option<u32>,
    module: Option<Text>,
    function: Option<Text>,
    entered: bool,
}

impl TaskQueue {
    /// Capture only tasks spawned after this call. Re-enabling discards the old
    /// capture. The scheduler's outcomes and handle lifetimes are unaffected.
    pub fn enable_diagnostics(
        &mut self,
        capacity: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        self.diagnostics = None;
        let capacity = capacity.min(MAX_ASYNC_TASK_ROWS);
        let mut recent = VecDeque::new();
        recent.try_reserve_exact(capacity)?;
        self.diagnostics = Some(TaskCapture {
            capacity,
            omitted: 0,
            active: None,
            recent,
        });
        Ok(())
    }

    pub(super) fn observe_spawn(&mut self, id: TaskId, function: Option<FunctionId>) {
        let Some(capture) = &mut self.diagnostics else {
            return;
        };
        if capture.recent.len() == capture.capacity {
            capture.omitted = capture.omitted.saturating_add(1);
            if capture.capacity == 0 {
                return;
            }
            capture.recent.pop_front();
        }
        capture.recent.push_back(TaskOrigin {
            task_id: id.0,
            function_id: function.map(|id| id.0),
            module: None,
            function: None,
            entered: false,
        });
    }

    pub(super) fn begin_diagnostic_task(&mut self, id: Option<TaskId>) -> Option<TaskId> {
        let capture = self.diagnostics.as_mut()?;
        std::mem::replace(&mut capture.active, id)
    }

    /// Called only after the actual callable has been validated and its frame
    /// entered. Nested synchronous helpers cannot overwrite the task entry.
    pub(super) fn observe_task_entry(&mut self, module: &VbcModule, function: FunctionId) {
        let Some(capture) = &mut self.diagnostics else {
            return;
        };
        let Some(id) = capture.active else {
            return;
        };
        let Some(row) = capture
            .recent
            .iter_mut()
            .find(|r| r.task_id == id.0 && !r.entered)
        else {
            return;
        };
        let label =
            |s: &str| (!s.is_empty() && s.len() <= MAX_TASK_NAME_BYTES).then(|| Text::from(s));
        row.function_id = Some(function.0);
        row.module = label(&module.name);
        row.function = module
            .get_function(function)
            .and_then(|f| module.get_string(f.name))
            .and_then(label);
        row.entered = true;
    }

    /// Snapshot a fixed number of recent tasks. Error text comes from the same
    /// recorded failure that Await/Join observe; detached tasks are included.
    pub fn diagnostics_report(&self) -> Option<AsyncTaskReport> {
        let capture = self.diagnostics.as_ref()?;
        let rows = capture
            .recent
            .iter()
            .map(|origin| {
                // Queue clearing also removes the capture, so every retained ID exists.
                let task = self
                    .get(TaskId(origin.task_id))
                    .expect("captured task remains in its queue");
                let status = match task.status {
                    TaskStatus::Pending => AsyncTaskStatus::Pending,
                    TaskStatus::Running => AsyncTaskStatus::Running,
                    TaskStatus::Suspended => AsyncTaskStatus::Suspended,
                    TaskStatus::Completed => AsyncTaskStatus::Completed,
                    TaskStatus::Failed => AsyncTaskStatus::Failed,
                };
                let mut row = AsyncTaskRow {
                    task_id: origin.task_id,
                    function_id: origin.function_id,
                    module: origin.module.clone(),
                    function: origin.function.clone(),
                    entered: origin.entered,
                    status,
                    error: None,
                    error_truncated: false,
                };
                if task.status == TaskStatus::Failed {
                    let mut text = BoundedError {
                        text: String::new(),
                        truncated: false,
                    };
                    if let Some(error) = &task.error {
                        let _ = write!(text, "{error}");
                    } else {
                        let _ = write!(
                            text,
                            "{}",
                            super::InterpreterError::TaskFailed { task_id: task.id }
                        );
                    }
                    row.error = Some(text.text.into());
                    row.error_truncated = text.truncated;
                }
                row
            })
            .collect();
        Some(AsyncTaskReport {
            schema: AsyncTaskSchema::V1,
            capacity: capture.capacity,
            omitted_tasks: capture.omitted,
            rows,
        })
    }
}

struct BoundedError {
    text: String,
    truncated: bool,
}
impl fmt::Write for BoundedError {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let remaining = MAX_TASK_ERROR_BYTES - self.text.len();
        if s.len() <= remaining {
            self.text.push_str(s);
            return Ok(());
        }
        let mut end = remaining;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&s[..end]);
        self.truncated = true;
        Err(fmt::Error)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/task_diagnostics.rs"]
mod tests;
