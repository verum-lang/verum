//! Memory issue detection harness for Verum
//!
//! This module detects memory-related issues in Verum programs:
//!
//! - Memory leaks
//! - Use-after-free
//! - Double-free
//! - Buffer overflows
//! - Invalid pointer dereferences
//! - Dangling references
//! - CBGR generation mismatches
//!
//! # Integration with CBGR
//!
//! This harness specifically tests the CBGR (Checked Borrow with
//! Generational References) system, which provides memory safety
//! at runtime with ~15ns overhead per reference access.
//!
//! # Architecture
//!
//! The harness uses multiple detection strategies:
//! 1. Built-in CBGR validation
//! 2. AddressSanitizer (when available)
//! 3. Custom memory tracking
//! 4. Heap profiling

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Types of memory issues
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MemoryIssue {
    /// Memory leak - allocated but never freed
    Leak {
        size: usize,
        allocation_site: Option<String>,
    },
    /// Use-after-free - accessing freed memory
    UseAfterFree {
        address: usize,
        freed_at: Option<String>,
        accessed_at: String,
    },
    /// Double-free - freeing already freed memory
    DoubleFree {
        address: usize,
        first_free: Option<String>,
        second_free: String,
    },
    /// Buffer overflow - accessing beyond allocation bounds
    BufferOverflow {
        address: usize,
        allocation_size: usize,
        access_offset: usize,
        direction: OverflowDirection,
    },
    /// Invalid pointer dereference
    InvalidPointer { address: usize, reason: String },
    /// CBGR generation mismatch
    GenerationMismatch {
        expected: u32,
        actual: u32,
        location: String,
    },
    /// Dangling reference - reference outlives referent
    DanglingReference {
        reference_location: String,
        referent_dropped_at: Option<String>,
    },
    /// Epoch violation in CBGR
    EpochViolation {
        reference_epoch: u64,
        current_epoch: u64,
    },
    /// Stack use after return
    StackUseAfterReturn { stack_frame: String },
    /// Uninitialized memory access
    UninitializedAccess { address: usize, size: usize },
    /// Memory corruption detected
    Corruption { address: usize, description: String },
}

/// Direction of buffer overflow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverflowDirection {
    Read,
    Write,
}

impl fmt::Display for MemoryIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryIssue::Leak {
                size,
                allocation_site,
            } => {
                write!(f, "Memory leak: {} bytes", size)?;
                if let Some(site) = allocation_site {
                    write!(f, " at {}", site)?;
                }
                Ok(())
            }
            MemoryIssue::UseAfterFree {
                address,
                freed_at,
                accessed_at,
            } => {
                write!(f, "Use-after-free at 0x{:x}", address)?;
                if let Some(freed) = freed_at {
                    write!(f, " (freed at {})", freed)?;
                }
                write!(f, ", accessed at {}", accessed_at)
            }
            MemoryIssue::DoubleFree {
                address,
                first_free,
                second_free,
            } => {
                write!(f, "Double-free at 0x{:x}", address)?;
                if let Some(first) = first_free {
                    write!(f, " (first free: {})", first)?;
                }
                write!(f, ", second free: {}", second_free)
            }
            MemoryIssue::BufferOverflow {
                address,
                allocation_size,
                access_offset,
                direction,
            } => {
                write!(
                    f,
                    "Buffer {:?} overflow at 0x{:x}: allocation size {}, access offset {}",
                    direction, address, allocation_size, access_offset
                )
            }
            MemoryIssue::InvalidPointer { address, reason } => {
                write!(f, "Invalid pointer 0x{:x}: {}", address, reason)
            }
            MemoryIssue::GenerationMismatch {
                expected,
                actual,
                location,
            } => {
                write!(
                    f,
                    "CBGR generation mismatch at {}: expected {}, got {}",
                    location, expected, actual
                )
            }
            MemoryIssue::DanglingReference {
                reference_location,
                referent_dropped_at,
            } => {
                write!(f, "Dangling reference at {}", reference_location)?;
                if let Some(dropped) = referent_dropped_at {
                    write!(f, ", referent dropped at {}", dropped)?;
                }
                Ok(())
            }
            MemoryIssue::EpochViolation {
                reference_epoch,
                current_epoch,
            } => {
                write!(
                    f,
                    "Epoch violation: reference epoch {}, current epoch {}",
                    reference_epoch, current_epoch
                )
            }
            MemoryIssue::StackUseAfterReturn { stack_frame } => {
                write!(f, "Stack use after return from {}", stack_frame)
            }
            MemoryIssue::UninitializedAccess { address, size } => {
                write!(
                    f,
                    "Uninitialized memory access: {} bytes at 0x{:x}",
                    size, address
                )
            }
            MemoryIssue::Corruption {
                address,
                description,
            } => {
                write!(f, "Memory corruption at 0x{:x}: {}", address, description)
            }
        }
    }
}

