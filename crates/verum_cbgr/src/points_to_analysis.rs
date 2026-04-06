//! Points-to Analysis for CBGR Escape Analysis
//!
//! Implements Andersen-style inclusion-based points-to analysis for CBGR. Determines
//! which memory locations each reference may point to, enabling precise alias analysis.
//! Two references with disjoint points-to sets are guaranteed NoAlias, allowing
//! independent promotion decisions. Works with the escape analysis pipeline to refine
//! escape categories based on actual pointer relationships.
//!
//! This module implements production-grade Andersen-style points-to analysis to track
//! pointer relationships across the entire program. This enables precise alias analysis
//! and escape detection by determining what memory locations each reference may point to.
//!
//! # Algorithm Overview
//!
//! **Andersen's Algorithm** (inclusion-based points-to analysis):
//! - Complexity: O(n³) worst-case, O(n²) typical
//! - Precision: Flow-insensitive, context-insensitive
//! - Constraint-based: Generate and solve inclusion constraints
//!
//! # Constraint Types
//!
//! 1. **Address-of**: `x = &y` → pts(x) ⊇ {y}
//! 2. **Copy**: `x = y` → pts(x) ⊇ pts(y)
//! 3. **Load**: `x = *y` → pts(x) ⊇ ⋃{pts(z) | z ∈ pts(y)}
//! 4. **Store**: `*x = y` → ∀z ∈ pts(x): pts(z) ⊇ pts(y)
//!
//! # Example
//!
//! ```rust,ignore
//! fn example() {
//!     let x = 42;          // Allocation: x
//!     let y = &x;          // Address-of: pts(y) = {x}
//!     let z = y;           // Copy: pts(z) = pts(y) = {x}
//!     let w = *z;          // Load: pts(w) = ⋃{pts(a) | a ∈ pts(z)} = ∅
//! }
//! ```
//!
//! # Performance
//!
//! Target: O(n³) worst-case, O(n) for typical programs
//! - Constraint generation: O(n) where n = instructions
//! - Constraint solving: O(n³) worst-case with optimizations
//! - Typical: O(n) to O(n²) for real programs
//!
//! # Integration
//!
//! Points-to analysis integrates with:
//! - Alias analysis: Precise may-alias/must-alias queries
//! - Escape analysis: Heap escape detection
//! - SSA representation: Precise use-def tracking
//! - Call graph: Interprocedural points-to propagation

use std::fmt;
use verum_common::{List, Map, Maybe, Set};

use crate::analysis::{AliasSets, ControlFlowGraph, RefId};

// ==================================================================================
// Core Data Structures
// ==================================================================================

/// Points-to location identifier
///
/// Represents a memory location that a pointer may point to.
/// This can be a stack allocation, heap allocation, or abstract location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocationId(pub u64);

/// Field identifier for field-sensitive analysis
///
/// Represents a specific field within a struct or tuple type.
/// Field 0 is typically the base object itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldId(pub u32);

impl FieldId {
    /// The base field (represents the entire object, not a specific field)
    pub const BASE: FieldId = FieldId(0);

    /// Create a new field ID
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Check if this is the base field
    #[must_use]
    pub fn is_base(&self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for FieldId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_base() {
            write!(f, "base")
        } else {
            write!(f, "field_{}", self.0)
        }
    }
}

/// Field-sensitive location combining a base location with a field offset
///
/// This enables tracking points-to relationships at the field level,
/// allowing for more precise alias analysis of struct members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldLocation {
    /// Base location (the struct/object allocation)
    pub base: LocationId,
    /// Field within the base location
    pub field: FieldId,
}

impl FieldLocation {
    /// Create a new field location
    #[must_use]
    pub fn new(base: LocationId, field: FieldId) -> Self {
        Self { base, field }
    }

    /// Create a base location (field 0)
    #[must_use]
    pub fn base_of(loc: LocationId) -> Self {
        Self {
            base: loc,
            field: FieldId::BASE,
        }
    }

    /// Get the base location ID (for backward compatibility)
    #[must_use]
    pub fn location_id(&self) -> LocationId {
        // Encode field into location for compatibility
        if self.field.is_base() {
            self.base
        } else {
            // High bits: base, low bits: field offset (shifted)
            LocationId(self.base.0 << 20 | u64::from(self.field.0))
        }
    }
}

impl fmt::Display for FieldLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.field.is_base() {
            write!(f, "{}", self.base)
        } else {
            write!(f, "{}.{}", self.base, self.field)
        }
    }
}

impl fmt::Display for LocationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Loc({})", self.0)
    }
}

/// Variable identifier for points-to analysis
///
/// Represents a program variable (SSA version) that can hold a pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarId(pub u64);

impl fmt::Display for VarId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Var({})", self.0)
    }
}

/// Points-to set for a single variable
///
/// Represents all memory locations that a variable may point to.
/// This is the fundamental building block of points-to analysis.
///
/// # Example
/// ```rust,ignore
/// let mut pts = PointsToSet::new(VarId(1));
/// pts.add_location(LocationId(42));  // Variable 1 may point to location 42
/// assert!(pts.may_point_to(LocationId(42)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointsToSet {
    /// Variable this set belongs to
    pub variable: VarId,

    /// Set of locations this variable may point to
    pub locations: Set<LocationId>,

    /// Whether this is a conservative approximation
    /// (true = may point to anything, used for unknown pointers)
    pub conservative: bool,
}

impl PointsToSet {
    /// Create new empty points-to set
    #[must_use]
    pub fn new(variable: VarId) -> Self {
        Self {
            variable,
            locations: Set::new(),
            conservative: false,
        }
    }

    /// Create a conservative points-to set (may point to anything)
    #[must_use]
    pub fn conservative(variable: VarId) -> Self {
        Self {
            variable,
            locations: Set::new(),
            conservative: true,
        }
    }

    /// Add a location to the points-to set
    ///
    /// Returns true if the set was modified (new location added)
    pub fn add_location(&mut self, location: LocationId) -> bool {
        if self.conservative {
            false // Already conservative, no change
        } else {
            self.locations.insert(location)
        }
    }

    /// Add all locations from another set (union operation)
    ///
    /// Returns true if the set was modified
    pub fn add_all(&mut self, other: &PointsToSet) -> bool {
        if other.conservative {
            if !self.conservative {
                self.conservative = true;
                return true;
            }
            return false;
        }

        let old_size = self.locations.len();
        self.locations.extend(other.locations.iter().copied());
        self.locations.len() > old_size
    }

    /// Check if variable may point to a specific location
    #[must_use]
    pub fn may_point_to(&self, location: LocationId) -> bool {
        self.conservative || self.locations.contains(&location)
    }

    /// Check if this is an empty set
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.conservative && self.locations.is_empty()
    }

    /// Get the number of locations (None if conservative)
    #[must_use]
    pub fn size(&self) -> Maybe<usize> {
        if self.conservative {
            Maybe::None
        } else {
            Maybe::Some(self.locations.len())
        }
    }
}

