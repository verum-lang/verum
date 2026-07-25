//! Debouncing system for real-time updates
//!
//! This module provides a debouncing mechanism to prevent excessive diagnostics
//! updates during rapid typing. It delays updates until the user has stopped
//! typing for a configurable duration (default: 300ms).
//!
//! # Design
//!
//! The debouncer uses a hash map of timers, one per document URI. When a change
//! is detected, the timer is reset. Only when the timer expires without being
//! reset is the callback executed.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep};
use tower_lsp::lsp_types::Url;

/// A debounced event with its associated URI
#[derive(Debug, Clone)]
struct DebouncedEvent {
    uri: Url,
    timestamp: Instant,
}

/// Debouncer for document updates
#[derive(Clone)]
pub struct Debouncer {
    /// Channel for sending debounce events
    sender: mpsc::UnboundedSender<DebouncedEvent>,
    /// Pending events by URI
    pending: Arc<Mutex<HashMap<Url, Instant>>>,
    /// Debounce delay duration
    delay: Duration,
}

impl Debouncer {
    /// Create a new debouncer with the given delay
    pub fn new(delay: Duration) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<DebouncedEvent>();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let pending_clone = Arc::clone(&pending);

        // Spawn the debouncer task
        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                // Record this event
                {
                    let mut pending = pending_clone.lock();
                    pending.insert(event.uri.clone(), event.timestamp);
                }
            }
        });

        Self {
            sender,
            pending,
            delay,
        }
    }

    /// Create a debouncer with default delay (300ms)
    pub fn with_default_delay() -> Self {
        Self::new(Duration::from_millis(300))
    }

    /// Debounce an event for a given URI
    ///
    /// Returns immediately. The caller should check `should_execute` after
    /// waiting for the delay period to determine if the callback should run.
    pub fn debounce(&self, uri: Url) -> DebouncedTask {
        let timestamp = Instant::now();
        let event = DebouncedEvent {
            uri: uri.clone(),
            timestamp,
        };

        // Send the event (ignore errors if receiver is dropped)
        let _ = self.sender.send(event);

        DebouncedTask {
            uri,
            timestamp,
            delay: self.delay,
            pending: Arc::clone(&self.pending),
        }
    }

    /// Check if an event should execute (not superseded by a newer event)
    pub fn should_execute(&self, uri: &Url, timestamp: Instant) -> bool {
        let pending = self.pending.lock();
        match pending.get(uri) {
            Some(&last_timestamp) => last_timestamp == timestamp,
            None => false, // Event was already processed
        }
    }

    /// Cancel any pending events for a URI
    pub fn cancel(&self, uri: &Url) {
        let mut pending = self.pending.lock();
        pending.remove(uri);
    }

    /// Get the number of pending events
    pub fn pending_count(&self) -> usize {
        self.pending.lock().len()
    }
}

/// A debounced task that can be awaited
pub struct DebouncedTask {
    uri: Url,
    timestamp: Instant,
    delay: Duration,
    pending: Arc<Mutex<HashMap<Url, Instant>>>,
}

impl DebouncedTask {
    /// Wait for the debounce delay and check if this task should execute
    pub async fn wait(self) -> Option<Url> {
        sleep(self.delay).await;

        // Check if this is still the latest event
        let pending = self.pending.lock();
        match pending.get(&self.uri) {
            Some(&last_timestamp) if last_timestamp == self.timestamp => {
                // This is the latest event
                drop(pending);

                // Remove from pending
                self.pending.lock().remove(&self.uri);

                Some(self.uri)
            }
            _ => {
                // Superseded by a newer event
                None
            }
        }
    }
}

/// Manager for multiple debounced callbacks
pub struct DebouncerManager {
    debouncer: Debouncer,
}

impl DebouncerManager {
    /// Create a new debouncer manager
    pub fn new(delay: Duration) -> Self {
        Self {
            debouncer: Debouncer::new(delay),
        }
    }

    /// Create with default delay (300ms)
    pub fn with_default_delay() -> Self {
        Self {
            debouncer: Debouncer::with_default_delay(),
        }
    }

    /// Schedule a debounced callback for a URI
    ///
    /// The callback will be executed after the delay period if no new events
    /// for the same URI are received.
    pub fn schedule<F>(&self, uri: Url, callback: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let task = self.debouncer.debounce(uri);

        tokio::spawn(async move {
            if task.wait().await.is_some() {
                callback();
            }
        });
    }

