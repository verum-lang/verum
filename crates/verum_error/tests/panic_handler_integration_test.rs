#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Integration tests for panic_handler module
//!
//! These tests validate that the panic handling infrastructure works correctly.

use serial_test::serial;
use verum_common::{Maybe, Text};
use verum_error::panic_handler::{
    PanicData, PanicLogger, catch_task_panics, clear_panic_statistics, get_panic_statistics,
    panic_logger, setup_panic_hook,
};
use verum_error::unified::VerumError;

#[test]
fn test_panic_logger_basic() {
    let logger = PanicLogger::new();
    assert_eq!(logger.total_panics(), 0);

    let panic_data = PanicData {
        message: Text::from("Test panic"),
        location: Maybe::Some(Text::from("test.rs:10:5")),
        backtrace: Maybe::None,
        thread_name: Text::from("main"),
        timestamp_ms: 1000,
    };

    logger.log_panic(panic_data);
    assert_eq!(logger.total_panics(), 1);
}

#[test]
fn test_catch_task_panics_success() {
    let result = catch_task_panics(|| 42);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 42);
}

#[test]
#[serial]
fn test_catch_task_panics_with_panic() {
    // Clear statistics before test
    clear_panic_statistics();

    let result = catch_task_panics(|| -> i32 {
        panic!("Test panic message");
    });

    assert!(result.is_err());

    match result {
        Err(VerumError::TaskPanicked { message }) => {
            assert!(message.contains("Test panic message"));
        }
        _ => panic!("Expected TaskPanicked error"),
    }

    // Check that panic was logged
    let stats = get_panic_statistics();
    assert!(stats.total_panics > 0);
}

#[test]
fn test_panic_statistics() {
    let logger = PanicLogger::new();

    for i in 0..5 {
        logger.log_panic(PanicData {
            message: Text::from(format!("Panic {}", i)),
            location: Maybe::Some(Text::from(format!("file{}.rs:10:5", i % 2))),
            backtrace: Maybe::None,
            thread_name: Text::from("worker"),
            timestamp_ms: 1000 + i,
        });
    }

    let stats = logger.statistics();
    assert_eq!(stats.total_panics, 5);
    assert!(matches!(stats.first_panic_timestamp, Maybe::Some(1000)));
    assert!(matches!(stats.last_panic_timestamp, Maybe::Some(1004)));

    let top_locations = stats.top_panic_locations(10);
    assert_eq!(top_locations.len(), 2); // file0.rs and file1.rs
}

#[test]
fn test_panic_history_bounded() {
    let logger = PanicLogger::with_config(3, false, false);

    for i in 0..10 {
        logger.log_panic(PanicData {
            message: Text::from(format!("Panic {}", i)),
            location: Maybe::None,
            backtrace: Maybe::None,
            thread_name: Text::from("main"),
            timestamp_ms: 1000 + i,
        });
    }

    let history = logger.panic_history();
    assert_eq!(history.len(), 3); // Only last 3 panics
}

#[test]
fn test_panic_data_conversion_to_error() {
    let panic_data = PanicData {
        message: Text::from("Test error"),
        location: Maybe::None,
        backtrace: Maybe::None,
        thread_name: Text::from("main"),
        timestamp_ms: 1000,
    };

    let error = panic_data.to_error();
    match error {
        VerumError::TaskPanicked { message } => {
            assert_eq!(message, Text::from("Test error"));
        }
        _ => panic!("Expected TaskPanicked variant"),
    }
}

#[test]
fn test_panic_statistics_rate_calculation() {
    let logger = PanicLogger::new();

    // Log panics at known timestamps
    logger.log_panic(PanicData {
        message: Text::from("First"),
        location: Maybe::None,
        backtrace: Maybe::None,
        thread_name: Text::from("main"),
        timestamp_ms: 1000,
    });

    logger.log_panic(PanicData {
        message: Text::from("Second"),
        location: Maybe::None,
        backtrace: Maybe::None,
        thread_name: Text::from("main"),
        timestamp_ms: 3000, // 2 seconds later
    });

    let stats = logger.statistics();

    // Should have mean time between panics
    if let Maybe::Some(mean_time) = stats.mean_time_between_panics_ms {
        assert_eq!(mean_time, 2000.0);
    } else {
        panic!("Expected mean time calculation");
    }

    // Should have panic rate
    // 2 panics over 2 second interval = 1 panic/second, or
    // 1 interval over 2 seconds = 0.5 intervals/second
    // Calculation depends on implementation, so we just verify it's reasonable
    if let Maybe::Some(rate) = stats.panic_rate_per_second {
        // Rate should be positive and reasonable (between 0.1 and 2.0)
        assert!(
            rate > 0.1 && rate < 2.0,
            "Panic rate {} is out of expected range",
            rate
        );
    } else {
        panic!("Expected panic rate calculation");
    }
}

#[test]
fn test_panic_logger_thread_statistics() {
    let logger = PanicLogger::new();

    for i in 0..5 {
        logger.log_panic(PanicData {
            message: Text::from("Test"),
            location: Maybe::None,
            backtrace: Maybe::None,
            thread_name: Text::from(format!("thread-{}", i % 2)),
            timestamp_ms: 1000 + i,
        });
    }

    let stats = logger.statistics();
    let top_threads = stats.top_panic_threads(10);
    assert_eq!(top_threads.len(), 2); // thread-0 and thread-1
}

#[test]
fn test_panic_data_format() {
    let panic_data = PanicData {
        message: Text::from("Critical failure"),
        location: Maybe::Some(Text::from("main.rs:100:20")),
        backtrace: Maybe::Some(Text::from("stack trace here")),
        thread_name: Text::from("async-worker-1"),
        timestamp_ms: 1234567890,
    };

    let formatted = panic_data.format_detailed();

    assert!(formatted.contains("Critical failure"));
    assert!(formatted.contains("main.rs:100:20"));
    assert!(formatted.contains("stack trace here"));
    assert!(formatted.contains("async-worker-1"));
}

#[test]
#[serial]
fn test_setup_panic_hook_does_not_crash() {
    // Should be safe to call multiple times
    setup_panic_hook();
    setup_panic_hook();

    // Clear any panics from other tests before verifying
    clear_panic_statistics();

    // Verify global logger is initialized and cleared
    let logger = panic_logger();
    assert_eq!(logger.total_panics(), 0);
}
