//! Verum MLIR Dialect - Industrial-Grade Implementation.
//!
//! This module defines the comprehensive Verum-specific MLIR dialect with custom types
//! and operations for:
//!
//! - **CBGR (Three-Tier Reference System)**: Complete memory safety with
//!   generation-based validation, borrow scopes, and tier promotion/demotion
//! - **Context System**: Full dependency injection with scoping, requirements,
//!   monomorphization support, and stack management
//! - **Async/Await**: State machine-based async compilation with await points,
//!   live variable analysis, polling, and waker support
//! - **Closures**: Capture analysis, environment management, and indirect calls
//! - **Pattern Matching**: Decision tree compilation for efficient matching
//! - **Collections**: `verum.list_*`, `verum.map_*`, `verum.set_*`
//! - **Refinement Types**: `verum.refinement_check` with predicate propagation
//!
//! # Dialect Structure
//!
//! ```text
//! Verum Dialect ("verum")
//! ├── Types
//! │   ├── RefType<T, tier>     - Three-tier reference with CBGR tracking
//! │   ├── ListType<T>          - List collection
//! │   ├── MapType<K, V>        - Map collection
//! │   ├── SetType<T>           - Set collection
//! │   ├── TextType             - Text string
//! │   ├── MaybeType<T>         - Optional value
//! │   ├── FutureType<T>        - Async future
//! │   ├── ContextType          - Context value for DI
//! │   ├── ClosureType          - Closure with captures
//! │   └── StateMachineType     - Async state machine
//! │
//! └── Operations
//!     ├── CBGR (Enhanced - 12 operations)
//!     │   ├── verum.cbgr_alloc     - Allocate with tracking
//!     │   ├── verum.cbgr_check     - Validate generation
//!     │   ├── verum.cbgr_deref     - Dereference with validation
//!     │   ├── verum.cbgr_store     - Store with mutation tracking
//!     │   ├── verum.cbgr_drop      - Drop with cleanup
//!     │   ├── verum.cbgr_borrow_scope  - Borrow scope management
//!     │   ├── verum.cbgr_promote   - Tier promotion
//!     │   ├── verum.cbgr_demote    - Tier demotion
//!     │   └── ... (escape annotations, generation ops)
//!     │
//!     ├── Context (Enhanced - 12 operations)
//!     │   ├── verum.context_get    - Get with resolution strategy
//!     │   ├── verum.context_provide - Provide with lifetime
//!     │   ├── verum.context_scope  - Scoped provision
//!     │   ├── verum.context_require - Assert availability
//!     │   ├── verum.context_with   - Transform context
//!     │   └── ... (stack management, monomorphization)
//!     │
//!     ├── Async (Enhanced - 20 operations)
//!     │   ├── verum.async_spawn    - Spawn task
//!     │   ├── verum.async_join     - Join task
//!     │   ├── verum.async_select   - Select first ready
//!     │   ├── verum.async_poll     - Poll future
//!     │   ├── verum.async_state_machine_create
//!     │   └── ... (state management, wakers)
//!     │
//!     ├── Closures (8 operations)
//!     │   ├── verum.closure_create - Create closure
//!     │   ├── verum.closure_call   - Call closure
//!     │   ├── verum.closure_env_load/store
//!     │   └── ... (environment management)
//!     │
//!     ├── Pattern Matching (6 operations)
//!     │   ├── verum.tuple_extract
//!     │   ├── verum.struct_extract
//!     │   ├── verum.variant_payload
//!     │   └── ... (destructuring ops)
//!     │
//!     ├── Collections
//!     │   ├── verum.list_new/push/get/...
//!     │   ├── verum.map_new/insert/get/...
//!     │   └── verum.set_new/insert/...
//!     │
//!     └── Refinements
//!         └── verum.refinement_check
//! ```

pub mod types;
pub mod ops;
pub mod builders;
pub mod cbgr;
pub mod context_system;
pub mod closure;
pub mod async_state;

