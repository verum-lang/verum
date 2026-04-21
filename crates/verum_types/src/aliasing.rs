//! Reference Aliasing Detection System
//!
//! Reference safety invariants: managed refs validated at dereference, checked refs proven safe at compile time, unsafe refs unchecked
//!
//! This module implements compile-time verification of Verum's borrowing rules:
//! - At most one mutable reference to a value at any time
//! - Multiple immutable references allowed if no mutable references exist
//! - References cannot outlive their referents
//! - Field-level aliasing detection for partial borrows
//!
//! # Borrow Rules
//!
//! ```verum
//! let mut x = 42;
//! let r1 = &x;      // OK: first immutable borrow
//! let r2 = &x;      // OK: multiple immutable borrows allowed
//! // let r3 = &mut x; // ERROR: mutable borrow while immutable borrows active
//! println("{}, {}", r1, r2);  // Use borrows
//! let r3 = &mut x;  // OK: now allowed, r1 and r2 no longer used
//! ```
//!
//! # Field-Level Borrowing
//!
//! ```verum
//! type Point is { x: Int, y: Int };
//! let mut p = Point { x: 1, y: 2 };
//! let rx = &mut p.x;  // Borrow field x
//! let ry = &mut p.y;  // OK: different fields, no conflict
//! // let rp = &mut p;  // ERROR: conflicts with field borrows
//! ```

use crate::TypeError;
use verum_ast::span::Span;
use verum_common::{Map, Maybe, Set, Text, List};
use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter for unique reference IDs
static REF_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Unique identifier for a reference
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RefId(u64);

impl RefId {
    /// Generate a new unique reference ID
    pub fn new() -> Self {
        Self(REF_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for RefId {
    fn default() -> Self {
        Self::new()
    }
}

/// Represents an active borrow
#[derive(Debug, Clone)]
pub struct Borrow {
    /// Unique identifier for this borrow
    pub id: RefId,
    /// The variable being borrowed
    pub target: Text,
    /// Field path if borrowing a field (e.g., "x" for point.x)
    pub field_path: Option<Text>,
    /// Whether this is a mutable borrow
    pub is_mutable: bool,
    /// Where the borrow was created
    pub span: Span,
    /// Scope depth where this borrow was created
    pub scope_depth: usize,
}

// ==================== Closure Capture Tracking ====================
// Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .5 - Closure captures

/// How a variable is captured by a closure
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Variable is moved into the closure (ownership transferred)
    /// Used for: move closures, consumed values
    Move,
    /// Variable is borrowed immutably (&T)
    /// Used for: read-only access in Fn closures
    Borrow,
    /// Variable is borrowed mutably (&mut T)
    /// Used for: mutable access in FnMut closures
    MutBorrow,
}

/// Represents a capture by a closure
#[derive(Debug, Clone)]
pub struct Capture {
    /// The variable being captured
    pub target: Text,
    /// Field path if capturing a field (e.g., "x" for point.x)
    pub field_path: Option<Text>,
    /// How the variable is captured
    pub mode: CaptureMode,
    /// Where the capture occurs (closure definition)
    pub span: Span,
}

/// Closure capture set - all variables captured by a closure
#[derive(Debug, Clone, Default)]
pub struct CaptureSet {
    /// All captures in this closure
    pub captures: List<Capture>,
    /// Whether this is a move closure (captures by value by default)
    pub is_move: bool,
}

impl CaptureSet {
    pub fn new(is_move: bool) -> Self {
        Self {
            captures: List::new(),
            is_move,
        }
    }

    /// Add a capture to the set
    pub fn add(&mut self, capture: Capture) {
        // Check if already captured - upgrade mode if needed
        for existing in self.captures.iter_mut() {
            if existing.target == capture.target && existing.field_path == capture.field_path {
                // Upgrade: Borrow -> MutBorrow, anything -> Move
                match (existing.mode, capture.mode) {
                    (CaptureMode::Borrow, CaptureMode::MutBorrow) => {
                        existing.mode = CaptureMode::MutBorrow;
                    }
                    (_, CaptureMode::Move) => {
                        existing.mode = CaptureMode::Move;
                    }
                    _ => {}
                }
                return;
            }
        }
        self.captures.push(capture);
    }

    /// Check if a variable is captured
    pub fn captures_var(&self, var: &str) -> bool {
        self.captures.iter().any(|c| c.target.as_str() == var)
    }

    /// Get the capture mode for a variable
    pub fn capture_mode(&self, var: &str) -> Option<CaptureMode> {
        self.captures.iter()
            .find(|c| c.target.as_str() == var)
            .map(|c| c.mode)
    }
}

// ==================== Iterator Invalidation Tracking ====================
// CBGR checking: generation counter validation at each dereference, epoch-based tracking prevents wraparound — .2 - Iterator safety

/// Tracks an active iterator and what it borrows
#[derive(Debug, Clone)]
pub struct ActiveIterator {
    /// Unique ID for this iterator
    pub id: RefId,
    /// Variable holding the iterator
    pub iter_var: Text,
    /// The collection being iterated over
    pub collection: Text,
    /// Whether the iterator borrows mutably
    pub is_mutable: bool,
    /// Where the iterator was created
    pub span: Span,
}

// ==================== Async Borrow Tracking ====================
// Context checking: verifying all required contexts are provided at call sites — Async lifetime bounds

/// Tracks borrows that cross async boundaries
#[derive(Debug, Clone)]
pub struct AsyncBorrow {
    /// The underlying borrow
    pub borrow: Borrow,
    /// Whether this borrow crosses an await point
    pub crosses_await: bool,
    /// The async scope depth where this borrow exists
    pub async_scope: usize,
}

/// Tracks active borrows and detects aliasing conflicts
#[derive(Debug, Clone)]
pub struct BorrowTracker {
    /// Active mutable borrows: target -> Borrow
    active_mut_borrows: Map<Text, List<Borrow>>,

    /// Active immutable borrows: target -> List<Borrow>
    active_immut_borrows: Map<Text, List<Borrow>>,

    /// Field-level borrows: "var.field" -> Borrow
    /// Tracks when specific fields are borrowed
    field_borrows: Map<Text, Borrow>,

    /// Current scope depth
    scope_depth: usize,

    /// Stack of scope boundaries (scope_depth at entry)
    scope_stack: List<usize>,

    /// All borrows created in current scope (for cleanup on scope exit)
    borrows_by_scope: Map<usize, List<RefId>>,

    /// Reference ID to borrow mapping for lookups
    ref_to_borrow: Map<RefId, Borrow>,

    /// Variables that have been reborrowed (allows &mut from &mut)
    reborrowed_from: Map<Text, Text>,

    /// NLL: Maps holder variable name to the RefId it holds
    /// When `let x = &foo.bar`, tracks that `x` holds a reference to `foo.bar`
    ref_holders: Map<Text, RefId>,

    /// NLL: Maps RefId to its holder variable name (reverse lookup)
    ref_id_to_holder: Map<RefId, Text>,

    // ==================== Closure Capture Tracking ====================

    /// Active closure captures: closure_id -> CaptureSet
    /// Tracks what each closure in the current scope captures
    closure_captures: Map<RefId, CaptureSet>,

    /// Variables currently captured by active closures
    /// Maps variable -> list of (closure_id, capture_mode)
    captured_by: Map<Text, List<(RefId, CaptureMode)>>,

    // ==================== Iterator Tracking ====================

    /// Active iterators: iter_var -> ActiveIterator
    active_iterators: Map<Text, ActiveIterator>,

    /// Collections currently being iterated: collection -> iterator_id
    iterated_collections: Map<Text, RefId>,

    // ==================== Async Tracking ====================

    /// Current async scope depth (0 = not in async)
    async_scope_depth: usize,

    /// Borrows that need to be Send for async
    async_borrows: List<AsyncBorrow>,

    /// Whether we're currently inside an await expression
    in_await: bool,

    // ==================== Two-Phase Borrow Tracking ====================
    // Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .6 - Two-phase borrows

    /// Variables with pending two-phase borrows.
    /// During argument evaluation, these allow additional immutable borrows.
    /// When the method call actually executes, they become full mutable borrows.
    two_phase_borrows: Set<Text>,

    // ==================== NLL Last Borrow Tracking ====================
    // Spec: L0-critical/reference_system/access_rules/ref_scope_valid

    /// The most recently created borrow (RefId, target, is_mutable).
    /// Used to link variables to borrows in let statements.
    last_borrow: Option<(RefId, Text, bool)>,

    // ==================== Current Closure Context ====================
    // Spec: L0-critical/reference_system/access_rules/ref_closure_capture

    /// Stack of closure IDs we're currently inside.
    /// Used to distinguish between code inside vs outside a closure.
    closure_scope_stack: List<RefId>,
}

impl BorrowTracker {
    /// Create a new borrow tracker
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            active_mut_borrows: Map::new(),
            active_immut_borrows: Map::new(),
            field_borrows: Map::new(),
            scope_depth: 0,
            scope_stack: List::new(),
            borrows_by_scope: Map::new(),
            ref_to_borrow: Map::new(),
            reborrowed_from: Map::new(),
            ref_holders: Map::new(),
            ref_id_to_holder: Map::new(),
            // Closure tracking
            closure_captures: Map::new(),
            captured_by: Map::new(),
            // Iterator tracking
            active_iterators: Map::new(),
            iterated_collections: Map::new(),
            // Async tracking
            async_scope_depth: 0,
            async_borrows: List::new(),
            in_await: false,
            // Two-phase borrow tracking
            two_phase_borrows: Set::new(),
            // NLL last borrow tracking
            last_borrow: None,
            // Closure scope tracking
            closure_scope_stack: List::new(),
        }
    }