/// Location type classification
///
/// Tracks whether a location is on the stack or heap.
/// This is crucial for escape analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocationType {
    /// Stack-allocated location
    Stack,

    /// Heap-allocated location
    Heap,

    /// Global/static location
    Global,

    /// Unknown location type (conservative)
    Unknown,
}

impl LocationType {
    /// Check if this location is definitely on the heap
    #[must_use]
    pub fn is_heap(&self) -> bool {
        matches!(self, LocationType::Heap)
    }

    /// Check if this location is definitely on the stack
    #[must_use]
    pub fn is_stack(&self) -> bool {
        matches!(self, LocationType::Stack)
    }

    /// Check if this location may be on the heap
    #[must_use]
    pub fn may_be_heap(&self) -> bool {
        matches!(self, LocationType::Heap | LocationType::Unknown)
    }
}

/// Points-to constraint
///
/// Represents a constraint that must be satisfied in the points-to analysis.
/// Constraints are generated from the program CFG and solved iteratively.
///
/// Constraints generated from IR: AddressOf (x = &y), Copy (x = y), Load (x = *y),
/// Store (*x = y). Solved via Andersen's inclusion-based fixpoint iteration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointsToConstraint {
    /// Address-of constraint: x = &y
    /// Means: pts(x) ⊇ {y}
    AddressOf {
        /// Variable receiving the address
        variable: VarId,
        /// Location being addressed
        location: LocationId,
    },

    /// Copy constraint: x = y
    /// Means: pts(x) ⊇ pts(y)
    Copy {
        /// Destination variable
        dest: VarId,
        /// Source variable
        src: VarId,
    },

    /// Load constraint: x = *y
    /// Means: pts(x) ⊇ ⋃{pts(z) | z ∈ pts(y)}
    Load {
        /// Destination variable
        dest: VarId,
        /// Pointer being dereferenced
        ptr: VarId,
    },

    /// Store constraint: *x = y
    /// Means: ∀z ∈ pts(x): pts(z) ⊇ pts(y)
    Store {
        /// Pointer being stored through
        ptr: VarId,
        /// Value being stored
        value: VarId,
    },

    /// Field-sensitive address-of: x = &y.f
    /// Means: pts(x) ⊇ {y.f}
    FieldAddressOf {
        /// Variable receiving the field address
        variable: VarId,
        /// Base location
        base: LocationId,
        /// Field being addressed
        field: FieldId,
    },

    /// Field-sensitive load: x = y->f (or x = (*y).f)
    /// Means: pts(x) ⊇ ⋃{field_pts(z, f) | z ∈ pts(y)}
    FieldLoad {
        /// Destination variable
        dest: VarId,
        /// Pointer to struct
        ptr: VarId,
        /// Field being loaded
        field: FieldId,
    },

    /// Field-sensitive store: x->f = y (or (*x).f = y)
    /// Means: ∀z ∈ pts(x): field_pts(z, f) ⊇ pts(y)
    FieldStore {
        /// Pointer to struct
        ptr: VarId,
        /// Field being stored to
        field: FieldId,
        /// Value being stored
        value: VarId,
    },

    /// Interprocedural constraint: parameter passing
    /// Used to model call/return value flow
    Interprocedural {
        /// Caller variable (actual parameter or return receiver)
        caller_var: VarId,
        /// Callee variable (formal parameter or return value)
        callee_var: VarId,
        /// Direction of flow (true = caller → callee, false = callee → caller)
        is_call: bool,
    },
}

impl PointsToConstraint {
    /// Get all variables referenced by this constraint
    #[must_use]
    pub fn referenced_variables(&self) -> List<VarId> {
        match self {
            PointsToConstraint::AddressOf { variable, .. } => vec![*variable].into(),
            PointsToConstraint::Copy { dest, src } => vec![*dest, *src].into(),
            PointsToConstraint::Load { dest, ptr } => vec![*dest, *ptr].into(),
            PointsToConstraint::Store { ptr, value } => vec![*ptr, *value].into(),
            PointsToConstraint::FieldAddressOf { variable, .. } => vec![*variable].into(),
            PointsToConstraint::FieldLoad { dest, ptr, .. } => vec![*dest, *ptr].into(),
            PointsToConstraint::FieldStore { ptr, value, .. } => vec![*ptr, *value].into(),
            PointsToConstraint::Interprocedural {
                caller_var,
                callee_var,
                ..
            } => vec![*caller_var, *callee_var].into(),
        }
    }
}

// ==================================================================================
// Points-to Graph
// ==================================================================================

/// Points-to graph
///
/// Represents the complete points-to relationship for a program.
/// Implements a proper two-level points-to graph:
///
/// Level 1: Variable → {Locations}
///   Maps variables to the locations they may point to.
///
/// Level 2: Location → {Locations}
///   Maps locations to the locations that values stored there may point to.
///   This is critical for proper Load constraint handling.
///
/// For `x = *y`:
/// 1. Find all locations L that y points to: L ∈ pts(y)
/// 2. For each l ∈ L, find what values stored at l point to: stored_pts(l)
/// 3. Add all those targets to pts(x): pts(x) ⊇ ⋃{stored_pts(l) | l ∈ pts(y)}
///
/// Two-level structure: Level 1 maps variables to their points-to sets (possible
/// target locations). Level 2 maps locations to stored value targets (for Load
/// constraint resolution: x = *y requires following y's targets to find stored values).
#[derive(Debug, Clone)]
pub struct PointsToGraph {
    /// Points-to sets for all variables (Level 1: var → locations)
    points_to_sets: Map<VarId, PointsToSet>,

    /// Location metadata (stack vs heap)
    location_types: Map<LocationId, LocationType>,

    /// Variable to `RefId` mapping (for escape analysis integration)
    var_to_ref: Map<VarId, RefId>,

    /// `RefId` to Variable mapping
    ref_to_var: Map<RefId, VarId>,

    /// Per-location points-to sets (Level 2: location → stored values' targets)
    /// Maps `LocationId` -> Set of `LocationIds` that values stored there may point to.
    /// This is populated by Store constraints and read by Load constraints.
    location_points_to: Map<LocationId, Set<LocationId>>,

    /// Variable associated with each location (for interprocedural analysis)
    /// When we allocate a location, we may associate a variable with it
    /// so that Load constraints can find the stored variable's points-to set.
    location_to_var: Map<LocationId, VarId>,

    /// Field-sensitive points-to sets for struct fields
    /// Maps (base_location, field_id) → Set of locations
    field_points_to: Map<(LocationId, FieldId), Set<LocationId>>,

    /// Conservative location marker (LocationId::MAX sentinel)
    /// If a location contains this, it may point to anything
    conservative_locations: Set<LocationId>,
}

