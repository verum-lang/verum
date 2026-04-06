//! Verum dialect types.
//!
//! Custom MLIR types for Verum language constructs. These types are
//! represented using MLIR's opaque type system with custom parsing/printing.
//!
//! # Type Hierarchy
//!
//! ```text
//! VerumType
//! ├── RefType<T, tier>      - CBGR three-tier reference
//! ├── ListType<T>           - Semantic list (Vec equivalent)
//! ├── MapType<K, V>         - Semantic map (HashMap equivalent)
//! ├── SetType<T>            - Semantic set (HashSet equivalent)
//! ├── TextType              - Semantic text (String equivalent)
//! ├── MaybeType<T>          - Optional value (Option equivalent)
//! ├── FutureType<T>         - Async future
//! ├── ContextType           - Context value for DI
//! └── HeapType<T>           - Heap-allocated value (Box equivalent)
//! ```

use verum_mlir::{
    Context,
    ir::{Type, TypeLike},
    ir::attribute::{IntegerAttribute, StringAttribute, TypeAttribute},
};
use verum_common::Text;
use crate::mlir::error::{MlirError, Result};

/// Reference tier for CBGR three-tier system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RefTier {
    /// Tier 0: Full CBGR protection (~15ns overhead)
    Managed = 0,

    /// Tier 1: Checked references (0ns after escape analysis proof)
    Checked = 1,

    /// Tier 2: Unsafe references (0ns, manual safety proof)
    Unsafe = 2,
}

impl RefTier {
    /// Create from integer value.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Managed),
            1 => Some(Self::Checked),
            2 => Some(Self::Unsafe),
            _ => None,
        }
    }

    /// Get the overhead in nanoseconds.
    pub fn overhead_ns(&self) -> u32 {
        match self {
            Self::Managed => 15,
            Self::Checked => 0,
            Self::Unsafe => 0,
        }
    }

    /// Check if this tier requires runtime checks.
    pub fn requires_runtime_check(&self) -> bool {
        matches!(self, Self::Managed)
    }
}

impl std::fmt::Display for RefTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Managed => write!(f, "managed"),
            Self::Checked => write!(f, "checked"),
            Self::Unsafe => write!(f, "unsafe"),
        }
    }
}

/// Verum type representation.
///
/// Wraps MLIR types with Verum-specific semantics.
#[derive(Debug, Clone)]
pub enum VerumType {
    /// Primitive integer type.
    Int { bits: u32, signed: bool },

    /// Floating point type.
    Float { bits: u32 },

    /// Boolean type.
    Bool,

    /// Unit type.
    Unit,

    /// Character type.
    Char,

    /// Reference type with CBGR tier.
    Ref(RefType),

    /// List collection type.
    List(ListType),

    /// Map collection type.
    Map(MapType),

    /// Set collection type.
    Set(SetType),

    /// Text string type.
    Text(TextType),

    /// Maybe/Optional type.
    Maybe(MaybeType),

    /// Future type for async.
    Future(FutureType),

    /// Context type for DI.
    Context(ContextType),

    /// Heap-allocated type.
    Heap(HeapType),

    /// Tuple type.
    Tuple(TupleType),

    /// Function type.
    Function(FunctionType),

    /// Record/struct type.
    Record(RecordType),

    /// Variant/enum type.
    Variant(VariantType),

    /// Opaque type (for FFI or unknown types).
    Opaque { name: Text },
}

