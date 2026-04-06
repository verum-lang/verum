//! Register Type Map — unified type tracking for VBC → LLVM lowering.
//!
//! This module replaces the 40+ `HashSet<u16>` register tracking fields in
//! `FunctionContext` with a single `HashMap<u16, RegisterType>` that derives
//! all type predicates from VBC TypeRef information.
//!
//! # Architecture
//!
//! The VBC bytecode already carries full type information via `TypeRef` on every
//! instruction. The old approach discarded this information and rebuilt it
//! through ad-hoc `mark_*_register()` / `is_*_register()` calls scattered
//! across 19,000+ lines of instruction.rs. This led to:
//!
//! - 40+ HashSet<u16> fields for separate type categories
//! - `set_register()` clearing 23 HashSets on every register write
//! - Name-based type detection (`starts_with("Map.")`, etc.)
//! - Chain-walking for type propagation through Mov/RefMut
//!
//! The new approach stores type information once per register assignment,
//! using the same `TypeRef` that VBC already provides. All boolean predicates
//! (is_list, is_map, is_float, etc.) are derived on demand via O(1) pattern
//! matching on the stored TypeRef.
//!
//! # Migration Strategy
//!
//! Phase 1 (current): RegisterTypeMap coexists with legacy HashSets.
//!   - Pre-pass populates RegisterTypeMap from VBC instructions
//!   - Legacy HashSets continue to be maintained
//!   - Debug assertions verify consistency
//!
//! Phase 2: Migrate `is_*_register()` call sites to use RegisterTypeMap
//!   - One category at a time (float, bool, list, map, ...)
//!   - Remove corresponding HashSet after each migration
//!
//! Phase 3: Remove all legacy HashSets and `mark_*` methods

use std::collections::{HashMap, HashSet};
use verum_vbc::types::{TypeId, TypeRef};

// ============================================================================
// RegisterType: rich type descriptor for a single register
// ============================================================================

/// Complete type information for a register, derived from VBC TypeRef.
///
/// This replaces all the individual boolean tracking sets (is_list, is_map, etc.)
/// with a single enum that carries the full type + metadata needed for lowering.
#[derive(Debug, Clone)]
pub enum RegisterType {
    /// Primitive integer (Int, I8, I16, I32, I64, U8, U16, U32, U64, ISize, USize).
    Int,
    /// Floating point (Float, F32).
    Float,
    /// Boolean.
    Bool,
    /// Text (C runtime string pointer OR compiled text.vr flat struct).
    Text {
        /// True if this is an owned allocation (Concat/ToString) that needs freeing.
        owned: bool,
        /// True if this is a compiled text.vr flat {ptr, len, cap} struct.
        compiled_layout: bool,
    },
    /// Unit type.
    Unit,
    /// List<T> collection.
    List {
        /// Element type, if known.
        element: Option<Box<RegisterType>>,
    },
    /// Map<K, V> collection.
    Map {
        /// Key type, if known.
        key: Option<Box<RegisterType>>,
        /// Value type, if known.
        value: Option<Box<RegisterType>>,
    },
    /// Set<T> collection.
    Set {
        element: Option<Box<RegisterType>>,
    },
    /// Deque<T> collection.
    Deque {
        element: Option<Box<RegisterType>>,
    },
    /// Channel<T>.
    Channel {
        element: Option<Box<RegisterType>>,
    },
    /// BTreeMap<K, V>.
    BTreeMap {
        key: Option<Box<RegisterType>>,
        value: Option<Box<RegisterType>>,
    },
    /// BTreeSet<T>.
    BTreeSet {
        element: Option<Box<RegisterType>>,
    },
    /// BinaryHeap<T>.
    BinaryHeap {
        element: Option<Box<RegisterType>>,
    },
    /// Range (start..end or start..=end).
    Range {
        /// True if this is a flat (no header) range from NewRange.
        flat: bool,
    },
    /// Maybe<T> variant.
    Maybe {
        /// Inner type.
        inner: Option<Box<RegisterType>>,
        /// If the inner type is Text, used for string propagation through unwrap.
        inner_is_text: bool,
        /// If the inner type is a reference (from compiled module), payload is a pointer.
        inner_is_ref: bool,
    },
    /// Result<T, E> variant.
    Result {
        ok: Option<Box<RegisterType>>,
        err: Option<Box<RegisterType>>,
    },
    /// User-defined variant type.
    Variant {
        /// Type name for method dispatch.
        type_name: Option<String>,
    },
    /// User-defined record/struct type.
    Struct {
        /// Type name for field lookup.
        type_name: String,
        /// Size in bytes (field_count * 8).
        size: u32,
    },
    /// Heap<T> — heap-allocated box.
    Heap {
        inner: Option<Box<RegisterType>>,
    },
    /// Shared<T> — reference-counted.
    Shared {
        inner: Option<Box<RegisterType>>,
    },
    /// AtomicInt / AtomicBool.
    Atomic,
    /// Generator handle.
    Generator,
    /// Slice {ptr, len}.
    Slice {
        element: Option<Box<RegisterType>>,
    },
    /// Text character iterator.
    TextIterator,
    /// Map key iterator.
    MapIterator,
    /// Custom user-defined iterator.
    CustomIterator {
        type_name: String,
    },
    /// Function/closure type.
    Function {
        return_type: Option<Box<RegisterType>>,
    },
    /// Tuple type.
    Tuple {
        elements: Vec<RegisterType>,
    },
    /// Raw pointer.
    Pointer,
    /// Inline struct within array (no object header).
    InlineStruct {
        type_name: String,
        size: u32,
    },
    /// Generic type parameter (compiled as ptr, value-as-pointer).
    GenericParam,
    /// Unknown type (fallback, treat as i64).
    Unknown,
}

