//! Device detection and management for tensor operations.
//!
//! This module provides runtime detection of available compute devices
//! (CPU, GPU) and their capabilities for optimal kernel dispatch.

use std::sync::OnceLock;

/// Device identifier with type encoding
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DeviceId(pub u16);

impl DeviceId {
    /// Mask for extracting device type from ID (upper 4 bits).
    pub const TYPE_MASK: u16 = 0xF000;
    /// Mask for extracting device index within type (lower 12 bits).
    pub const INDEX_MASK: u16 = 0x0FFF;

    /// CPU device
    pub const CPU: DeviceId = DeviceId(0x0000);
    /// GPU base (GPU0 = 0x1000, GPU1 = 0x1001, ...)
    pub const GPU_BASE: u16 = 0x1000;
    /// TPU base (TPU0 = 0x2000, ...)
    pub const TPU_BASE: u16 = 0x2000;

    /// Create GPU device ID
    pub const fn gpu(index: u16) -> Self {
        DeviceId(Self::GPU_BASE | index)
    }

    /// Check if CPU
    pub const fn is_cpu(&self) -> bool {
        self.0 == 0
    }

    /// Check if GPU
    pub const fn is_gpu(&self) -> bool {
        (self.0 & Self::TYPE_MASK) == Self::GPU_BASE
    }

    /// Check if TPU
    pub const fn is_tpu(&self) -> bool {
        (self.0 & Self::TYPE_MASK) == Self::TPU_BASE
    }

    /// Get device index within type
    pub const fn device_index(&self) -> u16 {
        self.0 & Self::INDEX_MASK
    }
}

impl Default for DeviceId {
    fn default() -> Self {
        Self::CPU
    }
}

/// GPU vendor
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    /// NVIDIA (CUDA)
    Nvidia,
    /// AMD (ROCm/HIP)
    Amd,
    /// Intel (oneAPI)
    Intel,
    /// Apple (Metal)
    Apple,
    /// Google (TPU)
    Google,
    /// Qualcomm (Adreno)
    Qualcomm,
    /// Unknown vendor
    Unknown,
}

/// CPU information
#[derive(Clone, Debug)]
pub struct CpuInfo {
    /// CPU vendor name
    pub vendor: String,
    /// Number of physical cores
    pub cores: usize,
    /// Number of logical threads
    pub threads: usize,
    /// L1 data cache size (bytes)
    pub l1_cache: usize,
    /// L2 cache size (bytes)
    pub l2_cache: usize,
    /// L3 cache size (bytes)
    pub l3_cache: usize,
    /// Has SSE4.2
    pub has_sse42: bool,
    /// Has AVX
    pub has_avx: bool,
    /// Has AVX2
    pub has_avx2: bool,
    /// Has FMA
    pub has_fma: bool,
    /// Has AVX-512F
    pub has_avx512f: bool,
    /// Has NEON (ARM)
    pub has_neon: bool,
}

impl Default for CpuInfo {
    fn default() -> Self {
        Self {
            vendor: String::new(),
            cores: 1,
            threads: 1,
            l1_cache: 32 * 1024,
            l2_cache: 256 * 1024,
            l3_cache: 8 * 1024 * 1024,
            has_sse42: false,
            has_avx: false,
            has_avx2: false,
            has_fma: false,
            has_avx512f: false,
            has_neon: false,
        }
    }
}

/// GPU information
#[derive(Clone, Debug)]
pub struct GpuInfo {
    /// GPU name
    pub name: String,
    /// Vendor
    pub vendor: Vendor,
    /// Total memory (bytes)
    pub memory_bytes: usize,
    /// Compute capability (major, minor)
    pub compute_capability: (u32, u32),
    /// Number of multiprocessors/compute units
    pub multiprocessors: u32,
    /// Maximum threads per block
    pub max_threads_per_block: u32,
    /// Warp/wavefront size
    pub warp_size: u32,
    /// Shared memory per block (bytes)
    pub shared_memory_per_block: usize,
    /// Has tensor cores
    pub has_tensor_cores: bool,
    /// Memory bandwidth (GB/s)
    pub memory_bandwidth_gbps: f64,
}

impl Default for GpuInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            vendor: Vendor::Unknown,
            memory_bytes: 0,
            compute_capability: (0, 0),
            multiprocessors: 0,
            max_threads_per_block: 1024,
            warp_size: 32,
            shared_memory_per_block: 48 * 1024,
            has_tensor_cores: false,
            memory_bandwidth_gbps: 0.0,
        }
    }
}

/// Device information
#[derive(Clone, Debug)]
pub enum DeviceInfo {
    /// CPU device
    Cpu(CpuInfo),
    /// GPU device
    Gpu(GpuInfo),
}

impl DeviceInfo {
    /// Check if GPU
    pub fn is_gpu(&self) -> bool {
        matches!(self, Self::Gpu(_))
    }

    /// Get memory bytes
    pub fn memory_bytes(&self) -> usize {
        match self {
            Self::Cpu(_) => {
                // Estimate available system memory
                8 * 1024 * 1024 * 1024 // 8 GB default
            }
            Self::Gpu(info) => info.memory_bytes,
        }
    }
}

/// Device selection strategy
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceSelection {
    /// Use first available GPU
    FirstGpu,
    /// Use GPU with most memory
    MaxMemoryGpu,
    /// Use GPU with highest compute capability
    MaxComputeGpu,
    /// Force CPU only
    CpuOnly,
    /// Use specific device by index
    ByIndex(usize),
}

