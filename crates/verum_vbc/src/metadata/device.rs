//! Device placement hints for VBC tensor operations.
//!
//! Enables explicit and automatic device placement for tensor operations,
//! supporting CPU, multiple GPU backends, and TPUs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Block identifier within a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockId(pub u32);

/// Value identifier (SSA register).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValueId(pub u32);

/// Device type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceType {
    /// CPU device.
    CPU,
    /// NVIDIA GPU (CUDA).
    CUDA,
    /// AMD GPU (ROCm/HIP).
    ROCm,
    /// Apple GPU (Metal).
    Metal,
    /// Vulkan compute.
    Vulkan,
    /// Intel GPU (SYCL).
    SYCL,
    /// Google TPU.
    TPU,
}

impl DeviceType {
    /// Returns the backend name.
    pub fn backend_name(&self) -> &'static str {
        match self {
            DeviceType::CPU => "cpu",
            DeviceType::CUDA => "cuda",
            DeviceType::ROCm => "rocm",
            DeviceType::Metal => "metal",
            DeviceType::Vulkan => "vulkan",
            DeviceType::SYCL => "sycl",
            DeviceType::TPU => "tpu",
        }
    }

    /// Returns whether this is a GPU device.
    pub fn is_gpu(&self) -> bool {
        !matches!(self, DeviceType::CPU | DeviceType::TPU)
    }
}

/// Device preference for block/function placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum DevicePreference {
    /// No preference, compiler decides.
    #[default]
    Any,
    /// CPU only.
    CPU,
    /// Specific GPU type with optional device index.
    GPU {
        /// The GPU device type (CUDA, ROCm, Metal, etc.).
        device_type: DeviceType,
        /// Optional device index (e.g., GPU 0, GPU 1).
        index: Option<u32>,
    },
    /// TPU.
    TPU {
        /// Optional TPU device index.
        index: Option<u32>,
    },
    /// Prefer GPU if available, fallback to CPU.
    PreferGPU,
    /// Prefer a specific device type.
    Prefer(DeviceType),
    /// Explicit device list in priority order.
    Priority(Vec<DeviceType>),
}

impl DevicePreference {
    /// Creates a GPU preference for any CUDA device.
    pub fn cuda() -> Self {
        Self::GPU {
            device_type: DeviceType::CUDA,
            index: None,
        }
    }

    /// Creates a GPU preference for a specific CUDA device.
    pub fn cuda_device(index: u32) -> Self {
        Self::GPU {
            device_type: DeviceType::CUDA,
            index: Some(index),
        }
    }

    /// Creates a preference for Metal.
    pub fn metal() -> Self {
        Self::GPU {
            device_type: DeviceType::Metal,
            index: None,
        }
    }

    /// Returns true if this preference allows CPU.
    pub fn allows_cpu(&self) -> bool {
        match self {
            DevicePreference::Any | DevicePreference::CPU | DevicePreference::PreferGPU => true,
            DevicePreference::Priority(list) => list.contains(&DeviceType::CPU),
            _ => false,
        }
    }
}


/// Device transfer specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTransfer {
    /// Value to transfer.
    pub value: ValueId,
    /// Source device.
    pub from: DeviceType,
    /// Destination device.
    pub to: DeviceType,
    /// Whether transfer can be asynchronous.
    pub async_transfer: bool,
    /// Optional stream/queue ID for async transfers.
    pub stream_id: Option<u32>,
}

impl DeviceTransfer {
    /// Creates a new device transfer.
    pub fn new(value: ValueId, from: DeviceType, to: DeviceType) -> Self {
        Self {
            value,
            from,
            to,
            async_transfer: false,
            stream_id: None,
        }
    }

    /// Makes this transfer asynchronous.
    pub fn with_async(mut self, stream_id: u32) -> Self {
        self.async_transfer = true;
        self.stream_id = Some(stream_id);
        self
    }

