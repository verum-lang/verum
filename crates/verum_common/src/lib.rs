#![allow(unexpected_cfgs)]
#![feature(pattern)]
// Note: vec_into_raw_parts is stable since Rust 1.93.0
#![allow(dead_code)]
// Allow std types in this crate - it implements the semantic type wrappers
#![allow(clippy::disallowed_types)]

//! Verum Core - Foundation types without dependencies
//!
//! This crate provides the core semantic types used throughout Verum.
//! It has NO dependencies on verum_cbgr or stdlib, breaking circular deps.
//!
//! # Architecture
//!
//! ```text
//! verum_common (foundation)
//!   ├─ Core types: List, Text, Map, Set, Maybe, Result, Heap
//!   ├─ No dependencies on other verum crates
//!   └─ Foundation layer
//!
//! verum_cbgr
//!   ├─ Depends on: verum_common (for types)
//!   ├─ Provides: ThinRef, FatRef, CheckedRef, UnsafeRef
//!   └─ No circular dependency
//!
//! stdlib
//!   ├─ Depends on: verum_common, verum_cbgr
//!   ├─ Re-exports: All verum_common types
//!   ├─ Adds: Async runtime, network, crypto, etc.
//!   └─ Can use CBGR types freely!
//! ```
//!
//! # MANDATORY SEMANTIC TYPES (v6.0-BALANCED)
//!
//! **ALL Verum crates MUST use these semantic types instead of Rust std types:**
//!
//! | Verum Type | Rust Type | Usage |
//! |------------|-----------|-------|
//! | `List<T>` | `List<T>` | Dynamic arrays |
//! | `Text` | `Text` | Owned strings |
//! | `Map<K,V>` | `HashMap<K,V>` | Hash maps |
//! | `Set<T>` | `HashSet<T>` | Hash sets |
//! | `Maybe<T>` | `Option<T>` | Optional values |
//! | `Result<T,E>` | `Result<T,E>` | Error handling |
//! | `Heap<T>` | `Box<T>` | Heap allocation |
//! | `OrderedMap<K,V>` | `BTreeMap<K,V>` | Ordered maps |
//! | `OrderedSet<T>` | `BTreeSet<T>` | Ordered sets |
//!
//! **FORBIDDEN in all crates except verum_common:**
//! - Direct use of `Vec`, `Text`, `HashMap`, `HashSet`
//! - Aliasing std types (e.g., `HashMap as StdHashMap`)
//! - Importing from std::collections or std::vec
//!
//! **CORRECT usage:**
//! ```rust
//! use verum_common::{List, Text, Map, Set};
//!
//! fn process_items(items: List<Text>) -> Map<Text, i32> {
//!     let mut result = Map::new();
//!     // ...
//!     result
//! }
//! ```
//!
//! **INCORRECT usage:**
//! ```rust,ignore
//! use std::collections::HashMap;  // ❌ FORBIDDEN
//! use stdlib::core::List;               // ❌ FORBIDDEN
//!
//! fn bad_function(items: List<Text>) -> HashMap<Text, i32> {
//!     // ❌ SPECIFICATION VIOLATION
//! }
//! ```
//!
//! All Verum crates MUST use semantic types (List, Text, Map, Set, Maybe, Heap)
//! instead of Rust std types (Vec, String, HashMap, HashSet, Option, Box).

// =============================================================================
// SEMANTIC TYPES - v6.0-BALANCED
// =============================================================================
//
// These are newtype wrappers providing semantic naming and rich APIs.
// Re-exported from the semantic_types module where full implementations live.
//
// MANDATORY: Use these types instead of Rust std types in all Verum code.
//
// Semantic type wrappers: meaningful names (List, Text, Map) over implementation names (Vec, String, HashMap)

// Re-export newtype wrapper implementations from semantic_types module
pub use semantic_types::{List, Map, OrderedMap, OrderedSet, Set, Text};

// Well-known type name constants (used throughout the compiler to replace hardcoded strings)
pub mod well_known_types;
pub use well_known_types::{
    WellKnownProtocol, WellKnownType, method_to_protocol, primitive_implements_protocol,
};