    /// Create a new scope-local tracker (for entering functions)
    pub fn new_scope(&self) -> Self {
        Self::new()
    }

    /// Enter a new scope
    pub fn enter_scope(&mut self) {
        self.scope_stack.push(self.scope_depth);
        self.scope_depth += 1;
    }

    /// Exit the current scope, releasing all borrows created in it
    pub fn exit_scope(&mut self) {
        if let Some(parent_depth) = self.scope_stack.pop() {
            // Remove all borrows created at current scope depth
            if let Some(borrow_ids) = self.borrows_by_scope.remove(&self.scope_depth) {
                for ref_id in borrow_ids.iter() {
                    self.release_borrow_by_id(*ref_id);
                }
            }
            self.scope_depth = parent_depth;
        }
    }

    /// Create an immutable borrow of a variable
    pub fn borrow_immut(&mut self, target: impl Into<Text>, span: Span) -> Result<RefId, TypeError> {
        let target = target.into();

        // Check: no active mutable borrows of this target
        if let Some(mut_borrows) = self.active_mut_borrows.get(&target) {
            if let Some(existing) = mut_borrows.first() {
                return Err(TypeError::BorrowConflict {
                    var: target,
                    existing_borrow_span: existing.span,
                    existing_is_mut: true,
                    new_borrow_span: span,
                    new_is_mut: false,
                });
            }
        }

        // Create the immutable borrow
        let borrow = Borrow {
            id: RefId::new(),
            target: target.clone(),
            field_path: None,
            is_mutable: false,
            span,
            scope_depth: self.scope_depth,
        };

        let ref_id = borrow.id;
        let target_clone = target.clone();
        self.register_borrow(borrow);
        // Track for NLL: link variable -> borrow in let statements
        self.last_borrow = Some((ref_id, target_clone, false));
        Ok(ref_id)
    }

    /// Create a mutable borrow of a variable
    pub fn borrow_mut(&mut self, target: impl Into<Text>, span: Span) -> Result<RefId, TypeError> {
        let target = target.into();

        // NOTE: Do NOT release expired borrows here. Standalone `&mut x` creation
        // must conflict with active immutable borrows of `x`. Only method calls
        // (via `borrow_mut_for_call`) release prior borrows at the call boundary,
        // because method calls are natural NLL release points (the borrow ends
        // at its last use, which is the prior method return).
        //
        // Example that must still error:
        //   let r1 = &data;       // immutable borrow active
        //   let r2 = &mut data;   // ERROR: cannot borrow mutably while r1 is active
        //
        // Example that should work (handled by borrow_mut_for_call):
        //   let len = data.len(); // immutable borrow released after call returns
        //   data.push(1);         // OK: no active immutable borrows

        // Check: no active mutable borrows of this target
        if let Some(mut_borrows) = self.active_mut_borrows.get(&target) {
            if let Some(existing) = mut_borrows.first() {
                return Err(TypeError::BorrowConflict {
                    var: target,
                    existing_borrow_span: existing.span,
                    existing_is_mut: true,
                    new_borrow_span: span,
                    new_is_mut: true,
                });
            }
        }

        // Check: no active immutable borrows of this target
        if let Some(immut_borrows) = self.active_immut_borrows.get(&target) {
            if let Some(existing) = immut_borrows.first() {
                return Err(TypeError::BorrowConflict {
                    var: target,
                    existing_borrow_span: existing.span,
                    existing_is_mut: false,
                    new_borrow_span: span,
                    new_is_mut: true,
                });
            }
        }

        // Check: no active field borrows of this target (e.g., &container.value
        // conflicts with &mut container — borrowing a field means parent is borrowed)
        let target_prefix = format!("{}.", target);
        for (field_key, field_borrow) in self.field_borrows.iter() {
            if field_key.starts_with(&target_prefix) {
                return Err(TypeError::BorrowConflict {
                    var: target,
                    existing_borrow_span: field_borrow.span,
                    existing_is_mut: field_borrow.is_mutable,
                    new_borrow_span: span,
                    new_is_mut: true,
                });
            }
        }

        // Create the mutable borrow
        let borrow = Borrow {
            id: RefId::new(),
            target: target.clone(),
            field_path: None,
            is_mutable: true,
            span,
            scope_depth: self.scope_depth,
        };

        let ref_id = borrow.id;
        let target_clone = target.clone();
        self.register_borrow(borrow);
        // Track for NLL: link variable -> borrow in let statements
        self.last_borrow = Some((ref_id, target_clone, true));
        Ok(ref_id)
    }

