//! Field-Sensitive Heap Tracking for CBGR Escape Analysis
//!
//! Enables per-field promotion decisions: if struct field A escapes to heap but
//! field B stays local, field B can still be promoted to &checked T (0ns) while
//! field A keeps full CBGR tracking (~15ns). This is critical for structs where
//! only some fields are stored in heap-allocated containers.
//!
//! This module implements production-grade field-sensitive heap tracking that
//! enables independent escape analysis for struct fields with respect to heap
//! allocations. This allows promotion of field references even when other fields
//! of the same struct escape to the heap.
//!
//! # Key Innovation
//!
//! Traditional heap escape analysis treats entire structs atomically. If any
//! field escapes to heap, the whole struct is marked as escaping. Field-sensitive
//! heap tracking analyzes each field independently, significantly improving
//! promotion opportunities.
//!
//! # Example
//!
//! ```rust,ignore
//! struct Data {
//!     cache: Vec<u8>,  // Stored in heap container → escapes to heap
//!     count: i32,      // Only accessed locally → does NOT escape
//! }
//!
//! fn process(d: &Data) -> i32 {
//!     // Without field-sensitive heap tracking:
//!     // - Entire &Data cannot be promoted (conservative)
//!
//!     // With field-sensitive heap tracking:
//!     // - d.cache: escapes to heap (CBGR required)
//!     // - d.count: does NOT escape (can promote to &checked i32)
//!
//!     d.count  // 0ns access with promotion!
//! }
//! ```
//!
//! # Core Components
//!
//! - [`FieldHeapInfo`]: Per-field heap escape information
//! - [`HeapStore`]: Tracks heap store operations
//! - [`FieldHeapTracker`]: Main tracking engine
//! - [`FieldHeapResult`]: Complete analysis results
//!
//! # Performance
//!
//! - **Complexity**: O(fields × `heap_stores`)
//! - **Typical overhead**: 2-5x base heap escape analysis
//! - **Memory**: ~120 bytes per field + heap store tracking
//! - **Target**: <100ms for 10K LOC with field-sensitive analysis

use std::fmt;
use verum_common::{List, Map, Maybe, Set, Text};

use crate::analysis::{EscapeResult, FieldPath, RefId};

/// Heap allocation site identifier
///
/// Uniquely identifies a heap allocation location in the program.
/// Used to track which heap allocations a reference may flow to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeapSiteId(pub u64);

impl fmt::Display for HeapSiteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "heap#{}", self.0)
    }
}

/// Heap store operation
///
/// Represents a store operation that writes a reference to a heap-allocated
/// memory location. Field-sensitive tracking analyzes which specific fields
/// flow into which heap stores.
///
/// # Example
///
/// ```rust,ignore
/// let data = Data { x: 1, y: 2 };
/// let boxed = Box::new(Container::default());
/// boxed.field = &data.x;  // HeapStore targeting data.x to boxed.field
/// ```
#[derive(Debug, Clone)]
pub struct HeapStore {
    /// Unique identifier for this heap store
    pub id: u64,

    /// Reference being stored
    pub reference: RefId,

    /// Field path within the reference being stored
    pub field_path: FieldPath,

    /// Heap allocation site receiving the store
    pub target_heap_site: HeapSiteId,

    /// Whether this is a definite escape (vs may-escape)
    pub is_definite: bool,
}

impl HeapStore {
    /// Create a new heap store operation
    #[must_use]
    pub fn new(
        id: u64,
        reference: RefId,
        field_path: FieldPath,
        target_heap_site: HeapSiteId,
    ) -> Self {
        Self {
            id,
            reference,
            field_path,
            target_heap_site,
            is_definite: true,
        }
    }

    /// Create a may-escape heap store (conservative)
    #[must_use]
    pub fn may_escape(
        id: u64,
        reference: RefId,
        field_path: FieldPath,
        target_heap_site: HeapSiteId,
    ) -> Self {
        Self {
            id,
            reference,
            field_path,
            target_heap_site,
            is_definite: false,
        }
    }