impl RegisterType {
    // ========================================================================
    // Boolean predicates — replace all `is_*_register()` methods
    // ========================================================================

    pub fn is_int(&self) -> bool {
        matches!(self, Self::Int)
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Self::Float)
    }

    pub fn is_bool(&self) -> bool {
        matches!(self, Self::Bool)
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }

    pub fn is_owned_text(&self) -> bool {
        matches!(self, Self::Text { owned: true, .. })
    }

    pub fn is_compiled_text(&self) -> bool {
        matches!(self, Self::Text { compiled_layout: true, .. })
    }

    pub fn is_list(&self) -> bool {
        matches!(self, Self::List { .. })
    }

    pub fn is_string_list(&self) -> bool {
        matches!(self, Self::List { element: Some(e) } if e.is_text())
    }

    pub fn is_map(&self) -> bool {
        matches!(self, Self::Map { .. })
    }

    pub fn is_map_with_list_values(&self) -> bool {
        matches!(self, Self::Map { value: Some(v), .. } if v.is_list())
    }

    pub fn is_map_with_text_values(&self) -> bool {
        matches!(self, Self::Map { value: Some(v), .. } if v.is_text())
    }

    pub fn is_set(&self) -> bool {
        matches!(self, Self::Set { .. })
    }

    pub fn is_deque(&self) -> bool {
        matches!(self, Self::Deque { .. })
    }

    pub fn is_channel(&self) -> bool {
        matches!(self, Self::Channel { .. })
    }

    pub fn is_btreemap(&self) -> bool {
        matches!(self, Self::BTreeMap { .. })
    }

    pub fn is_btreeset(&self) -> bool {
        matches!(self, Self::BTreeSet { .. })
    }

    pub fn is_binaryheap(&self) -> bool {
        matches!(self, Self::BinaryHeap { .. })
    }

    pub fn is_range(&self) -> bool {
        matches!(self, Self::Range { .. })
    }

    pub fn is_flat_range(&self) -> bool {
        matches!(self, Self::Range { flat: true })
    }

    pub fn is_maybe(&self) -> bool {
        matches!(self, Self::Maybe { .. })
    }

    pub fn is_maybe_text(&self) -> bool {
        matches!(self, Self::Maybe { inner_is_text: true, .. })
    }

    pub fn is_maybe_ref(&self) -> bool {
        matches!(self, Self::Maybe { inner_is_ref: true, .. })
    }

    pub fn is_variant(&self) -> bool {
        matches!(self, Self::Variant { .. } | Self::Maybe { .. } | Self::Result { .. })
    }

    pub fn is_struct(&self) -> bool {
        matches!(self, Self::Struct { .. })
    }

    pub fn is_atomic(&self) -> bool {
        matches!(self, Self::Atomic)
    }

    pub fn is_generator(&self) -> bool {
        matches!(self, Self::Generator)
    }

    pub fn is_slice(&self) -> bool {
        matches!(self, Self::Slice { .. })
    }

    pub fn is_text_iterator(&self) -> bool {
        matches!(self, Self::TextIterator)
    }

    pub fn is_map_iterator(&self) -> bool {
        matches!(self, Self::MapIterator)
    }

    pub fn is_custom_iterator(&self) -> bool {
        matches!(self, Self::CustomIterator { .. })
    }

    pub fn is_function(&self) -> bool {
        matches!(self, Self::Function { .. })
    }

    pub fn is_tuple(&self) -> bool {
        matches!(self, Self::Tuple { .. })
    }

    pub fn is_inline_struct(&self) -> bool {
        matches!(self, Self::InlineStruct { .. })
    }

    pub fn is_generic_param(&self) -> bool {
        matches!(self, Self::GenericParam)
    }

    pub fn is_heap(&self) -> bool {
        matches!(self, Self::Heap { .. })
    }

    pub fn is_shared(&self) -> bool {
        matches!(self, Self::Shared { .. })
    }

    pub fn is_collection(&self) -> bool {
        matches!(
            self,
            Self::List { .. }
                | Self::Map { .. }
                | Self::Set { .. }
                | Self::Deque { .. }
                | Self::BTreeMap { .. }
                | Self::BTreeSet { .. }
                | Self::BinaryHeap { .. }
        )
    }

    // ========================================================================
    // Type name extraction — replace obj_register_types lookups
    // ========================================================================

    /// Get the type name for method dispatch.
    pub fn type_name(&self) -> Option<&str> {
        match self {
            Self::List { .. } => Some("List"),
            Self::Map { .. } => Some("Map"),
            Self::Set { .. } => Some("Set"),
            Self::Deque { .. } => Some("Deque"),
            Self::Channel { .. } => Some("Channel"),
            Self::BTreeMap { .. } => Some("BTreeMap"),
            Self::BTreeSet { .. } => Some("BTreeSet"),
            Self::BinaryHeap { .. } => Some("BinaryHeap"),
            Self::Text { .. } => Some("Text"),
            Self::Atomic => Some("AtomicInt"),
            Self::Generator => Some("Generator"),
            Self::Struct { type_name, .. } => Some(type_name.as_str()),
            Self::Variant { type_name } => type_name.as_deref(),
            Self::InlineStruct { type_name, .. } => Some(type_name.as_str()),
            Self::CustomIterator { type_name } => Some(type_name.as_str()),
            _ => None,
        }
    }

    /// Get the struct size in bytes.
    pub fn struct_size(&self) -> Option<u32> {
        match self {
            Self::Struct { size, .. } => Some(*size),
            Self::InlineStruct { size, .. } => Some(*size),
            _ => None,
        }
    }

    /// Get the element type of a collection.
    pub fn element_type(&self) -> Option<&RegisterType> {
        match self {
            Self::List { element } => element.as_deref(),
            Self::Set { element } => element.as_deref(),
            Self::Deque { element } => element.as_deref(),
            Self::BTreeSet { element } => element.as_deref(),
            Self::BinaryHeap { element } => element.as_deref(),
            Self::Slice { element } => element.as_deref(),
            _ => None,
        }
    }

    /// Get the key type of a keyed collection.
    pub fn key_type(&self) -> Option<&RegisterType> {
        match self {
            Self::Map { key, .. } => key.as_deref(),
            Self::BTreeMap { key, .. } => key.as_deref(),
            _ => None,
        }
    }

    /// Get the value type of a keyed collection.
    pub fn value_type(&self) -> Option<&RegisterType> {
        match self {
            Self::Map { value, .. } => value.as_deref(),
            Self::BTreeMap { value, .. } => value.as_deref(),
            _ => None,
        }
    }

    /// Get the closure/function return type.
    pub fn return_type(&self) -> Option<&RegisterType> {
        match self {
            Self::Function { return_type } => return_type.as_deref(),
            _ => None,
        }
    }

    /// Get the Maybe inner type.
    pub fn maybe_inner(&self) -> Option<&RegisterType> {
        match self {
            Self::Maybe { inner, .. } => inner.as_deref(),
            _ => None,
        }
    }

    // ========================================================================
    // Construction from VBC TypeRef
    // ========================================================================

    /// Convert a VBC TypeRef to a RegisterType.
    pub fn from_type_ref(ty: &TypeRef) -> Self {
        match ty {
            TypeRef::Concrete(id) => Self::from_type_id(*id),

            TypeRef::Instantiated { base, args } => {
                Self::from_instantiated(*base, args)
            }

            TypeRef::Generic(_) => Self::GenericParam,

            TypeRef::Function {
                return_type, ..
            } => Self::Function {
                return_type: Some(Box::new(Self::from_type_ref(return_type))),
            },

            TypeRef::Rank2Function { return_type, .. } => Self::Function {
                return_type: Some(Box::new(Self::from_type_ref(return_type))),
            },

            TypeRef::Reference { inner, .. } => {
                // References are transparent in AOT — use inner type
                Self::from_type_ref(inner)
            }

            TypeRef::Tuple(elements) => Self::Tuple {
                elements: elements.iter().map(Self::from_type_ref).collect(),
            },

            TypeRef::Array { element, .. } => Self::List {
                element: Some(Box::new(Self::from_type_ref(element))),
            },

            TypeRef::Slice(element) => Self::Slice {
                element: Some(Box::new(Self::from_type_ref(element))),
            },
        }
    }

    /// Convert a concrete TypeId to RegisterType.
    pub fn from_type_id(id: TypeId) -> Self {
        match id {
            TypeId::UNIT => Self::Unit,
            TypeId::BOOL => Self::Bool,
            TypeId::INT | TypeId::U8 | TypeId::U16 | TypeId::U32 | TypeId::U64
            | TypeId::I8 | TypeId::I16 | TypeId::I32 => Self::Int,
            TypeId::FLOAT | TypeId::F32 => Self::Float,
            TypeId::TEXT => Self::Text {
                owned: false,
                compiled_layout: false,
            },
            TypeId::PTR => Self::Pointer,
            TypeId::LIST => Self::List { element: None },
            TypeId::MAP => Self::Map {
                key: None,
                value: None,
            },
            TypeId::SET => Self::Set { element: None },
            TypeId::MAYBE => Self::Maybe {
                inner: None,
                inner_is_text: false,
                inner_is_ref: false,
            },
            TypeId::RESULT => Self::Result {
                ok: None,
                err: None,
            },
            TypeId::RANGE => Self::Range { flat: false },
            TypeId::HEAP => Self::Heap { inner: None },
            TypeId::SHARED => Self::Shared { inner: None },
            TypeId::DEQUE => Self::Deque { element: None },
            TypeId::CHANNEL => Self::Channel { element: None },
            _ => {
                // User-defined type — will be resolved by name later
                Self::Unknown
            }
        }
    }

    /// Convert an instantiated generic type (e.g., List<Int>) to RegisterType.
    fn from_instantiated(base: TypeId, args: &[TypeRef]) -> Self {
        match base {
            TypeId::LIST => Self::List {
                element: args.first().map(|a| Box::new(Self::from_type_ref(a))),
            },
            TypeId::MAP => Self::Map {
                key: args.first().map(|a| Box::new(Self::from_type_ref(a))),
                value: args.get(1).map(|a| Box::new(Self::from_type_ref(a))),
            },
            TypeId::SET => Self::Set {
                element: args.first().map(|a| Box::new(Self::from_type_ref(a))),
            },
            TypeId::DEQUE => Self::Deque {
                element: args.first().map(|a| Box::new(Self::from_type_ref(a))),
            },
            TypeId::CHANNEL => Self::Channel {
                element: args.first().map(|a| Box::new(Self::from_type_ref(a))),
            },
            TypeId::MAYBE => {
                let inner = args.first().map(|a| Box::new(Self::from_type_ref(a)));
                let inner_is_text = inner
                    .as_ref()
                    .map_or(false, |i| i.is_text());
                Self::Maybe {
                    inner,
                    inner_is_text,
                    inner_is_ref: false,
                }
            }
            TypeId::RESULT => Self::Result {
                ok: args.first().map(|a| Box::new(Self::from_type_ref(a))),
                err: args.get(1).map(|a| Box::new(Self::from_type_ref(a))),
            },
            TypeId::HEAP => Self::Heap {
                inner: args.first().map(|a| Box::new(Self::from_type_ref(a))),
            },
            TypeId::SHARED => Self::Shared {
                inner: args.first().map(|a| Box::new(Self::from_type_ref(a))),
            },
            TypeId::RANGE => Self::Range { flat: false },
            _ => {
                // User-defined generic type — resolve by name
                Self::Unknown
            }
        }
    }
}