    /// Create an *immutable* borrow for a function call argument (NLL behavior).
    ///
    /// Analogue of `borrow_mut_for_call` for the common `call(&value)` pattern.
    /// The returned borrow is NOT persisted in the tracker — a function call
    /// argument is live only for the duration of the call itself, and the
    /// callee cannot leak the reference into any enclosing binding unless
    /// the return type explicitly contains a reference (that case is handled
    /// elsewhere by `link_holder_to_last_borrow`).
    ///
    /// Root fix for Issue #4 (NLL liveness over-retain): before this
    /// existed, `borrow_immut` was the only entry point and it always
    /// created a tracked, scope-lifetime borrow. A sequence like
    ///
    /// ```verum
    /// let sz = call(&value);     // `&value` tracked past the call return
    /// mutate(&mut value);        // ERROR: "previous immutable borrow"
    /// ```
    ///
    /// reported a false conflict because the immutable borrow was still
    /// "active" at the `&mut value` site even though it had no live holder.
    /// This function fixes the asymmetry with `borrow_mut_for_call`.
    pub fn borrow_immut_for_call(
        &mut self,
        target: impl Into<Text>,
        span: Span,
    ) -> Result<RefId, TypeError> {
        let target = target.into();

        // An immutable call-arg read is compatible with existing immutable
        // borrows (multiple readers are fine). Still reject if a *mutable*
        // borrow is outstanding — the mutable side must be sole-owner by
        // contract.
        if let Some(mut_borrows) = self.active_mut_borrows.get(&target)
            && let Some(existing) = mut_borrows.first()
        {
            return Err(TypeError::BorrowConflict {
                var: target,
                existing_borrow_span: existing.span,
                existing_is_mut: true,
                new_borrow_span: span,
                new_is_mut: false,
            });
        }

        // Do NOT register the borrow: it has no persistent holder and will
        // not outlive the call. Returning a fresh RefId keeps the infer.rs
        // call sites uniform with `borrow_mut_for_call`.
        Ok(RefId::new())
    }

    /// Create a mutable borrow for a function call argument (NLL behavior).
    /// Unlike `borrow_mut`, this releases field borrows first to simulate NLL.
    /// This is used when passing `&mut whole_struct` to a function, where
    /// field borrows should end at the call site.
    pub fn borrow_mut_for_call(&mut self, target: impl Into<Text>, span: Span) -> Result<RefId, TypeError> {
        let target = target.into();

        // NLL: Release field borrows before checking - they end at function call
        self.release_field_borrows_for_call(&target);

        // NOTE: Do NOT call nll_release_expired_borrows_for here.
        // That function assumes all held borrows are expired, but for function call
        // arguments like `modify_data(&mut numbers)`, borrows held by named variables
        // (e.g., `let sum_ref = &numbers`) may still be live after the call.
        // The unheld-borrow release logic below handles NLL correctly.

        // NLL: Release only UNHELD immutable borrows of the target for method calls.
        // When a mutable method call like `data.push(1)` occurs, prior immutable
        // borrows from earlier statements (e.g., `data.len()`) that are NOT stored
        // in a named variable are no longer live — the method call boundary is a
        // natural NLL release point.
        //
        // However, borrows held by named variables (e.g., `let sum_ref = &numbers`)
        // are still live and MUST NOT be released — they conflict with the mutable
        // borrow attempt. This prevents the aliasing violation in patterns like:
        //   let sum_ref = &numbers;       // immutable borrow stored in variable
        //   modify_data(&mut numbers);    // ERROR: conflicts with sum_ref
        //   read_data(sum_ref);           // sum_ref is still used
        if let Some(immut_borrows) = self.active_immut_borrows.remove(&target) {
            let mut kept_borrows = List::new();
            for borrow in immut_borrows.iter() {
                // Check if this borrow is held by a named variable
                let is_held = self.ref_id_to_holder.contains_key(&borrow.id);
                if is_held {
                    // Keep the borrow — it's still live via a holder variable
                    kept_borrows.push(borrow.clone());
                } else {
                    // Release the unheld borrow
                    self.ref_to_borrow.remove(&borrow.id);
                    for (_, borrow_list) in self.borrows_by_scope.iter_mut() {
                        borrow_list.retain(|id| *id != borrow.id);
                    }
                }
            }
            if !kept_borrows.is_empty() {
                self.active_immut_borrows.insert(target.clone(), kept_borrows);
            }
        }

        // Check: no active mutable borrows of this target
        if let Some(mut_borrows) = self.active_mut_borrows.get(&target) {
            if let Some(existing) = mut_borrows.first() {
                return Err(TypeError::BorrowConflict {
                    var: target,
                    existing_borrow_span: existing.span,
                    existing_is_mut: true,
                    new_borrow_span: span,
                    new_is_mut: true,
                });
            }
        }

        // Check: no remaining immutable borrows (held by named variables)
        if let Some(immut_borrows) = self.active_immut_borrows.get(&target) {
            if let Some(existing) = immut_borrows.first() {
                return Err(TypeError::BorrowConflict {
                    var: target,
                    existing_borrow_span: existing.span,
                    existing_is_mut: false,
                    new_borrow_span: span,
                    new_is_mut: true,
                });
            }
        }

        // Create the temporary mutable borrow (not tracked - it's only for the call)
        // For function calls, we don't need to track the borrow since it ends immediately
        Ok(RefId::new())
    }

    /// Create an immutable borrow of a field
    pub fn borrow_field_immut(
        &mut self,
        target: impl Into<Text>,
        field: impl Into<Text>,
        span: Span,
    ) -> Result<RefId, TypeError> {
        let target = target.into();
        let field = field.into();
        let field_key = format!("{}.{}", target, field);

        // Check: no mutable borrow of the whole struct
        if let Some(mut_borrows) = self.active_mut_borrows.get(&target) {
            if let Some(existing) = mut_borrows.first() {
                return Err(TypeError::BorrowConflict {
                    var: target,
                    existing_borrow_span: existing.span,
                    existing_is_mut: true,
                    new_borrow_span: span,
                    new_is_mut: false,
                });
            }
        }

        // Check: no mutable borrow of this specific field
        if let Some(existing) = self.field_borrows.get(&Text::from(field_key.as_str())) {
            if existing.is_mutable {
                return Err(TypeError::FieldBorrowConflict {
                    var: target,
                    field: field.clone(),
                    existing_span: existing.span,
                    new_span: span,
                });
            }
        }

        let borrow = Borrow {
            id: RefId::new(),
            target: target.clone(),
            field_path: Some(field),
            is_mutable: false,
            span,
            scope_depth: self.scope_depth,
        };

        let ref_id = borrow.id;
        self.register_field_borrow(Text::from(field_key), borrow);
        Ok(ref_id)
    }

