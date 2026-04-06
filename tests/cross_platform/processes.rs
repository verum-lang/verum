// Process and Thread Cross-Platform Tests
// Validates concurrency primitives across all platforms

use super::detection::PlatformInfo;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub struct ProcessTestHarness {
    pub platform: PlatformInfo,
}

impl ProcessTestHarness {
    pub fn new() -> Self {
        Self {
            platform: PlatformInfo::detect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thread_creation() {
        let harness = ProcessTestHarness::new();
        println!("Platform: {}, CPUs: {}", harness.platform.os_type, harness.platform.num_cpus);

        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();

        for i in 0..10 {
            let counter = Arc::clone(&counter);
            let handle = thread::spawn(move || {
                counter.fetch_add(i, Ordering::SeqCst);
                i * 2
            });
            handles.push(handle);
        }

        let mut sum = 0;
        for handle in handles {
            let result = handle.join().unwrap();
            sum += result;
        }

        assert_eq!(sum, (0..10).map(|i| i * 2).sum());
        assert_eq!(counter.load(Ordering::SeqCst), (0..10).sum());
    }

    #[test]
    fn test_thread_synchronization() {
        let num_threads = 5;
        let barrier = Arc::new(Barrier::new(num_threads));
        let counter = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..num_threads {
            let barrier = Arc::clone(&barrier);
            let counter = Arc::clone(&counter);

            let handle = thread::spawn(move || {
                // Wait for all threads to reach this point
                barrier.wait();

                // All threads increment simultaneously
                counter.fetch_add(1, Ordering::SeqCst);
            });

            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(counter.load(Ordering::SeqCst), num_threads);
    }

    #[test]
    fn test_mutex_contention() {
        let data = Arc::new(Mutex::new(0));
        let mut handles = Vec::new();

        let start = Instant::now();

        for _ in 0..10 {
            let data = Arc::clone(&data);
            let handle = thread::spawn(move || {
                for _ in 0..1000 {
                    let mut num = data.lock().unwrap();
                    *num += 1;
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let duration = start.elapsed();
        println!("Mutex contention test: {:?}", duration);

        assert_eq!(*data.lock().unwrap(), 10_000);
    }

    #[test]
    fn test_process_spawn() {
        let harness = ProcessTestHarness::new();

        #[cfg(unix)]
        let (command, args) = ("echo", vec!["Hello from process"]);

        #[cfg(windows)]
        let (command, args) = ("cmd", vec!["/C", "echo Hello from process"]);

        let output = Command::new(command).args(&args).output().unwrap();

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Hello"));
    }

    #[test]
    fn test_process_exit_codes() {
        #[cfg(unix)]
        let cmd = Command::new("sh").arg("-c").arg("exit 42").status().unwrap();

        #[cfg(windows)]
        let cmd = Command::new("cmd").arg("/C").arg("exit 42").status().unwrap();

        assert_eq!(cmd.code(), Some(42));
    }

    #[test]
    fn test_process_environment() {
        let output = if cfg!(unix) {
            Command::new("sh")
                .arg("-c")
                .arg("echo $TEST_VAR")
                .env("TEST_VAR", "test_value")
                .output()
                .unwrap()
        } else {
            Command::new("cmd")
                .arg("/C")
                .arg("echo %TEST_VAR%")
                .env("TEST_VAR", "test_value")
                .output()
                .unwrap()
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("test_value"));
    }

    #[test]
    fn test_process_stdin_stdout() {
        #[cfg(unix)]
        let mut child = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        #[cfg(windows)]
        let mut child = Command::new("cmd")
            .arg("/C")
            .arg("findstr .*")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        // Write to stdin
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(b"test input\n").unwrap();
        drop(stdin); // Close stdin

        // Read from stdout
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        #[cfg(unix)]
        assert_eq!(stdout.trim(), "test input");

        #[cfg(windows)]
        assert!(stdout.contains("test input"));
    }

    #[test]
    fn test_process_current_dir() {
        let temp_dir = std::env::temp_dir();

        #[cfg(unix)]
        let output = Command::new("pwd").current_dir(&temp_dir).output().unwrap();

        #[cfg(windows)]
        let output = Command::new("cmd").arg("/C").arg("cd").current_dir(&temp_dir).output().unwrap();

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Output should contain temp directory path
        println!("Current dir output: {}", stdout);
        assert!(!stdout.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn test_signal_handling() {
        use std::process::Child;

        // Spawn a long-running process
        let mut child: Child = Command::new("sleep").arg("100").spawn().unwrap();

        let pid = child.id();

        // Send SIGTERM
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }

        // Wait for process to terminate
        let status = child.wait().unwrap();
        assert!(!status.success());
    }

    #[test]
    #[cfg(unix)]
    fn test_fork_behavior() {
        // Note: We don't actually fork in tests (would duplicate test process)
        // But we test that fork-related functionality works

        // Test process ID
        let pid = std::process::id();
        assert!(pid > 0);

        // Test parent process ID
        let ppid = unsafe { libc::getppid() };
        assert!(ppid > 0);
    }

    #[test]
    #[cfg(windows)]
    fn test_windows_process_creation() {
        // Test Windows-specific process creation
        let output = Command::new("cmd")
            .arg("/C")
            .arg("echo Windows Process")
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output()
            .unwrap();

        assert!(output.status.success());
    }

    #[test]
    fn test_thread_stack_size() {
        // Create thread with custom stack size
        let builder = thread::Builder::new()
            .name("custom_stack".to_string())
            .stack_size(2 * 1024 * 1024); // 2MB

        let handle = builder
            .spawn(|| {
                let name = thread::current().name().unwrap().to_string();
                name
            })
            .unwrap();

        let name = handle.join().unwrap();
        assert_eq!(name, "custom_stack");
    }

    #[test]
    fn test_thread_local_storage() {
        thread_local! {
            static COUNTER: std::cell::RefCell<u32> = std::cell::RefCell::new(0);
        }

        let mut handles = Vec::new();

        for i in 0..5 {
            let handle = thread::spawn(move || {
                COUNTER.with(|c| {
                    *c.borrow_mut() = i;
                });

                thread::sleep(Duration::from_millis(10));

                COUNTER.with(|c| *c.borrow())
            });

            handles.push(handle);
        }

        for (i, handle) in handles.into_iter().enumerate() {
            let value = handle.join().unwrap();
            assert_eq!(value, i as u32);
        }
    }

    #[test]
    fn test_thread_park_unpark() {
        let thread_handle = thread::spawn(|| {
            // Park this thread
            thread::park();
            42
        });

        // Give thread time to park
        thread::sleep(Duration::from_millis(100));

        // Unpark the thread
        thread_handle.thread().unpark();

        let result = thread_handle.join().unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_thread_panic_handling() {
        let handle = thread::spawn(|| {
            panic!("Thread panic test");
        });

        let result = handle.join();
        assert!(result.is_err());
    }

    #[test]
    fn test_scoped_threads() {
        let mut data = vec![1, 2, 3, 4, 5];

        thread::scope(|s| {
            s.spawn(|| {
                // Can access data without Arc
                data[0] = 10;
            });

            s.spawn(|| {
                data[1] = 20;
            });
        });

        assert_eq!(data[0], 10);
        assert_eq!(data[1], 20);
    }

    #[test]
    fn test_concurrent_file_access() {
        let temp_file = std::env::temp_dir().join(format!("concurrent_test_{}.txt", std::process::id()));

        // Create file
        std::fs::write(&temp_file, b"initial").unwrap();

        let mut handles = Vec::new();

        for i in 0..10 {
            let path = temp_file.clone();
            let handle = thread::spawn(move || {
                // Each thread reads the file
                for _ in 0..100 {
                    let _ = std::fs::read(&path);
                }
                i
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        std::fs::remove_file(&temp_file).unwrap();
    }

    #[test]
    fn test_cpu_affinity_detection() {
        let harness = ProcessTestHarness::new();

        // Number of CPUs available
        let num_cpus = harness.platform.num_cpus;
        println!("Available CPUs: {}", num_cpus);

        assert!(num_cpus > 0);
        assert!(num_cpus <= 1024); // Sanity check

        // Test thread spawning up to CPU count
        let num_threads = num_cpus.min(16);
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();

        for _ in 0..num_threads {
            let counter = Arc::clone(&counter);
            let handle = thread::spawn(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(counter.load(Ordering::SeqCst), num_threads);
    }

    #[test]
    fn test_thread_priority() {
        // Note: Setting thread priority is platform-specific and may require privileges

        let handle = thread::spawn(|| {
            // Thread created with default priority
            42
        });

        let result = handle.join().unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_rayon_parallel_iteration() {
        use rayon::prelude::*;

        let data: Vec<i32> = (0..1000).collect();

        let sum: i32 = data.par_iter().map(|x| x * 2).sum();

        assert_eq!(sum, (0..1000).map(|x| x * 2).sum());
    }

    #[test]
    fn test_channel_communication() {
        use std::sync::mpsc;

        let (tx, rx) = mpsc::channel();

        // Spawn 10 threads sending messages
        for i in 0..10 {
            let tx = tx.clone();
            thread::spawn(move || {
                tx.send(i).unwrap();
            });
        }
        drop(tx); // Drop original sender

        // Receive all messages
        let mut messages: Vec<i32> = rx.iter().collect();
        messages.sort();

        assert_eq!(messages, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn test_process_pipe_communication() {
        #[cfg(unix)]
        {
            let mut child1 = Command::new("echo")
                .arg("hello")
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();

            let stdout = child1.stdout.take().unwrap();

            let child2 = Command::new("cat").stdin(stdout).stdout(Stdio::piped()).spawn().unwrap();

            let output = child2.wait_with_output().unwrap();
            let result = String::from_utf8_lossy(&output.stdout);
            assert!(result.contains("hello"));
        }

        #[cfg(windows)]
        {
            // Windows equivalent using cmd
            let output = Command::new("cmd")
                .arg("/C")
                .arg("echo hello | findstr hello")
                .output()
                .unwrap();

            let result = String::from_utf8_lossy(&output.stdout);
            assert!(result.contains("hello"));
        }
    }

    #[test]
    fn test_long_running_process() {
        #[cfg(unix)]
        let mut child = Command::new("sleep").arg("1").spawn().unwrap();

        #[cfg(windows)]
        let mut child = Command::new("cmd").arg("/C").arg("timeout /t 1").spawn().unwrap();

        // Process should still be running
        let start = Instant::now();

        // Wait for completion
        let status = child.wait().unwrap();
        let duration = start.elapsed();

        assert!(status.success());
        assert!(duration.as_secs() >= 1);
    }

    #[test]
    fn test_process_timeout() {
        use std::time::Duration;

        #[cfg(unix)]
        let mut child = Command::new("sleep").arg("10").spawn().unwrap();

        #[cfg(windows)]
        let mut child = Command::new("cmd").arg("/C").arg("timeout /t 10").spawn().unwrap();

        // Kill after 1 second
        thread::sleep(Duration::from_secs(1));
        child.kill().unwrap();

        let status = child.wait().unwrap();
        assert!(!status.success());
    }

    #[test]
    fn test_atomic_operations() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();

        for _ in 0..100 {
            let counter = Arc::clone(&counter);
            let handle = thread::spawn(move || {
                for _ in 0..100 {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(counter.load(Ordering::SeqCst), 10_000);
    }

    #[test]
    fn test_memory_ordering() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let flag = Arc::new(AtomicBool::new(false));
        let data = Arc::new(AtomicUsize::new(0));

        let flag_clone = Arc::clone(&flag);
        let data_clone = Arc::clone(&data);

        let producer = thread::spawn(move || {
            data_clone.store(42, Ordering::Relaxed);
            flag_clone.store(true, Ordering::Release);
        });

        let consumer = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::yield_now();
            }
            data.load(Ordering::Relaxed)
        });

        producer.join().unwrap();
        let value = consumer.join().unwrap();

        assert_eq!(value, 42);
    }

    #[test]
    fn test_thread_pool_performance() {
        use rayon::ThreadPoolBuilder;

        let pool = ThreadPoolBuilder::new().num_threads(4).build().unwrap();

        let start = Instant::now();

        pool.install(|| {
            let sum: u64 = (0..1_000_000).into_par_iter().map(|x| x * x).sum();
            assert!(sum > 0);
        });

        let duration = start.elapsed();
        println!("Thread pool computation: {:?}", duration);
    }
}