/// Sentinel value indicating conservative (may point to anything)
const CONSERVATIVE_SENTINEL: LocationId = LocationId(u64::MAX);

impl PointsToGraph {
    /// Create new empty points-to graph
    #[must_use]
    pub fn new() -> Self {
        Self {
            points_to_sets: Map::new(),
            location_types: Map::new(),
            var_to_ref: Map::new(),
            ref_to_var: Map::new(),
            location_points_to: Map::new(),
            location_to_var: Map::new(),
            field_points_to: Map::new(),
            conservative_locations: Set::new(),
        }
    }

    /// Get points-to set for a variable
    #[must_use]
    pub fn get_points_to_set(&self, var: VarId) -> Maybe<&PointsToSet> {
        self.points_to_sets.get(&var)
    }

    /// Get mutable points-to set for a variable (creates if not exists)
    fn get_or_create_pts(&mut self, var: VarId) -> &mut PointsToSet {
        self.points_to_sets
            .entry(var)
            .or_insert_with(|| PointsToSet::new(var))
    }

    /// Add a location to a variable's points-to set
    ///
    /// Returns true if the set was modified
    pub fn add_points_to(&mut self, var: VarId, location: LocationId) -> bool {
        self.get_or_create_pts(var).add_location(location)
    }

    /// Record location type (stack/heap/global)
    pub fn set_location_type(&mut self, location: LocationId, loc_type: LocationType) {
        self.location_types.insert(location, loc_type);
    }

    /// Get location type
    #[must_use]
    pub fn get_location_type(&self, location: LocationId) -> LocationType {
        self.location_types
            .get(&location)
            .copied()
            .unwrap_or(LocationType::Unknown)
    }

    /// Map `VarId` to `RefId` (for integration with escape analysis)
    pub fn map_var_to_ref(&mut self, var: VarId, reference: RefId) {
        self.var_to_ref.insert(var, reference);
        self.ref_to_var.insert(reference, var);
    }

    /// Get `RefId` for a `VarId`
    #[must_use]
    pub fn get_ref_for_var(&self, var: VarId) -> Maybe<RefId> {
        self.var_to_ref.get(&var).copied()
    }

    /// Get `VarId` for a `RefId`
    #[must_use]
    pub fn get_var_for_ref(&self, reference: RefId) -> Maybe<VarId> {
        self.ref_to_var.get(&reference).copied()
    }

    /// Check if two variables may alias
    #[must_use]
    pub fn may_alias(&self, var1: VarId, var2: VarId) -> bool {
        let pts1 = match self.get_points_to_set(var1) {
            Maybe::Some(pts) => pts,
            Maybe::None => return false,
        };

        let pts2 = match self.get_points_to_set(var2) {
            Maybe::Some(pts) => pts,
            Maybe::None => return false,
        };

        // Conservative if either is conservative
        if pts1.conservative || pts2.conservative {
            return true;
        }

        // Check for intersection
        !pts1.locations.is_disjoint(&pts2.locations)
    }

    /// Check if two variables must alias
    #[must_use]
    pub fn must_alias(&self, var1: VarId, var2: VarId) -> bool {
        let pts1 = match self.get_points_to_set(var1) {
            Maybe::Some(pts) => pts,
            Maybe::None => return false,
        };

        let pts2 = match self.get_points_to_set(var2) {
            Maybe::Some(pts) => pts,
            Maybe::None => return false,
        };

        // Must have exactly one location each, and they must be the same
        if pts1.conservative || pts2.conservative {
            return false;
        }

        pts1.locations.len() == 1 && pts1.locations == pts2.locations
    }

    /// Check if variable points to heap
    #[must_use]
    pub fn points_to_heap(&self, var: VarId) -> bool {
        let pts = match self.get_points_to_set(var) {
            Maybe::Some(pts) => pts,
            Maybe::None => return false,
        };

        // Conservative: assume may point to heap
        if pts.conservative {
            return true;
        }

        // Check if any location is on heap
        pts.locations
            .iter()
            .any(|&loc| self.get_location_type(loc).is_heap())
    }

    /// Get all variables in the graph
    #[must_use]
    pub fn all_variables(&self) -> List<VarId> {
        self.points_to_sets.keys().copied().collect()
    }

    /// Add a location to another location's points-to set (for Store constraint)
    ///
    /// This tracks what locations a value stored at `target_loc` may point to.
    /// Returns true if the set was modified.
    pub fn add_location_points_to(
        &mut self,
        target_loc: LocationId,
        points_to_loc: LocationId,
    ) -> bool {
        self.location_points_to
            .entry(target_loc)
            .or_default()
            .insert(points_to_loc)
    }

    /// Get all locations that values stored at this location may point to
    #[must_use]
    pub fn get_location_points_to(&self, location: LocationId) -> Maybe<&Set<LocationId>> {
        self.location_points_to.get(&location)
    }

    /// Union location points-to sets: `target_loc`'s pts ⊇ `source_pts`
    ///
    /// Returns true if target's set was modified.
    pub fn union_location_points_to(
        &mut self,
        target_loc: LocationId,
        source_pts: &Set<LocationId>,
    ) -> bool {
        if source_pts.is_empty() {
            return false;
        }

        let target_pts = self.location_points_to.entry(target_loc).or_default();

        let original_len = target_pts.len();
        for &loc in source_pts {
            target_pts.insert(loc);
        }
        target_pts.len() > original_len
    }

    // ==================================================================================
    // Two-Level Points-To Graph: Location-to-Variable Mapping
    // ==================================================================================

    /// Associate a variable with a location
    ///
    /// This creates the second level of the points-to graph, allowing Load constraints
    /// to properly resolve what a dereferenced pointer points to.
    ///
    /// When we have:
    ///   let x = &y;  // AddressOf: pts(x) = {loc_y}
    ///   let z = *x;  // Load: pts(z) = pts(value_at_loc_y)
    ///
    /// We need to know that loc_y is associated with variable y, so we can look up
    /// what y (or values stored at loc_y) points to.
    pub fn associate_location_with_var(&mut self, location: LocationId, var: VarId) {
        self.location_to_var.insert(location, var);
    }

    /// Get the variable associated with a location
    ///
    /// This is used by Load constraint handling to find the points-to set of the
    /// value stored at a location.
    #[must_use]
    pub fn get_var_for_location(&self, location: LocationId) -> Maybe<VarId> {
        self.location_to_var.get(&location).copied()
    }

    /// Check if a location is marked as conservative (may point to anything)
    #[must_use]
    pub fn is_location_conservative(&self, location: LocationId) -> bool {
        self.conservative_locations.contains(&location)
    }

    /// Mark a location as conservative
    pub fn mark_location_conservative(&mut self, location: LocationId) {
        self.conservative_locations.insert(location);
    }

    // ==================================================================================
    // Field-Sensitive Analysis Support
    // ==================================================================================