    /// Create a mutable borrow of a field
    pub fn borrow_field_mut(
        &mut self,
        target: impl Into<Text>,
        field: impl Into<Text>,
        span: Span,
    ) -> Result<RefId, TypeError> {
        let target = target.into();
        let field = field.into();
        let field_key = format!("{}.{}", target, field);

        // Check: no mutable borrow of the whole struct
        if let Some(mut_borrows) = self.active_mut_borrows.get(&target) {
            if let Some(existing) = mut_borrows.first() {
                return Err(TypeError::BorrowConflict {
                    var: target,
                    existing_borrow_span: existing.span,
                    existing_is_mut: true,
                    new_borrow_span: span,
                    new_is_mut: true,
                });
            }
        }

        // Check: no immutable borrow of the whole struct
        if let Some(immut_borrows) = self.active_immut_borrows.get(&target) {
            if let Some(existing) = immut_borrows.first() {
                return Err(TypeError::BorrowConflict {
                    var: target,
                    existing_borrow_span: existing.span,
                    existing_is_mut: false,
                    new_borrow_span: span,
                    new_is_mut: true,
                });
            }
        }

        // Check: no existing borrow of this specific field
        if let Some(existing) = self.field_borrows.get(&Text::from(field_key.as_str())) {
            return Err(TypeError::FieldBorrowConflict {
                var: target,
                field: field.clone(),
                existing_span: existing.span,
                new_span: span,
            });
        }

        let borrow = Borrow {
            id: RefId::new(),
            target: target.clone(),
            field_path: Some(field),
            is_mutable: true,
            span,
            scope_depth: self.scope_depth,
        };

        let ref_id = borrow.id;
        self.register_field_borrow(Text::from(field_key), borrow);
        Ok(ref_id)
    }

    /// Release a borrow explicitly (when reference goes out of use)
    pub fn release_borrow(&mut self, ref_id: RefId) {
        self.release_borrow_by_id(ref_id);
    }

    /// Register that a variable holds a borrow.
    /// Called when `let x = &foo;` to track that `x` holds a reference.
    pub fn register_holder(&mut self, holder: impl Into<Text>, ref_id: RefId) {
        let holder = holder.into();
        self.ref_holders.insert(holder.clone(), ref_id);
        self.ref_id_to_holder.insert(ref_id, holder);
    }

    /// NLL: Link a variable to the last created borrow.
    /// Called when `let x = &value;` is processed to track that `x` holds
    /// a reference to `value`. This enables `*x` to release the borrow.
    /// Spec: L0-critical/reference_system/access_rules/ref_scope_valid
    pub fn link_holder_to_last_borrow(&mut self, holder: impl Into<Text>) {
        if let Some((ref_id, _target, _is_mut)) = self.last_borrow.take() {
            let holder = holder.into();
            self.ref_holders.insert(holder.clone(), ref_id);
            self.ref_id_to_holder.insert(ref_id, holder);
        }
    }

    /// NLL: Check if there is a last borrow that can be linked to a holder.
    pub fn has_last_borrow(&self) -> bool {
        self.last_borrow.is_some()
    }

    /// NLL: Clear the last borrow tracking (e.g., after processing non-reference let).
    pub fn clear_last_borrow(&mut self) {
        self.last_borrow = None;
    }

    /// Mark a variable as being used (for NLL liveness tracking).
    /// Returns true if the variable holds a borrow.
    pub fn mark_variable_used(&mut self, var_name: &str) -> bool {
        self.ref_holders.contains_key(&Text::from(var_name))
    }

    /// NLL: Release a borrow when its holder variable is no longer used.
    /// This is the core NLL mechanism - borrows end at their last use point,
    /// not at the end of their lexical scope.
    ///
    /// Example:
    /// ```verum
    /// let mut data = 42;
    /// let r = &data;     // Borrow starts
    /// println(*r);       // Last use of r
    /// data = 100;        // OK: borrow ended at last use of r
    /// ```
    pub fn release_borrow_at_last_use(&mut self, holder: &str) {
        let holder_text = Text::from(holder);
        if let Some(ref_id) = self.ref_holders.remove(&holder_text) {
            self.ref_id_to_holder.remove(&ref_id);
            self.release_borrow_by_id(ref_id);
        }
    }

    /// NLL: Check if a variable is currently used (not yet at its last use).
    /// Used to determine if a borrow can be released early.
    pub fn is_holder_active(&self, holder: &str) -> bool {
        self.ref_holders.contains_key(&Text::from(holder))
    }

    /// NLL: Get the borrow held by a variable, if any.
    pub fn get_held_borrow(&self, holder: &str) -> Option<&Borrow> {
        let holder_text = Text::from(holder);
        self.ref_holders.get(&holder_text)
            .and_then(|ref_id| self.ref_to_borrow.get(ref_id))
    }

    /// NLL: Release all borrows held by variables in a set.
    /// Used when a group of variables go out of use simultaneously.
    pub fn release_borrows_for_variables(&mut self, variables: &[&str]) {
        for var in variables {
            self.release_borrow_at_last_use(var);
        }
    }

    /// NLL: Release expired borrows on a target variable.
    /// When a new borrow of `target` is requested, any existing borrows held
    /// by holder variables are considered expired — the holder's last use must
    /// have already passed for control flow to reach the new borrow point.
    /// This implements lazy NLL release without requiring full liveness analysis.
    pub fn nll_release_expired_borrows_for(&mut self, target: &Text) {
        // Collect holder variables that hold borrows on this target
        let holders_to_release: List<Text> = self.ref_holders.iter()
            .filter_map(|(holder, ref_id)| {
                if let Some(borrow) = self.ref_to_borrow.get(ref_id) {
                    if &borrow.target == target {
                        return Some(holder.clone());
                    }
                }
                None
            })
            .collect();

        for holder in &holders_to_release {
            self.release_borrow_at_last_use(holder.as_str());
        }

        // Also release direct (non-holder) mutable borrows on the target.
        // Method calls create temporary &mut self borrows that should end
        // when the method returns.
        self.active_mut_borrows.remove(target);
    }

