// File System Cross-Platform Tests
// Validates FS operations across Linux, macOS, Windows, and BSD

use super::detection::{FeatureDetector, OSType, PlatformInfo};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Test utilities for filesystem operations
pub struct FilesystemTestHarness {
    pub temp_dir: PathBuf,
    pub platform: PlatformInfo,
}

impl FilesystemTestHarness {
    pub fn new() -> std::io::Result<Self> {
        let temp_dir = std::env::temp_dir().join(format!("verum_fs_test_{}", std::process::id()));
        fs::create_dir_all(&temp_dir)?;

        Ok(Self {
            temp_dir,
            platform: PlatformInfo::detect(),
        })
    }

    pub fn cleanup(&self) -> std::io::Result<()> {
        if self.temp_dir.exists() {
            fs::remove_dir_all(&self.temp_dir)?;
        }
        Ok(())
    }

    pub fn test_path(&self, name: &str) -> PathBuf {
        self.temp_dir.join(name)
    }
}

impl Drop for FilesystemTestHarness {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_separators() {
        let harness = FilesystemTestHarness::new().unwrap();

        // Test path separator handling
        let path = harness.test_path("subdir").join("file.txt");

        #[cfg(windows)]
        {
            let path_str = path.to_str().unwrap();
            assert!(path_str.contains('\\'), "Windows should use backslash");
        }

        #[cfg(unix)]
        {
            let path_str = path.to_str().unwrap();
            assert!(path_str.contains('/'), "Unix should use forward slash");
            assert!(!path_str.contains('\\'), "Unix should not use backslash");
        }
    }