    /// Add a field-sensitive points-to relationship
    ///
    /// Records that the field `field` of allocation at `base` may point to `target`.
    /// Returns true if the set was modified.
    pub fn add_field_points_to(
        &mut self,
        base: LocationId,
        field: FieldId,
        target: LocationId,
    ) -> bool {
        self.field_points_to
            .entry((base, field))
            .or_default()
            .insert(target)
    }

    /// Get all locations that a specific field may point to
    #[must_use]
    pub fn get_field_points_to(&self, base: LocationId, field: FieldId) -> Maybe<&Set<LocationId>> {
        self.field_points_to.get(&(base, field))
    }

    /// Union field points-to sets
    ///
    /// Returns true if the target's set was modified.
    pub fn union_field_points_to(
        &mut self,
        base: LocationId,
        field: FieldId,
        source_pts: &Set<LocationId>,
    ) -> bool {
        if source_pts.is_empty() {
            return false;
        }

        let target_pts = self.field_points_to.entry((base, field)).or_default();
        let original_len = target_pts.len();

        for &loc in source_pts {
            target_pts.insert(loc);
        }

        target_pts.len() > original_len
    }

    /// Get all locations that a field location may point to
    /// (combines base location and field-specific information)
    #[must_use]
    pub fn get_field_location_points_to(&self, field_loc: &FieldLocation) -> Set<LocationId> {
        let mut result = Set::new();

        // First, check field-specific points-to
        if let Maybe::Some(field_pts) = self.get_field_points_to(field_loc.base, field_loc.field) {
            result.extend(field_pts.iter().copied());
        }

        // If field is base, also include location points-to
        if field_loc.field.is_base() {
            if let Maybe::Some(loc_pts) = self.get_location_points_to(field_loc.base) {
                result.extend(loc_pts.iter().copied());
            }
        }

        result
    }

    // ==================================================================================
    // Interprocedural Analysis Support
    // ==================================================================================

    /// Copy points-to set from one variable to another
    ///
    /// Used for interprocedural analysis when propagating points-to information
    /// across function boundaries.
    pub fn propagate_points_to(&mut self, from: VarId, to: VarId) -> bool {
        let from_pts = match self.get_points_to_set(from) {
            Maybe::Some(pts) => pts.clone(),
            Maybe::None => return false,
        };

        self.get_or_create_pts(to).add_all(&from_pts)
    }

    /// Get the transitive closure of locations reachable from a variable
    ///
    /// This follows the two-level points-to graph to find all locations
    /// that can be reached by dereferencing chains.
    #[must_use]
    pub fn get_reachable_locations(&self, var: VarId) -> Set<LocationId> {
        let mut reachable = Set::new();
        let mut worklist: List<LocationId> = List::new();

        // Start with direct points-to set
        if let Maybe::Some(pts) = self.get_points_to_set(var) {
            if pts.conservative {
                // Conservative: return empty set (can't enumerate)
                return reachable;
            }
            for &loc in &pts.locations {
                worklist.push(loc);
                reachable.insert(loc);
            }
        }

        // Transitively follow location points-to
        while let Some(loc) = worklist.pop() {
            if let Maybe::Some(loc_pts) = self.get_location_points_to(loc) {
                for &target in loc_pts {
                    if target != CONSERVATIVE_SENTINEL && reachable.insert(target) {
                        worklist.push(target);
                    }
                }
            }
        }

        reachable
    }
}

impl Default for PointsToGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ==================================================================================
// Andersen-style Points-to Analyzer
// ==================================================================================

/// Andersen-style points-to analyzer
///
/// Implements inclusion-based points-to analysis using constraint generation
/// and iterative solving to fixpoint.
///
/// # Algorithm
/// 1. Generate constraints from CFG
/// 2. Initialize points-to sets
/// 3. Solve constraints iteratively until fixpoint
/// 4. Return final points-to graph
///
/// # Complexity
/// - Constraint generation: O(n) where n = instructions
/// - Fixpoint iteration: O(n³) worst-case, O(n²) typical
/// - Total: O(n³) worst-case, O(n) to O(n²) typical
///
/// Runs the Andersen-style fixpoint: generate constraints from CFG, initialize
/// points-to sets, iterate until no set grows, return final points-to graph.
#[derive(Debug)]
pub struct PointsToAnalyzer {
    /// Constraints generated from the program
    constraints: List<PointsToConstraint>,

    /// Points-to graph (solution)
    graph: PointsToGraph,

    /// Next available location ID
    next_location_id: u64,

    /// Next available variable ID
    next_var_id: u64,

    /// Statistics
    iterations: usize,
    constraints_applied: usize,
}

impl PointsToAnalyzer {
    /// Create new points-to analyzer
    #[must_use]
    pub fn new() -> Self {
        Self {
            constraints: List::new(),
            graph: PointsToGraph::new(),
            next_location_id: 0,
            next_var_id: 0,
            iterations: 0,
            constraints_applied: 0,
        }
    }

    /// Allocate a new location ID
    pub fn allocate_location(&mut self) -> LocationId {
        let id = LocationId(self.next_location_id);
        self.next_location_id += 1;
        id
    }

    /// Allocate a new variable ID
    pub fn allocate_variable(&mut self) -> VarId {
        let id = VarId(self.next_var_id);
        self.next_var_id += 1;
        id
    }

    /// Add a constraint
    pub fn add_constraint(&mut self, constraint: PointsToConstraint) {
        self.constraints.push(constraint);
    }

    /// Get the number of constraints
    #[must_use]
    pub fn constraint_count(&self) -> usize {
        self.constraints.len()
    }

    /// Generate constraints from a control flow graph
    ///
    /// Generates Andersen-style points-to constraints from CFG definitions and uses.
    /// This operates on the CFG abstraction layer; for direct MIR integration,
    /// use `generate_constraints_from_ir`.
    ///
    /// # Algorithm
    /// 1. For each basic block:
    ///    - Process definitions (allocations, address-of)
    ///    - Process uses (loads, stores, copies)
    /// 2. Map `RefIds` to `VarIds` for integration
    pub fn generate_constraints_from_cfg(
        &mut self,
        cfg: &ControlFlowGraph,
    ) -> PointsToGenerationResult {
        let mut stats = PointsToGenerationStats::default();

        for block in cfg.blocks.values() {
            // Process definitions (allocations)
            for def in &block.definitions {
                let var = self.allocate_variable();
                let loc = self.allocate_location();

                // Map RefId to VarId
                self.graph.map_var_to_ref(var, def.reference);

                // Track location type
                let loc_type = if def.is_stack_allocated {
                    LocationType::Stack
                } else {
                    LocationType::Heap
                };
                self.graph.set_location_type(loc, loc_type);

                // Address-of constraint: var points to loc
                self.add_constraint(PointsToConstraint::AddressOf {
                    variable: var,
                    location: loc,
                });

                stats.address_of_constraints += 1;
            }

            // Process uses: generate conservative copy constraints between colocated uses
            // This captures may-alias relationships within the same basic block
            let use_vars: List<VarId> = block
                .uses
                .iter()
                .filter_map(|u| self.graph.get_var_for_ref(u.reference))
                .collect();

            for i in 0..use_vars.len() {
                for j in i + 1..use_vars.len() {
                    // Generate copy constraint (may-alias)
                    self.add_constraint(PointsToConstraint::Copy {
                        dest: use_vars[i],
                        src: use_vars[j],
                    });
                    stats.copy_constraints += 1;
                }
            }
        }

        stats.total_constraints = self.constraints.len();
        PointsToGenerationResult {
            stats,
            variables: self.next_var_id,
            locations: self.next_location_id,
        }
    }