    // ==================== Two-Phase Borrow Support ====================
    // Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .6 - Two-phase borrows
    //
    // Two-phase borrows allow patterns like `vec.push(vec.len())`:
    // 1. First phase: "Reserve" mutable borrow of receiver
    // 2. During argument evaluation: Allow immutable borrows
    // 3. Second phase: Activate the mutable borrow for method execution

    /// Begin a two-phase borrow for a method receiver.
    /// During the two-phase period, additional immutable borrows are allowed.
    pub fn begin_two_phase_borrow(&mut self, target: impl Into<Text>, _span: Span) {
        let target = target.into();
        self.two_phase_borrows.insert(target);
    }

    /// End the two-phase borrow period and activate the full mutable borrow.
    pub fn end_two_phase_borrow(&mut self, target: impl Into<Text>) {
        let target = target.into();
        self.two_phase_borrows.remove(&target);
    }

    /// Check if a variable is in a two-phase borrow state.
    /// Used to allow additional immutable borrows during argument evaluation.
    pub fn is_in_two_phase_borrow(&self, target: &str) -> bool {
        self.two_phase_borrows.contains(&Text::from(target))
    }

    /// Release all field borrows for a target (NLL release for function calls).
    /// When calling a function with `&mut whole_struct`, release field borrows.
    /// This simulates NLL: field borrows end when whole struct is borrowed for a call.
    pub fn release_field_borrows_for_call(&mut self, target: &str) {
        let target_prefix = format!("{}.", target);
        let mut to_remove: List<Text> = List::new();

        for (field_key, _borrow) in self.field_borrows.iter() {
            if field_key.starts_with(&target_prefix) {
                to_remove.push(field_key.clone());
            }
        }

        for field_key in to_remove.iter() {
            if let Some(borrow) = self.field_borrows.remove(field_key) {
                self.ref_to_borrow.remove(&borrow.id);
                // Remove from borrows_by_scope
                for (_, borrow_list) in self.borrows_by_scope.iter_mut() {
                    borrow_list.retain(|id| *id != borrow.id);
                }
                // Clean up holder tracking
                if let Some(holder) = self.ref_id_to_holder.remove(&borrow.id) {
                    self.ref_holders.remove(&holder);
                }
            }
        }
    }

    /// Clear all field borrows (simple NLL approximation).
    /// Called after each statement to allow borrow splitting across statements.
    /// Field borrows are only checked within the same statement.
    pub fn clear_field_borrows(&mut self) {
        // Collect all field borrow ref IDs first
        let field_ref_ids: List<RefId> = self.field_borrows.values()
            .map(|b| b.id)
            .collect();

        // Clear the field borrows map
        self.field_borrows = Map::new();

        // Remove from ref_to_borrow and borrows_by_scope
        for ref_id in field_ref_ids.iter() {
            self.ref_to_borrow.remove(ref_id);
            // Also clean up from borrows_by_scope
            for (_, borrow_list) in self.borrows_by_scope.iter_mut() {
                borrow_list.retain(|id| *id != *ref_id);
            }
            // Clean up holder tracking
            if let Some(holder) = self.ref_id_to_holder.remove(ref_id) {
                self.ref_holders.remove(&holder);
            }
        }
    }

    /// Check if a variable has any active borrows
    pub fn has_active_borrows(&self, target: &str) -> bool {
        let target = Text::from(target);
        self.active_mut_borrows.contains_key(&target)
            || self.active_immut_borrows.contains_key(&target)
    }

    /// Check if a variable has an active mutable borrow
    pub fn has_mutable_borrow(&self, target: &str) -> bool {
        let target = Text::from(target);
        self.active_mut_borrows.get(&target)
            .map(|list| !list.is_empty())
            .unwrap_or(false)
    }

    /// Check if a variable has active immutable borrows
    pub fn has_immutable_borrows(&self, target: &str) -> bool {
        let target = Text::from(target);
        self.active_immut_borrows.get(&target)
            .map(|list| !list.is_empty())
            .unwrap_or(false)
    }

    /// Get the span of an existing mutable borrow (for error messages)
    pub fn get_mutable_borrow_span(&self, target: &str) -> Option<Span> {
        let target = Text::from(target);
        self.active_mut_borrows.get(&target)
            .and_then(|list| list.first())
            .map(|b| b.span)
    }

    /// Get the span of an existing immutable borrow (for error messages)
    pub fn get_immutable_borrow_span(&self, target: &str) -> Option<Span> {
        let target = Text::from(target);
        self.active_immut_borrows.get(&target)
            .and_then(|list| list.first())
            .map(|b| b.span)
    }

    /// Register a reborrow relationship (allows &mut from &mut)
    pub fn register_reborrow(&mut self, new_ref: impl Into<Text>, from_ref: impl Into<Text>) {
        self.reborrowed_from.insert(new_ref.into(), from_ref.into());
    }

    /// Check if borrowing would conflict with existing borrows
    /// Returns Some(error) if conflict exists, None if borrow is allowed
    pub fn check_borrow_allowed(
        &self,
        target: &str,
        is_mutable: bool,
        span: Span,
    ) -> Option<TypeError> {
        let target_text = Text::from(target);

        if is_mutable {
            // Check for existing mutable borrows
            if let Some(mut_borrows) = self.active_mut_borrows.get(&target_text) {
                if let Some(existing) = mut_borrows.first() {
                    return Some(TypeError::BorrowConflict {
                        var: target_text,
                        existing_borrow_span: existing.span,
                        existing_is_mut: true,
                        new_borrow_span: span,
                        new_is_mut: true,
                    });
                }
            }

            // Check for existing immutable borrows
            if let Some(immut_borrows) = self.active_immut_borrows.get(&target_text) {
                if let Some(existing) = immut_borrows.first() {
                    return Some(TypeError::BorrowConflict {
                        var: target_text,
                        existing_borrow_span: existing.span,
                        existing_is_mut: false,
                        new_borrow_span: span,
                        new_is_mut: true,
                    });
                }
            }

            // Check for existing field/index borrows of this target
            // Mutating a collection conflicts with any element borrow
            // Spec: L0-critical/reference_system/access_rules/ref_conflict_error
            let prefix = format!("{}.", target);
            for (field_key, field_borrow) in self.field_borrows.iter() {
                if field_key.starts_with(&prefix) {
                    return Some(TypeError::BorrowConflict {
                        var: target_text,
                        existing_borrow_span: field_borrow.span,
                        existing_is_mut: field_borrow.is_mutable,
                        new_borrow_span: span,
                        new_is_mut: true,
                    });
                }
            }
        } else {
            // Immutable borrow: only conflicts with mutable borrows
            if let Some(mut_borrows) = self.active_mut_borrows.get(&target_text) {
                if let Some(existing) = mut_borrows.first() {
                    return Some(TypeError::BorrowConflict {
                        var: target_text,
                        existing_borrow_span: existing.span,
                        existing_is_mut: true,
                        new_borrow_span: span,
                        new_is_mut: false,
                    });
                }
            }

            // Also check for mutable field borrows
            let prefix = format!("{}.", target);
            for (field_key, field_borrow) in self.field_borrows.iter() {
                if field_key.starts_with(&prefix) && field_borrow.is_mutable {
                    return Some(TypeError::BorrowConflict {
                        var: target_text,
                        existing_borrow_span: field_borrow.span,
                        existing_is_mut: true,
                        new_borrow_span: span,
                        new_is_mut: false,
                    });
                }
            }
        }

        None
    }