impl VerumType {
    /// Get the MLIR type string representation.
    pub fn to_mlir_type_string(&self) -> Text {
        match self {
            Self::Int { bits, signed } => {
                if *signed {
                    Text::from(format!("i{}", bits))
                } else {
                    Text::from(format!("ui{}", bits))
                }
            }
            Self::Float { bits } => Text::from(format!("f{}", bits)),
            Self::Bool => Text::from("i1"),
            Self::Unit => Text::from("()"),
            Self::Char => Text::from("i32"), // Unicode code point
            Self::Ref(r) => r.to_mlir_type_string(),
            Self::List(l) => l.to_mlir_type_string(),
            Self::Map(m) => m.to_mlir_type_string(),
            Self::Set(s) => s.to_mlir_type_string(),
            Self::Text(t) => t.to_mlir_type_string(),
            Self::Maybe(m) => m.to_mlir_type_string(),
            Self::Future(f) => f.to_mlir_type_string(),
            Self::Context(c) => c.to_mlir_type_string(),
            Self::Heap(h) => h.to_mlir_type_string(),
            Self::Tuple(t) => t.to_mlir_type_string(),
            Self::Function(f) => f.to_mlir_type_string(),
            Self::Record(r) => r.to_mlir_type_string(),
            Self::Variant(v) => v.to_mlir_type_string(),
            Self::Opaque { name } => Text::from(format!("!verum.opaque<{}>", name)),
        }
    }

    /// Convert to MLIR Type.
    pub fn to_mlir_type<'c>(&self, ctx: &'c Context) -> Result<Type<'c>> {
        let type_str = self.to_mlir_type_string();
        Type::parse(ctx, type_str.as_str())
            .ok_or_else(|| MlirError::type_translation(type_str.clone(), "failed to parse MLIR type"))
    }

    /// Create Int type.
    pub fn int(bits: u32, signed: bool) -> Self {
        Self::Int { bits, signed }
    }

    /// Create i64 (default Int).
    pub fn i64() -> Self {
        Self::Int { bits: 64, signed: true }
    }

    /// Create f64 (default Float).
    pub fn f64() -> Self {
        Self::Float { bits: 64 }
    }

    /// Create Bool type.
    pub fn bool() -> Self {
        Self::Bool
    }

    /// Create Unit type.
    pub fn unit() -> Self {
        Self::Unit
    }
}

/// CBGR reference type.
#[derive(Debug, Clone)]
pub struct RefType {
    /// The inner type being referenced.
    pub inner: Box<VerumType>,

    /// Reference tier (0=managed, 1=checked, 2=unsafe).
    pub tier: RefTier,

    /// Whether this is a mutable reference.
    pub mutable: bool,
}

impl RefType {
    /// Create a new reference type.
    pub fn new(inner: VerumType, tier: RefTier, mutable: bool) -> Self {
        Self {
            inner: Box::new(inner),
            tier,
            mutable,
        }
    }

    /// Create a managed (tier 0) reference.
    pub fn managed(inner: VerumType, mutable: bool) -> Self {
        Self::new(inner, RefTier::Managed, mutable)
    }

    /// Create a checked (tier 1) reference.
    pub fn checked(inner: VerumType, mutable: bool) -> Self {
        Self::new(inner, RefTier::Checked, mutable)
    }

    /// Create an unsafe (tier 2) reference.
    pub fn unsafe_ref(inner: VerumType, mutable: bool) -> Self {
        Self::new(inner, RefTier::Unsafe, mutable)
    }

    /// Get MLIR type string representation.
    ///
    /// References are lowered to LLVM struct types:
    /// - ThinRef: { ptr, i32 generation, i32 epoch_caps } = 16 bytes
    /// - FatRef: { ptr, i32 gen, i32 epoch_caps, i64 metadata } = 24 bytes
    pub fn to_mlir_type_string(&self) -> Text {
        // For now, represent as a pointer type with metadata
        // The actual structure depends on CBGR tier
        let inner_str = self.inner.to_mlir_type_string();
        match self.tier {
            RefTier::Managed => {
                // ThinRef structure: { ptr, gen: i32, epoch_caps: i32 }
                Text::from(format!(
                    "!llvm.struct<(ptr, i32, i32)>",
                ))
            }
            RefTier::Checked | RefTier::Unsafe => {
                // Just a pointer for checked/unsafe
                Text::from("!llvm.ptr")
            }
        }
    }
}