use verum_mlir::Context;
use verum_common::Text;

// Re-export core types and operations
pub use types::*;
pub use ops::*;
pub use builders::*;

// Re-export enhanced modules
pub use cbgr::{
    CbgrRefLayout, CbgrCapability, EscapeCategory, BorrowKind,
    CbgrAllocOp, CbgrCheckOp, CbgrDerefOp, CbgrStoreOp, CbgrDropOp,
    CbgrBorrowScopeOp, CbgrPromoteOp, CbgrDemoteOp, CbgrEscapeAnnotateOp,
    CbgrTypeBuilder,
};
pub use context_system::{
    ContextCapability, ContextLifetime, ContextResolution,
    ContextGetOp, ContextProvideOp, ContextScopeOp, ContextRequireOp,
    ContextWithOp, ContextHasOp, ContextMonoOp,
    ContextTypeBuilder, ContextAnalysis,
};
pub use closure::{
    CaptureMode, CapturedVar, ClosureEnv,
    ClosureCreateOp, ClosureCallOp, ClosureEnvLoadOp, ClosureEnvStoreOp,
    ClosureDropOp, FnPtrCreateOp, IndirectCallOp, MethodCallOp, VTableLookupOp,
    ClosureTypeBuilder, ClosureLowering,
};
pub use async_state::{
    PollState, AwaitPoint, LiveVariable, AsyncAnalysis,
    AsyncStateMachineCreateOp, AsyncGetStateOp, AsyncSetStateOp,
    AsyncSaveLocalsOp, AsyncRestoreLocalsOp, AsyncSetResultOp, AsyncGetResultOp,
    AsyncPollOp, AsyncPollIsReadyOp, AsyncPollValueOp,
    AsyncReturnPendingOp, AsyncReturnReadyOp,
    AsyncSpawnOp, AsyncJoinOp, AsyncSelectOp, AsyncRaceOp,
    AsyncGetWakerOp, AsyncWakeOp, AsyncCloneWakerOp,
    AsyncTypeBuilder, AsyncStateMachineGenerator,
};

/// The Verum MLIR dialect.
///
/// This dialect provides Verum-specific operations and types that cannot
/// be directly represented in standard MLIR dialects.
pub struct VerumDialect {
    /// Dialect name.
    name: Text,
}

impl VerumDialect {
    /// Dialect namespace.
    pub const NAMESPACE: &'static str = "verum";

    /// Create a new Verum dialect.
    pub fn new() -> Self {
        Self {
            name: Text::from(Self::NAMESPACE),
        }
    }

    /// Get the dialect name.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Register the dialect with an MLIR context.
    ///
    /// Note: Since we're using existing MLIR dialects for lowering,
    /// we don't need to register a custom dialect. The Verum operations
    /// are represented using standard MLIR constructs with special
    /// attributes and naming conventions.
    pub fn register(_context: &Context) {
        // The Verum dialect operations are implemented using:
        // - func dialect for functions
        // - arith dialect for arithmetic
        // - scf dialect for structured control flow
        // - memref dialect for memory operations
        // - llvm dialect for final lowering
        //
        // Custom Verum semantics are captured through:
        // - Operation names with "verum." prefix
        // - Custom attributes for CBGR metadata
        // - Custom types encoded as opaque types
    }
}

impl Default for VerumDialect {
    fn default() -> Self {
        Self::new()
    }
}