    /// Clone the tracker for branch analysis
    pub fn clone_for_branch(&self) -> Self {
        self.clone()
    }

    /// Merge borrow states from two branches
    /// Borrows that exist in BOTH branches remain active
    pub fn merge_branch(&mut self, other: &BorrowTracker) {
        // For mutable borrows: keep only those present in both
        let mut to_remove_mut: Vec<Text> = Vec::new();
        for (target, _) in self.active_mut_borrows.iter() {
            if !other.active_mut_borrows.contains_key(target) {
                to_remove_mut.push(target.clone());
            }
        }
        for target in to_remove_mut {
            self.active_mut_borrows.remove(&target);
        }

        // For immutable borrows: keep only those present in both
        let mut to_remove_immut: Vec<Text> = Vec::new();
        for (target, _) in self.active_immut_borrows.iter() {
            if !other.active_immut_borrows.contains_key(target) {
                to_remove_immut.push(target.clone());
            }
        }
        for target in to_remove_immut {
            self.active_immut_borrows.remove(&target);
        }
    }

    // ==================== Internal Methods ====================

    fn register_borrow(&mut self, borrow: Borrow) {
        let ref_id = borrow.id;
        let target = borrow.target.clone();
        let is_mutable = borrow.is_mutable;
        let scope_depth = borrow.scope_depth;

        // Store in ref_to_borrow for lookups
        self.ref_to_borrow.insert(ref_id, borrow.clone());

        // Add to appropriate active borrows map
        if is_mutable {
            let mut_list = self.active_mut_borrows
                .entry(target)
                .or_default();
            mut_list.push(borrow);
        } else {
            let immut_list = self.active_immut_borrows
                .entry(target)
                .or_default();
            immut_list.push(borrow);
        }

        // Track by scope for cleanup
        let scope_borrows = self.borrows_by_scope
            .entry(scope_depth)
            .or_default();
        scope_borrows.push(ref_id);
    }

    fn register_field_borrow(&mut self, field_key: Text, borrow: Borrow) {
        let ref_id = borrow.id;
        let scope_depth = borrow.scope_depth;

        self.ref_to_borrow.insert(ref_id, borrow.clone());
        self.field_borrows.insert(field_key, borrow);

        let scope_borrows = self.borrows_by_scope
            .entry(scope_depth)
            .or_default();
        scope_borrows.push(ref_id);
    }

    fn release_borrow_by_id(&mut self, ref_id: RefId) {
        if let Some(borrow) = self.ref_to_borrow.remove(&ref_id) {
            let target = &borrow.target;

            if let Some(field_path) = &borrow.field_path {
                // Remove field borrow
                let field_key = Text::from(format!("{}.{}", target, field_path));
                self.field_borrows.remove(&field_key);
            } else if borrow.is_mutable {
                // Remove from mutable borrows
                if let Some(mut_list) = self.active_mut_borrows.get_mut(target) {
                    mut_list.retain(|b| b.id != ref_id);
                    if mut_list.is_empty() {
                        self.active_mut_borrows.remove(target);
                    }
                }
            } else {
                // Remove from immutable borrows
                if let Some(immut_list) = self.active_immut_borrows.get_mut(target) {
                    immut_list.retain(|b| b.id != ref_id);
                    if immut_list.is_empty() {
                        self.active_immut_borrows.remove(target);
                    }
                }
            }
        }
    }

    // ==================== Closure Capture Tracking ====================
    // Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .5 - Closure captures

    /// Register a new closure and begin tracking its captures
    pub fn enter_closure(&mut self, is_move: bool) -> RefId {
        let closure_id = RefId::new();
        self.closure_captures.insert(closure_id, CaptureSet::new(is_move));
        // Track that we're inside this closure
        self.closure_scope_stack.push(closure_id);
        closure_id
    }

    /// Register a capture for the current closure being analyzed
    pub fn register_capture(
        &mut self,
        closure_id: RefId,
        target: impl Into<Text>,
        field_path: Option<Text>,
        mode: CaptureMode,
        span: Span,
    ) -> Result<(), TypeError> {
        let target = target.into();

        // Check for aliasing conflicts with existing borrows
        match mode {
            CaptureMode::Move => {
                // Move capture conflicts with any existing borrow
                if self.has_active_borrows(&target) {
                    return Err(TypeError::MoveWhileBorrowed {
                        var: target.clone(),
                        move_span: span,
                        borrow_span: self.get_mutable_borrow_span(&target)
                            .or_else(|| self.get_immutable_borrow_span(&target))
                            .unwrap_or(span),
                    });
                }
            }
            CaptureMode::MutBorrow => {
                // Mutable capture conflicts with existing borrows
                if let Some(mut_borrows) = self.active_mut_borrows.get(&target) {
                    if let Some(existing) = mut_borrows.first() {
                        return Err(TypeError::BorrowConflict {
                            var: target.clone(),
                            existing_borrow_span: existing.span,
                            existing_is_mut: true,
                            new_borrow_span: span,
                            new_is_mut: true,
                        });
                    }
                }
                if let Some(immut_borrows) = self.active_immut_borrows.get(&target) {
                    if let Some(existing) = immut_borrows.first() {
                        return Err(TypeError::BorrowConflict {
                            var: target.clone(),
                            existing_borrow_span: existing.span,
                            existing_is_mut: false,
                            new_borrow_span: span,
                            new_is_mut: true,
                        });
                    }
                }
            }
            CaptureMode::Borrow => {
                // Immutable capture conflicts only with mutable borrows
                if let Some(mut_borrows) = self.active_mut_borrows.get(&target) {
                    if let Some(existing) = mut_borrows.first() {
                        return Err(TypeError::BorrowConflict {
                            var: target.clone(),
                            existing_borrow_span: existing.span,
                            existing_is_mut: true,
                            new_borrow_span: span,
                            new_is_mut: false,
                        });
                    }
                }
            }
        }

        // Register the capture in the closure's capture set
        let is_move_closure = self.closure_captures.get(&closure_id)
            .map(|cs| cs.is_move)
            .unwrap_or(false);

        if let Some(capture_set) = self.closure_captures.get_mut(&closure_id) {
            capture_set.add(Capture {
                target: target.clone(),
                field_path,
                mode,
                span,
            });
        }

        // Track which closures capture this variable - BUT only for borrow captures
        // Move closures MOVE the variable into the closure, so there's no ongoing
        // borrow to track. The variable becomes owned by the closure.
        // Spec: L0-critical/reference_system/access_rules/ref_closure_capture
        if !is_move_closure && !matches!(mode, CaptureMode::Move) {
            let captures = self.captured_by.entry(target).or_default();
            captures.push((closure_id, mode));
        }

        Ok(())
    }