/// List type (semantic Vec).
#[derive(Debug, Clone)]
pub struct ListType {
    /// Element type.
    pub element: Box<VerumType>,
}

impl ListType {
    /// Create a new list type.
    pub fn new(element: VerumType) -> Self {
        Self {
            element: Box::new(element),
        }
    }

    /// Get MLIR type string.
    pub fn to_mlir_type_string(&self) -> Text {
        // Lists are represented as opaque pointers to CBGR-tracked structures
        Text::from("!llvm.ptr")
    }
}

/// Map type (semantic HashMap).
#[derive(Debug, Clone)]
pub struct MapType {
    /// Key type.
    pub key: Box<VerumType>,

    /// Value type.
    pub value: Box<VerumType>,
}

impl MapType {
    /// Create a new map type.
    pub fn new(key: VerumType, value: VerumType) -> Self {
        Self {
            key: Box::new(key),
            value: Box::new(value),
        }
    }

    /// Get MLIR type string.
    pub fn to_mlir_type_string(&self) -> Text {
        Text::from("!llvm.ptr")
    }
}

/// Set type (semantic HashSet).
#[derive(Debug, Clone)]
pub struct SetType {
    /// Element type.
    pub element: Box<VerumType>,
}

impl SetType {
    /// Create a new set type.
    pub fn new(element: VerumType) -> Self {
        Self {
            element: Box::new(element),
        }
    }

    /// Get MLIR type string.
    pub fn to_mlir_type_string(&self) -> Text {
        Text::from("!llvm.ptr")
    }
}

/// Text type (semantic String).
#[derive(Debug, Clone)]
pub struct TextType;

impl TextType {
    /// Create a new text type.
    pub fn new() -> Self {
        Self
    }

    /// Get MLIR type string.
    ///
    /// Text is represented as { ptr: i8*, len: i64 }
    pub fn to_mlir_type_string(&self) -> Text {
        Text::from("!llvm.struct<(ptr, i64)>")
    }
}

impl Default for TextType {
    fn default() -> Self {
        Self::new()
    }
}

/// Maybe type (semantic Option).
#[derive(Debug, Clone)]
pub struct MaybeType {
    /// Inner type.
    pub inner: Box<VerumType>,
}

impl MaybeType {
    /// Create a new maybe type.
    pub fn new(inner: VerumType) -> Self {
        Self {
            inner: Box::new(inner),
        }
    }

    /// Get MLIR type string.
    ///
    /// Maybe is represented as { tag: i1, value: T }
    pub fn to_mlir_type_string(&self) -> Text {
        let inner_str = self.inner.to_mlir_type_string();
        Text::from(format!("!llvm.struct<(i1, {})>", inner_str))
    }
}

/// Future type for async.
#[derive(Debug, Clone)]
pub struct FutureType {
    /// Result type.
    pub result: Box<VerumType>,
}

impl FutureType {
    /// Create a new future type.
    pub fn new(result: VerumType) -> Self {
        Self {
            result: Box::new(result),
        }
    }

    /// Get MLIR type string.
    pub fn to_mlir_type_string(&self) -> Text {
        // Futures are opaque pointers to state machines
        Text::from("!llvm.ptr")
    }
}

/// Context type for dependency injection.
#[derive(Debug, Clone)]
pub struct ContextType {
    /// Context name.
    pub name: Text,
}

impl ContextType {
    /// Create a new context type.
    pub fn new(name: impl Into<Text>) -> Self {
        Self { name: name.into() }
    }

    /// Get MLIR type string.
    pub fn to_mlir_type_string(&self) -> Text {
        // Contexts are opaque pointers
        Text::from("!llvm.ptr")
    }
}

/// Heap type (semantic Box).
#[derive(Debug, Clone)]
pub struct HeapType {
    /// Inner type.
    pub inner: Box<VerumType>,
}