    /// Check if this store affects a specific field path
    ///
    /// Returns true if the stored field path aliases with the queried path.
    /// Uses field aliasing rules (prefix relationships).
    #[must_use]
    pub fn affects_field(&self, query_path: &FieldPath) -> bool {
        self.field_path.may_alias(query_path)
    }
}

/// Per-field heap escape information
///
/// Tracks whether a specific field of a reference escapes to heap,
/// independent of other fields in the same struct.
///
/// # Properties
///
/// - `escapes_to_heap`: Whether this field flows to heap storage
/// - `heap_sites`: Set of heap allocation sites this field escapes to
/// - `store_operations`: List of heap store operations affecting this field
/// - `is_conservative`: Whether analysis is conservative (unknown stores)
///
/// # Example
///
/// ```rust,ignore
/// // Field d.cache escapes to multiple heap sites
/// let cache_info = FieldHeapInfo {
///     field_path: FieldPath::named("cache"),
///     escapes_to_heap: true,
///     heap_sites: vec![HeapSiteId(1), HeapSiteId(2)].into_iter().collect(),
///     // ...
/// };
///
/// // Field d.count does NOT escape
/// let count_info = FieldHeapInfo {
///     field_path: FieldPath::named("count"),
///     escapes_to_heap: false,
///     heap_sites: Set::new(),
///     // ...
/// };
/// ```
#[derive(Debug, Clone)]
pub struct FieldHeapInfo {
    /// The field path this info applies to
    pub field_path: FieldPath,

    /// Whether this field escapes to heap
    pub escapes_to_heap: bool,

    /// Set of heap allocation sites this field escapes to
    pub heap_sites: Set<HeapSiteId>,

    /// Store operations that cause this field to escape
    pub store_operations: List<HeapStore>,

    /// Conservative flag (unknown stores present)
    pub is_conservative: bool,

    /// Number of definite heap escapes
    pub definite_escapes: usize,

    /// Number of may-escape stores (conservative)
    pub may_escapes: usize,
}

impl FieldHeapInfo {
    /// Create new field heap info (no escapes initially)
    #[must_use]
    pub fn new(field_path: FieldPath) -> Self {
        Self {
            field_path,
            escapes_to_heap: false,
            heap_sites: Set::new(),
            store_operations: List::new(),
            is_conservative: false,
            definite_escapes: 0,
            may_escapes: 0,
        }
    }

    /// Record a heap store affecting this field
    pub fn add_heap_store(&mut self, store: HeapStore) {
        if store.affects_field(&self.field_path) {
            self.escapes_to_heap = true;
            self.heap_sites.insert(store.target_heap_site);

            if store.is_definite {
                self.definite_escapes += 1;
            } else {
                self.may_escapes += 1;
            }

            self.store_operations.push(store);
        }
    }

    /// Mark as conservative (unknown heap stores)
    pub fn mark_conservative(&mut self) {
        self.is_conservative = true;
        self.escapes_to_heap = true;
    }

    /// Check if this field can be promoted (does not escape to heap)
    #[must_use]
    pub fn can_promote(&self) -> bool {
        !self.escapes_to_heap && !self.is_conservative
    }

    /// Get the escape result for this field
    #[must_use]
    pub fn escape_result(&self) -> EscapeResult {
        if self.can_promote() {
            EscapeResult::DoesNotEscape
        } else {
            EscapeResult::EscapesViaHeap
        }
    }

    /// Merge another field's heap info into this one (conservative)
    pub fn merge(&mut self, other: &FieldHeapInfo) {
        // Conservative merge: union of all escapes
        self.escapes_to_heap |= other.escapes_to_heap;
        self.is_conservative |= other.is_conservative;

        // Union of heap sites
        for &site in &other.heap_sites {
            self.heap_sites.insert(site);
        }

        // Merge store operations
        for store in &other.store_operations {
            self.store_operations.push(store.clone());
        }

        self.definite_escapes += other.definite_escapes;
        self.may_escapes += other.may_escapes;
    }
}