// ============================================================================
// RegisterTypeMap: the unified register type tracker
// ============================================================================

/// Unified register type tracker.
///
/// Stores a single `RegisterType` per register, replacing 40+ separate
/// HashSet<u16> tracking fields. All type predicates are derived from the
/// stored RegisterType via O(1) pattern matching.
#[derive(Debug, Default)]
pub struct RegisterTypeMap {
    /// Register → type mapping.
    types: HashMap<u16, RegisterType>,
    /// Orthogonal flags: pass-through ref (Deref passes value instead of loading).
    pass_through_ref: HashSet<u16>,
    /// Orthogonal flags: pass-through ref list (List whose elements are pass-through refs).
    pass_through_ref_list: HashSet<u16>,
    /// Orthogonal flags: prescan float (survives set_register clearing).
    prescan_float: HashSet<u16>,
    /// Orthogonal flags: prescan text (survives set_register clearing).
    prescan_text: HashSet<u16>,
    /// FFI-allocated pointers (TransferFrom ownership).
    owned_ffi: HashSet<u16>,
    /// FFI registers whose ownership has been transferred (use-after-transfer warning).
    consumed_ffi: HashSet<u16>,
}

impl RegisterTypeMap {
    /// Create a new empty map.
    pub fn new() -> Self {
        Self {
            types: HashMap::new(),
            pass_through_ref: HashSet::new(),
            pass_through_ref_list: HashSet::new(),
            prescan_float: HashSet::new(),
            prescan_text: HashSet::new(),
            owned_ffi: HashSet::new(),
            consumed_ffi: HashSet::new(),
        }
    }

