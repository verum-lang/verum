// Platform Detection Utilities
// Comprehensive OS, architecture, and feature detection

use std::fmt;
use std::sync::OnceLock;

/// Operating System Type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OSType {
    Linux,
    MacOS,
    Windows,
    FreeBSD,
    OpenBSD,
    NetBSD,
    DragonFlyBSD,
    Android,
    IOS,
    Solaris,
    Illumos,
    Unknown,
}

impl fmt::Display for OSType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OSType::Linux => write!(f, "Linux"),
            OSType::MacOS => write!(f, "macOS"),
            OSType::Windows => write!(f, "Windows"),
            OSType::FreeBSD => write!(f, "FreeBSD"),
            OSType::OpenBSD => write!(f, "OpenBSD"),
            OSType::NetBSD => write!(f, "NetBSD"),
            OSType::DragonFlyBSD => write!(f, "DragonFlyBSD"),
            OSType::Android => write!(f, "Android"),
            OSType::IOS => write!(f, "iOS"),
            OSType::Solaris => write!(f, "Solaris"),
            OSType::Illumos => write!(f, "Illumos"),
            OSType::Unknown => write!(f, "Unknown"),
        }
    }
}

/// CPU Architecture
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Architecture {
    X86_64,
    Aarch64,
    Arm,
    Riscv64,
    Wasm32,
    PowerPC64,
    S390x,
    Unknown,
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Architecture::X86_64 => write!(f, "x86_64"),
            Architecture::Aarch64 => write!(f, "aarch64"),
            Architecture::Arm => write!(f, "arm"),
            Architecture::Riscv64 => write!(f, "riscv64"),
            Architecture::Wasm32 => write!(f, "wasm32"),
            Architecture::PowerPC64 => write!(f, "powerpc64"),
            Architecture::S390x => write!(f, "s390x"),
            Architecture::Unknown => write!(f, "unknown"),
        }
    }
}

/// Linux Distribution
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinuxDistro {
    Ubuntu { version: String },
    Fedora { version: u32 },
    Arch,
    Debian { version: u32 },
    CentOS { version: u32 },
    RHEL { version: u32 },
    OpenSUSE,
    Alpine,
    Unknown,
}

/// OS Version Information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OSVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub build: Option<String>,
}

impl fmt::Display for OSVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref build) = self.build {
            write!(f, "{}.{}.{} ({})", self.major, self.minor, self.patch, build)
        } else {
            write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
        }
    }
}

/// Comprehensive Platform Information
#[derive(Debug, Clone)]
pub struct PlatformInfo {
    pub os_type: OSType,
    pub os_version: OSVersion,
    pub architecture: Architecture,
    pub linux_distro: Option<LinuxDistro>,
    pub kernel_version: Option<String>,
    pub num_cpus: usize,
    pub page_size: usize,
    pub endianness: Endianness,
    pub pointer_width: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endianness {
    Little,
    Big,
}

impl PlatformInfo {
    /// Detect current platform information
    pub fn detect() -> Self {
        static CACHED: OnceLock<PlatformInfo> = OnceLock::new();
        CACHED.get_or_init(Self::detect_impl).clone()
    }

    fn detect_impl() -> Self {
        let os_type = Self::detect_os();
        let architecture = Self::detect_arch();
        let os_version = Self::detect_os_version(os_type);
        let linux_distro = if os_type == OSType::Linux {
            Some(Self::detect_linux_distro())
        } else {
            None
        };
        let kernel_version = Self::detect_kernel_version();
        let num_cpus = num_cpus::get();
        let page_size = Self::detect_page_size();
        let endianness = if cfg!(target_endian = "little") {
            Endianness::Little
        } else {
            Endianness::Big
        };
        let pointer_width = std::mem::size_of::<usize>() * 8;

        PlatformInfo {
            os_type,
            os_version,
            architecture,
            linux_distro,
            kernel_version,
            num_cpus,
            page_size,
            endianness,
            pointer_width,
        }
    }

