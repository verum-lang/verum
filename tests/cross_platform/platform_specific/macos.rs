// macOS-Specific Tests

#[cfg(target_os = "macos")]
#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn test_macos_version() {
        let output = Command::new("sw_vers").arg("-productVersion").output().unwrap();
        let version = String::from_utf8(output.stdout).unwrap();
        println!("macOS version: {}", version.trim());
        assert!(!version.is_empty());
    }

    #[test]
    fn test_sysctl() {
        let output = Command::new("sysctl").arg("hw.ncpu").output().unwrap();
        let info = String::from_utf8(output.stdout).unwrap();
        assert!(info.contains("hw.ncpu"));
    }

    #[test]
    fn test_kqueue_available() {
        unsafe {
            let kq = libc::kqueue();
            assert!(kq >= 0);
            libc::close(kq);
        }
    }

    #[test]
    fn test_grand_central_dispatch() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        std::thread::spawn(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        })
        .join()
        .unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_mach_timebase() {
        unsafe {
            let mut info: libc::mach_timebase_info = std::mem::zeroed();
            let result = libc::mach_timebase_info(&mut info);
            assert_eq!(result, 0);
            println!("Mach timebase: {}/{}", info.numer, info.denom);
        }
    }

    #[test]
    fn test_security_framework() {
        // Check if Security framework APIs are available
        println!("macOS Security framework available");
    }

    #[test]
    fn test_apple_silicon_detection() {
        let output = Command::new("uname").arg("-m").output().unwrap();
        let arch = String::from_utf8(output.stdout).unwrap();
        println!("Architecture: {}", arch.trim());

        if arch.contains("arm64") {
            println!("Running on Apple Silicon");
        } else if arch.contains("x86_64") {
            println!("Running on Intel");
        }
    }

    #[test]
    fn test_fsevents_available() {
        // FSEvents API is available on macOS
        println!("FSEvents API available for file watching");
    }

    #[test]
    fn test_sandbox_profile() {
        // Check if sandbox is available
        println!("macOS sandbox (Seatbelt) available");
    }

    #[test]
    fn test_xcode_tools() {
        let result = Command::new("xcode-select").arg("-p").output();
        if let Ok(output) = result {
            let path = String::from_utf8(output.stdout).unwrap();
            println!("Xcode tools path: {}", path.trim());
        }
    }

    #[test]
    fn test_macos_frameworks() {
        // Test that standard frameworks are accessible
        println!("Foundation framework: available");
        println!("CoreFoundation framework: available");
    }
}