    /// Exit closure analysis and return the capture set.
    /// NOTE: Captures are NOT removed from `captured_by` tracking here.
    /// The captures remain active until the scope containing the closure is exited,
    /// ensuring that mutations to captured variables are detected.
    /// Spec: L0-critical/reference_system/access_rules/ref_aliasing_closure
    pub fn exit_closure(&mut self, closure_id: RefId) -> Option<CaptureSet> {
        // Pop from scope stack
        if let Some(top_id) = self.closure_scope_stack.last() {
            if *top_id == closure_id {
                self.closure_scope_stack.pop();
            }
        }
        // Return the capture set but keep captures in captured_by
        // Captures will be cleaned up when the containing scope is exited
        self.closure_captures.remove(&closure_id)
    }

    /// Release all captures made by closures at the current scope level.
    /// Called when exiting a scope to clean up closure captures.
    pub fn release_closure_captures_at_scope(&mut self, _scope_depth: usize) {
        // For now, we don't track which captures belong to which scope
        // This is a simplified implementation - full NLL would track this
        // The captures will be cleaned up when the function returns
    }

    /// Check if a variable is currently captured by any closure.
    /// Returns false if we're INSIDE a closure that captures this variable,
    /// because using captured variables inside the closure is fine.
    /// Returns true only for code OUTSIDE the closure that tries to access
    /// a variable captured by the closure.
    /// Spec: L0-critical/reference_system/access_rules/ref_closure_capture
    pub fn is_captured(&self, target: &str) -> bool {
        let target_text = Text::from(target);

        // Get the list of closures that capture this variable
        if let Some(capture_list) = self.captured_by.get(&target_text) {
            if capture_list.is_empty() {
                return false;
            }

            // Check if we're inside one of the closures that captures this variable
            // If so, it's not a capture conflict - we're using the variable inside
            // the closure that captured it
            for (closure_id, _mode) in capture_list.iter() {
                if self.closure_scope_stack.contains(closure_id) {
                    // We're inside this closure - not a conflict
                    return false;
                }
            }

            // We're outside all closures that capture this variable - it IS captured
            true
        } else {
            false
        }
    }

    /// Check if we're currently inside any closure
    pub fn in_closure(&self) -> bool {
        !self.closure_scope_stack.is_empty()
    }

    /// Get the current closure ID if we're inside a closure
    pub fn current_closure_id(&self) -> Option<RefId> {
        self.closure_scope_stack.last().copied()
    }

    /// Get capture mode for a variable by a specific closure
    pub fn get_capture_mode(&self, closure_id: RefId, target: &str) -> Option<CaptureMode> {
        self.closure_captures.get(&closure_id)
            .and_then(|cs| cs.capture_mode(target))
    }

    // ==================== Iterator Invalidation Tracking ====================
    // CBGR checking: generation counter validation at each dereference, epoch-based tracking prevents wraparound — .2 - Iterator safety

    /// Register a new iterator over a collection
    pub fn register_iterator(
        &mut self,
        iter_var: impl Into<Text>,
        collection: impl Into<Text>,
        is_mutable: bool,
        span: Span,
    ) -> Result<RefId, TypeError> {
        let iter_var = iter_var.into();
        let collection = collection.into();

        // Check: no mutable borrow of the collection
        if let Some(mut_borrows) = self.active_mut_borrows.get(&collection) {
            if let Some(existing) = mut_borrows.first() {
                return Err(TypeError::BorrowConflict {
                    var: collection,
                    existing_borrow_span: existing.span,
                    existing_is_mut: true,
                    new_borrow_span: span,
                    new_is_mut: is_mutable,
                });
            }
        }

        // For mutable iterators, check no existing iterators on same collection
        if is_mutable {
            if let Some(existing_iter_id) = self.iterated_collections.get(&collection) {
                if let Some(existing_iter) = self.active_iterators.values()
                    .find(|it| it.id == *existing_iter_id)
                {
                    return Err(TypeError::Other(Text::from(format!(
                        "Cannot create mutable iterator over '{}': collection is already being iterated",
                        collection
                    ))));
                }
            }
        }

        let iter_id = RefId::new();
        let iterator = ActiveIterator {
            id: iter_id,
            iter_var: iter_var.clone(),
            collection: collection.clone(),
            is_mutable,
            span,
        };

        self.active_iterators.insert(iter_var, iterator);
        self.iterated_collections.insert(collection, iter_id);

        Ok(iter_id)
    }

    /// Check if modifying a collection would invalidate an iterator
    /// Returns E310 (BorrowConflict) since iterator invalidation is a form of aliasing violation:
    /// the iterator holds an immutable borrow of the collection, preventing mutation.
    /// Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .4 - Iterator invalidation
    pub fn check_iterator_invalidation(
        &self,
        collection: &str,
        modification_span: Span,
    ) -> Result<(), TypeError> {
        if let Some(iter_id) = self.iterated_collections.get(&Text::from(collection)) {
            // Find the iterator
            for iterator in self.active_iterators.values() {
                if iterator.id == *iter_id {
                    // Return BorrowConflict (E310) - the iterator holds an immutable borrow
                    return Err(TypeError::BorrowConflict {
                        var: Text::from(collection),
                        existing_borrow_span: iterator.span,
                        existing_is_mut: false, // Iterator holds immutable borrow
                        new_borrow_span: modification_span,
                        new_is_mut: true, // Mutation requires mutable borrow
                    });
                }
            }
        }
        Ok(())
    }

    /// Release an iterator
    pub fn release_iterator(&mut self, iter_var: &str) {
        if let Some(iterator) = self.active_iterators.remove(&Text::from(iter_var)) {
            self.iterated_collections.remove(&iterator.collection);
        }
    }

    // ==================== Async Boundary Tracking ====================
    // Context checking: verifying all required contexts are provided at call sites — Async lifetime bounds

    /// Enter an async block or function
    pub fn enter_async_scope(&mut self) {
        self.async_scope_depth += 1;
    }

    /// Exit an async block or function
    pub fn exit_async_scope(&mut self) {
        if self.async_scope_depth > 0 {
            self.async_scope_depth -= 1;
            // Clear async borrows for this scope
            self.async_borrows.retain(|ab| ab.async_scope < self.async_scope_depth);
        }
    }