/// Field-sensitive heap tracking result
///
/// Complete analysis result tracking heap escape status for all fields
/// of a reference. Enables independent promotion decisions per field.
///
/// # Statistics
///
/// - `total_fields`: Number of fields analyzed
/// - `escaping_fields`: Number of fields that escape to heap
/// - `promotable_fields`: Number of fields that can be promoted
/// - `heap_sites_accessed`: Total unique heap sites referenced
///
/// # Example
///
/// ```rust,ignore
/// let result = tracker.analyze(reference);
///
/// // Check specific field
/// if result.can_promote_field(&FieldPath::named("count")) {
///     println!("Can promote count field!");
/// }
///
/// // Get statistics
/// println!("Promotable: {}/{} fields",
///     result.promotable_count(),
///     result.total_fields()
/// );
/// ```
#[derive(Debug, Clone)]
pub struct FieldHeapResult {
    /// Reference being analyzed
    pub reference: RefId,

    /// Per-field heap information tracking where each field was stored
    pub field_info: Map<FieldPath, FieldHeapInfo>,

    /// Overall result: true if any field of the base reference escapes to heap
    pub base_escapes_to_heap: bool,

    /// Union of all heap sites accessed by any field of this reference
    pub all_heap_sites: Set<HeapSiteId>,

    /// Total number of store operations across all fields
    pub total_stores: usize,
    /// Number of definite heap escapes (must-alias stores)
    pub definite_stores: usize,
    /// Number of potential heap escapes (may-alias stores)
    pub may_stores: usize,
}

impl FieldHeapResult {
    /// Create new empty result
    #[must_use]
    pub fn new(reference: RefId) -> Self {
        Self {
            reference,
            field_info: Map::new(),
            base_escapes_to_heap: false,
            all_heap_sites: Set::new(),
            total_stores: 0,
            definite_stores: 0,
            may_stores: 0,
        }
    }

    /// Add field heap information
    pub fn add_field_info(&mut self, info: FieldHeapInfo) {
        self.base_escapes_to_heap |= info.escapes_to_heap;

        // Union of all heap sites
        for &site in &info.heap_sites {
            self.all_heap_sites.insert(site);
        }

        self.total_stores += info.store_operations.len();
        self.definite_stores += info.definite_escapes;
        self.may_stores += info.may_escapes;

        self.field_info.insert(info.field_path.clone(), info);
    }

    /// Check if a specific field escapes to heap
    ///
    /// For field-sensitive analysis, unknown fields are assumed NOT to escape.
    /// If the caller wants conservative behavior for unknown fields, they should
    /// check `base_escapes_to_heap` separately.
    #[must_use]
    pub fn field_escapes_to_heap(&self, field_path: &FieldPath) -> bool {
        // For field-sensitive analysis, only report escape if we have specific info
        // Unknown fields are NOT assumed to escape (field-sensitivity precision)
        self.field_info
            .get(field_path)
            .is_some_and(|info| info.escapes_to_heap)
    }

    /// Check if a specific field can be promoted
    #[must_use]
    pub fn can_promote_field(&self, field_path: &FieldPath) -> bool {
        self.field_info
            .get(field_path)
            .is_some_and(FieldHeapInfo::can_promote) // Conservative: unknown fields cannot promote
    }