    /// Schedule a debounced async callback for a URI
    pub fn schedule_async<F, Fut>(&self, uri: Url, callback: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let task = self.debouncer.debounce(uri);

        tokio::spawn(async move {
            if task.wait().await.is_some() {
                callback().await;
            }
        });
    }

    /// Cancel any pending callbacks for a URI
    pub fn cancel(&self, uri: &Url) {
        self.debouncer.cancel(uri);
    }

    /// Get the number of pending callbacks
    pub fn pending_count(&self) -> usize {
        self.debouncer.pending_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_uri(id: u32) -> Url {
        Url::parse(&format!("file:///test{}.vr", id)).unwrap()
    }

    #[tokio::test]
    async fn test_debouncer_single_event() {
        let debouncer = Debouncer::new(Duration::from_millis(100));
        let uri = create_test_uri(1);

        let task = debouncer.debounce(uri.clone());
        let result = task.wait().await;

        assert!(result.is_some());
        assert_eq!(result.unwrap(), uri);
    }

    #[tokio::test]
    async fn test_debouncer_multiple_events_same_uri() {
        let debouncer = Debouncer::new(Duration::from_millis(100));
        let uri = create_test_uri(1);

        // Fire multiple events rapidly
        let task1 = debouncer.debounce(uri.clone());
        tokio::task::yield_now().await;
        sleep(Duration::from_millis(30)).await;
        let task2 = debouncer.debounce(uri.clone());
        tokio::task::yield_now().await;
        sleep(Duration::from_millis(30)).await;
        let task3 = debouncer.debounce(uri.clone());
        tokio::task::yield_now().await;

        // Wait for all tasks concurrently
        let (result1, result2, result3) = tokio::join!(task1.wait(), task2.wait(), task3.wait());

        // Only the last task should execute
        assert!(result1.is_none());
        assert!(result2.is_none());
        assert!(result3.is_some());
    }

    #[tokio::test]
    async fn test_debouncer_different_uris() {
        let debouncer = Debouncer::new(Duration::from_millis(100));
        let uri1 = create_test_uri(1);
        let uri2 = create_test_uri(2);

        let task1 = debouncer.debounce(uri1.clone());
        let task2 = debouncer.debounce(uri2.clone());

        let result1 = task1.wait().await;
        let result2 = task2.wait().await;

        // Both should execute since they're for different URIs
        assert!(result1.is_some());
        assert!(result2.is_some());
    }

    #[tokio::test]
    async fn test_debouncer_cancel() {
        let debouncer = Debouncer::new(Duration::from_millis(100));
        let uri = create_test_uri(1);

        let task = debouncer.debounce(uri.clone());

        // Give the background task time to register the event
        sleep(Duration::from_millis(20)).await;

        // Cancel after event is registered
        debouncer.cancel(&uri);

        let result = task.wait().await;

        // Should not execute since it was cancelled
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_debouncer_manager() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let manager = DebouncerManager::new(Duration::from_millis(100));
        let uri = create_test_uri(1);
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        manager.schedule(uri.clone(), move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Wait for the debounce delay + extra time for callback execution
        sleep(Duration::from_millis(150)).await;

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_debouncer_manager_multiple_rapid() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let manager = DebouncerManager::new(Duration::from_millis(100));
        let uri = create_test_uri(1);
        let counter = Arc::new(AtomicU32::new(0));

        // Schedule multiple callbacks rapidly
        for _ in 0..5 {
            let counter_clone = Arc::clone(&counter);
            manager.schedule(uri.clone(), move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            });
            sleep(Duration::from_millis(20)).await;
        }

        // Wait for debounce delay
        sleep(Duration::from_millis(200)).await;

        // Only the last callback should have executed
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_debouncer_pending_count() {
        let debouncer = Debouncer::new(Duration::from_millis(500));
        let uri1 = create_test_uri(1);
        let uri2 = create_test_uri(2);

        // Initial count should be 0
        assert_eq!(debouncer.pending_count(), 0);

        // Add events - need to yield to let background task process
        let _task1 = debouncer.debounce(uri1);
        tokio::task::yield_now().await;
        sleep(Duration::from_millis(10)).await;
        assert!(debouncer.pending_count() > 0);

        let _task2 = debouncer.debounce(uri2);
        tokio::task::yield_now().await;
        sleep(Duration::from_millis(10)).await;
        assert_eq!(debouncer.pending_count(), 2);
    }
}