    /// Clear the type for a register (register reused for different type).
    /// Note: prescan flags are NOT cleared (they survive register reuse by design).
    pub fn clear(&mut self, reg: u16) {
        self.types.remove(&reg);
        self.pass_through_ref.remove(&reg);
        self.pass_through_ref_list.remove(&reg);
    }

    /// Set the type for a register.
    pub fn set(&mut self, reg: u16, ty: RegisterType) {
        self.types.insert(reg, ty);
    }

    /// Set from a VBC TypeRef.
    pub fn set_from_type_ref(&mut self, reg: u16, ty: &TypeRef) {
        self.types.insert(reg, RegisterType::from_type_ref(ty));
    }

    /// Get the type of a register.
    pub fn get(&self, reg: u16) -> Option<&RegisterType> {
        self.types.get(&reg)
    }

    /// Remove a register's type (when the register is overwritten).
    pub fn remove(&mut self, reg: u16) {
        self.types.remove(&reg);
    }

    /// Copy type from one register to another (for Mov instructions).
    pub fn propagate(&mut self, dst: u16, src: u16) {
        if let Some(ty) = self.types.get(&src).cloned() {
            self.types.insert(dst, ty);
        }
    }

    // ========================================================================
    // Boolean predicates — delegating to RegisterType methods
    // ========================================================================