impl HeapType {
    /// Create a new heap type.
    pub fn new(inner: VerumType) -> Self {
        Self {
            inner: Box::new(inner),
        }
    }

    /// Get MLIR type string.
    pub fn to_mlir_type_string(&self) -> Text {
        Text::from("!llvm.ptr")
    }
}

/// Tuple type.
#[derive(Debug, Clone)]
pub struct TupleType {
    /// Element types.
    pub elements: Vec<VerumType>,
}

impl TupleType {
    /// Create a new tuple type.
    pub fn new(elements: Vec<VerumType>) -> Self {
        Self { elements }
    }

    /// Get MLIR type string.
    pub fn to_mlir_type_string(&self) -> Text {
        if self.elements.is_empty() {
            return Text::from("()");
        }

        let elements_str: Vec<String> = self.elements
            .iter()
            .map(|t| t.to_mlir_type_string().to_string())
            .collect();

        Text::from(format!("!llvm.struct<({})>", elements_str.join(", ")))
    }
}

/// Function type.
#[derive(Debug, Clone)]
pub struct FunctionType {
    /// Parameter types.
    pub params: Vec<VerumType>,

    /// Return type.
    pub result: Box<VerumType>,

    /// Whether this is async.
    pub is_async: bool,

    /// Required contexts.
    pub contexts: Vec<Text>,
}

impl FunctionType {
    /// Create a new function type.
    pub fn new(params: Vec<VerumType>, result: VerumType) -> Self {
        Self {
            params,
            result: Box::new(result),
            is_async: false,
            contexts: Vec::new(),
        }
    }

    /// Mark as async.
    pub fn with_async(mut self) -> Self {
        self.is_async = true;
        self
    }

    /// Add context requirements.
    pub fn with_contexts(mut self, contexts: Vec<Text>) -> Self {
        self.contexts = contexts;
        self
    }

    /// Get MLIR type string.
    pub fn to_mlir_type_string(&self) -> Text {
        let params_str: Vec<String> = self.params
            .iter()
            .map(|t| t.to_mlir_type_string().to_string())
            .collect();

        let result_str = self.result.to_mlir_type_string();

        Text::from(format!(
            "({}) -> {}",
            params_str.join(", "),
            result_str
        ))
    }
}

/// Record/struct type.
#[derive(Debug, Clone)]
pub struct RecordType {
    /// Type name.
    pub name: Text,

    /// Field names and types.
    pub fields: Vec<(Text, VerumType)>,
}

impl RecordType {
    /// Create a new record type.
    pub fn new(name: impl Into<Text>, fields: Vec<(Text, VerumType)>) -> Self {
        Self {
            name: name.into(),
            fields,
        }
    }

    /// Get MLIR type string.
    pub fn to_mlir_type_string(&self) -> Text {
        let fields_str: Vec<String> = self.fields
            .iter()
            .map(|(_, t)| t.to_mlir_type_string().to_string())
            .collect();

        Text::from(format!("!llvm.struct<({})>", fields_str.join(", ")))
    }
}

/// Variant/enum type.
#[derive(Debug, Clone)]
pub struct VariantType {
    /// Type name.
    pub name: Text,

    /// Variant names and optional payload types.
    pub variants: Vec<(Text, Option<VerumType>)>,
}

impl VariantType {
    /// Create a new variant type.
    pub fn new(name: impl Into<Text>, variants: Vec<(Text, Option<VerumType>)>) -> Self {
        Self {
            name: name.into(),
            variants,
        }
    }

    /// Get MLIR type string.
    ///
    /// Variants are represented as { tag: i32, payload: max_payload_size }
    pub fn to_mlir_type_string(&self) -> Text {
        // Simplified: tag + pointer to payload
        Text::from("!llvm.struct<(i32, ptr)>")
    }
}

/// Type translator from Verum AST types to MLIR types.
pub struct TypeTranslator<'c> {
    context: &'c Context,
}