/// Operation names in the Verum dialect.
pub mod op_names {
    // ============================================
    // CBGR operations (12 total)
    // ============================================
    pub const CBGR_ALLOC: &str = "verum.cbgr_alloc";
    pub const CBGR_REALLOC: &str = "verum.cbgr_realloc";
    pub const CBGR_CHECK: &str = "verum.cbgr_check";
    pub const CBGR_GET_GEN: &str = "verum.cbgr_get_gen";
    pub const CBGR_INC_GEN: &str = "verum.cbgr_inc_gen";
    pub const CBGR_DEREF: &str = "verum.cbgr_deref";
    pub const CBGR_DEREF_UNCHECKED: &str = "verum.cbgr_deref_unchecked";
    pub const CBGR_STORE: &str = "verum.cbgr_store";
    pub const CBGR_DROP: &str = "verum.cbgr_drop";
    pub const CBGR_BORROW_SCOPE: &str = "verum.cbgr_borrow_scope";
    pub const CBGR_PROMOTE: &str = "verum.cbgr_promote";
    pub const CBGR_DEMOTE: &str = "verum.cbgr_demote";
    pub const CBGR_ESCAPE_ANNOTATE: &str = "verum.cbgr_escape_annotate";

    // ============================================
    // Context operations (12 total)
    // ============================================
    pub const CONTEXT_GET: &str = "verum.context_get";
    pub const CONTEXT_GET_OR: &str = "verum.context_get_or";
    pub const CONTEXT_TRY_GET: &str = "verum.context_try_get";
    pub const CONTEXT_PROVIDE: &str = "verum.context_provide";
    pub const CONTEXT_PROVIDE_AS: &str = "verum.context_provide_as";
    pub const CONTEXT_SCOPE: &str = "verum.context_scope";
    pub const CONTEXT_YIELD: &str = "verum.context_yield";
    pub const CONTEXT_REQUIRE: &str = "verum.context_require";
    pub const CONTEXT_HAS: &str = "verum.context_has";
    pub const CONTEXT_WITH: &str = "verum.context_with";
    pub const CONTEXT_PUSH_FRAME: &str = "verum.context_push_frame";
    pub const CONTEXT_POP_FRAME: &str = "verum.context_pop_frame";
    pub const CONTEXT_MONO: &str = "verum.context_mono";

    // ============================================
    // Async operations (20 total)
    // ============================================
    pub const SPAWN: &str = "verum.spawn";
    pub const AWAIT: &str = "verum.await";
    pub const SELECT: &str = "verum.select";
    pub const YIELD: &str = "verum.yield";
    pub const ASYNC_STATE_MACHINE_CREATE: &str = "verum.async_state_machine_create";
    pub const ASYNC_GET_STATE: &str = "verum.async_get_state";
    pub const ASYNC_SET_STATE: &str = "verum.async_set_state";
    pub const ASYNC_SAVE_LOCALS: &str = "verum.async_save_locals";
    pub const ASYNC_RESTORE_LOCALS: &str = "verum.async_restore_locals";
    pub const ASYNC_SET_RESULT: &str = "verum.async_set_result";
    pub const ASYNC_GET_RESULT: &str = "verum.async_get_result";
    pub const ASYNC_POLL: &str = "verum.async_poll";
    pub const ASYNC_POLL_IS_READY: &str = "verum.async_poll_is_ready";
    pub const ASYNC_POLL_VALUE: &str = "verum.async_poll_value";
    pub const ASYNC_RETURN_PENDING: &str = "verum.async_return_pending";
    pub const ASYNC_RETURN_READY: &str = "verum.async_return_ready";
    pub const ASYNC_SPAWN: &str = "verum.async_spawn";
    pub const ASYNC_JOIN: &str = "verum.async_join";
    pub const ASYNC_SELECT: &str = "verum.async_select";
    pub const ASYNC_RACE: &str = "verum.async_race";
    pub const ASYNC_GET_WAKER: &str = "verum.async_get_waker";
    pub const ASYNC_WAKE: &str = "verum.async_wake";
    pub const ASYNC_CLONE_WAKER: &str = "verum.async_clone_waker";