    /// Returns true if this is a host-to-device transfer.
    pub fn is_h2d(&self) -> bool {
        matches!(self.from, DeviceType::CPU) && self.to.is_gpu()
    }

    /// Returns true if this is a device-to-host transfer.
    pub fn is_d2h(&self) -> bool {
        self.from.is_gpu() && matches!(self.to, DeviceType::CPU)
    }

    /// Returns true if this is a device-to-device transfer.
    pub fn is_d2d(&self) -> bool {
        self.from.is_gpu() && self.to.is_gpu()
    }
}

/// Device placement hints for a VBC module.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceHints {
    /// Device placement preference per block.
    pub placements: HashMap<BlockId, DevicePreference>,
    /// Explicit device transfers.
    pub transfers: Vec<DeviceTransfer>,
    /// Default device preference for unmarked blocks.
    pub default_preference: DevicePreference,
    /// Pinned memory allocations (for faster transfers).
    pub pinned_allocations: Vec<ValueId>,
}

impl DeviceHints {
    /// Creates empty device hints.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates device hints with a default preference.
    pub fn with_default(preference: DevicePreference) -> Self {
        Self {
            default_preference: preference,
            ..Self::default()
        }
    }

    /// Sets the placement for a block.
    pub fn set_placement(&mut self, block: BlockId, preference: DevicePreference) {
        self.placements.insert(block, preference);
    }

    /// Gets the placement for a block.
    pub fn get_placement(&self, block: BlockId) -> &DevicePreference {
        self.placements
            .get(&block)
            .unwrap_or(&self.default_preference)
    }

    /// Adds a device transfer.
    pub fn add_transfer(&mut self, transfer: DeviceTransfer) {
        self.transfers.push(transfer);
    }

    /// Marks an allocation as pinned memory.
    pub fn pin_allocation(&mut self, value: ValueId) {
        if !self.pinned_allocations.contains(&value) {
            self.pinned_allocations.push(value);
        }
    }

    /// Returns all blocks that prefer GPU.
    pub fn gpu_blocks(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.placements.iter().filter_map(|(&block, pref)| {
            let is_gpu_preference = match pref {
                DevicePreference::GPU { .. } => true,
                DevicePreference::PreferGPU => true,
                DevicePreference::Prefer(dt) => dt.is_gpu(),
                _ => false,
            };
            if is_gpu_preference {
                Some(block)
            } else {
                None
            }
        })
    }

    /// Returns true if this metadata is empty (no placements, transfers, or pinned allocations).
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
            && self.transfers.is_empty()
            && self.pinned_allocations.is_empty()
            && matches!(self.default_preference, DevicePreference::Any)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_preference() {
        let pref = DevicePreference::cuda_device(0);
        assert!(!pref.allows_cpu());

        let pref = DevicePreference::PreferGPU;
        assert!(pref.allows_cpu());
    }

    #[test]
    fn test_device_transfer() {
        let transfer = DeviceTransfer::new(ValueId(0), DeviceType::CPU, DeviceType::CUDA);
        assert!(transfer.is_h2d());
        assert!(!transfer.is_d2h());
        assert!(!transfer.is_d2d());

        let transfer =
            DeviceTransfer::new(ValueId(1), DeviceType::CUDA, DeviceType::ROCm).with_async(0);
        assert!(transfer.is_d2d());
        assert!(transfer.async_transfer);
    }

    #[test]
    fn test_device_hints() {
        let mut hints = DeviceHints::with_default(DevicePreference::PreferGPU);
        hints.set_placement(BlockId(0), DevicePreference::CPU);
        hints.set_placement(BlockId(1), DevicePreference::cuda());

        assert!(matches!(
            hints.get_placement(BlockId(0)),
            DevicePreference::CPU
        ));
        assert!(matches!(
            hints.get_placement(BlockId(1)),
            DevicePreference::GPU { .. }
        ));
        assert!(matches!(
            hints.get_placement(BlockId(99)),
            DevicePreference::PreferGPU
        ));
    }
}