impl<'c> TypeTranslator<'c> {
    /// Create a new type translator.
    pub fn new(context: &'c Context) -> Self {
        Self { context }
    }

    /// Translate a Verum AST type to VerumType.
    pub fn translate(&self, ast_type: &verum_ast::Type) -> Result<VerumType> {
        use verum_ast::TypeKind;

        match &ast_type.kind {
            TypeKind::Path(path) => {
                let name = path.to_string();
                self.translate_named_type(&name)
            }

            TypeKind::Reference { mutable, inner, .. } => {
                let inner_type = self.translate(inner)?;
                Ok(VerumType::Ref(RefType::managed(inner_type, *mutable)))
            }

            TypeKind::Tuple(elements) => {
                let element_types: Result<Vec<_>> = elements
                    .iter()
                    .map(|e| self.translate(e))
                    .collect();
                Ok(VerumType::Tuple(TupleType::new(element_types?)))
            }

            TypeKind::Array { element, .. } => {
                let elem_type = self.translate(element)?;
                Ok(VerumType::List(ListType::new(elem_type)))
            }

            TypeKind::Slice(element) => {
                let elem_type = self.translate(element)?;
                Ok(VerumType::List(ListType::new(elem_type)))
            }

            TypeKind::Function { params, return_type, .. } => {
                let param_types: Result<Vec<_>> = params
                    .iter()
                    .map(|p| self.translate(p))
                    .collect();

                let result_type = self.translate(return_type)?;

                Ok(VerumType::Function(FunctionType::new(param_types?, result_type)))
            }

            TypeKind::Generic { base, args } => {
                // Extract the base type name from the type kind
                let base_name = match &base.kind {
                    TypeKind::Path(path) => {
                        // Get the last segment's name from the path
                        path.as_ident()
                            .map(|ident| ident.name.as_str())
                            .unwrap_or_else(|| {
                                // For multi-segment paths, try to extract from last segment
                                path.segments.last()
                                    .and_then(|seg| match seg {
                                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                                        _ => None,
                                    })
                                    .unwrap_or("unknown")
                            })
                    }
                    _ => "unknown",
                };
                self.translate_generic_type(base_name, args)
            }

            TypeKind::Inferred => {
                // Placeholder for inferred types
                Ok(VerumType::Opaque { name: Text::from("inferred") })
            }

            TypeKind::Never => {
                Ok(VerumType::Opaque { name: Text::from("never") })
            }

            TypeKind::Unit => Ok(VerumType::Unit),

            _ => Err(MlirError::type_translation(
                format!("{:?}", ast_type.kind),
                "unsupported type kind",
            )),
        }
    }

    /// Translate a named type (Int, Float, Bool, Text, etc.).
    fn translate_named_type(&self, name: &str) -> Result<VerumType> {
        match name {
            "Int" | "i64" => Ok(VerumType::i64()),
            "i8" => Ok(VerumType::int(8, true)),
            "i16" => Ok(VerumType::int(16, true)),
            "i32" => Ok(VerumType::int(32, true)),
            "i128" => Ok(VerumType::int(128, true)),

            "u8" => Ok(VerumType::int(8, false)),
            "u16" => Ok(VerumType::int(16, false)),
            "u32" => Ok(VerumType::int(32, false)),
            "u64" => Ok(VerumType::int(64, false)),
            "u128" => Ok(VerumType::int(128, false)),

            "Float" | "f64" => Ok(VerumType::f64()),
            "f32" => Ok(VerumType::Float { bits: 32 }),

            "Bool" => Ok(VerumType::Bool),

            "Text" => Ok(VerumType::Text(TextType::new())),

            "Char" => Ok(VerumType::Char),

            "()" | "Unit" => Ok(VerumType::Unit),

            _ => Ok(VerumType::Opaque { name: Text::from(name) }),
        }
    }