/// Report of a memory issue
#[derive(Debug, Clone)]
pub struct MemoryReport {
    /// The issue detected
    pub issue: MemoryIssue,
    /// Source code that triggered the issue
    pub source: String,
    /// Stack trace at detection
    pub stack_trace: Option<String>,
    /// Timestamp of detection
    pub timestamp: Instant,
    /// Severity (1-10)
    pub severity: u8,
    /// Whether the issue was detected by CBGR
    pub detected_by_cbgr: bool,
}

/// Allocation tracking entry
#[derive(Debug, Clone)]
struct AllocationEntry {
    address: usize,
    size: usize,
    allocation_site: Option<String>,
    allocation_time: Instant,
    generation: u32,
    is_freed: bool,
    free_site: Option<String>,
}

/// Reference tracking entry for CBGR validation
#[derive(Debug, Clone)]
struct ReferenceEntry {
    address: usize,
    generation: u32,
    epoch: u64,
    creation_site: String,
    is_mutable: bool,
}

/// Configuration for memory harness
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Whether to track all allocations
    pub track_allocations: bool,
    /// Whether to track all references
    pub track_references: bool,
    /// Whether to detect leaks
    pub detect_leaks: bool,
    /// Whether to detect use-after-free
    pub detect_use_after_free: bool,
    /// Whether to validate CBGR generations
    pub validate_generations: bool,
    /// Whether to check buffer bounds
    pub check_bounds: bool,
    /// Maximum allocation size to allow
    pub max_allocation_size: usize,
    /// Maximum number of live allocations
    pub max_live_allocations: usize,
    /// Quarantine size for freed memory
    pub quarantine_size: usize,
    /// Whether to fill freed memory with pattern
    pub poison_freed_memory: bool,
    /// Pattern to fill freed memory with
    pub poison_pattern: u8,
    /// Whether to enable epoch tracking
    pub enable_epochs: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            track_allocations: true,
            track_references: true,
            detect_leaks: true,
            detect_use_after_free: true,
            validate_generations: true,
            check_bounds: true,
            max_allocation_size: 100 * 1024 * 1024, // 100 MB
            max_live_allocations: 1_000_000,
            quarantine_size: 1024,
            poison_freed_memory: true,
            poison_pattern: 0xDE, // "DEAD"
            enable_epochs: true,
        }
    }
}

/// Memory tracking state
struct MemoryState {
    /// All allocations
    allocations: HashMap<usize, AllocationEntry>,
    /// Active references
    references: HashMap<usize, Vec<ReferenceEntry>>,
    /// Quarantine queue (recently freed)
    quarantine: Vec<AllocationEntry>,
    /// Current generation counter
    generation_counter: u32,
    /// Current epoch
    current_epoch: u64,
    /// Detected issues
    issues: Vec<MemoryReport>,
    /// Statistics
    stats: MemoryStats,
}

impl MemoryState {
    fn new() -> Self {
        Self {
            allocations: HashMap::new(),
            references: HashMap::new(),
            quarantine: Vec::new(),
            generation_counter: 0,
            current_epoch: 0,
            issues: Vec::new(),
            stats: MemoryStats::default(),
        }
    }
}

/// Memory statistics
#[derive(Debug, Default, Clone)]
pub struct MemoryStats {
    pub total_allocations: usize,
    pub total_frees: usize,
    pub total_bytes_allocated: usize,
    pub total_bytes_freed: usize,
    pub peak_memory_usage: usize,
    pub current_memory_usage: usize,
    pub live_allocations: usize,
    pub reference_validations: usize,
    pub generation_checks: usize,
    pub epoch_transitions: usize,
}

/// Memory issue detection harness
pub struct MemoryHarness {
    config: MemoryConfig,
    state: Arc<Mutex<MemoryState>>,
}

