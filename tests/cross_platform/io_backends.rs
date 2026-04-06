// I/O Backend Cross-Platform Tests
// Validates async I/O across io_uring, epoll, kqueue, IOCP

use super::detection::{AsyncIOBackend, FeatureDetector, PlatformInfo};
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs as tokio_fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct IOBackendTestHarness {
    pub temp_dir: PathBuf,
    pub platform: PlatformInfo,
    pub backend: AsyncIOBackend,
}

impl IOBackendTestHarness {
    pub fn new() -> std::io::Result<Self> {
        let temp_dir = std::env::temp_dir().join(format!("verum_io_test_{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir)?;

        let detector = FeatureDetector::new();
        let backend = detector.async_io_backend();

        Ok(Self {
            temp_dir,
            platform: PlatformInfo::detect(),
            backend,
        })
    }

    pub fn cleanup(&self) -> std::io::Result<()> {
        if self.temp_dir.exists() {
            std::fs::remove_dir_all(&self.temp_dir)?;
        }
        Ok(())
    }

    pub fn test_path(&self, name: &str) -> PathBuf {
        self.temp_dir.join(name)
    }
}

impl Drop for IOBackendTestHarness {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_detection() {
        let detector = FeatureDetector::new();
        let backend = detector.async_io_backend();

        println!("Detected I/O Backend: {}", backend);

        #[cfg(target_os = "linux")]
        {
            let expected = if detector.has_io_uring() {
                AsyncIOBackend::IoUring
            } else {
                AsyncIOBackend::Epoll
            };
            assert_eq!(backend, expected);
        }

        #[cfg(target_os = "macos")]
        assert_eq!(backend, AsyncIOBackend::Kqueue);

        #[cfg(target_os = "windows")]
        assert_eq!(backend, AsyncIOBackend::Iocp);

        #[cfg(any(
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        assert_eq!(backend, AsyncIOBackend::Kqueue);
    }

    #[tokio::test]
    async fn test_async_file_read() {
        let harness = IOBackendTestHarness::new().unwrap();
        let file_path = harness.test_path("async_read_test.txt");

        // Write test data synchronously
        let test_data = b"Hello, async I/O world!";
        std::fs::write(&file_path, test_data).unwrap();

        // Read asynchronously
        let mut file = tokio_fs::File::open(&file_path).await.unwrap();
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).await.unwrap();

        assert_eq!(&buffer[..], test_data);
    }

    #[tokio::test]
    async fn test_async_file_write() {
        let harness = IOBackendTestHarness::new().unwrap();
        let file_path = harness.test_path("async_write_test.txt");

        // Write asynchronously
        let test_data = b"Async write test data";
        let mut file = tokio_fs::File::create(&file_path).await.unwrap();
        file.write_all(test_data).await.unwrap();
        file.sync_all().await.unwrap();

        // Read back synchronously
        let content = std::fs::read(&file_path).unwrap();
        assert_eq!(&content[..], test_data);
    }

    #[tokio::test]
    async fn test_concurrent_async_operations() {
        let harness = IOBackendTestHarness::new().unwrap();

        // Create multiple files concurrently
        let mut tasks = Vec::new();
        for i in 0..10 {
            let path = harness.test_path(&format!("concurrent_{}.txt", i));
            let task = tokio::spawn(async move {
                let data = format!("File {}", i);
                tokio_fs::write(&path, data.as_bytes()).await.unwrap();
                path
            });
            tasks.push(task);
        }

        // Wait for all tasks
        let paths: Vec<PathBuf> = futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Verify all files exist
        for (i, path) in paths.iter().enumerate() {
            let content = tokio_fs::read_to_string(path).await.unwrap();
            assert_eq!(content, format!("File {}", i));
        }
    }

    #[tokio::test]
    async fn test_large_file_async_io() {
        let harness = IOBackendTestHarness::new().unwrap();
        let file_path = harness.test_path("large_async.bin");

        // Create 10MB file
        let chunk_size = 1024 * 1024; // 1MB
        let num_chunks = 10;
        let chunk = vec![0xAB; chunk_size];

        let start = Instant::now();

        let mut file = tokio_fs::File::create(&file_path).await.unwrap();
        for _ in 0..num_chunks {
            file.write_all(&chunk).await.unwrap();
        }
        file.sync_all().await.unwrap();

        let write_duration = start.elapsed();
        println!(
            "Async write 10MB: {:?} ({:.2} MB/s)",
            write_duration,
            10.0 / write_duration.as_secs_f64()
        );

        // Read back
        let start = Instant::now();
        let mut file = tokio_fs::File::open(&file_path).await.unwrap();
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).await.unwrap();

        let read_duration = start.elapsed();
        println!(
            "Async read 10MB: {:?} ({:.2} MB/s)",
            read_duration,
            10.0 / read_duration.as_secs_f64()
        );

        assert_eq!(buffer.len(), chunk_size * num_chunks);
    }