    pub fn is_float(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_float())
    }

    pub fn is_bool(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_bool())
    }

    pub fn is_text(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_text())
    }

    pub fn is_list(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_list())
    }

    pub fn is_string_list(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_string_list())
    }

    pub fn is_map(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_map())
    }

    pub fn is_map_with_list_values(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_map_with_list_values())
    }

    pub fn is_map_with_text_values(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_map_with_text_values())
    }

    pub fn is_set(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_set())
    }

    pub fn is_deque(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_deque())
    }

    pub fn is_channel(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_channel())
    }

    pub fn is_btreemap(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_btreemap())
    }

    pub fn is_btreeset(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_btreeset())
    }

    pub fn is_binaryheap(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_binaryheap())
    }

    pub fn is_range(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_range())
    }

    pub fn is_flat_range(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_flat_range())
    }

    pub fn is_variant(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_variant())
    }

    pub fn is_struct(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_struct())
    }

    pub fn is_atomic(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_atomic())
    }

    pub fn is_generator(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_generator())
    }

    pub fn is_slice(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_slice())
    }

    pub fn is_text_iterator(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_text_iterator())
    }

    pub fn is_map_iterator(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_map_iterator())
    }

    pub fn is_custom_iterator(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_custom_iterator())
    }

    pub fn is_inline_struct(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_inline_struct())
    }

    pub fn is_generic_param(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_generic_param())
    }

    pub fn is_heap(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_heap())
    }

    pub fn is_maybe_text(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_maybe_text())
    }

    pub fn is_maybe_ref(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_maybe_ref())
    }

    pub fn is_collection(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_collection())
    }

    /// Check if register holds an owned text (needs freeing).
    pub fn is_owned_text(&self, reg: u16) -> bool {
        self.get(reg).map_or(false, |t| t.is_owned_text())
    }

    /// Mark a text register as owned (allocated by Concat/ToString, needs freeing).
    pub fn mark_owned_text(&mut self, reg: u16) {
        if let Some(RegisterType::Text { compiled_layout, .. }) = self.types.get(&reg) {
            let cl = *compiled_layout;
            self.types.insert(reg, RegisterType::Text { owned: true, compiled_layout: cl });
        } else {
            // Register not yet marked as Text — set it as owned text with default layout
            self.types.insert(reg, RegisterType::Text { owned: true, compiled_layout: false });
        }
    }

    /// Unmark a text register as owned (e.g., transferring ownership).
    pub fn unmark_owned_text(&mut self, reg: u16) {
        if let Some(RegisterType::Text { compiled_layout, .. }) = self.types.get(&reg) {
            let cl = *compiled_layout;
            self.types.insert(reg, RegisterType::Text { owned: false, compiled_layout: cl });
        }
    }

    /// Get all registers holding owned text (for cleanup at function exit).
    pub fn owned_text_registers(&self) -> Vec<u16> {
        self.types.iter()
            .filter(|(_, ty)| ty.is_owned_text())
            .map(|(&reg, _)| reg)
            .collect()
    }

    // ========================================================================
    // Orthogonal flags (independent of RegisterType)
    // ========================================================================

    /// Mark a register as pass-through ref (Deref passes value instead of loading).
    pub fn mark_pass_through_ref(&mut self, reg: u16) {
        self.pass_through_ref.insert(reg);
    }

    pub fn is_pass_through_ref(&self, reg: u16) -> bool {
        self.pass_through_ref.contains(&reg)
    }

    pub fn mark_pass_through_ref_list(&mut self, reg: u16) {
        self.pass_through_ref_list.insert(reg);
    }

    pub fn is_pass_through_ref_list(&self, reg: u16) -> bool {
        self.pass_through_ref_list.contains(&reg)
    }

    /// Set all prescan float registers (survives register reuse).
    pub fn set_prescan_float(&mut self, regs: HashSet<u16>) {
        self.prescan_float = regs;
    }

    pub fn is_prescan_float(&self, reg: u16) -> bool {
        self.prescan_float.contains(&reg)
    }

    pub fn mark_prescan_float(&mut self, reg: u16) {
        self.prescan_float.insert(reg);
    }

    /// Set all prescan text registers (survives register reuse).
    pub fn set_prescan_text(&mut self, regs: HashSet<u16>) {
        self.prescan_text = regs;
    }

    pub fn is_prescan_text(&self, reg: u16) -> bool {
        self.prescan_text.contains(&reg)
    }

    pub fn mark_prescan_text(&mut self, reg: u16) {
        self.prescan_text.insert(reg);
    }

    /// Mark a register as holding an FFI-owned pointer.
    pub fn mark_owned_ffi(&mut self, reg: u16) {
        self.owned_ffi.insert(reg);
    }

    pub fn is_owned_ffi(&self, reg: u16) -> bool {
        self.owned_ffi.contains(&reg)
    }

    /// Mark a register as consumed (ownership transferred).
    pub fn mark_consumed_ffi(&mut self, reg: u16) {
        self.consumed_ffi.insert(reg);
    }

    pub fn is_consumed_ffi(&self, reg: u16) -> bool {
        self.consumed_ffi.contains(&reg)
    }

    // ========================================================================
    // Type name and metadata extraction
    // ========================================================================

    /// Get the type name for a register (for method dispatch).
    pub fn type_name(&self, reg: u16) -> Option<&str> {
        self.get(reg).and_then(|t| t.type_name())
    }

    /// Get struct size for a register.
    pub fn struct_size(&self, reg: u16) -> Option<u32> {
        self.get(reg).and_then(|t| t.struct_size())
    }

    /// Get element type of a collection register.
    pub fn element_type(&self, reg: u16) -> Option<&RegisterType> {
        self.get(reg).and_then(|t| t.element_type())
    }

    /// Get value type of a map register.
    pub fn map_value_type(&self, reg: u16) -> Option<&RegisterType> {
        self.get(reg).and_then(|t| t.value_type())
    }

    /// Get custom iterator type name.
    pub fn custom_iter_type_name(&self, reg: u16) -> Option<&str> {
        match self.get(reg) {
            Some(RegisterType::CustomIterator { type_name }) => Some(type_name.as_str()),
            _ => None,
        }
    }

    /// Get closure return type.
    pub fn closure_return_type(&self, reg: u16) -> Option<&RegisterType> {
        self.get(reg).and_then(|t| t.return_type())
    }

    /// Get tuple element types.
    pub fn tuple_elements(&self, reg: u16) -> Option<&[RegisterType]> {
        match self.get(reg) {
            Some(RegisterType::Tuple { elements }) => Some(elements.as_slice()),
            _ => None,
        }
    }

    /// Get total number of tracked registers.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Check if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    // ========================================================================
    // Phase 3: Patch methods for nested type refinement
    // ========================================================================

    /// Mark a List register's elements as Text (List<Text>).
    pub fn mark_list_text_elements(&mut self, reg: u16) {
        let text = Box::new(RegisterType::Text { owned: false, compiled_layout: false });
        match self.types.get_mut(&reg) {
            Some(RegisterType::List { element, .. }) => { *element = Some(text); }
            _ => { self.types.insert(reg, RegisterType::List { element: Some(text) }); }
        }
    }

    /// Mark a Map register's values as List (Map<K, List<V>>).
    pub fn mark_map_list_values(&mut self, reg: u16) {
        let list = Box::new(RegisterType::List { element: None });
        match self.types.get_mut(&reg) {
            Some(RegisterType::Map { value, .. }) => { *value = Some(list); }
            _ => { self.types.insert(reg, RegisterType::Map { key: None, value: Some(list) }); }
        }
    }

    /// Mark a Map register's values as Text (Map<K, Text>).
    pub fn mark_map_text_values(&mut self, reg: u16) {
        let text = Box::new(RegisterType::Text { owned: false, compiled_layout: false });
        match self.types.get_mut(&reg) {
            Some(RegisterType::Map { value, .. }) => { *value = Some(text); }
            _ => { self.types.insert(reg, RegisterType::Map { key: None, value: Some(text) }); }
        }
    }

    /// Mark a Maybe register's inner as Text (Maybe<Text>).
    pub fn mark_maybe_text_inner(&mut self, reg: u16) {
        let text = Box::new(RegisterType::Text { owned: false, compiled_layout: false });
        match self.types.get_mut(&reg) {
            Some(RegisterType::Maybe { inner, inner_is_text, .. }) => {
                *inner = Some(text);
                *inner_is_text = true;
            }
            _ => {
                self.types.insert(reg, RegisterType::Maybe {
                    inner: Some(text), inner_is_text: true, inner_is_ref: false,
                });
            }
        }
    }

    /// Mark a Maybe register's inner as a reference (Maybe<&T>).
    pub fn mark_maybe_ref_inner(&mut self, reg: u16) {
        match self.types.get_mut(&reg) {
            Some(RegisterType::Maybe { inner_is_ref, .. }) => { *inner_is_ref = true; }
            _ => {
                self.types.insert(reg, RegisterType::Maybe {
                    inner: None, inner_is_text: false, inner_is_ref: true,
                });
            }
        }
    }
}