    fn detect_os() -> OSType {
        #[cfg(target_os = "linux")]
        return OSType::Linux;
        #[cfg(target_os = "macos")]
        return OSType::MacOS;
        #[cfg(target_os = "windows")]
        return OSType::Windows;
        #[cfg(target_os = "freebsd")]
        return OSType::FreeBSD;
        #[cfg(target_os = "openbsd")]
        return OSType::OpenBSD;
        #[cfg(target_os = "netbsd")]
        return OSType::NetBSD;
        #[cfg(target_os = "dragonfly")]
        return OSType::DragonFlyBSD;
        #[cfg(target_os = "android")]
        return OSType::Android;
        #[cfg(target_os = "ios")]
        return OSType::IOS;
        #[cfg(target_os = "solaris")]
        return OSType::Solaris;
        #[cfg(target_os = "illumos")]
        return OSType::Illumos;
        #[cfg(not(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly",
            target_os = "android",
            target_os = "ios",
            target_os = "solaris",
            target_os = "illumos"
        )))]
        return OSType::Unknown;
    }

    fn detect_arch() -> Architecture {
        #[cfg(target_arch = "x86_64")]
        return Architecture::X86_64;
        #[cfg(target_arch = "aarch64")]
        return Architecture::Aarch64;
        #[cfg(target_arch = "arm")]
        return Architecture::Arm;
        #[cfg(target_arch = "riscv64")]
        return Architecture::Riscv64;
        #[cfg(target_arch = "wasm32")]
        return Architecture::Wasm32;
        #[cfg(target_arch = "powerpc64")]
        return Architecture::PowerPC64;
        #[cfg(target_arch = "s390x")]
        return Architecture::S390x;
        #[cfg(not(any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "arm",
            target_arch = "riscv64",
            target_arch = "wasm32",
            target_arch = "powerpc64",
            target_arch = "s390x"
        )))]
        return Architecture::Unknown;
    }

    #[cfg(target_os = "linux")]
    fn detect_os_version(_os: OSType) -> OSVersion {
        // Parse /etc/os-release or uname
        if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
            for line in contents.lines() {
                if line.starts_with("VERSION_ID=") {
                    let version_str = line.trim_start_matches("VERSION_ID=").trim_matches('"');
                    if let Some(parsed) = Self::parse_version(version_str) {
                        return parsed;
                    }
                }
            }
        }

        // Fallback to uname
        Self::uname_version()
    }

    #[cfg(target_os = "macos")]
    fn detect_os_version(_os: OSType) -> OSVersion {
        use std::process::Command;

        if let Ok(output) = Command::new("sw_vers").arg("-productVersion").output() {
            if let Ok(version_str) = String::from_utf8(output.stdout) {
                if let Some(version) = Self::parse_version(version_str.trim()) {
                    return version;
                }
            }
        }

        Self::uname_version()
    }

    #[cfg(target_os = "windows")]
    fn detect_os_version(_os: OSType) -> OSVersion {
        // Windows version detection via registry or WinAPI
        // For simplicity, use compile-time detection
        OSVersion {
            major: 10,
            minor: 0,
            patch: 0,
            build: Some("Windows".to_string()),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    fn detect_os_version(_os: OSType) -> OSVersion {
        Self::uname_version()
    }

    fn parse_version(version_str: &str) -> Option<OSVersion> {
        let parts: Vec<&str> = version_str.split('.').collect();
        if parts.is_empty() {
            return None;
        }

        let major = parts[0].parse().ok()?;
        let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        let build = parts.get(3).map(|s| s.to_string());

        Some(OSVersion {
            major,
            minor,
            patch,
            build,
        })
    }

    #[cfg(unix)]
    fn uname_version() -> OSVersion {
        use std::process::Command;

        if let Ok(output) = Command::new("uname").arg("-r").output() {
            if let Ok(version_str) = String::from_utf8(output.stdout) {
                if let Some(version) = Self::parse_version(version_str.trim()) {
                    return version;
                }
            }
        }

        OSVersion {
            major: 0,
            minor: 0,
            patch: 0,
            build: None,
        }
    }

    #[cfg(not(unix))]
    fn uname_version() -> OSVersion {
        OSVersion {
            major: 0,
            minor: 0,
            patch: 0,
            build: None,
        }
    }

    #[cfg(target_os = "linux")]
    fn detect_linux_distro() -> LinuxDistro {
        if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
            let mut name = None;
            let mut version_id = None;

            for line in contents.lines() {
                if line.starts_with("ID=") {
                    name = Some(line.trim_start_matches("ID=").trim_matches('"').to_string());
                } else if line.starts_with("VERSION_ID=") {
                    version_id = Some(line.trim_start_matches("VERSION_ID=").trim_matches('"').to_string());
                }
            }

            if let Some(name) = name {
                match name.as_str() {
                    "ubuntu" => return LinuxDistro::Ubuntu {
                        version: version_id.unwrap_or_else(|| "unknown".to_string()),
                    },
                    "fedora" => {
                        let version = version_id.and_then(|v| v.parse().ok()).unwrap_or(0);
                        return LinuxDistro::Fedora { version };
                    }
                    "arch" => return LinuxDistro::Arch,
                    "debian" => {
                        let version = version_id.and_then(|v| v.parse().ok()).unwrap_or(0);
                        return LinuxDistro::Debian { version };
                    }
                    "centos" => {
                        let version = version_id.and_then(|v| v.parse().ok()).unwrap_or(0);
                        return LinuxDistro::CentOS { version };
                    }
                    "rhel" => {
                        let version = version_id.and_then(|v| v.parse().ok()).unwrap_or(0);
                        return LinuxDistro::RHEL { version };
                    }
                    "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" => {
                        return LinuxDistro::OpenSUSE;
                    }
                    "alpine" => return LinuxDistro::Alpine,
                    _ => {}
                }
            }
        }

        LinuxDistro::Unknown
    }

    #[cfg(not(target_os = "linux"))]
    fn detect_linux_distro() -> LinuxDistro {
        LinuxDistro::Unknown
    }

    #[cfg(unix)]
    fn detect_kernel_version() -> Option<String> {
        use std::process::Command;

        Command::new("uname")
            .arg("-r")
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|s| s.trim().to_string())
    }

    #[cfg(not(unix))]
    fn detect_kernel_version() -> Option<String> {
        None
    }

    #[cfg(unix)]
    fn detect_page_size() -> usize {
        unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
    }

    #[cfg(not(unix))]
    fn detect_page_size() -> usize {
        4096 // Default page size
    }

    /// Check if this is a Unix-like system
    pub fn is_unix(&self) -> bool {
        matches!(
            self.os_type,
            OSType::Linux
                | OSType::MacOS
                | OSType::FreeBSD
                | OSType::OpenBSD
                | OSType::NetBSD
                | OSType::DragonFlyBSD
                | OSType::Android
                | OSType::IOS
                | OSType::Solaris
                | OSType::Illumos
        )
    }

    /// Check if this is Windows
    pub fn is_windows(&self) -> bool {
        self.os_type == OSType::Windows
    }

    /// Check if this is a BSD system
    pub fn is_bsd(&self) -> bool {
        matches!(
            self.os_type,
            OSType::FreeBSD | OSType::OpenBSD | OSType::NetBSD | OSType::DragonFlyBSD
        )
    }

    /// Generate a platform string identifier
    pub fn platform_id(&self) -> String {
        format!("{}-{}", self.os_type, self.architecture)
    }
}