/// Maybe type - Semantic name for optional values
///
/// **MANDATORY**: Use `Maybe<T>` instead of `Option<T>` in all Verum code.
///
/// # Examples
/// ```
/// use verum_common::{Maybe, Text};
///
/// fn find_item(id: i32) -> Maybe<Text> {
///     if id > 0 {
///         Some(Text::from("Found"))
///     } else {
///         None
///     }
/// }
/// ```
///
/// Verum semantic type — mandatory in all Verum code
pub type Maybe<T> = Option<T>;

/// Result type - Semantic name for error handling
///
/// Re-export of std::result::Result with semantic naming context.
///
/// # Examples
/// ```
/// use verum_common::{Result, Text};
///
/// fn parse_number(s: &str) -> Result<i32, Text> {
///     s.parse().map_err(|e| Text::from(format!("Parse error: {}", e)))
/// }
/// ```
///
/// Verum semantic type — mandatory in all Verum code
pub type Result<T, E> = std::result::Result<T, E>;

/// Heap type - Semantic name for heap-allocated values
///
/// **MANDATORY**: Use `Heap<T>` instead of `Box<T>` in all Verum code.
///
/// # Examples
/// ```
/// use verum_common::Heap;
///
/// let boxed: Heap<i32> = Heap::new(42);
/// ```
///
/// Verum semantic type — mandatory in all Verum code
#[allow(clippy::disallowed_types)]
pub type Heap<T> = Box<T>;

// Core modules
pub mod cbgr; // CBGR runtime types (headers, validation, allocation)
pub mod const_value; // Unified compile-time constant values
pub mod conversions; // Type conversions (std ↔ verum)
pub mod formatting; // Centralized formatting utilities
pub mod promotion; // Unified reference promotion system
pub mod semantic_types; // Complete semantic types with full API
pub mod shared; // Thread-safe reference counting (Shared<T>, Weak<T>)
pub mod span;
pub mod span_utils;
pub mod to_text; // ToText trait for converting to Text
pub mod type_level; // Unified type-level computation traits
pub mod unsafe_cell;

// Re-export ConstValue as the canonical compile-time constant type
pub use const_value::ConstValue;

// Re-export type-level computation traits and types
pub use type_level::{
    BackendCapabilities, ReductionStrategy, SmtCapableComputation, TypeLevelComputation,
    TypeLevelConfig, TypeLevelError, TypeLevelResult, VerificationResult,
};

// Re-export Shared and Weak types
pub use shared::{Shared, Weak};

// Re-export ToText trait
pub use to_text::ToText;

// Legacy modules for compatibility
pub mod error;
pub mod execution_env;
pub mod traits;
pub mod types;

// Re-export span types for convenience
pub use span::{
    FileId, LineColSpan, SourceFile, Span, Spanned, global_get_filename, global_span_to_line_col,
    register_source_file,
};

// Re-export UnsafeCell for interior mutability
pub use unsafe_cell::UnsafeCell;

// Re-export conversion utilities
pub use conversions::{
    ToMaybe, ToOption, box_to_heap, btreemap_to_ordered_map, btreeset_to_ordered_set,
    hashmap_to_map, hashset_to_set, heap_to_box, list_to_vec, map_to_hashmap, maybe_to_option,
    maybes_to_options, option_to_maybe, options_to_maybes, ordered_map_to_btreemap,
    ordered_set_to_btreeset, result_to_verum, set_to_hashset, str_to_text, string_to_text,
    text_to_string, vec_to_list, verum_to_result,
};

// Re-export old types module for compatibility
pub use error::*;
pub use execution_env::ExecutionEnv;
pub use traits::*;
pub use types::*;

// Re-export CBGR violation types (single source of truth)
// CBGR violation types — single source of truth for memory safety error reporting
pub use error::{CbgrViolation, CbgrViolationKind};

// Re-export CBGR runtime types
// These provide runtime header validation for the interpreter (Tier 0 execution)
pub use cbgr::{
    AllocationHeader, Capability, CbgrErrorCode, CbgrHeader, TrackedAllocation,
    advance_epoch, caps, current_epoch, tracked_alloc_zeroed, tracked_dealloc,
    GEN_INITIAL, GEN_MAX, GEN_PERMANENT, GEN_UNALLOCATED,
};

// Re-export convenience macros for semantic types
// Note: #[macro_export] macros are automatically exported at the crate root
// They're already available as verum_common::{list, map, set, text}