impl MemoryHarness {
    /// Create a new memory harness
    pub fn new(config: MemoryConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(MemoryState::new())),
        }
    }

    /// Record an allocation
    pub fn record_allocation(&self, address: usize, size: usize, site: Option<&str>) {
        if !self.config.track_allocations {
            return;
        }

        let mut state = self.state.lock().unwrap();

        // Check allocation limits
        if size > self.config.max_allocation_size {
            state.issues.push(MemoryReport {
                issue: MemoryIssue::InvalidPointer {
                    address,
                    reason: format!(
                        "Allocation size {} exceeds maximum {}",
                        size, self.config.max_allocation_size
                    ),
                },
                source: String::new(),
                stack_trace: None,
                timestamp: Instant::now(),
                severity: 7,
                detected_by_cbgr: false,
            });
            return;
        }

        if state.allocations.len() >= self.config.max_live_allocations {
            state.issues.push(MemoryReport {
                issue: MemoryIssue::Leak {
                    size,
                    allocation_site: site.map(String::from),
                },
                source: String::new(),
                stack_trace: None,
                timestamp: Instant::now(),
                severity: 5,
                detected_by_cbgr: false,
            });
            return;
        }

        // Assign generation
        state.generation_counter = state.generation_counter.wrapping_add(1);

        let entry = AllocationEntry {
            address,
            size,
            allocation_site: site.map(String::from),
            allocation_time: Instant::now(),
            generation: state.generation_counter,
            is_freed: false,
            free_site: None,
        };

        state.allocations.insert(address, entry);

        // Update stats
        state.stats.total_allocations += 1;
        state.stats.total_bytes_allocated += size;
        state.stats.current_memory_usage += size;
        state.stats.live_allocations += 1;

        if state.stats.current_memory_usage > state.stats.peak_memory_usage {
            state.stats.peak_memory_usage = state.stats.current_memory_usage;
        }
    }

    /// Record a deallocation
    pub fn record_free(&self, address: usize, site: Option<&str>) {
        if !self.config.track_allocations {
            return;
        }

        let mut state = self.state.lock().unwrap();

        // Check for double-free
        let double_free_info = state.allocations.get(&address).and_then(|entry| {
            if entry.is_freed {
                Some(entry.free_site.clone())
            } else {
                None
            }
        });
        if let Some(first_free) = double_free_info {
            state.issues.push(MemoryReport {
                issue: MemoryIssue::DoubleFree {
                    address,
                    first_free,
                    second_free: site.unwrap_or("unknown").to_string(),
                },
                source: String::new(),
                stack_trace: None,
                timestamp: Instant::now(),
                severity: 9,
                detected_by_cbgr: false,
            });
            return;
        }

        // Check if address was ever allocated
        if !state.allocations.contains_key(&address) {
            state.issues.push(MemoryReport {
                issue: MemoryIssue::InvalidPointer {
                    address,
                    reason: "Freeing unallocated memory".to_string(),
                },
                source: String::new(),
                stack_trace: None,
                timestamp: Instant::now(),
                severity: 8,
                detected_by_cbgr: false,
            });
            return;
        }

        // Get the entry size and prepare quarantine entry before mutating
        let (entry_size, quarantine_entry) = {
            if let Some(entry) = state.allocations.get_mut(&address) {
                entry.is_freed = true;
                entry.free_site = site.map(String::from);
                let size = entry.size;
                let quarantine = if self.config.quarantine_size > 0 {
                    Some(entry.clone())
                } else {
                    None
                };
                (Some(size), quarantine)
            } else {
                (None, None)
            }
        };

        // Update stats now that the mutable borrow is released
        if let Some(size) = entry_size {
            state.stats.total_frees += 1;
            state.stats.total_bytes_freed += size;
            state.stats.current_memory_usage -= size;
            state.stats.live_allocations -= 1;
        }

        // Move to quarantine
        if let Some(quarantine_entry) = quarantine_entry {
            state.quarantine.push(quarantine_entry);

            // Trim quarantine if too large
            while state.quarantine.len() > self.config.quarantine_size {
                state.quarantine.remove(0);
            }
        }
    }

    /// Record a memory access and check for issues
    pub fn record_access(&self, address: usize, size: usize, is_write: bool, site: &str) {
        let mut state = self.state.lock().unwrap();

        // Find containing allocation
        let mut containing_allocation = None;
        for (alloc_addr, entry) in &state.allocations {
            if address >= *alloc_addr && address < *alloc_addr + entry.size {
                containing_allocation = Some((*alloc_addr, entry.clone()));
                break;
            }
        }

        match containing_allocation {
            Some((alloc_addr, entry)) => {
                // Check if freed
                if entry.is_freed {
                    state.issues.push(MemoryReport {
                        issue: MemoryIssue::UseAfterFree {
                            address,
                            freed_at: entry.free_site.clone(),
                            accessed_at: site.to_string(),
                        },
                        source: String::new(),
                        stack_trace: None,
                        timestamp: Instant::now(),
                        severity: 10,
                        detected_by_cbgr: false,
                    });
                    return;
                }

                // Check bounds
                if self.config.check_bounds {
                    let offset = address - alloc_addr;
                    if offset + size > entry.size {
                        state.issues.push(MemoryReport {
                            issue: MemoryIssue::BufferOverflow {
                                address,
                                allocation_size: entry.size,
                                access_offset: offset,
                                direction: if is_write {
                                    OverflowDirection::Write
                                } else {
                                    OverflowDirection::Read
                                },
                            },
                            source: String::new(),
                            stack_trace: None,
                            timestamp: Instant::now(),
                            severity: 9,
                            detected_by_cbgr: false,
                        });
                    }
                }
            }
            None => {
                // Check quarantine for use-after-free
                // First, find the matching entry and clone its data
                let quarantine_match = state
                    .quarantine
                    .iter()
                    .find(|entry| address >= entry.address && address < entry.address + entry.size)
                    .map(|entry| entry.free_site.clone());

                if let Some(freed_at) = quarantine_match {
                    state.issues.push(MemoryReport {
                        issue: MemoryIssue::UseAfterFree {
                            address,
                            freed_at,
                            accessed_at: site.to_string(),
                        },
                        source: String::new(),
                        stack_trace: None,
                        timestamp: Instant::now(),
                        severity: 10,
                        detected_by_cbgr: false,
                    });
                    return;
                }

                // Unknown memory access
                state.issues.push(MemoryReport {
                    issue: MemoryIssue::InvalidPointer {
                        address,
                        reason: "Accessing untracked memory".to_string(),
                    },
                    source: String::new(),
                    stack_trace: None,
                    timestamp: Instant::now(),
                    severity: 7,
                    detected_by_cbgr: false,
                });
            }
        }
    }

    /// Create a reference and get its generation
    pub fn create_reference(&self, address: usize, is_mutable: bool, site: &str) -> u32 {
        if !self.config.track_references {
            return 0;
        }

        let mut state = self.state.lock().unwrap();

        // Get generation from allocation
        let generation = state
            .allocations
            .get(&address)
            .map(|e| e.generation)
            .unwrap_or(0);

        let entry = ReferenceEntry {
            address,
            generation,
            epoch: state.current_epoch,
            creation_site: site.to_string(),
            is_mutable,
        };

        state
            .references
            .entry(address)
            .or_insert_with(Vec::new)
            .push(entry);

        generation
    }

    /// Validate a reference access (CBGR check)
    pub fn validate_reference(&self, address: usize, expected_generation: u32, site: &str) -> bool {
        if !self.config.validate_generations {
            return true;
        }

        let mut state = self.state.lock().unwrap();
        state.stats.reference_validations += 1;
        state.stats.generation_checks += 1;

        // Get current generation from allocation
        let current_generation = state
            .allocations
            .get(&address)
            .map(|e| e.generation)
            .unwrap_or(0);

        if current_generation != expected_generation {
            state.issues.push(MemoryReport {
                issue: MemoryIssue::GenerationMismatch {
                    expected: expected_generation,
                    actual: current_generation,
                    location: site.to_string(),
                },
                source: String::new(),
                stack_trace: None,
                timestamp: Instant::now(),
                severity: 10,
                detected_by_cbgr: true,
            });
            return false;
        }

        // Check epoch if enabled
        if self.config.enable_epochs {
            // First, collect epoch violation info to avoid borrow conflict
            let current_epoch = state.current_epoch;
            let epoch_violation = state.references.get(&address).and_then(|refs| {
                refs.iter()
                    .find(|ref_entry| {
                        ref_entry.generation == expected_generation
                            && ref_entry.epoch != current_epoch
                    })
                    .map(|ref_entry| ref_entry.epoch)
            });

            if let Some(reference_epoch) = epoch_violation {
                state.issues.push(MemoryReport {
                    issue: MemoryIssue::EpochViolation {
                        reference_epoch,
                        current_epoch,
                    },
                    source: String::new(),
                    stack_trace: None,
                    timestamp: Instant::now(),
                    severity: 8,
                    detected_by_cbgr: true,
                });
                return false;
            }
        }

        true
    }

    /// Begin a new epoch
    pub fn begin_epoch(&self) {
        if !self.config.enable_epochs {
            return;
        }

        let mut state = self.state.lock().unwrap();
        state.current_epoch += 1;
        state.stats.epoch_transitions += 1;
    }

    /// Check for memory leaks
    pub fn check_leaks(&self) -> Vec<MemoryReport> {
        if !self.config.detect_leaks {
            return Vec::new();
        }

        let state = self.state.lock().unwrap();
        let mut leaks = Vec::new();

        for entry in state.allocations.values() {
            if !entry.is_freed {
                leaks.push(MemoryReport {
                    issue: MemoryIssue::Leak {
                        size: entry.size,
                        allocation_site: entry.allocation_site.clone(),
                    },
                    source: String::new(),
                    stack_trace: None,
                    timestamp: Instant::now(),
                    severity: 5,
                    detected_by_cbgr: false,
                });
            }
        }

        leaks
    }

    /// Get all detected issues
    pub fn get_issues(&self) -> Vec<MemoryReport> {
        let mut issues = self.state.lock().unwrap().issues.clone();

        // Add leak detection
        issues.extend(self.check_leaks());

        issues
    }

    /// Get memory statistics
    pub fn get_stats(&self) -> MemoryStats {
        self.state.lock().unwrap().stats.clone()
    }

    /// Reset the harness state
    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        *state = MemoryState::new();
    }

    /// Run memory analysis on source code
    pub fn analyze(&self, source: &str) -> MemoryAnalysisResult {
        // In production, this would:
        // 1. Compile the source with instrumentation
        // 2. Run the program with tracking enabled
        // 3. Collect and return issues

        // Placeholder implementation
        let issues = self.get_issues();
        let stats = self.get_stats();

        MemoryAnalysisResult {
            source: source.to_string(),
            issues,
            stats,
            duration: Duration::from_millis(100),
        }
    }
}