/// Feature Detection for Runtime Capabilities
#[derive(Debug, Clone)]
pub struct FeatureDetector {
    platform: PlatformInfo,
}

impl FeatureDetector {
    pub fn new() -> Self {
        Self {
            platform: PlatformInfo::detect(),
        }
    }

    /// Check if SIMD support is available
    pub fn has_simd(&self) -> bool {
        match self.platform.architecture {
            Architecture::X86_64 => {
                #[cfg(target_arch = "x86_64")]
                {
                    is_x86_feature_detected!("sse2")
                }
                #[cfg(not(target_arch = "x86_64"))]
                false
            }
            Architecture::Aarch64 => {
                // NEON is always available on aarch64
                true
            }
            _ => false,
        }
    }

    /// Check for specific x86_64 SIMD features
    #[cfg(target_arch = "x86_64")]
    pub fn x86_64_features(&self) -> X86Features {
        X86Features {
            sse2: is_x86_feature_detected!("sse2"),
            sse3: is_x86_feature_detected!("sse3"),
            ssse3: is_x86_feature_detected!("ssse3"),
            sse4_1: is_x86_feature_detected!("sse4.1"),
            sse4_2: is_x86_feature_detected!("sse4.2"),
            avx: is_x86_feature_detected!("avx"),
            avx2: is_x86_feature_detected!("avx2"),
            avx512f: is_x86_feature_detected!("avx512f"),
            bmi1: is_x86_feature_detected!("bmi1"),
            bmi2: is_x86_feature_detected!("bmi2"),
            fma: is_x86_feature_detected!("fma"),
            aes: is_x86_feature_detected!("aes"),
        }
    }

    /// Check for io_uring support (Linux 5.1+)
    #[cfg(target_os = "linux")]
    pub fn has_io_uring(&self) -> bool {
        if let Some(ref kernel) = self.platform.kernel_version {
            if let Some(major_str) = kernel.split('.').next() {
                if let Ok(major) = major_str.parse::<u32>() {
                    return major >= 5;
                }
            }
        }
        false
    }