    /// Mark that we're entering an await expression
    pub fn enter_await(&mut self) {
        self.in_await = true;
    }

    /// Mark that we're exiting an await expression
    pub fn exit_await(&mut self) {
        self.in_await = false;
    }

    /// Check if we're inside an async context
    pub fn in_async(&self) -> bool {
        self.async_scope_depth > 0
    }

    /// Track a borrow that might cross an await point
    /// Returns error if borrow is not Send-safe for async
    pub fn track_async_borrow(&mut self, borrow: Borrow, is_send: bool) -> Result<(), TypeError> {
        if self.async_scope_depth > 0 && !is_send {
            // Non-Send borrow in async context
            return Err(TypeError::Other(Text::from(format!(
                "Cannot borrow '{}' across await point: type is not Send",
                borrow.target
            ))));
        }

        if self.async_scope_depth > 0 {
            self.async_borrows.push(AsyncBorrow {
                borrow,
                crosses_await: self.in_await,
                async_scope: self.async_scope_depth,
            });
        }

        Ok(())
    }

    /// Check all borrows before an await point
    /// Non-Send borrows that cross await points are errors
    pub fn check_await_safety(&self) -> Result<(), TypeError> {
        for async_borrow in self.async_borrows.iter() {
            if async_borrow.crosses_await {
                // This borrow was created before an await and is still active
                // Caller should verify it's Send
                // For now we just track, actual Send checking is in type system
            }
        }
        Ok(())
    }

    // ==================== Send/Sync Checking ====================
    // Escape analysis: compiler proves reference safety at compile time, enabling promotion from &T to &checked T (zero cost) — Thread safety

    /// Check if sharing a borrow would violate thread safety
    pub fn check_thread_safety(&self, target: &str, needs_send: bool, needs_sync: bool) -> Result<(), TypeError> {
        // If we have a mutable borrow and need Sync, that's an error
        // (mutable borrows aren't Sync)
        if needs_sync {
            if let Some(mut_borrows) = self.active_mut_borrows.get(&Text::from(target)) {
                if !mut_borrows.is_empty() {
                    return Err(TypeError::Other(Text::from(format!(
                        "Cannot share '{}' across threads: mutable borrow is not Sync",
                        target
                    ))));
                }
            }
        }

        // Note: Send checking requires type information, not just borrow tracking
        // The actual Send check is done in the type system, this just provides context

        Ok(())
    }
}

impl Default for BorrowTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Alias analysis result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasResult {
    /// Definitely no aliasing
    NoAlias,
    /// Might alias (conservative)
    MayAlias,
    /// Definitely aliases
    MustAlias,
}

/// Analyze whether two paths may alias
pub fn analyze_alias(path1: &str, path2: &str) -> AliasResult {
    if path1 == path2 {
        return AliasResult::MustAlias;
    }

    // Check if one is a prefix of the other (field access)
    // e.g., "x" and "x.field" -> MustAlias (field is part of x)
    if path1.starts_with(&format!("{}.", path2)) || path2.starts_with(&format!("{}.", path1)) {
        return AliasResult::MustAlias;
    }

    // Same base, different fields -> NoAlias
    // e.g., "x.a" and "x.b" -> NoAlias
    let parts1: Vec<&str> = path1.split('.').collect();
    let parts2: Vec<&str> = path2.split('.').collect();

    if !parts1.is_empty() && !parts2.is_empty() && parts1[0] == parts2[0] {
        // Same base variable
        if parts1.len() > 1 && parts2.len() > 1 && parts1[1] != parts2[1] {
            // Different fields
            return AliasResult::NoAlias;
        }
    }

    // Different base variables -> NoAlias
    if !parts1.is_empty() && !parts2.is_empty() && parts1[0] != parts2[0] {
        return AliasResult::NoAlias;
    }

    // Conservative: MayAlias
    AliasResult::MayAlias
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::span::FileId;

    fn dummy_span() -> Span {
        Span::new(0, 1, FileId::dummy())
    }

    #[test]
    fn test_immutable_borrows_allowed() {
        let mut tracker = BorrowTracker::new();

        // Multiple immutable borrows should be allowed
        assert!(tracker.borrow_immut("x", dummy_span()).is_ok());
        assert!(tracker.borrow_immut("x", dummy_span()).is_ok());
        assert!(tracker.borrow_immut("x", dummy_span()).is_ok());
    }

    #[test]
    fn test_mutable_borrow_conflict() {
        let mut tracker = BorrowTracker::new();

        // First mutable borrow should succeed
        assert!(tracker.borrow_mut("x", dummy_span()).is_ok());

        // Second mutable borrow should fail
        assert!(tracker.borrow_mut("x", dummy_span()).is_err());
    }

    #[test]
    fn test_mut_after_immut_conflict() {
        let mut tracker = BorrowTracker::new();

        // Immutable borrow
        assert!(tracker.borrow_immut("x", dummy_span()).is_ok());

        // Mutable borrow should fail
        assert!(tracker.borrow_mut("x", dummy_span()).is_err());
    }

    #[test]
    fn test_immut_after_mut_conflict() {
        let mut tracker = BorrowTracker::new();

        // Mutable borrow
        assert!(tracker.borrow_mut("x", dummy_span()).is_ok());

        // Immutable borrow should fail
        assert!(tracker.borrow_immut("x", dummy_span()).is_err());
    }

    #[test]
    fn test_field_borrows_different_fields() {
        let mut tracker = BorrowTracker::new();

        // Borrow different fields should succeed
        assert!(tracker.borrow_field_mut("point", "x", dummy_span()).is_ok());
        assert!(tracker.borrow_field_mut("point", "y", dummy_span()).is_ok());
    }

    #[test]
    fn test_field_borrow_same_field_conflict() {
        let mut tracker = BorrowTracker::new();

        // First field borrow
        assert!(tracker.borrow_field_mut("point", "x", dummy_span()).is_ok());

        // Same field again should fail
        assert!(tracker.borrow_field_mut("point", "x", dummy_span()).is_err());
    }

    #[test]
    fn test_scope_release() {
        let mut tracker = BorrowTracker::new();

        tracker.enter_scope();
        assert!(tracker.borrow_mut("x", dummy_span()).is_ok());
        tracker.exit_scope();

        // After scope exit, borrow should be released
        assert!(tracker.borrow_mut("x", dummy_span()).is_ok());
    }

    #[test]
    fn test_alias_analysis() {
        assert_eq!(analyze_alias("x", "x"), AliasResult::MustAlias);
        assert_eq!(analyze_alias("x", "x.field"), AliasResult::MustAlias);
        assert_eq!(analyze_alias("x.a", "x.b"), AliasResult::NoAlias);
        assert_eq!(analyze_alias("x", "y"), AliasResult::NoAlias);
    }
}