    /// Solve constraints iteratively to fixpoint
    ///
    /// # Algorithm (Andersen's)
    /// ```text
    /// while changed:
    ///     changed = false
    ///     for each constraint:
    ///         if applying constraint changes any points-to set:
    ///             changed = true
    /// ```
    ///
    /// # Complexity
    /// - Iterations: O(n) to O(n²) in practice
    /// - Per iteration: O(n × m) where m = avg points-to set size
    /// - Total: O(n³) worst-case, O(n²) typical
    pub fn solve(&mut self) -> PointsToSolveResult {
        let start_time = std::time::Instant::now();
        let mut changed = true;
        self.iterations = 0;
        self.constraints_applied = 0;

        while changed && self.iterations < 1000 {
            // Limit iterations
            changed = false;
            self.iterations += 1;

            for constraint in &self.constraints.clone() {
                let applied = self.apply_constraint(constraint);
                if applied {
                    changed = true;
                    self.constraints_applied += 1;
                }
            }
        }

        let elapsed = start_time.elapsed();

        PointsToSolveResult {
            iterations: self.iterations,
            constraints_applied: self.constraints_applied,
            elapsed_ns: elapsed.as_nanos() as u64,
            converged: !changed,
        }
    }

    /// Apply a single constraint
    ///
    /// Returns true if any points-to set was modified
    fn apply_constraint(&mut self, constraint: &PointsToConstraint) -> bool {
        match constraint {
            PointsToConstraint::AddressOf { variable, location } => {
                // pts(variable) ⊇ {location}
                self.graph.add_points_to(*variable, *location)
            }

            PointsToConstraint::Copy { dest, src } => {
                // pts(dest) ⊇ pts(src)
                let src_pts = self.graph.get_points_to_set(*src).cloned();
                match src_pts {
                    Maybe::Some(pts) => self.graph.get_or_create_pts(*dest).add_all(&pts),
                    Maybe::None => false,
                }
            }

            PointsToConstraint::Load { dest, ptr } => {
                // Load constraint: x = *y
                // pts(dest) ⊇ ⋃{stored_pts(z) | z ∈ pts(ptr)}
                //
                // This is the CRITICAL two-level points-to handling:
                // 1. Find all locations L that ptr points to: L = pts(ptr)
                // 2. For each l ∈ L, find what values stored at l point to
                // 3. Add all those targets to pts(dest)
                //
                // The stored values' targets come from prior Store constraints
                // which populate location_points_to.

                let ptr_pts = match self.graph.get_points_to_set(*ptr) {
                    Maybe::Some(pts) => pts.clone(),
                    Maybe::None => return false,
                };

                if ptr_pts.conservative {
                    // Conservative: dest may point to anything
                    let dest_pts = self.graph.get_or_create_pts(*dest);
                    if !dest_pts.conservative {
                        dest_pts.conservative = true;
                        return true;
                    }
                    return false;
                }

                let mut changed = false;

                for &loc in &ptr_pts.locations {
                    // Check if this location is marked as conservative
                    if self.graph.is_location_conservative(loc) {
                        let dest_pts = self.graph.get_or_create_pts(*dest);
                        if !dest_pts.conservative {
                            dest_pts.conservative = true;
                            changed = true;
                        }
                        continue;
                    }

                    // PROPER TWO-LEVEL HANDLING:
                    // Look up what values have been stored at this location
                    // (populated by Store constraints via location_points_to)
                    if let Maybe::Some(stored_pts) = self.graph.get_location_points_to(loc) {
                        let stored_pts_clone = stored_pts.clone();
                        for &stored_target in &stored_pts_clone {
                            // Skip conservative sentinel
                            if stored_target == CONSERVATIVE_SENTINEL {
                                let dest_pts = self.graph.get_or_create_pts(*dest);
                                if !dest_pts.conservative {
                                    dest_pts.conservative = true;
                                    changed = true;
                                }
                                continue;
                            }
                            // Add the stored value's target to dest's points-to set
                            if self.graph.add_points_to(*dest, stored_target) {
                                changed = true;
                            }
                        }
                    }

                    // Also check if there's a variable associated with this location
                    // This handles cases like: let x = &y; let z = *x;
                    // where we need pts(z) = pts(y), and y is associated with loc
                    if let Maybe::Some(loc_var) = self.graph.get_var_for_location(loc) {
                        // Get the points-to set of the variable at this location
                        if let Maybe::Some(var_pts) = self.graph.get_points_to_set(loc_var) {
                            let var_pts_clone = var_pts.clone();
                            if var_pts_clone.conservative {
                                let dest_pts = self.graph.get_or_create_pts(*dest);
                                if !dest_pts.conservative {
                                    dest_pts.conservative = true;
                                    changed = true;
                                }
                            } else {
                                for &target in &var_pts_clone.locations {
                                    if self.graph.add_points_to(*dest, target) {
                                        changed = true;
                                    }
                                }
                            }
                        }
                    }
                }

                changed
            }

            PointsToConstraint::Store { ptr, value } => {
                // Store constraint: *x = y
                // ∀z ∈ pts(ptr): stored_pts(z) ⊇ pts(value)
                //
                // This is the second half of two-level points-to handling:
                // Store populates location_points_to, which Load reads.
                //
                // Store constraint semantics:
                // - ptr points to some locations
                // - value points to some locations
                // - After *ptr = value, each location ptr points to should contain
                //   all locations that value points to

                let ptr_pts = match self.graph.get_points_to_set(*ptr) {
                    Maybe::Some(pts) => pts.clone(),
                    Maybe::None => return false,
                };

                let value_pts = match self.graph.get_points_to_set(*value) {
                    Maybe::Some(pts) => pts.clone(),
                    Maybe::None => return false,
                };

                if ptr_pts.conservative {
                    // Conservative: ptr may point to any location
                    // We can't track where we're storing, so this is imprecise
                    // but sound (we don't add wrong information)
                    return false;
                }

                if value_pts.conservative {
                    // Value may point to anything - mark all target locations as conservative
                    let mut changed = false;
                    for &target_loc in &ptr_pts.locations {
                        // Mark this location as potentially pointing to anything
                        // using the conservative sentinel
                        if self
                            .graph
                            .add_location_points_to(target_loc, CONSERVATIVE_SENTINEL)
                        {
                            changed = true;
                        }
                        // Also mark in conservative_locations set for quick lookup
                        self.graph.mark_location_conservative(target_loc);
                    }
                    return changed;
                }

                // For each location ptr points to, union value's points-to set
                // This is the critical step that enables Load to work correctly
                let mut changed = false;
                for &target_loc in &ptr_pts.locations {
                    // stored_pts(target_loc) ⊇ pts(value)
                    if self
                        .graph
                        .union_location_points_to(target_loc, &value_pts.locations)
                    {
                        changed = true;
                    }
                }
                changed
            }

            PointsToConstraint::FieldAddressOf {
                variable,
                base,
                field,
            } => {
                // Field-sensitive address-of: x = &y.f
                // pts(variable) ⊇ {base.field}
                // Create a derived location for the field and add it to variable's pts
                let field_loc = LocationId(base.0 * 1000 + field.0 as u64);
                self.graph.add_points_to(*variable, field_loc)
            }

            PointsToConstraint::FieldLoad { dest, ptr, field } => {
                // Field-sensitive load: x = y->f
                // pts(dest) ⊇ ⋃{field_pts(z, f) | z ∈ pts(ptr)}
                //
                // This uses the field-sensitive two-level points-to graph:
                // 1. Find all locations L that ptr points to
                // 2. For each l ∈ L, look up the field-specific points-to set
                // 3. Add all those targets to pts(dest)

                let ptr_pts = match self.graph.get_points_to_set(*ptr) {
                    Maybe::Some(pts) => pts.clone(),
                    Maybe::None => return false,
                };

                if ptr_pts.conservative {
                    let dest_pts = self.graph.get_or_create_pts(*dest);
                    if !dest_pts.conservative {
                        dest_pts.conservative = true;
                        return true;
                    }
                    return false;
                }

                let mut changed = false;
                for &base_loc in &ptr_pts.locations {
                    // Check if base location is conservative
                    if self.graph.is_location_conservative(base_loc) {
                        let dest_pts = self.graph.get_or_create_pts(*dest);
                        if !dest_pts.conservative {
                            dest_pts.conservative = true;
                            changed = true;
                        }
                        continue;
                    }

                    // PROPER FIELD-SENSITIVE LOAD:
                    // Look up what values have been stored at this field
                    if let Maybe::Some(field_pts) = self.graph.get_field_points_to(base_loc, *field)
                    {
                        let field_pts_clone = field_pts.clone();
                        for &stored_target in &field_pts_clone {
                            if stored_target == CONSERVATIVE_SENTINEL {
                                let dest_pts = self.graph.get_or_create_pts(*dest);
                                if !dest_pts.conservative {
                                    dest_pts.conservative = true;
                                    changed = true;
                                }
                                continue;
                            }
                            if self.graph.add_points_to(*dest, stored_target) {
                                changed = true;
                            }
                        }
                    }

                    // Also check location_points_to for the encoded field location
                    let field_loc = LocationId(base_loc.0 << 20 | u64::from(field.0));
                    if let Maybe::Some(loc_pts) = self.graph.get_location_points_to(field_loc) {
                        let loc_pts_clone = loc_pts.clone();
                        for &stored_target in &loc_pts_clone {
                            if stored_target == CONSERVATIVE_SENTINEL {
                                let dest_pts = self.graph.get_or_create_pts(*dest);
                                if !dest_pts.conservative {
                                    dest_pts.conservative = true;
                                    changed = true;
                                }
                                continue;
                            }
                            if self.graph.add_points_to(*dest, stored_target) {
                                changed = true;
                            }
                        }
                    }
                }
                changed
            }

            PointsToConstraint::FieldStore { ptr, field, value } => {
                // Field-sensitive store: x->f = y
                // ∀z ∈ pts(ptr): field_pts(z, f) ⊇ pts(value)
                //
                // This stores value's points-to set at each field location,
                // enabling subsequent FieldLoad constraints to find it.

                let ptr_pts = match self.graph.get_points_to_set(*ptr) {
                    Maybe::Some(pts) => pts.clone(),
                    Maybe::None => return false,
                };

                let value_pts = match self.graph.get_points_to_set(*value) {
                    Maybe::Some(pts) => pts.clone(),
                    Maybe::None => return false,
                };

                if ptr_pts.conservative {
                    // Conservative ptr: can't track where we're storing
                    return false;
                }

                let mut changed = false;

                if value_pts.conservative {
                    // Value is conservative: mark all target fields as conservative
                    for &target_loc in &ptr_pts.locations {
                        // Mark the field as conservative
                        if self
                            .graph
                            .add_field_points_to(target_loc, *field, CONSERVATIVE_SENTINEL)
                        {
                            changed = true;
                        }
                    }
                    return changed;
                }

                for &target_loc in &ptr_pts.locations {
                    // Use the proper field_points_to mechanism
                    if self
                        .graph
                        .union_field_points_to(target_loc, *field, &value_pts.locations)
                    {
                        changed = true;
                    }

                    // Also update location_points_to for backward compatibility
                    let field_loc = LocationId(target_loc.0 << 20 | u64::from(field.0));
                    if self
                        .graph
                        .union_location_points_to(field_loc, &value_pts.locations)
                    {
                        changed = true;
                    }
                }
                changed
            }

            PointsToConstraint::Interprocedural {
                caller_var,
                callee_var,
                is_call,
            } => {
                // Interprocedural constraint: parameter passing
                // If is_call: pts(callee_var) ⊇ pts(caller_var) (call direction)
                // If !is_call: pts(caller_var) ⊇ pts(callee_var) (return direction)
                let (dest, src) = if *is_call {
                    (*callee_var, *caller_var)
                } else {
                    (*caller_var, *callee_var)
                };

                let src_pts = self.graph.get_points_to_set(src).cloned();
                match src_pts {
                    Maybe::Some(pts) => self.graph.get_or_create_pts(dest).add_all(&pts),
                    Maybe::None => false,
                }
            }
        }
    }