/// Result of memory analysis
#[derive(Debug)]
pub struct MemoryAnalysisResult {
    pub source: String,
    pub issues: Vec<MemoryReport>,
    pub stats: MemoryStats,
    pub duration: Duration,
}

impl MemoryAnalysisResult {
    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }

    pub fn critical_issues(&self) -> Vec<&MemoryReport> {
        self.issues.iter().filter(|i| i.severity >= 8).collect()
    }

    pub fn cbgr_issues(&self) -> Vec<&MemoryReport> {
        self.issues.iter().filter(|i| i.detected_by_cbgr).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_harness_creation() {
        let harness = MemoryHarness::new(MemoryConfig::default());
        assert_eq!(harness.get_issues().len(), 0);
    }

    #[test]
    fn test_allocation_tracking() {
        let harness = MemoryHarness::new(MemoryConfig::default());

        harness.record_allocation(0x1000, 100, Some("test.vr:10"));
        let stats = harness.get_stats();

        assert_eq!(stats.total_allocations, 1);
        assert_eq!(stats.total_bytes_allocated, 100);
        assert_eq!(stats.live_allocations, 1);
    }

    #[test]
    fn test_double_free_detection() {
        let harness = MemoryHarness::new(MemoryConfig::default());

        harness.record_allocation(0x1000, 100, None);
        harness.record_free(0x1000, Some("first"));
        harness.record_free(0x1000, Some("second"));

        let issues = harness.get_issues();
        assert!(
            issues
                .iter()
                .any(|i| matches!(&i.issue, MemoryIssue::DoubleFree { .. }))
        );
    }

    #[test]
    fn test_generation_mismatch() {
        let harness = MemoryHarness::new(MemoryConfig::default());

        harness.record_allocation(0x1000, 100, None);
        let gen1 = harness.create_reference(0x1000, false, "ref1");

        // Free and reallocate at same address
        harness.record_free(0x1000, None);
        harness.record_allocation(0x1000, 100, None);

        // Old generation should mismatch
        let valid = harness.validate_reference(0x1000, gen1, "access");
        assert!(!valid);

        let issues = harness.get_issues();
        assert!(issues.iter().any(|i| i.detected_by_cbgr));
    }

    #[test]
    fn test_memory_stats() {
        let harness = MemoryHarness::new(MemoryConfig::default());

        harness.record_allocation(0x1000, 100, None);
        harness.record_allocation(0x2000, 200, None);
        harness.record_free(0x1000, None);

        let stats = harness.get_stats();
        assert_eq!(stats.total_allocations, 2);
        assert_eq!(stats.total_frees, 1);
        assert_eq!(stats.total_bytes_allocated, 300);
        assert_eq!(stats.total_bytes_freed, 100);
        assert_eq!(stats.current_memory_usage, 200);
    }
}
