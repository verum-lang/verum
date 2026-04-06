// Platform Compatibility Matrix

use super::detection::{AsyncIOBackend, FeatureDetector, OSType, PlatformInfo};
use std::collections::HashMap;

/// Platform feature support matrix
#[derive(Debug, Clone)]
pub struct TestMatrix {
    pub platform: PlatformInfo,
    pub features: HashMap<String, FeatureSupport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureSupport {
    Supported,
    PartiallySupported,
    NotSupported,
    Unknown,
}

impl TestMatrix {
    pub fn generate() -> Self {
        let platform = PlatformInfo::detect();
        let detector = FeatureDetector::new();
        let mut features = HashMap::new();

        // Filesystem features
        features.insert("filesystem.symlinks".to_string(), if detector.has_symlink_support() {
            FeatureSupport::Supported
        } else {
            FeatureSupport::PartiallySupported
        });

        features.insert(
            "filesystem.case_sensitive".to_string(),
            if detector.is_filesystem_case_sensitive() {
                FeatureSupport::Supported
            } else {
                FeatureSupport::NotSupported
            },
        );

        features.insert("filesystem.hardlinks".to_string(), FeatureSupport::Supported);

        // I/O backends
        features.insert("io.async".to_string(), FeatureSupport::Supported);
        features.insert("io.io_uring".to_string(), if detector.has_io_uring() {
            FeatureSupport::Supported
        } else {
            FeatureSupport::NotSupported
        });
        features.insert("io.epoll".to_string(), if detector.has_epoll() {
            FeatureSupport::Supported
        } else {
            FeatureSupport::NotSupported
        });
        features.insert("io.kqueue".to_string(), if detector.has_kqueue() {
            FeatureSupport::Supported
        } else {
            FeatureSupport::NotSupported
        });
        features.insert("io.iocp".to_string(), if detector.has_iocp() {
            FeatureSupport::Supported
        } else {
            FeatureSupport::NotSupported
        });

        // SIMD features
        features.insert("simd".to_string(), if detector.has_simd() {
            FeatureSupport::Supported
        } else {
            FeatureSupport::NotSupported
        });

        #[cfg(target_arch = "x86_64")]
        {
            let x86_features = detector.x86_64_features();
            features.insert("simd.sse2".to_string(), if x86_features.sse2 {
                FeatureSupport::Supported
            } else {
                FeatureSupport::NotSupported
            });
            features.insert("simd.sse4_2".to_string(), if x86_features.sse4_2 {
                FeatureSupport::Supported
            } else {
                FeatureSupport::NotSupported
            });
            features.insert("simd.avx2".to_string(), if x86_features.avx2 {
                FeatureSupport::Supported
            } else {
                FeatureSupport::NotSupported
            });
            features.insert("simd.avx512f".to_string(), if x86_features.avx512f {
                FeatureSupport::Supported
            } else {
                FeatureSupport::NotSupported
            });
        }

        #[cfg(target_arch = "aarch64")]
        {
            features.insert("simd.neon".to_string(), FeatureSupport::Supported);
        }

        // Concurrency
        features.insert("concurrency.threads".to_string(), FeatureSupport::Supported);
        features.insert("concurrency.async".to_string(), FeatureSupport::Supported);

        // Network
        features.insert("network.tcp".to_string(), FeatureSupport::Supported);
        features.insert("network.udp".to_string(), FeatureSupport::Supported);
        features.insert("network.unix_sockets".to_string(), if platform.is_unix() {
            FeatureSupport::Supported
        } else {
            FeatureSupport::NotSupported
        });

        // Security
        features.insert("security.seccomp".to_string(), if platform.os_type == OSType::Linux {
            FeatureSupport::Supported
        } else {
            FeatureSupport::NotSupported
        });
        features.insert("security.sandbox".to_string(), if platform.os_type == OSType::MacOS {
            FeatureSupport::Supported
        } else {
            FeatureSupport::NotSupported
        });

        Self { platform, features }
    }

    pub fn is_supported(&self, feature: &str) -> bool {
        matches!(self.features.get(feature), Some(FeatureSupport::Supported))
    }

    pub fn print_report(&self) {
        println!("\n=== Platform Compatibility Matrix ===");
        println!("OS: {} {}", self.platform.os_type, self.platform.os_version);
        println!("Architecture: {}", self.platform.architecture);
        println!("CPUs: {}", self.platform.num_cpus);
        println!("Page Size: {} bytes", self.platform.page_size);

        if let Some(ref distro) = self.platform.linux_distro {
            println!("Linux Distribution: {:?}", distro);
        }

        println!("\n=== Feature Support ===");
        let mut feature_list: Vec<_> = self.features.iter().collect();
        feature_list.sort_by_key(|(k, _)| *k);

        for (feature, support) in feature_list {
            let status = match support {
                FeatureSupport::Supported => "✓ Supported",
                FeatureSupport::PartiallySupported => "◐ Partial",
                FeatureSupport::NotSupported => "✗ Not Supported",
                FeatureSupport::Unknown => "? Unknown",
            };
            println!("  {:<30} {}", feature, status);
        }
    }

    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("# Platform Compatibility Matrix\n\n");
        md.push_str(&format!("**OS:** {} {}\n", self.platform.os_type, self.platform.os_version));
        md.push_str(&format!("**Architecture:** {}\n", self.platform.architecture));
        md.push_str(&format!("**CPUs:** {}\n", self.platform.num_cpus));
        md.push_str(&format!("**Page Size:** {} bytes\n\n", self.platform.page_size));

        md.push_str("## Feature Support\n\n");
        md.push_str("| Feature | Status |\n");
        md.push_str("|---------|--------|\n");

        let mut feature_list: Vec<_> = self.features.iter().collect();
        feature_list.sort_by_key(|(k, _)| *k);

        for (feature, support) in feature_list {
            let status = match support {
                FeatureSupport::Supported => "✓ Supported",
                FeatureSupport::PartiallySupported => "◐ Partial",
                FeatureSupport::NotSupported => "✗ Not Supported",
                FeatureSupport::Unknown => "? Unknown",
            };
            md.push_str(&format!("| {} | {} |\n", feature, status));
        }

        md
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matrix_generation() {
        let matrix = TestMatrix::generate();
        matrix.print_report();

        // Verify some basic features are present
        assert!(matrix.features.contains_key("filesystem.hardlinks"));
        assert!(matrix.features.contains_key("io.async"));
        assert!(matrix.features.contains_key("concurrency.threads"));
    }

    #[test]
    fn test_platform_specific_features() {
        let matrix = TestMatrix::generate();

        #[cfg(target_os = "linux")]
        {
            assert!(matrix.is_supported("io.epoll"));
        }

        #[cfg(target_os = "macos")]
        {
            assert!(matrix.is_supported("io.kqueue"));
        }

        #[cfg(target_os = "windows")]
        {
            assert!(matrix.is_supported("io.iocp"));
        }
    }

    #[test]
    fn test_markdown_export() {
        let matrix = TestMatrix::generate();
        let md = matrix.to_markdown();

        assert!(md.contains("# Platform Compatibility Matrix"));
        assert!(md.contains("Feature Support"));
        println!("\n{}", md);
    }
}