    #[cfg(not(target_os = "linux"))]
    pub fn has_io_uring(&self) -> bool {
        false
    }

    /// Check for kqueue support (macOS, BSD)
    pub fn has_kqueue(&self) -> bool {
        self.platform.os_type == OSType::MacOS || self.platform.is_bsd()
    }

    /// Check for IOCP support (Windows)
    pub fn has_iocp(&self) -> bool {
        self.platform.is_windows()
    }

    /// Check for epoll support (Linux)
    pub fn has_epoll(&self) -> bool {
        self.platform.os_type == OSType::Linux
    }

    /// Detect async I/O backend
    pub fn async_io_backend(&self) -> AsyncIOBackend {
        #[cfg(target_os = "linux")]
        {
            if self.has_io_uring() {
                return AsyncIOBackend::IoUring;
            }
            return AsyncIOBackend::Epoll;
        }

        #[cfg(target_os = "macos")]
        return AsyncIOBackend::Kqueue;

        #[cfg(target_os = "windows")]
        return AsyncIOBackend::Iocp;

        #[cfg(any(
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        return AsyncIOBackend::Kqueue;

        #[cfg(not(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        )))]
        return AsyncIOBackend::Poll;
    }

    /// Check filesystem case sensitivity
    pub fn is_filesystem_case_sensitive(&self) -> bool {
        match self.platform.os_type {
            OSType::Linux | OSType::Android => true,
            OSType::MacOS | OSType::IOS => {
                // macOS can be case-sensitive (APFS) or case-insensitive
                // Default to case-insensitive
                false
            }
            OSType::Windows => false,
            _ => true, // Most Unix systems are case-sensitive
        }
    }

    /// Check if symlinks are fully supported
    pub fn has_symlink_support(&self) -> bool {
        // Windows has limited symlink support (requires admin or dev mode)
        !self.platform.is_windows()
    }

    /// Get the platform info
    pub fn platform(&self) -> &PlatformInfo {
        &self.platform
    }
}

impl Default for FeatureDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone)]
pub struct X86Features {
    pub sse2: bool,
    pub sse3: bool,
    pub ssse3: bool,
    pub sse4_1: bool,
    pub sse4_2: bool,
    pub avx: bool,
    pub avx2: bool,
    pub avx512f: bool,
    pub bmi1: bool,
    pub bmi2: bool,
    pub fma: bool,
    pub aes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncIOBackend {
    IoUring,  // Linux 5.1+
    Epoll,    // Linux
    Kqueue,   // macOS, BSD
    Iocp,     // Windows
    Poll,     // Generic fallback
}

impl fmt::Display for AsyncIOBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AsyncIOBackend::IoUring => write!(f, "io_uring"),
            AsyncIOBackend::Epoll => write!(f, "epoll"),
            AsyncIOBackend::Kqueue => write!(f, "kqueue"),
            AsyncIOBackend::Iocp => write!(f, "IOCP"),
            AsyncIOBackend::Poll => write!(f, "poll"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_detection() {
        let info = PlatformInfo::detect();
        println!("Platform: {} {}", info.os_type, info.architecture);
        println!("OS Version: {}", info.os_version);
        println!("CPUs: {}", info.num_cpus);
        println!("Page Size: {}", info.page_size);
        println!("Pointer Width: {}", info.pointer_width);

        assert!(info.num_cpus > 0);
        assert!(info.page_size > 0);
        assert!(info.pointer_width == 32 || info.pointer_width == 64);
    }

    #[test]
    fn test_feature_detection() {
        let detector = FeatureDetector::new();
        let backend = detector.async_io_backend();
        println!("Async I/O Backend: {}", backend);

        #[cfg(target_os = "linux")]
        assert!(detector.has_epoll());

        #[cfg(target_os = "macos")]
        assert!(detector.has_kqueue());

        #[cfg(target_os = "windows")]
        assert!(detector.has_iocp());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn test_simd_features() {
        let detector = FeatureDetector::new();
        if detector.platform.architecture == Architecture::X86_64 {
            let features = detector.x86_64_features();
            println!("SSE2: {}", features.sse2);
            println!("SSE4.2: {}", features.sse4_2);
            println!("AVX2: {}", features.avx2);
            println!("AVX-512F: {}", features.avx512f);

            // SSE2 is required for x86_64
            assert!(features.sse2);
        }
    }

    #[test]
    fn test_platform_id() {
        let info = PlatformInfo::detect();
        let id = info.platform_id();
        println!("Platform ID: {}", id);
        assert!(!id.is_empty());
    }
}
