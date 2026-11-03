//! Bounded snapshots of interpreter task state, including detached tasks.
use super::methods::decimal;
use crate::{List, Text};
use serde::{Deserialize, Serialize};

pub const MAX_ASYNC_TASK_ROWS: usize = 64;
pub const MAX_TASK_ERROR_BYTES: usize = 2048;
pub const MAX_TASK_NAME_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsyncTaskSchema {
    #[serde(rename = "verum.async-tasks/1")]
    V1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsyncTaskStatus {
    Pending,
    Running,
    Suspended,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AsyncTaskRow {
    #[serde(with = "decimal")]
    pub task_id: u64,
    /// Function actually entered, or the declared target for a named task
    /// that has not entered a frame. Unresolved closures have no target ID.
    pub function_id: Option<u32>,
    pub module: Option<Text>,
    pub function: Option<Text>,
    pub entered: bool,
    pub status: AsyncTaskStatus,
    /// Original interpreter error, bounded without first allocating its full text.
    pub error: Option<Text>,
    pub error_truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AsyncTaskReport {
    pub schema: AsyncTaskSchema,
    pub capacity: usize,
    /// Oldest tasks omitted from this snapshot; recent tasks are retained.
    #[serde(with = "decimal")]
    pub omitted_tasks: u64,
    pub rows: List<AsyncTaskRow>,
}

impl AsyncTaskReport {
    pub fn validate(&self) -> bool {
        self.capacity <= MAX_ASYNC_TASK_ROWS
            && self.rows.len() <= self.capacity
            && (self.omitted_tasks == 0 || self.rows.len() == self.capacity)
            && self.rows.windows(2).all(|w| w[0].task_id < w[1].task_id)
            && self.rows.iter().all(|r| {
                (!r.entered || r.function_id.is_some())
                    && (r.entered || (r.module.is_none() && r.function.is_none()))
                    && [&r.module, &r.function]
                        .into_iter()
                        .flatten()
                        .all(|s| !s.is_empty() && s.len() <= MAX_TASK_NAME_BYTES)
                    && (r.status == AsyncTaskStatus::Failed) == r.error.is_some()
                    && (!r.error_truncated || r.error.is_some())
                    && r.error
                        .as_ref()
                        .is_none_or(|s| !s.is_empty() && s.len() <= MAX_TASK_ERROR_BYTES)
            })
    }
}