    /// Get the final points-to graph
    #[must_use]
    pub fn get_graph(&self) -> &PointsToGraph {
        &self.graph
    }

    /// Get statistics
    #[must_use]
    pub fn get_statistics(&self) -> PointsToAnalysisStats {
        PointsToAnalysisStats {
            total_variables: self.next_var_id,
            total_locations: self.next_location_id,
            total_constraints: self.constraints.len(),
            iterations: self.iterations,
            constraints_applied: self.constraints_applied,
        }
    }
}

impl Default for PointsToAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// ==================================================================================
// Statistics and Results
// ==================================================================================

/// Points-to constraint generation statistics
#[derive(Debug, Clone, Default)]
pub struct PointsToGenerationStats {
    /// Number of address-of constraints generated
    pub address_of_constraints: usize,

    /// Number of copy constraints generated
    pub copy_constraints: usize,

    /// Number of load constraints generated
    pub load_constraints: usize,

    /// Number of store constraints generated
    pub store_constraints: usize,

    /// Total constraints generated
    pub total_constraints: usize,
}

/// Points-to constraint generation result
#[derive(Debug, Clone)]
pub struct PointsToGenerationResult {
    /// Generation statistics
    pub stats: PointsToGenerationStats,

    /// Number of variables
    pub variables: u64,

