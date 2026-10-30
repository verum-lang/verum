//! Cooperative control for bounded compiler analysis. Cancellation is an
//! unavailable verdict, never evidence that a program is valid or invalid.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Debug, Default)]
pub struct AnalysisControl {
    cancelled: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl AnalysisControl {
    pub fn with_deadline(deadline: Instant) -> Self {
        Self {
            deadline: Some(deadline),
            ..Self::default()
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
    }

    pub fn check(&self) -> crate::Result<()> {
        if self.is_cancelled() {
            Err(crate::TypeError::AnalysisCancelled)
        } else {
            Ok(())
        }
    }
}