    /// Get all promotable field paths
    #[must_use]
    pub fn promotable_fields(&self) -> Set<FieldPath> {
        self.field_info
            .iter()
            .filter(|(_, info)| info.can_promote())
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// Get all escaping field paths
    #[must_use]
    pub fn escaping_fields(&self) -> Set<FieldPath> {
        self.field_info
            .iter()
            .filter(|(_, info)| info.escapes_to_heap)
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// Get total number of fields analyzed
    #[must_use]
    pub fn total_fields(&self) -> usize {
        self.field_info.len()
    }

    /// Get number of promotable fields
    #[must_use]
    pub fn promotable_count(&self) -> usize {
        self.promotable_fields().len()
    }

    /// Get number of escaping fields
    #[must_use]
    pub fn escaping_count(&self) -> usize {
        self.escaping_fields().len()
    }

    /// Get promotion rate (0.0-1.0)
    #[must_use]
    pub fn promotion_rate(&self) -> f64 {
        if self.total_fields() == 0 {
            0.0
        } else {
            self.promotable_count() as f64 / self.total_fields() as f64
        }
    }

    /// Get heap sites accessed by a specific field
    #[must_use]
    pub fn field_heap_sites(&self, field_path: &FieldPath) -> Set<HeapSiteId> {
        self.field_info
            .get(field_path)
            .map(|info| info.heap_sites.clone())
            .unwrap_or_default()
    }

    /// Merge another result into this one (conservative union)
    pub fn merge(&mut self, other: &FieldHeapResult) {
        self.base_escapes_to_heap |= other.base_escapes_to_heap;

        // Union of all heap sites
        for &site in &other.all_heap_sites {
            self.all_heap_sites.insert(site);
        }

        // Merge field information
        for (path, other_info) in &other.field_info {
            if let Maybe::Some(existing_info) = self.field_info.get_mut(path) {
                existing_info.merge(other_info);
            } else {
                self.field_info.insert(path.clone(), other_info.clone());
            }
        }

        self.total_stores += other.total_stores;
        self.definite_stores += other.definite_stores;
        self.may_stores += other.may_stores;
    }
}

/// Field-sensitive heap tracker
///
/// Main engine for tracking heap allocations per field. Analyzes store
/// operations to determine which fields escape to heap independently.
///
/// # Algorithm
///
/// 1. **Extract field paths** - Identify all field accesses for reference
/// 2. **Track heap stores** - Collect all store operations to heap locations
/// 3. **Analyze per field** - For each field, check which heap stores affect it
/// 4. **Compute results** - Generate per-field heap escape information
///
/// # Performance
///
/// - **Complexity**: O(fields × stores)
/// - **Typical case**: 5 fields × 10 stores = 50 checks
/// - **Target**: <100µs for typical struct
///
/// # Example
///
/// ```rust,ignore
/// use verum_cbgr::field_heap_tracking::FieldHeapTracker;
///
/// let mut tracker = FieldHeapTracker::new();
///
/// // Track heap allocation site
/// let heap_site = tracker.register_heap_allocation("Box::new");
///
/// // Record heap store
/// tracker.add_heap_store(
///     reference,
///     FieldPath::named("cache"),
///     heap_site,
///     true  // definite escape
/// );
///
/// // Analyze
/// let result = tracker.analyze(reference);
/// assert!(result.field_escapes_to_heap(&FieldPath::named("cache")));
/// assert!(!result.can_promote_field(&FieldPath::named("cache")));
/// ```
#[derive(Debug, Clone)]
pub struct FieldHeapTracker {
    /// All heap stores tracked
    heap_stores: List<HeapStore>,

    /// Counter for generating unique store IDs
    next_store_id: u64,

    /// Counter for generating heap site IDs
    next_heap_site_id: u64,

    /// Registered heap allocation sites
    heap_allocation_sites: Map<HeapSiteId, Text>,

    /// Per-reference field paths extracted
    field_paths: Map<RefId, Set<FieldPath>>,
}

impl FieldHeapTracker {
    /// Create a new field heap tracker
    #[must_use]
    pub fn new() -> Self {
        Self {
            heap_stores: List::new(),
            next_store_id: 0,
            next_heap_site_id: 0,
            heap_allocation_sites: Map::new(),
            field_paths: Map::new(),
        }
    }

    /// Register a heap allocation site
    ///
    /// Returns a unique `HeapSiteId` for this allocation.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let box_site = tracker.register_heap_allocation("Box::new");
    /// let vec_site = tracker.register_heap_allocation("Vec::push");
    /// ```
    pub fn register_heap_allocation(&mut self, description: impl Into<Text>) -> HeapSiteId {
        let site_id = HeapSiteId(self.next_heap_site_id);
        self.next_heap_site_id += 1;
        self.heap_allocation_sites
            .insert(site_id, description.into());
        site_id
    }

    /// Add a heap store operation
    ///
    /// Records that a reference field is stored to a heap location.
    ///
    /// # Parameters
    ///
    /// - `reference`: The reference whose field is being stored
    /// - `field_path`: The specific field being stored
    /// - `heap_site`: The heap allocation site receiving the store
    /// - `is_definite`: Whether this is a definite escape (true) or may-escape (false)
    pub fn add_heap_store(
        &mut self,
        reference: RefId,
        field_path: FieldPath,
        heap_site: HeapSiteId,
        is_definite: bool,
    ) {
        let store_id = self.next_store_id;
        self.next_store_id += 1;

        let store = if is_definite {
            HeapStore::new(store_id, reference, field_path, heap_site)
        } else {
            HeapStore::may_escape(store_id, reference, field_path, heap_site)
        };

        self.heap_stores.push(store);
    }

    /// Register field paths for a reference
    ///
    /// Should be called before analysis to establish which fields exist.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut paths = Set::new();
    /// paths.insert(FieldPath::named("x"));
    /// paths.insert(FieldPath::named("y"));
    /// tracker.register_fields(reference, paths);
    /// ```
    pub fn register_fields(&mut self, reference: RefId, paths: Set<FieldPath>) {
        self.field_paths.insert(reference, paths);
    }

    /// Track heap allocations for all fields of a reference
    ///
    /// Main analysis entry point. Analyzes all heap stores and determines
    /// which fields escape to heap.
    ///
    /// # Algorithm
    ///
    /// 1. Get or extract field paths for reference
    /// 2. For each field path:
    ///    - Create `FieldHeapInfo`
    ///    - Find all heap stores affecting this field
    ///    - Record escape information
    /// 3. Build and return `FieldHeapResult`
    ///
    /// # Returns
    ///
    /// Complete field-sensitive heap analysis result
    #[must_use]
    pub fn track_field_heap_allocations(&self, reference: RefId) -> FieldHeapResult {
        let mut result = FieldHeapResult::new(reference);

        // Get field paths (or use base reference if none registered)
        let field_paths = self
            .field_paths
            .get(&reference)
            .cloned()
            .unwrap_or_else(|| {
                let mut paths = Set::new();
                paths.insert(FieldPath::new()); // Base reference
                paths
            });

        // Analyze each field independently
        for field_path in field_paths {
            let mut field_info = FieldHeapInfo::new(field_path.clone());

            // Find all heap stores affecting this field
            for store in &self.heap_stores {
                if store.reference == reference && store.affects_field(&field_path) {
                    field_info.add_heap_store(store.clone());
                }
            }

            result.add_field_info(field_info);
        }

        result
    }

    /// Check if a specific field escapes to heap
    ///
    /// Convenience method for quick field escape queries.
    #[must_use]
    pub fn field_escapes_to_heap(&self, reference: RefId, field_path: &FieldPath) -> bool {
        self.heap_stores
            .iter()
            .any(|store| store.reference == reference && store.affects_field(field_path))
    }

    /// Refine field escape using heap tracking
    ///
    /// Integrates field heap tracking with existing escape analysis results.
    /// If a field escapes to heap according to heap tracking, the escape
    /// result is updated to `EscapesViaHeap`.
    ///
    /// # Parameters
    ///
    /// - `reference`: The reference being analyzed
    /// - `field_path`: The specific field
    /// - `current_result`: Current escape analysis result
    ///
    /// # Returns
    ///
    /// Refined escape result incorporating heap tracking
    #[must_use]
    pub fn refine_field_escape_with_heap(
        &self,
        reference: RefId,
        field_path: &FieldPath,
        current_result: EscapeResult,
    ) -> EscapeResult {
        // If already known to escape, keep that result
        if !current_result.can_promote() {
            return current_result;
        }

        // Check if field escapes to heap according to heap tracking
        if self.field_escapes_to_heap(reference, field_path) {
            EscapeResult::EscapesViaHeap
        } else {
            current_result
        }
    }

    /// Get statistics about heap tracking
    #[must_use]
    pub fn statistics(&self) -> HeapTrackingStatistics {
        let total_heap_sites = self.heap_allocation_sites.len();
        let total_stores = self.heap_stores.len();
        let definite_stores = self.heap_stores.iter().filter(|s| s.is_definite).count();
        let may_stores = total_stores - definite_stores;

        HeapTrackingStatistics {
            total_heap_sites,
            total_stores,
            definite_stores,
            may_stores,
            references_tracked: self.field_paths.len(),
        }
    }

    /// Clear all tracking data
    pub fn clear(&mut self) {
        self.heap_stores.clear();
        self.heap_allocation_sites.clear();
        self.field_paths.clear();
        self.next_store_id = 0;
        self.next_heap_site_id = 0;
    }
}

impl Default for FieldHeapTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about heap tracking
#[derive(Debug, Clone, Copy)]
pub struct HeapTrackingStatistics {
    /// Total number of heap allocation sites tracked
    pub total_heap_sites: usize,

    /// Total number of heap store operations
    pub total_stores: usize,

    /// Number of definite heap escapes
    pub definite_stores: usize,

    /// Number of may-escape stores (conservative)
    pub may_stores: usize,

    /// Number of references tracked
    pub references_tracked: usize,
}

impl HeapTrackingStatistics {
    /// Get a formatted report
    #[must_use]
    pub fn report(&self) -> Text {
        format!(
            "Heap Tracking Statistics:\n\
             - Heap sites: {}\n\
             - Total stores: {} ({} definite, {} may)\n\
             - References tracked: {}",
            self.total_heap_sites,
            self.total_stores,
            self.definite_stores,
            self.may_stores,
            self.references_tracked
        ).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heap_site_creation() {
        let site = HeapSiteId(42);
        assert_eq!(format!("{}", site), "heap#42");
    }

    #[test]
    fn test_heap_store_creation() {
        let store = HeapStore::new(1, RefId(1), FieldPath::named("field".into()), HeapSiteId(1));

        assert_eq!(store.id, 1);
        assert_eq!(store.reference, RefId(1));
        assert!(store.is_definite);
    }

    #[test]
    fn test_heap_store_affects_field() {
        let store = HeapStore::new(1, RefId(1), FieldPath::named("x".into()), HeapSiteId(1));

        assert!(store.affects_field(&FieldPath::named("x".into())));
        assert!(!store.affects_field(&FieldPath::named("y".into())));
    }

    #[test]
    fn test_field_heap_info_creation() {
        let info = FieldHeapInfo::new(FieldPath::named("field".into()));
        assert!(!info.escapes_to_heap);
        assert!(info.can_promote());
        assert_eq!(info.escape_result(), EscapeResult::DoesNotEscape);
    }

    #[test]
    fn test_field_heap_info_add_store() {
        let mut info = FieldHeapInfo::new(FieldPath::named("x".into()));

        let store = HeapStore::new(1, RefId(1), FieldPath::named("x".into()), HeapSiteId(1));

        info.add_heap_store(store);

        assert!(info.escapes_to_heap);
        assert!(!info.can_promote());
        assert_eq!(info.definite_escapes, 1);
        assert_eq!(info.heap_sites.len(), 1);
    }
}