    /// Number of locations
    pub locations: u64,
}

/// Points-to constraint solving result
#[derive(Debug, Clone)]
pub struct PointsToSolveResult {
    /// Number of fixpoint iterations
    pub iterations: usize,

    /// Number of constraint applications
    pub constraints_applied: usize,

    /// Elapsed time in nanoseconds
    pub elapsed_ns: u64,

    /// Whether the analysis converged
    pub converged: bool,
}

/// Points-to analysis statistics
#[derive(Debug, Clone)]
pub struct PointsToAnalysisStats {
    /// Total number of variables
    pub total_variables: u64,

    /// Total number of locations
    pub total_locations: u64,

    /// Total number of constraints
    pub total_constraints: usize,

    /// Number of fixpoint iterations
    pub iterations: usize,

    /// Number of constraint applications
    pub constraints_applied: usize,
}

// ==================================================================================
// Builder Pattern
// ==================================================================================

/// Builder for points-to analyzer
///
/// Provides a fluent API for configuring and running points-to analysis.
///
/// # Example
/// ```rust,ignore
/// let result = PointsToAnalyzerBuilder::new()
///     .with_cfg(&cfg)
///     .build()
///     .analyze();
/// ```
pub struct PointsToAnalyzerBuilder {
    cfg: Maybe<ControlFlowGraph>,
}

impl PointsToAnalyzerBuilder {
    /// Create new builder
    #[must_use]
    pub fn new() -> Self {
        Self { cfg: Maybe::None }
    }

    /// Set the control flow graph
    #[must_use]
    pub fn with_cfg(mut self, cfg: &ControlFlowGraph) -> Self {
        self.cfg = Maybe::Some(cfg.clone());
        self
    }

    /// Build and run the analysis
    #[must_use]
    pub fn build(self) -> PointsToAnalysisResult {
        let mut analyzer = PointsToAnalyzer::new();

        // Generate constraints from CFG if provided
        let generation_result = self
            .cfg
            .map(|cfg| analyzer.generate_constraints_from_cfg(&cfg));

        // Solve constraints
        let solve_result = analyzer.solve();

        // Get final graph
        let graph = analyzer.get_graph().clone();
        let stats = analyzer.get_statistics();

        PointsToAnalysisResult {
            graph,
            generation_result,
            solve_result,
            stats,
        }
    }
}

impl Default for PointsToAnalyzerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Complete points-to analysis result
#[derive(Debug, Clone)]
pub struct PointsToAnalysisResult {
    /// Final points-to graph
    pub graph: PointsToGraph,

    /// Constraint generation result (if CFG was provided)
    pub generation_result: Maybe<PointsToGenerationResult>,

    /// Constraint solving result
    pub solve_result: PointsToSolveResult,

    /// Overall statistics
    pub stats: PointsToAnalysisStats,
}

// ==================================================================================
// Integration with Alias Analysis
// ==================================================================================

/// Convert points-to graph to alias sets
///
/// Integrates points-to analysis results with the existing alias analysis framework.
///
/// Converts points-to results into AliasSets for the existing alias analysis framework.
/// Two references with disjoint points-to sets are NoAlias; overlapping sets are MayAlias.
#[must_use]
pub fn points_to_graph_to_alias_sets(graph: &PointsToGraph, reference: RefId) -> Maybe<AliasSets> {
    let var = graph.get_var_for_ref(reference)?;
    let pts = graph.get_points_to_set(var)?;

    let mut alias_sets = AliasSets::new(reference);

    if pts.conservative {
        alias_sets.mark_conservative_aliasing();
        return Maybe::Some(alias_sets);
    }

    // All locations in points-to set are SSA versions (simplified)
    for &loc in &pts.locations {
        alias_sets.add_ssa_version(loc.0 as u32);
    }

    Maybe::Some(alias_sets)
}

/// Check if reference points to heap using points-to graph
///
/// Returns true if any location in the points-to set is heap-allocated.
#[must_use]
pub fn reference_points_to_heap(graph: &PointsToGraph, reference: RefId) -> bool {
    match graph.get_var_for_ref(reference) {
        Maybe::Some(var) => graph.points_to_heap(var),
        Maybe::None => false,
    }
}