// ============================================================================
// MethodDispatchTarget: where to route method calls
// ============================================================================

/// How to compile a method call on a given type.
///
/// Replaces the Strategy 0/1/2/3/4 cascade in instruction.rs with a
/// declarative dispatch table.
#[derive(Debug, Clone)]
pub enum MethodDispatchTarget {
    /// Call a compiled .vr function found in the VBC module.
    /// The function was compiled from stdlib or user code.
    Compiled {
        /// Function index in the VBC module.
        func_index: usize,
        /// True if the compiled function returns Maybe<T> but the caller
        /// expects raw T (the needs_maybe_unwrap pattern).
        needs_unwrap: bool,
    },

    /// Call a C runtime function by symbol name.
    CRuntime {
        /// C function symbol (e.g., "verum_chan_send", "verum_map_get").
        symbol: &'static str,
    },

    /// Emit inline LLVM IR (for performance-critical operations).
    /// The handler function name identifies which inline emitter to use.
    InlineLlvm {
        /// Handler identifier (e.g., "list_contains", "list_sort").
        handler: &'static str,
    },

    /// Use the standard compiled function lookup (Strategy 1/2 in current code).
    /// This is the default for user-defined types.
    StandardLookup,
}

// ============================================================================
// MethodDispatchTable: declarative method routing
// ============================================================================