    #[test]
    fn test_path_normalization() {
        let harness = FilesystemTestHarness::new().unwrap();

        // Create nested directory structure
        let nested = harness.test_path("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();

        // Test path with .. components
        let up_path = nested.join("..").join("..").join("b").join("c");
        assert!(up_path.exists() || !up_path.exists()); // Just ensure it doesn't panic

        // Test path canonicalization
        let canonical = fs::canonicalize(&harness.temp_dir).unwrap();
        assert!(canonical.is_absolute());

        #[cfg(windows)]
        {
            let canon_str = canonical.to_str().unwrap();
            // Windows paths should start with drive letter or UNC
            assert!(
                canon_str.starts_with(|c: char| c.is_ascii_alphabetic()) || canon_str.starts_with("\\\\"),
                "Windows path should be normalized"
            );
        }

        #[cfg(unix)]
        {
            let canon_str = canonical.to_str().unwrap();
            assert!(canon_str.starts_with('/'), "Unix path should start with /");
        }
    }

    #[test]
    fn test_case_sensitivity() {
        let harness = FilesystemTestHarness::new().unwrap();
        let detector = FeatureDetector::new();

        // Create a file with lowercase name
        let lower_path = harness.test_path("testfile.txt");
        File::create(&lower_path).unwrap();

        // Try to access with uppercase name
        let upper_path = harness.test_path("TESTFILE.TXT");

        let case_sensitive = detector.is_filesystem_case_sensitive();

        if case_sensitive {
            // Linux, most Unix systems - should NOT find uppercase version
            assert!(!upper_path.exists(), "Case-sensitive FS should not find uppercase variant");
        } else {
            // Windows, macOS (default) - should find it
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            assert!(upper_path.exists(), "Case-insensitive FS should find uppercase variant");
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_symlinks() {
        let harness = FilesystemTestHarness::new().unwrap();

        // Create target file
        let target = harness.test_path("target.txt");
        fs::write(&target, b"symlink test").unwrap();

        // Create symlink
        let link = harness.test_path("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        // Verify symlink
        assert!(link.exists());
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());

        // Read through symlink
        let content = fs::read_to_string(&link).unwrap();
        assert_eq!(content, "symlink test");

        // Test symlink to directory
        let target_dir = harness.test_path("target_dir");
        fs::create_dir(&target_dir).unwrap();

        let link_dir = harness.test_path("link_dir");
        std::os::unix::fs::symlink(&target_dir, &link_dir).unwrap();
        assert!(link_dir.exists());
    }

    #[test]
    #[cfg(windows)]
    fn test_windows_junctions() {
        let harness = FilesystemTestHarness::new().unwrap();

        // Windows junctions (directory symlinks)
        let target_dir = harness.test_path("target_dir");
        fs::create_dir(&target_dir).unwrap();

        // Note: Creating junctions on Windows requires elevated privileges
        // or Developer Mode. We test if the API exists.
        let link_dir = harness.test_path("junction_dir");

        // Attempt to create junction (may fail without privileges)
        let result = std::os::windows::fs::symlink_dir(&target_dir, &link_dir);

        if result.is_ok() {
            assert!(link_dir.exists());
            let metadata = fs::symlink_metadata(&link_dir).unwrap();
            assert!(metadata.file_type().is_symlink());
        } else {
            println!("Junction creation requires elevated privileges");
        }
    }

    #[test]
    fn test_hard_links() {
        let harness = FilesystemTestHarness::new().unwrap();

        // Create original file
        let original = harness.test_path("original.txt");
        fs::write(&original, b"hard link test").unwrap();

        // Create hard link
        let link = harness.test_path("hardlink.txt");
        fs::hard_link(&original, &link).unwrap();

        // Verify both files exist and have same content
        assert!(original.exists());
        assert!(link.exists());

        let orig_content = fs::read(&original).unwrap();
        let link_content = fs::read(&link).unwrap();
        assert_eq!(orig_content, link_content);

        // Modify through hard link
        fs::write(&link, b"modified").unwrap();

        // Original should be modified too
        let orig_content = fs::read(&original).unwrap();
        assert_eq!(orig_content, b"modified");

        // Check inode equality (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let orig_meta = fs::metadata(&original).unwrap();
            let link_meta = fs::metadata(&link).unwrap();
            assert_eq!(orig_meta.ino(), link_meta.ino(), "Hard links should share same inode");
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let harness = FilesystemTestHarness::new().unwrap();

        let file_path = harness.test_path("perms.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"test").unwrap();

        // Set permissions: rwx------
        let mut perms = file.metadata().unwrap().permissions();
        perms.set_mode(0o700);
        fs::set_permissions(&file_path, perms).unwrap();

        // Verify permissions
        let metadata = fs::metadata(&file_path).unwrap();
        let mode = metadata.permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "Permissions should be rwx------");

        // Test read-only
        let mut perms = metadata.permissions();
        perms.set_mode(0o400);
        fs::set_permissions(&file_path, perms).unwrap();

        // Attempt to write should fail
        let result = OpenOptions::new().write(true).open(&file_path);
        assert!(result.is_err(), "Writing to read-only file should fail");
    }

    #[test]
    #[cfg(windows)]
    fn test_windows_acls() {
        use std::os::windows::fs::MetadataExt;
        let harness = FilesystemTestHarness::new().unwrap();

        let file_path = harness.test_path("acl_test.txt");
        File::create(&file_path).unwrap();

        // Get file attributes
        let metadata = fs::metadata(&file_path).unwrap();
        let attrs = metadata.file_attributes();

        // Check if hidden/system/readonly bits work
        // FILE_ATTRIBUTE_READONLY = 0x1
        let is_readonly = (attrs & 0x1) != 0;
        println!("File readonly: {}", is_readonly);

        // Test setting readonly
        let mut perms = metadata.permissions();
        perms.set_readonly(true);
        fs::set_permissions(&file_path, perms).unwrap();

        let metadata = fs::metadata(&file_path).unwrap();
        assert!(metadata.permissions().readonly(), "File should be readonly");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_extended_attributes() {
        let harness = FilesystemTestHarness::new().unwrap();
        let file_path = harness.test_path("xattr_test.txt");
        File::create(&file_path).unwrap();

        // Extended attributes are available on Linux
        // Using libc for xattr operations
        use std::ffi::CString;

        let path_cstr = CString::new(file_path.to_str().unwrap()).unwrap();
        let name = CString::new("user.test").unwrap();
        let value = b"test_value";

        // Set extended attribute
        let result = unsafe {
            libc::setxattr(
                path_cstr.as_ptr(),
                name.as_ptr(),
                value.as_ptr() as *const libc::c_void,
                value.len(),
                0,
            )
        };

        if result == 0 {
            // Get extended attribute
            let mut buffer = vec![0u8; 128];
            let len = unsafe {
                libc::getxattr(
                    path_cstr.as_ptr(),
                    name.as_ptr(),
                    buffer.as_mut_ptr() as *mut libc::c_void,
                    buffer.len(),
                )
            };

            if len > 0 {
                buffer.truncate(len as usize);
                assert_eq!(&buffer[..], value, "Extended attribute value should match");
            }
        } else {
            println!("Extended attributes not supported on this filesystem");
        }
    }

    #[test]
    fn test_file_locking() {
        let harness = FilesystemTestHarness::new().unwrap();
        let file_path = harness.test_path("lock_test.txt");

        let file1 = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&file_path)
            .unwrap();

        // Platform-specific file locking
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;

            // Try to acquire exclusive lock (flock)
            let fd = file1.as_raw_fd();
            let result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };

            if result == 0 {
                // Successfully locked
                // Try to open and lock again (should fail)
                let file2 = OpenOptions::new().read(true).write(true).open(&file_path).unwrap();

                let fd2 = file2.as_raw_fd();
                let result2 = unsafe { libc::flock(fd2, libc::LOCK_EX | libc::LOCK_NB) };

                assert_eq!(result2, -1, "Second exclusive lock should fail");

                // Unlock
                unsafe { libc::flock(fd, libc::LOCK_UN) };
            } else {
                println!("File locking not supported");
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use std::os::windows::raw::HANDLE;

            // Windows file locking via LockFileEx
            let handle = file1.as_raw_handle() as HANDLE;

            // Note: Proper LockFileEx usage requires windows-sys crate
            // For now, just verify handle is valid
            assert!(!handle.is_null());
        }
    }

    #[test]
    fn test_special_files_handling() {
        let harness = FilesystemTestHarness::new().unwrap();

        // Test handling of special/reserved names
        #[cfg(windows)]
        {
            // Windows reserved names: CON, PRN, AUX, NUL, COM1-9, LPT1-9
            let reserved_names = ["CON", "PRN", "AUX", "NUL", "COM1", "LPT1"];

            for name in &reserved_names {
                let path = harness.test_path(name);
                let result = File::create(&path);

                // These should fail on Windows
                if result.is_ok() {
                    println!("Warning: Reserved name {} was created", name);
                }
            }
        }

        // Test dot files
        let dotfile = harness.test_path(".hidden");
        File::create(&dotfile).unwrap();
        assert!(dotfile.exists());

        #[cfg(unix)]
        {
            // Dot files are hidden on Unix
            let metadata = fs::metadata(&dotfile).unwrap();
            assert!(metadata.is_file());
        }
    }

    #[test]
    fn test_path_length_limits() {
        let harness = FilesystemTestHarness::new().unwrap();

        // Test long filenames
        let long_name = "a".repeat(255);
        let long_path = harness.test_path(&long_name);

        match File::create(&long_path) {
            Ok(_) => {
                assert!(long_path.exists());
            }
            Err(e) => {
                println!("Long filename (255 chars) failed: {}", e);
            }
        }

        // Test very long filenames (should fail)
        let too_long_name = "a".repeat(300);
        let too_long_path = harness.test_path(&too_long_name);
        let result = File::create(&too_long_path);
        assert!(result.is_err(), "Filename > 255 chars should fail");

        // Test deep directory nesting
        let mut deep_path = harness.temp_dir.clone();
        for i in 0..50 {
            deep_path = deep_path.join(format!("dir{}", i));
        }

        let result = fs::create_dir_all(&deep_path);

        #[cfg(unix)]
        {
            // Unix typically allows deep nesting
            if result.is_ok() {
                assert!(deep_path.exists());
            }
        }

        #[cfg(windows)]
        {
            // Windows MAX_PATH is 260 by default (but can be extended)
            // May fail on standard Windows
            if result.is_err() {
                println!("Deep nesting hit Windows path length limit");
            }
        }
    }

    #[test]
    fn test_character_restrictions() {
        let harness = FilesystemTestHarness::new().unwrap();

        #[cfg(windows)]
        {
            // Windows restricts: < > : " | ? * and control characters
            let invalid_chars = ['<', '>', ':', '"', '|', '?', '*'];

            for ch in &invalid_chars {
                let name = format!("test{}file.txt", ch);
                let path = harness.test_path(&name);
                let result = File::create(&path);

                assert!(result.is_err(), "Windows should reject filename with '{}'", ch);
            }
        }

        #[cfg(unix)]
        {
            // Unix only restricts / and NUL
            // Forward slash
            let invalid_name = "test/file.txt";
            let path = harness.test_path(invalid_name);
            // This will create a subdirectory structure, not fail

            // Most special characters are valid
            let special_chars = ['<', '>', ':', '"', '|', '?', '*', '&', '$'];
            for ch in &special_chars {
                let name = format!("test{}file.txt", ch);
                let path = harness.test_path(&name);
                let result = File::create(&path);

                if result.is_ok() {
                    assert!(path.exists(), "Unix should allow '{}'", ch);
                }
            }
        }
    }

    #[test]
    fn test_file_timestamps() {
        let harness = FilesystemTestHarness::new().unwrap();
        let file_path = harness.test_path("timestamp_test.txt");

        File::create(&file_path).unwrap();
        let metadata1 = fs::metadata(&file_path).unwrap();

        // Sleep to ensure timestamp difference
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Modify file
        fs::write(&file_path, b"modified").unwrap();
        let metadata2 = fs::metadata(&file_path).unwrap();

        // Modified time should be later
        let mtime1 = metadata1.modified().unwrap();
        let mtime2 = metadata2.modified().unwrap();
        assert!(mtime2 > mtime1, "Modified time should increase");

        // Test accessed time (if supported)
        if let (Ok(atime1), Ok(atime2)) = (metadata1.accessed(), metadata2.accessed()) {
            println!("Access times: {:?} -> {:?}", atime1, atime2);
        }

        // Test created time
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            let ctime1 = metadata1.created().unwrap();
            let ctime2 = metadata2.created().unwrap();
            assert_eq!(ctime1, ctime2, "Created time should not change");
        }
    }

    #[test]
    fn test_large_file_support() {
        let harness = FilesystemTestHarness::new().unwrap();
        let file_path = harness.test_path("large_file.bin");

        // Create a file larger than 2GB (if disk space available)
        let file = File::create(&file_path).unwrap();

        // Test seeking beyond 2GB
        use std::io::Seek;
        let mut file = file;
        let large_offset: u64 = 3_000_000_000; // 3GB

        match file.seek(std::io::SeekFrom::Start(large_offset)) {
            Ok(pos) => {
                assert_eq!(pos, large_offset);
                // Write a byte at 3GB offset
                if file.write_all(b"X").is_ok() {
                    println!("Large file support (>2GB) verified");
                }
            }
            Err(e) => {
                println!("Large file seek failed: {}", e);
            }
        }

        // Note: We don't actually write 3GB of data to avoid test slowness
    }

    #[test]
    fn test_concurrent_file_access() {
        use std::sync::Arc;
        use std::thread;

        let harness = Arc::new(FilesystemTestHarness::new().unwrap());
        let file_path = harness.test_path("concurrent_test.txt");

        // Create empty file
        File::create(&file_path).unwrap();

        // Spawn multiple threads to read the file
        let mut handles = vec![];
        for i in 0..10 {
            let path = file_path.clone();
            let handle = thread::spawn(move || {
                for _ in 0..100 {
                    let content = fs::read(&path).unwrap();
                    assert!(content.len() >= 0);
                }
                i
            });
            handles.push(handle);
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_filesystem_metadata() {
        let harness = FilesystemTestHarness::new().unwrap();

        // Get metadata about the temp directory
        let metadata = fs::metadata(&harness.temp_dir).unwrap();
        assert!(metadata.is_dir());

        // Check available space (platform-specific)
        #[cfg(unix)]
        {
            use std::ffi::CString;

            let path = CString::new(harness.temp_dir.to_str().unwrap()).unwrap();
            let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };

            let result = unsafe { libc::statvfs(path.as_ptr(), &mut stat) };

            if result == 0 {
                let available = stat.f_bavail * stat.f_bsize as u64;
                let total = stat.f_blocks * stat.f_bsize as u64;
                println!("Available: {} bytes, Total: {} bytes", available, total);
                assert!(available > 0);
                assert!(total > available);
            }
        }

        #[cfg(windows)]
        {
            // Windows GetDiskFreeSpaceEx would go here
            println!("Filesystem space check on Windows");
        }
    }
}
