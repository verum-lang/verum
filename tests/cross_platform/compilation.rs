// Compilation Cross-Platform Tests

use super::detection::PlatformInfo;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compiler_detection() {
        let platform = PlatformInfo::detect();
        println!("Platform: {} {}", platform.os_type, platform.architecture);

        // Test that we're compiled with expected features
        #[cfg(debug_assertions)]
        println!("Debug build");

        #[cfg(not(debug_assertions))]
        println!("Release build");
    }

    #[test]
    fn test_target_features() {
        println!("Target features:");

        #[cfg(target_feature = "sse2")]
        println!("  - SSE2");

        #[cfg(target_feature = "avx2")]
        println!("  - AVX2");

        #[cfg(target_feature = "neon")]
        println!("  - NEON");
    }

    #[test]
    fn test_optimization_level() {
        // This function should be optimized away in release mode
        let result = (0..1000).sum::<i32>();
        assert_eq!(result, 499500);
    }

    #[test]
    fn test_linking() {
        // Verify standard library linking
        let vec: Vec<i32> = vec![1, 2, 3];
        assert_eq!(vec.len(), 3);

        // Verify libc linking
        #[cfg(unix)]
        unsafe {
            let pid = libc::getpid();
            assert!(pid > 0);
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_linux_toolchain() {
        println!("Linux toolchain test");
        // GCC/Clang specific tests
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_macos_toolchain() {
        println!("macOS toolchain test");
        // Xcode/Clang specific tests
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_windows_toolchain() {
        println!("Windows toolchain test");
        // MSVC/MinGW specific tests
    }

    #[test]
    fn test_static_vs_dynamic() {
        // Most Rust code is statically linked
        println!("Binary type: {}", if cfg!(target_feature = "crt-static") {
            "static"
        } else {
            "dynamic"
        });
    }

    #[test]
    fn test_panic_unwinding() {
        let result = std::panic::catch_unwind(|| {
            panic!("Test panic");
        });

        assert!(result.is_err());
    }

    #[test]
    fn test_const_evaluation() {
        const VALUE: i32 = 42 * 2;
        assert_eq!(VALUE, 84);

        const fn compute() -> i32 {
            1 + 2 + 3
        }
        const COMPUTED: i32 = compute();
        assert_eq!(COMPUTED, 6);
    }
}