/// Declarative method dispatch table.
///
/// Maps (type_category, method_name) → DispatchTarget. Built once per
/// module compilation, replacing the 3000-line Strategy cascade.
#[derive(Debug, Default)]
pub struct MethodDispatchTable {
    /// Overrides for specific (type, method) combinations.
    /// Key: "TypeName.method_name"
    overrides: HashMap<String, MethodDispatchTarget>,
}

impl MethodDispatchTable {
    /// Create a new dispatch table with all built-in overrides.
    pub fn new() -> Self {
        let mut table = Self {
            overrides: HashMap::with_capacity(128),
        };
        table.register_builtins();
        table
    }

    /// Look up the dispatch target for a method call.
    pub fn lookup(&self, type_name: &str, method_name: &str) -> Option<&MethodDispatchTarget> {
        // Try exact match first: "List.push"
        let key = format!("{}.{}", type_name, method_name);
        self.overrides.get(&key)
    }

    /// Register a dispatch override.
    pub fn register(&mut self, type_name: &str, method_name: &str, target: MethodDispatchTarget) {
        self.overrides
            .insert(format!("{}.{}", type_name, method_name), target);
    }

    /// Register all built-in method dispatch overrides.
    ///
    /// These are methods that cannot go through the standard compiled
    /// function lookup because they need C runtime, inline LLVM IR,
    /// or special handling.
    fn register_builtins(&mut self) {
        // ================================================================
        // Channel methods → C runtime (thread safety requires C ABI)
        // ================================================================
        for method in &[
            "send", "recv", "receive", "close", "len", "try_send",
            "try_recv", "is_closed", "is_empty", "is_full",
        ] {
            self.register("Channel", method, MethodDispatchTarget::CRuntime {
                symbol: match *method {
                    "send" => "verum_chan_send",
                    "recv" | "receive" => "verum_chan_recv",
                    "close" => "verum_chan_close",
                    "len" => "verum_chan_len",
                    "try_send" => "verum_chan_try_send",
                    "try_recv" => "verum_chan_try_recv",
                    "is_closed" => "verum_chan_is_closed",
                    "is_empty" => "verum_chan_is_empty",
                    "is_full" => "verum_chan_is_full",
                    _ => unreachable!(),
                },
            });
        }

        // ================================================================
        // List methods → inline LLVM IR (performance + ABI compatibility)
        // ================================================================
        for method in &[
            "contains", "index_of", "sort", "extend", "first", "last",
            "clone", "clear", "join", "push", "insert", "remove",
            "reverse", "swap",
        ] {
            self.register("List", method, MethodDispatchTarget::InlineLlvm {
                handler: match *method {
                    "contains" => "list_contains",
                    "index_of" => "list_index_of",
                    "sort" => "list_sort",
                    "extend" => "list_extend",
                    "first" => "list_first",
                    "last" => "list_last",
                    "clone" => "list_clone",
                    "clear" => "list_clear",
                    "join" => "list_join",
                    "push" => "list_push",
                    "insert" => "list_insert",
                    "remove" => "list_remove",
                    "reverse" => "list_reverse",
                    "swap" => "list_swap",
                    _ => unreachable!(),
                },
            });
        }

        // ================================================================
        // Text methods returning raw values (needs_maybe_unwrap bridge)
        // ================================================================
        for method in &[
            "find", "index_of", "rfind", "byte_at", "char_at", "nth_char",
        ] {
            // These go through compiled text.vr but return Maybe<Int>,
            // while user code expects raw Int (-1 sentinel for not-found).
            self.register("Text", method, MethodDispatchTarget::Compiled {
                func_index: 0, // resolved at lookup time
                needs_unwrap: true,
            });
        }

        // ================================================================
        // AtomicInt/AtomicBool → inline LLVM atomics
        // ================================================================
        for method in &[
            "new", "load", "store", "fetch_add", "fetch_sub", "fetch_and",
            "fetch_or", "fetch_xor", "swap", "compare_exchange",
            "increment", "decrement", "compare_and_swap",
        ] {
            self.register("AtomicInt", method, MethodDispatchTarget::InlineLlvm {
                handler: "atomic_op",
            });
            self.register("AtomicBool", method, MethodDispatchTarget::InlineLlvm {
                handler: "atomic_op",
            });
        }

        // ================================================================
        // Synchronization primitives → C runtime
        // ================================================================
        for method in &[
            "new", "try_acquire", "release", "acquire",
            "try_acquire_many", "release_many", "add_permits",
            "forget_permit", "available_permits", "max_permits",
        ] {
            self.register("Semaphore", method, MethodDispatchTarget::CRuntime {
                symbol: "verum_sem_op",
            });
        }

        for method in &[
            "new", "read", "write", "try_read", "try_write", "is_poisoned",
        ] {
            self.register("RwLock", method, MethodDispatchTarget::CRuntime {
                symbol: "verum_rwlock_op",
            });
        }

        for method in &["new", "call_once", "do_once", "is_completed"] {
            self.register("Once", method, MethodDispatchTarget::CRuntime {
                symbol: "verum_once_op",
            });
        }
    }