    // ============================================
    // Closure operations (8 total)
    // ============================================
    pub const CLOSURE_CREATE: &str = "verum.closure_create";
    pub const CLOSURE_CALL: &str = "verum.closure_call";
    pub const CLOSURE_ENV_LOAD: &str = "verum.closure_env_load";
    pub const CLOSURE_ENV_STORE: &str = "verum.closure_env_store";
    pub const CLOSURE_ENV_ALLOC: &str = "verum.closure_env_alloc";
    pub const CLOSURE_ENV_FREE: &str = "verum.closure_env_free";
    pub const CLOSURE_DROP: &str = "verum.closure_drop";
    pub const FN_PTR: &str = "verum.fn_ptr";
    pub const INDIRECT_CALL: &str = "verum.indirect_call";
    pub const METHOD_CALL: &str = "verum.method_call";
    pub const VTABLE_LOOKUP: &str = "verum.vtable_lookup";

    // ============================================
    // Pattern matching operations (6 total)
    // ============================================
    pub const TUPLE_EXTRACT: &str = "verum.tuple_extract";
    pub const STRUCT_EXTRACT: &str = "verum.struct_extract";
    pub const VARIANT_PAYLOAD: &str = "verum.variant_payload";
    pub const SLICE_GET: &str = "verum.slice_get";
    pub const BIND: &str = "verum.bind";
    pub const DEREF: &str = "verum.deref";

    // ============================================
    // Collection operations - List
    // ============================================
    pub const LIST_NEW: &str = "verum.list_new";
    pub const LIST_PUSH: &str = "verum.list_push";
    pub const LIST_POP: &str = "verum.list_pop";
    pub const LIST_GET: &str = "verum.list_get";
    pub const LIST_SET: &str = "verum.list_set";
    pub const LIST_LEN: &str = "verum.list_len";

    // Collection operations - Map
    pub const MAP_NEW: &str = "verum.map_new";
    pub const MAP_INSERT: &str = "verum.map_insert";
    pub const MAP_GET: &str = "verum.map_get";
    pub const MAP_REMOVE: &str = "verum.map_remove";
    pub const MAP_CONTAINS: &str = "verum.map_contains";

    // Collection operations - Set
    pub const SET_NEW: &str = "verum.set_new";
    pub const SET_INSERT: &str = "verum.set_insert";
    pub const SET_CONTAINS: &str = "verum.set_contains";
    pub const SET_REMOVE: &str = "verum.set_remove";

    // ============================================
    // Text operations
    // ============================================
    pub const TEXT_NEW: &str = "verum.text_new";
    pub const TEXT_CONCAT: &str = "verum.text_concat";
    pub const TEXT_LEN: &str = "verum.text_len";

    // ============================================
    // Maybe operations
    // ============================================
    pub const MAYBE_SOME: &str = "verum.maybe_some";
    pub const MAYBE_NONE: &str = "verum.maybe_none";
    pub const MAYBE_IS_SOME: &str = "verum.maybe_is_some";
    pub const MAYBE_UNWRAP: &str = "verum.maybe_unwrap";

    // ============================================
    // Refinement operations
    // ============================================
    pub const REFINEMENT_CHECK: &str = "verum.refinement_check";

    // ============================================
    // Call operations (for stdlib FFI)
    // ============================================
    pub const STDLIB_CALL: &str = "verum.stdlib_call";

    // ============================================
    // Intrinsic operations
    // ============================================
    pub const PRINT: &str = "verum.print";
    pub const PANIC: &str = "verum.panic";
    pub const ASSERT: &str = "verum.assert";
}

/// Attribute names used in the Verum dialect.
pub mod attr_names {
    // ============================================
    // CBGR attributes
    // ============================================
    pub const CBGR_TIER: &str = "verum.cbgr_tier";
    pub const CBGR_GENERATION: &str = "verum.cbgr_generation";
    pub const CBGR_EPOCH: &str = "verum.cbgr_epoch";
    pub const CBGR_ELIMINATED: &str = "verum.cbgr_eliminated";
    pub const CBGR_LAYOUT: &str = "verum.cbgr_layout";
    pub const CBGR_CAPS: &str = "verum.cbgr_caps";

