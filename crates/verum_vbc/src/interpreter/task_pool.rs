//! Eager blocking workers in the interpreter. Each VM owns its local pools
//! and completed jobs. Reset drops outstanding owners and results, and a
//! collected handle is never reissued inside that VM.

use super::{InterpreterError, InterpreterResult};
use verum_common::{Map, Set};

#[derive(Default)]
pub(super) struct TaskPools {
    next_handle: i64,
    owners: Set<i64>,
    completed: Map<i64, i64>,
}

impl TaskPools {
    fn next(&mut self) -> InterpreterResult<i64> {
        self.next_handle =
            self.next_handle
                .checked_add(1)
                .ok_or_else(|| InterpreterError::Panic {
                    message: "blocking worker handle space exhausted".into(),
                })?;
        Ok(self.next_handle)
    }

    pub(super) fn create(&mut self) -> InterpreterResult<i64> {
        let handle = self.next()?;
        self.owners.insert(handle);
        Ok(handle)
    }

    pub(super) fn require(&self, handle: i64) -> InterpreterResult<()> {
        if self.owners.contains(&handle) {
            Ok(())
        } else {
            Err(InterpreterError::Panic {
                message: "blocking pool is closed or unknown".into(),
            })
        }
    }

    pub(super) fn destroy(&mut self, handle: i64) -> InterpreterResult<()> {
        self.require(handle)?;
        self.owners.remove(&handle);
        Ok(())
    }

    pub(super) fn complete(&mut self, result: i64) -> InterpreterResult<i64> {
        let handle = self.next()?;
        self.completed.insert(handle, result);
        Ok(handle)
    }

    pub(super) fn take(&mut self, handle: i64) -> InterpreterResult<i64> {
        self.completed
            .remove(&handle)
            .ok_or_else(|| InterpreterError::Panic {
                message: "blocking task is unknown or already collected".into(),
            })
    }

    pub(super) fn clear(&mut self) {
        self.owners.clear();
        self.completed.clear();
    }
}