    /// Get the number of registered overrides.
    pub fn override_count(&self) -> usize {
        self.overrides.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_type_from_type_id() {
        assert!(RegisterType::from_type_id(TypeId::INT).is_int());
        assert!(RegisterType::from_type_id(TypeId::FLOAT).is_float());
        assert!(RegisterType::from_type_id(TypeId::BOOL).is_bool());
        assert!(RegisterType::from_type_id(TypeId::TEXT).is_text());
        assert!(RegisterType::from_type_id(TypeId::LIST).is_list());
        assert!(RegisterType::from_type_id(TypeId::MAP).is_map());
        assert!(RegisterType::from_type_id(TypeId::SET).is_set());
        assert!(RegisterType::from_type_id(TypeId::DEQUE).is_deque());
        assert!(RegisterType::from_type_id(TypeId::CHANNEL).is_channel());
        assert!(RegisterType::from_type_id(TypeId::MAYBE).is_maybe());
        assert!(RegisterType::from_type_id(TypeId::RESULT).is_variant());
        assert!(RegisterType::from_type_id(TypeId::RANGE).is_range());
    }

    #[test]
    fn test_register_type_from_instantiated() {
        let list_int = RegisterType::from_type_ref(&TypeRef::Instantiated {
            base: TypeId::LIST,
            args: vec![TypeRef::Concrete(TypeId::INT)],
        });
        assert!(list_int.is_list());
        assert!(list_int.element_type().unwrap().is_int());

        let map_text_list = RegisterType::from_type_ref(&TypeRef::Instantiated {
            base: TypeId::MAP,
            args: vec![
                TypeRef::Concrete(TypeId::TEXT),
                TypeRef::Instantiated {
                    base: TypeId::LIST,
                    args: vec![TypeRef::Concrete(TypeId::INT)],
                },
            ],
        });
        assert!(map_text_list.is_map());
        assert!(map_text_list.is_map_with_list_values());
        assert!(map_text_list.key_type().unwrap().is_text());
    }

    #[test]
    fn test_register_type_map_operations() {
        let mut map = RegisterTypeMap::new();
        map.set(0, RegisterType::Int);
        map.set(1, RegisterType::List { element: None });
        map.set(2, RegisterType::Float);

        assert!(map.is_list(1));
        assert!(!map.is_list(0));
        assert!(map.is_float(2));
        assert_eq!(map.type_name(1), Some("List"));

        // Test propagation (Mov)
        map.propagate(3, 1);
        assert!(map.is_list(3));

        // Test overwrite
        map.set(1, RegisterType::Text {
            owned: false,
            compiled_layout: false,
        });
        assert!(map.is_text(1));
        assert!(!map.is_list(1));
    }

    #[test]
    fn test_method_dispatch_table() {
        let table = MethodDispatchTable::new();
        assert!(table.lookup("Channel", "send").is_some());
        assert!(table.lookup("List", "push").is_some());
        assert!(table.lookup("AtomicInt", "fetch_add").is_some());
        assert!(table.lookup("UserType", "custom_method").is_none());
    }

    #[test]
    fn test_type_names() {
        assert_eq!(RegisterType::List { element: None }.type_name(), Some("List"));
        assert_eq!(RegisterType::Map { key: None, value: None }.type_name(), Some("Map"));
        assert_eq!(
            RegisterType::Struct {
                type_name: "Point".to_string(),
                size: 16,
            }
            .type_name(),
            Some("Point")
        );
        assert_eq!(RegisterType::Int.type_name(), None);
    }
}