    // ============================================
    // Escape analysis attributes
    // ============================================
    pub const ESCAPE_CATEGORY: &str = "verum.escape_category";
    pub const ESCAPE_PROVEN: &str = "verum.escape_proven";

    // ============================================
    // Borrow scope attributes
    // ============================================
    pub const BORROW_KIND: &str = "verum.borrow_kind";

    // ============================================
    // Context attributes
    // ============================================
    pub const CONTEXT_NAME: &str = "verum.context_name";
    pub const CONTEXT_TYPE: &str = "verum.context_type";
    pub const REQUIRED_CONTEXTS: &str = "verum.required_contexts";
    pub const PROVIDED_CONTEXTS: &str = "verum.provided_contexts";
    pub const CONTEXT_LIFETIME: &str = "verum.context_lifetime";
    pub const CONTEXT_RESOLUTION: &str = "verum.context_resolution";
    pub const CONTEXT_INHERIT: &str = "verum.context_inherit";
    pub const CONTEXT_CACHED: &str = "verum.context_cached";

    // ============================================
    // Closure attributes
    // ============================================
    pub const CAPTURE_MODES: &str = "verum.capture_modes";
    pub const CLOSURE_FN_NAME: &str = "verum.closure_fn_name";
    pub const CLOSURE_FN_TYPE: &str = "verum.closure_fn_type";
    pub const ENV_SIZE: &str = "verum.env_size";
    pub const ENV_ALIGNMENT: &str = "verum.env_alignment";

    // ============================================
    // Async/state machine attributes
    // ============================================
    pub const STATE_COUNT: &str = "verum.state_count";
    pub const LOCALS_SIZE: &str = "verum.locals_size";
    pub const AWAIT_POINT_ID: &str = "verum.await_point_id";
    pub const RESUME_STATE: &str = "verum.resume_state";
    pub const LIVE_VARS: &str = "verum.live_vars";
    pub const OFFSETS: &str = "verum.offsets";

    // ============================================
    // Pattern matching attributes
    // ============================================
    pub const PATTERN_INDEX: &str = "verum.pattern_index";
    pub const FIELD_NAME: &str = "verum.field_name";
    pub const VARIANT_NAME: &str = "verum.variant_name";

    // ============================================
    // Refinement attributes
    // ============================================
    pub const REFINEMENT_PREDICATE: &str = "verum.refinement_predicate";
    pub const REFINEMENT_PROVEN: &str = "verum.refinement_proven";

    // ============================================
    // Function attributes
    // ============================================
    pub const IS_ASYNC: &str = "verum.is_async";
    pub const IS_GENERATOR: &str = "verum.is_generator";
    pub const IS_CLOSURE: &str = "verum.is_closure";
    pub const FN_NAME: &str = "verum.fn_name";

    // ============================================
    // Type attributes
    // ============================================
    pub const ELEMENT_TYPE: &str = "verum.element_type";
    pub const KEY_TYPE: &str = "verum.key_type";
    pub const VALUE_TYPE: &str = "verum.value_type";
    pub const RESULT_TYPE: &str = "verum.result_type";
    pub const CONCRETE_TYPE: &str = "verum.concrete_type";

    // ============================================
    // Method call attributes
    // ============================================
    pub const METHOD_NAME: &str = "verum.method_name";
    pub const VTABLE_INDEX: &str = "verum.vtable_index";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dialect_creation() {
        let dialect = VerumDialect::new();
        assert_eq!(dialect.name(), "verum");
    }

    #[test]
    fn test_op_names() {
        assert_eq!(op_names::CBGR_ALLOC, "verum.cbgr_alloc");
        assert_eq!(op_names::CONTEXT_GET, "verum.context_get");
        assert_eq!(op_names::SPAWN, "verum.spawn");
    }
}
