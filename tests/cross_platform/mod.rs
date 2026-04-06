// Cross-Platform Testing Infrastructure for Verum Language Platform
// Comprehensive validation across Linux, macOS, Windows, and BSD systems

pub mod detection;
pub mod filesystem;
pub mod io_backends;
pub mod processes;
pub mod network;
pub mod memory;
pub mod simd;
pub mod compilation;
pub mod runtime;
pub mod security;
pub mod compatibility;

pub mod platform_specific {
    #[cfg(target_os = "linux")]
    pub mod linux;

    #[cfg(target_os = "macos")]
    pub mod macos;

    #[cfg(target_os = "windows")]
    pub mod windows;

    #[cfg(any(
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    pub mod bsd;
}

// Re-export core types for convenience
pub use detection::{PlatformInfo, FeatureDetector, Architecture, OSType, OSVersion};
pub use compatibility::TestMatrix;