    /// Translate a generic type (List<T>, Map<K,V>, etc.).
    fn translate_generic_type(
        &self,
        base: &str,
        args: &[verum_ast::ty::GenericArg],
    ) -> Result<VerumType> {
        match base {
            "List" => {
                if let Some(verum_ast::ty::GenericArg::Type(elem)) = args.first() {
                    let elem_type = self.translate(elem)?;
                    Ok(VerumType::List(ListType::new(elem_type)))
                } else {
                    Err(MlirError::type_translation("List", "missing element type"))
                }
            }

            "Map" => {
                if args.len() >= 2 {
                    if let (
                        Some(verum_ast::ty::GenericArg::Type(key)),
                        Some(verum_ast::ty::GenericArg::Type(val)),
                    ) = (args.first(), args.get(1))
                    {
                        let key_type = self.translate(key)?;
                        let val_type = self.translate(val)?;
                        Ok(VerumType::Map(MapType::new(key_type, val_type)))
                    } else {
                        Err(MlirError::type_translation("Map", "invalid type arguments"))
                    }
                } else {
                    Err(MlirError::type_translation("Map", "missing type arguments"))
                }
            }

            "Set" => {
                if let Some(verum_ast::ty::GenericArg::Type(elem)) = args.first() {
                    let elem_type = self.translate(elem)?;
                    Ok(VerumType::Set(SetType::new(elem_type)))
                } else {
                    Err(MlirError::type_translation("Set", "missing element type"))
                }
            }

            "Maybe" => {
                if let Some(verum_ast::ty::GenericArg::Type(inner)) = args.first() {
                    let inner_type = self.translate(inner)?;
                    Ok(VerumType::Maybe(MaybeType::new(inner_type)))
                } else {
                    Err(MlirError::type_translation("Maybe", "missing inner type"))
                }
            }

            "Heap" => {
                if let Some(verum_ast::ty::GenericArg::Type(inner)) = args.first() {
                    let inner_type = self.translate(inner)?;
                    Ok(VerumType::Heap(HeapType::new(inner_type)))
                } else {
                    Err(MlirError::type_translation("Heap", "missing inner type"))
                }
            }

            "Future" => {
                if let Some(verum_ast::ty::GenericArg::Type(result)) = args.first() {
                    let result_type = self.translate(result)?;
                    Ok(VerumType::Future(FutureType::new(result_type)))
                } else {
                    Ok(VerumType::Future(FutureType::new(VerumType::Unit)))
                }
            }

            _ => Ok(VerumType::Opaque { name: Text::from(base) }),
        }
    }

    /// Convert VerumType to MLIR Type.
    pub fn to_mlir_type(&self, verum_type: &VerumType) -> Result<Type<'c>> {
        verum_type.to_mlir_type(self.context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ref_tier() {
        assert_eq!(RefTier::Managed.overhead_ns(), 15);
        assert_eq!(RefTier::Checked.overhead_ns(), 0);
        assert!(RefTier::Managed.requires_runtime_check());
        assert!(!RefTier::Checked.requires_runtime_check());
    }

    #[test]
    fn test_verum_type_strings() {
        assert_eq!(VerumType::i64().to_mlir_type_string().as_str(), "i64");
        assert_eq!(VerumType::f64().to_mlir_type_string().as_str(), "f64");
        assert_eq!(VerumType::bool().to_mlir_type_string().as_str(), "i1");
    }

    #[test]
    fn test_list_type() {
        let list = ListType::new(VerumType::i64());
        assert_eq!(list.to_mlir_type_string().as_str(), "!llvm.ptr");
    }

    #[test]
    fn test_ref_type() {
        let managed = RefType::managed(VerumType::i64(), false);
        assert!(managed.to_mlir_type_string().contains("struct"));

        let checked = RefType::checked(VerumType::i64(), false);
        assert_eq!(checked.to_mlir_type_string().as_str(), "!llvm.ptr");
    }
}