/// Device registry containing all detected devices
#[derive(Debug)]
pub struct DeviceRegistry {
    /// All detected devices
    pub devices: Vec<(DeviceId, DeviceInfo)>,
    /// Default device for operations
    pub default_device: DeviceId,
}

impl DeviceRegistry {
    /// Initialize device registry with detection
    pub fn init() -> Self {
        let mut registry = Self {
            devices: Vec::new(),
            default_device: DeviceId::CPU,
        };

        // Always detect CPU
        registry.detect_cpu();

        // Detect GPUs
        #[cfg(target_os = "macos")]
        registry.detect_metal();

        // Select best default device
        registry.default_device = registry.select_best_device();

        registry
    }

    /// Detect CPU capabilities
    fn detect_cpu(&mut self) {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        let mut info = CpuInfo {
            vendor: detect_cpu_vendor(),
            cores: threads / 2,
            threads,
            ..Default::default()
        };

        #[cfg(target_arch = "x86_64")]
        {
            info.has_sse42 = std::arch::is_x86_feature_detected!("sse4.2");
            info.has_avx = std::arch::is_x86_feature_detected!("avx");
            info.has_avx2 = std::arch::is_x86_feature_detected!("avx2");
            info.has_fma = std::arch::is_x86_feature_detected!("fma");
            info.has_avx512f = std::arch::is_x86_feature_detected!("avx512f");
        }

        #[cfg(target_arch = "aarch64")]
        {
            info.has_neon = true;
        }

        self.devices.push((DeviceId::CPU, DeviceInfo::Cpu(info)));
    }

    /// Detect Metal devices (macOS)
    #[cfg(target_os = "macos")]
    fn detect_metal(&mut self) {
        // Metal detection would go here
        // For now, just note that Metal is available on Apple Silicon
        #[cfg(target_arch = "aarch64")]
        {
            let info = GpuInfo {
                name: "Apple GPU".to_string(),
                vendor: Vendor::Apple,
                memory_bytes: 8 * 1024 * 1024 * 1024, // Estimate
                compute_capability: (1, 0),
                multiprocessors: 8,
                max_threads_per_block: 1024,
                warp_size: 32,
                shared_memory_per_block: 32 * 1024,
                has_tensor_cores: false,
                memory_bandwidth_gbps: 200.0,
            };
            self.devices.push((DeviceId::gpu(0), DeviceInfo::Gpu(info)));
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn detect_metal(&mut self) {}

    /// Select best available device
    fn select_best_device(&self) -> DeviceId {
        // Prefer GPU with most memory
        self.devices
            .iter()
            .filter(|(_, info)| info.is_gpu())
            .max_by_key(|(_, info)| info.memory_bytes())
            .map(|(id, _)| *id)
            .unwrap_or(DeviceId::CPU)
    }

    /// Get device info by ID
    pub fn get_device(&self, id: DeviceId) -> Option<&DeviceInfo> {
        self.devices.iter()
            .find(|(dev_id, _)| *dev_id == id)
            .map(|(_, info)| info)
    }

    /// Get CPU info
    pub fn cpu_info(&self) -> Option<&CpuInfo> {
        self.get_device(DeviceId::CPU).and_then(|info| {
            match info {
                DeviceInfo::Cpu(cpu) => Some(cpu),
                _ => None,
            }
        })
    }

    /// Get all GPU devices
    pub fn gpus(&self) -> impl Iterator<Item = (DeviceId, &GpuInfo)> {
        self.devices.iter().filter_map(|(id, info)| {
            match info {
                DeviceInfo::Gpu(gpu) => Some((*id, gpu)),
                _ => None,
            }
        })
    }

    /// Check if any GPU is available
    pub fn has_gpu(&self) -> bool {
        self.devices.iter().any(|(_, info)| info.is_gpu())
    }
}

/// Detect CPU vendor string
fn detect_cpu_vendor() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        // Use CPUID to get vendor string
        use std::arch::x86_64::__cpuid;
        unsafe {
            let result = __cpuid(0);
            let mut vendor = [0u8; 12];
            vendor[0..4].copy_from_slice(&result.ebx.to_le_bytes());
            vendor[4..8].copy_from_slice(&result.edx.to_le_bytes());
            vendor[8..12].copy_from_slice(&result.ecx.to_le_bytes());
            String::from_utf8_lossy(&vendor).trim().to_string()
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        "ARM".to_string()
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        "Unknown".to_string()
    }
}

/// Global device registry (lazy-initialized)
static REGISTRY: OnceLock<DeviceRegistry> = OnceLock::new();

/// Get global device registry
pub fn get_registry() -> &'static DeviceRegistry {
    REGISTRY.get_or_init(DeviceRegistry::init)
}

/// Get default device
pub fn default_device() -> DeviceId {
    get_registry().default_device
}

/// Check if GPU is available
pub fn has_gpu() -> bool {
    get_registry().has_gpu()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_id() {
        assert!(DeviceId::CPU.is_cpu());
        assert!(!DeviceId::CPU.is_gpu());

        let gpu0 = DeviceId::gpu(0);
        assert!(gpu0.is_gpu());
        assert!(!gpu0.is_cpu());
        assert_eq!(gpu0.device_index(), 0);

        let gpu1 = DeviceId::gpu(1);
        assert_eq!(gpu1.device_index(), 1);
    }

    #[test]
    fn test_registry_init() {
        let registry = DeviceRegistry::init();
        assert!(!registry.devices.is_empty());
        assert!(registry.cpu_info().is_some());
    }

    #[test]
    fn test_cpu_detection() {
        let registry = get_registry();
        let cpu = registry.cpu_info().unwrap();
        assert!(cpu.threads >= 1);
    }
}