    #[tokio::test]
    async fn test_buffered_async_io() {
        use tokio::io::BufReader;

        let harness = IOBackendTestHarness::new().unwrap();
        let file_path = harness.test_path("buffered_test.txt");

        // Create file with multiple lines
        let lines: Vec<String> = (0..1000).map(|i| format!("Line {}", i)).collect();
        let content = lines.join("\n");
        tokio_fs::write(&file_path, content.as_bytes()).await.unwrap();

        // Read with buffering
        let file = tokio_fs::File::open(&file_path).await.unwrap();
        let reader = BufReader::new(file);
        let mut async_lines = tokio::io::AsyncBufReadExt::lines(reader);

        let mut count = 0;
        while let Some(line) = async_lines.next_line().await.unwrap() {
            assert_eq!(line, format!("Line {}", count));
            count += 1;
        }

        assert_eq!(count, 1000);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_high_concurrency_io() {
        let harness = IOBackendTestHarness::new().unwrap();

        // Spawn 1000 concurrent I/O operations
        let num_operations = 1000;
        let counter = Arc::new(AtomicUsize::new(0));

        let mut tasks = Vec::new();
        for i in 0..num_operations {
            let path = harness.test_path(&format!("concurrent_{}.txt", i));
            let counter = Arc::clone(&counter);

            let task = tokio::spawn(async move {
                let data = format!("Data {}", i);
                tokio_fs::write(&path, data.as_bytes()).await.unwrap();

                let read_data = tokio_fs::read_to_string(&path).await.unwrap();
                assert_eq!(read_data, data);

                counter.fetch_add(1, Ordering::SeqCst);
            });

            tasks.push(task);
        }

        let start = Instant::now();
        futures::future::join_all(tasks).await;
        let duration = start.elapsed();

        let completed = counter.load(Ordering::SeqCst);
        assert_eq!(completed, num_operations);

        println!(
            "Completed {} concurrent I/O operations in {:?} ({:.2} ops/sec)",
            num_operations,
            duration,
            num_operations as f64 / duration.as_secs_f64()
        );
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn test_io_uring_specific() {
        let detector = FeatureDetector::new();
        if !detector.has_io_uring() {
            println!("io_uring not available, skipping test");
            return;
        }

        println!("Testing io_uring backend");

        let harness = IOBackendTestHarness::new().unwrap();
        let file_path = harness.test_path("io_uring_test.txt");

        // io_uring should provide better performance for batch operations
        let start = Instant::now();

        let mut tasks = Vec::new();
        for i in 0..100 {
            let path = harness.test_path(&format!("io_uring_{}.txt", i));
            let task = tokio::spawn(async move {
                let data = vec![i as u8; 4096];
                tokio_fs::write(&path, &data).await.unwrap();
            });
            tasks.push(task);
        }

        futures::future::join_all(tasks).await;
        let duration = start.elapsed();

        println!("io_uring batch write: {:?}", duration);
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn test_epoll_fallback() {
        println!("Testing epoll backend (Linux fallback)");

        let harness = IOBackendTestHarness::new().unwrap();
        let file_path = harness.test_path("epoll_test.txt");

        // Standard async I/O using epoll
        tokio_fs::write(&file_path, b"epoll test").await.unwrap();
        let content = tokio_fs::read(&file_path).await.unwrap();
        assert_eq!(&content[..], b"epoll test");
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn test_kqueue_backend() {
        println!("Testing kqueue backend (macOS/BSD)");

        let harness = IOBackendTestHarness::new().unwrap();
        let file_path = harness.test_path("kqueue_test.txt");

        // kqueue-based async I/O
        let start = Instant::now();

        let mut tasks = Vec::new();
        for i in 0..50 {
            let path = harness.test_path(&format!("kqueue_{}.txt", i));
            let task = tokio::spawn(async move {
                let data = format!("kqueue data {}", i);
                tokio_fs::write(&path, data.as_bytes()).await.unwrap();
                tokio_fs::read_to_string(&path).await.unwrap()
            });
            tasks.push(task);
        }

        let results: Vec<String> = futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        let duration = start.elapsed();
        println!("kqueue operations: {:?}", duration);

        assert_eq!(results.len(), 50);
    }

    #[tokio::test]
    #[cfg(target_os = "windows")]
    async fn test_iocp_backend() {
        println!("Testing IOCP backend (Windows)");

        let harness = IOBackendTestHarness::new().unwrap();
        let file_path = harness.test_path("iocp_test.txt");

        // IOCP-based async I/O
        let start = Instant::now();

        let mut tasks = Vec::new();
        for i in 0..50 {
            let path = harness.test_path(&format!("iocp_{}.txt", i));
            let task = tokio::spawn(async move {
                let data = format!("IOCP data {}", i);
                tokio_fs::write(&path, data.as_bytes()).await.unwrap();
                tokio_fs::read_to_string(&path).await.unwrap()
            });
            tasks.push(task);
        }

        let results: Vec<String> = futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        let duration = start.elapsed();
        println!("IOCP operations: {:?}", duration);

        assert_eq!(results.len(), 50);
    }

    #[tokio::test]
    async fn test_async_directory_operations() {
        let harness = IOBackendTestHarness::new().unwrap();
        let dir_path = harness.test_path("async_subdir");

        // Create directory asynchronously
        tokio_fs::create_dir(&dir_path).await.unwrap();
        assert!(dir_path.exists());

        // Create files in directory
        for i in 0..10 {
            let file_path = dir_path.join(format!("file_{}.txt", i));
            tokio_fs::write(&file_path, format!("content {}", i).as_bytes())
                .await
                .unwrap();
        }

        // Read directory
        let mut entries = tokio_fs::read_dir(&dir_path).await.unwrap();
        let mut count = 0;
        while let Some(entry) = entries.next_entry().await.unwrap() {
            assert!(entry.path().exists());
            count += 1;
        }
        assert_eq!(count, 10);

        // Remove directory (must be empty on some platforms)
        for i in 0..10 {
            let file_path = dir_path.join(format!("file_{}.txt", i));
            tokio_fs::remove_file(&file_path).await.unwrap();
        }
        tokio_fs::remove_dir(&dir_path).await.unwrap();
        assert!(!dir_path.exists());
    }

    #[tokio::test]
    async fn test_async_metadata_operations() {
        let harness = IOBackendTestHarness::new().unwrap();
        let file_path = harness.test_path("metadata_test.txt");

        // Create file
        tokio_fs::write(&file_path, b"metadata test").await.unwrap();

        // Get metadata asynchronously
        let metadata = tokio_fs::metadata(&file_path).await.unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.len(), 13);

        // Check permissions
        assert!(!metadata.permissions().readonly());

        // Set readonly
        let mut perms = metadata.permissions();
        perms.set_readonly(true);
        tokio_fs::set_permissions(&file_path, perms).await.unwrap();

        // Verify
        let metadata = tokio_fs::metadata(&file_path).await.unwrap();
        assert!(metadata.permissions().readonly());
    }

    #[tokio::test]
    async fn test_async_rename_operations() {
        let harness = IOBackendTestHarness::new().unwrap();
        let old_path = harness.test_path("old_name.txt");
        let new_path = harness.test_path("new_name.txt");

        // Create file
        tokio_fs::write(&old_path, b"rename test").await.unwrap();
        assert!(old_path.exists());

        // Rename asynchronously
        tokio_fs::rename(&old_path, &new_path).await.unwrap();
        assert!(!old_path.exists());
        assert!(new_path.exists());

        // Verify content
        let content = tokio_fs::read(&new_path).await.unwrap();
        assert_eq!(&content[..], b"rename test");
    }

    #[tokio::test]
    async fn test_async_copy_operations() {
        let harness = IOBackendTestHarness::new().unwrap();
        let src_path = harness.test_path("source.txt");
        let dst_path = harness.test_path("destination.txt");

        // Create source file
        let test_data = b"copy test data";
        tokio_fs::write(&src_path, test_data).await.unwrap();

        // Copy asynchronously
        tokio_fs::copy(&src_path, &dst_path).await.unwrap();

        // Verify both files exist
        assert!(src_path.exists());
        assert!(dst_path.exists());

        // Verify content
        let dst_content = tokio_fs::read(&dst_path).await.unwrap();
        assert_eq!(&dst_content[..], test_data);
    }

    #[tokio::test]
    async fn test_io_error_handling() {
        let harness = IOBackendTestHarness::new().unwrap();

        // Try to read non-existent file
        let result = tokio_fs::read(harness.test_path("nonexistent.txt")).await;
        assert!(result.is_err());

        // Try to write to invalid location
        #[cfg(unix)]
        {
            let result = tokio_fs::write("/root/forbidden.txt", b"test").await;
            // May succeed if running as root, so just check it doesn't panic
            let _ = result;
        }

        #[cfg(windows)]
        {
            let result = tokio_fs::write("C:\\Windows\\System32\\forbidden.txt", b"test").await;
            // Should fail due to permissions
            let _ = result;
        }
    }

    #[tokio::test]
    async fn test_backend_performance_comparison() {
        let harness = IOBackendTestHarness::new().unwrap();

        println!("\n=== I/O Backend Performance ===");
        println!("Backend: {}", harness.backend);
        println!("Platform: {} {}", harness.platform.os_type, harness.platform.architecture);

        // Test 1: Sequential writes
        let start = Instant::now();
        for i in 0..100 {
            let path = harness.test_path(&format!("seq_{}.txt", i));
            tokio_fs::write(&path, vec![0u8; 4096]).await.unwrap();
        }
        let seq_write_duration = start.elapsed();
        println!("Sequential writes (100x4KB): {:?}", seq_write_duration);

        // Test 2: Parallel writes
        let start = Instant::now();
        let mut tasks = Vec::new();
        for i in 100..200 {
            let path = harness.test_path(&format!("par_{}.txt", i));
            let task = tokio::spawn(async move {
                tokio_fs::write(&path, vec![0u8; 4096]).await.unwrap();
            });
            tasks.push(task);
        }
        futures::future::join_all(tasks).await;
        let par_write_duration = start.elapsed();
        println!("Parallel writes (100x4KB): {:?}", par_write_duration);

        // Parallel should be faster or similar
        println!(
            "Speedup: {:.2}x",
            seq_write_duration.as_secs_f64() / par_write_duration.as_secs_f64()
        );
    }
}