// ==================================================================================
// Tests
// ==================================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_points_to_set_basic() {
        let mut pts = PointsToSet::new(VarId(1));
        assert!(pts.is_empty());

        // Add location
        assert!(pts.add_location(LocationId(42)));
        assert!(pts.may_point_to(LocationId(42)));
        assert_eq!(pts.size(), Maybe::Some(1));

        // Add same location again (no change)
        assert!(!pts.add_location(LocationId(42)));
        assert_eq!(pts.size(), Maybe::Some(1));
    }

    #[test]
    fn test_points_to_set_conservative() {
        let mut pts = PointsToSet::conservative(VarId(1));
        assert!(pts.conservative);

        // Adding locations has no effect
        assert!(!pts.add_location(LocationId(42)));

        // May point to anything
        assert!(pts.may_point_to(LocationId(42)));
        assert!(pts.may_point_to(LocationId(999)));

        // Size is unknown
        assert_eq!(pts.size(), Maybe::None);
    }

    #[test]
    fn test_points_to_graph_alias() {
        let mut graph = PointsToGraph::new();

        // Two variables pointing to same location
        let var1 = VarId(1);
        let var2 = VarId(2);
        let loc = LocationId(42);

        graph.add_points_to(var1, loc);
        graph.add_points_to(var2, loc);

        // They may alias
        assert!(graph.may_alias(var1, var2));

        // They must alias (both point to exactly one location)
        assert!(graph.must_alias(var1, var2));
    }

    #[test]
    fn test_andersen_address_of() {
        let mut analyzer = PointsToAnalyzer::new();

        let var = analyzer.allocate_variable();
        let loc = analyzer.allocate_location();

        analyzer.add_constraint(PointsToConstraint::AddressOf {
            variable: var,
            location: loc,
        });

        let result = analyzer.solve();
        assert!(result.converged);

        let graph = analyzer.get_graph();
        let pts = graph.get_points_to_set(var).unwrap();
        assert!(pts.may_point_to(loc));
    }

    #[test]
    fn test_andersen_copy() {
        let mut analyzer = PointsToAnalyzer::new();

        let var1 = analyzer.allocate_variable();
        let var2 = analyzer.allocate_variable();
        let loc = analyzer.allocate_location();

        // var1 = &loc
        analyzer.add_constraint(PointsToConstraint::AddressOf {
            variable: var1,
            location: loc,
        });

        // var2 = var1
        analyzer.add_constraint(PointsToConstraint::Copy {
            dest: var2,
            src: var1,
        });

        let result = analyzer.solve();
        assert!(result.converged);

        let graph = analyzer.get_graph();

        // Both variables point to loc
        assert!(graph.get_points_to_set(var1).unwrap().may_point_to(loc));
        assert!(graph.get_points_to_set(var2).unwrap().may_point_to(loc));
    }

    #[test]
    fn test_two_level_store_load() {
        // Test the critical two-level points-to analysis:
        //   let x = &a;       // x points to loc_a
        //   let y = &b;       // y points to loc_b
        //   *x = y;           // Store: loc_a now contains pointer to loc_b
        //   let z = *x;       // Load: z should point to loc_b (not loc_a!)

        let mut analyzer = PointsToAnalyzer::new();

        let x = analyzer.allocate_variable(); // VarId(0)
        let y = analyzer.allocate_variable(); // VarId(1)
        let z = analyzer.allocate_variable(); // VarId(2)
        let loc_a = analyzer.allocate_location(); // LocationId(0)
        let loc_b = analyzer.allocate_location(); // LocationId(1)

        // x = &a
        analyzer.add_constraint(PointsToConstraint::AddressOf {
            variable: x,
            location: loc_a,
        });

        // y = &b
        analyzer.add_constraint(PointsToConstraint::AddressOf {
            variable: y,
            location: loc_b,
        });

        // *x = y (Store: at location loc_a, store a pointer to loc_b)
        analyzer.add_constraint(PointsToConstraint::Store { ptr: x, value: y });

        // z = *x (Load: z should get what's stored at loc_a, which is a pointer to loc_b)
        analyzer.add_constraint(PointsToConstraint::Load { dest: z, ptr: x });

        let result = analyzer.solve();
        assert!(result.converged);

        let graph = analyzer.get_graph();

        // x points to loc_a
        assert!(graph.get_points_to_set(x).unwrap().may_point_to(loc_a));

        // y points to loc_b
        assert!(graph.get_points_to_set(y).unwrap().may_point_to(loc_b));

        // z should point to loc_b (via the two-level store/load chain)
        // This is the critical fix - the old code would have z pointing to loc_a!
        let z_pts = graph.get_points_to_set(z).unwrap();
        assert!(
            z_pts.may_point_to(loc_b),
            "z should point to loc_b (the stored pointer target)"
        );
    }

    #[test]
    fn test_field_sensitive_store_load() {
        // Test field-sensitive two-level analysis:
        //   let s = &struct_loc;  // s points to struct
        //   let p = &target;      // p points to target
        //   s->field = p;         // Store p into s.field
        //   let q = s->field;     // Load from s.field - should get target

        let mut analyzer = PointsToAnalyzer::new();

        let s = analyzer.allocate_variable();
        let p = analyzer.allocate_variable();
        let q = analyzer.allocate_variable();
        let struct_loc = analyzer.allocate_location();
        let target_loc = analyzer.allocate_location();
        let field = FieldId::new(1);

        // s = &struct_loc
        analyzer.add_constraint(PointsToConstraint::AddressOf {
            variable: s,
            location: struct_loc,
        });

        // p = &target
        analyzer.add_constraint(PointsToConstraint::AddressOf {
            variable: p,
            location: target_loc,
        });

        // s->field = p
        analyzer.add_constraint(PointsToConstraint::FieldStore {
            ptr: s,
            field,
            value: p,
        });

        // q = s->field
        analyzer.add_constraint(PointsToConstraint::FieldLoad {
            dest: q,
            ptr: s,
            field,
        });

        let result = analyzer.solve();
        assert!(result.converged);

        let graph = analyzer.get_graph();

        // q should point to target_loc
        let q_pts = graph.get_points_to_set(q).unwrap();
        assert!(
            q_pts.may_point_to(target_loc),
            "q should point to target_loc via field load"
        );
    }

    #[test]
    fn test_location_to_var_association() {
        // Test that location-to-variable mapping works for address-of patterns
        let mut graph = PointsToGraph::new();

        let var = VarId(0);
        let loc = LocationId(42);

        // Associate location with variable
        graph.associate_location_with_var(loc, var);

        // Should be able to retrieve the variable
        assert_eq!(graph.get_var_for_location(loc), Maybe::Some(var));
    }

    #[test]
    fn test_interprocedural_constraint() {
        // Test interprocedural parameter passing
        let mut analyzer = PointsToAnalyzer::new();

        let caller_arg = analyzer.allocate_variable();
        let callee_param = analyzer.allocate_variable();
        let loc = analyzer.allocate_location();

        // caller_arg = &loc
        analyzer.add_constraint(PointsToConstraint::AddressOf {
            variable: caller_arg,
            location: loc,
        });

        // Call: callee_param receives caller_arg
        analyzer.add_constraint(PointsToConstraint::Interprocedural {
            caller_var: caller_arg,
            callee_var: callee_param,
            is_call: true,
        });

        let result = analyzer.solve();
        assert!(result.converged);

        let graph = analyzer.get_graph();

        // callee_param should point to loc
        assert!(
            graph
                .get_points_to_set(callee_param)
                .unwrap()
                .may_point_to(loc)
        );
    }

    #[test]
    fn test_reachable_locations() {
        // Test transitive closure of reachable locations
        let mut analyzer = PointsToAnalyzer::new();

        let ptr = analyzer.allocate_variable();
        let loc1 = analyzer.allocate_location();
        let _loc2 = analyzer.allocate_location(); // Reserved for future chain tests
        let _loc3 = analyzer.allocate_location(); // Reserved for future chain tests

        // ptr -> loc1
        analyzer.add_constraint(PointsToConstraint::AddressOf {
            variable: ptr,
            location: loc1,
        });

        let result = analyzer.solve();
        assert!(result.converged);

        let graph = analyzer.get_graph();

        // Direct reachability
        let reachable = graph.get_reachable_locations(ptr);
        assert!(reachable.contains(&loc1));
    }

    #[test]
    fn test_field_id_display() {
        assert_eq!(format!("{}", FieldId::BASE), "base");
        assert_eq!(format!("{}", FieldId::new(1)), "field_1");
        assert_eq!(format!("{}", FieldId::new(42)), "field_42");
    }

    #[test]
    fn test_field_location() {
        let base = LocationId(10);
        let field = FieldId::new(2);

        let field_loc = FieldLocation::new(base, field);
        assert_eq!(field_loc.base, base);
        assert_eq!(field_loc.field, field);

        let base_loc = FieldLocation::base_of(base);
        assert!(base_loc.field.is_base());
    }
}
