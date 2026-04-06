//! Unified Hashing Infrastructure for the Verum Compiler
//!
//! This module provides Blake3-based hashing utilities used throughout the compiler.
//! Blake3 is chosen for its:
//! - Extreme speed (3-10x faster than SHA-256)
//! - High security (256-bit output, resistant to known attacks)
//! - Parallelizable design (SIMD + multi-threaded)
//! - Incremental hashing support
//!
//! # Usage
//!
//! ```rust
//! use verum_compiler::hash::{ContentHash, hash_bytes, hash_str};
//!
//! // Simple hashing
//! let hash = hash_bytes(b"hello world");
//! let hash = hash_str("hello world");
//!
//! // Incremental hashing
//! let mut hasher = ContentHash::new();
//! hasher.update(b"part1");
//! hasher.update(b"part2");
//! let hash = hasher.finalize();
//! ```
//!
//! # Migration from SHA-256
//!
//! This module replaces SHA-256 usage in:
//! - Incremental compilation hashes
//! - Cache keys
//! - Content-addressable storage
//! - Module fingerprints
//!
//! Note: AWS signing (distributed_cache.rs) must continue using SHA-256
//! as required by the AWS Signature V4 protocol.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// A 256-bit (32-byte) Blake3 hash value.
///
/// This is the standard hash output used throughout the Verum compiler
/// for content-addressable storage, cache keys, and fingerprinting.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct HashValue([u8; 32]);

impl HashValue {
    /// The size of a hash value in bytes.
    pub const SIZE: usize = 32;

    /// A zero hash value (used as placeholder).
    pub const ZERO: Self = Self([0u8; 32]);

    /// Create a hash value from raw bytes.
    ///
    /// # Panics
    ///
    /// Panics if the slice is not exactly 32 bytes.
    pub fn from_slice(slice: &[u8]) -> Self {
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(slice);
        Self(bytes)
    }

    /// Create a hash value from a fixed-size array.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Get the raw bytes of the hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Get the hash as a hexadecimal string.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parse a hash from a hexadecimal string.
    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            return Err(hex::FromHexError::InvalidStringLength);
        }
        Ok(Self::from_slice(&bytes))
    }

    /// Get a shortened display version (first 8 hex chars).
    pub fn short(&self) -> String {
        hex::encode(&self.0[..4])
    }

    /// Check if this is the zero hash.
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }

    /// Convert the hash to a u64 by taking the first 8 bytes.
    ///
    /// This is useful for compatibility with APIs that expect u64 hashes,
    /// though it loses collision resistance. Use the full HashValue where
    /// possible.
    pub fn to_u64(&self) -> u64 {
        u64::from_le_bytes(self.0[..8].try_into().unwrap())
    }
}

impl fmt::Debug for HashValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.short())
    }
}

impl fmt::Display for HashValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Default for HashValue {
    fn default() -> Self {
        Self::ZERO
    }
}

impl From<[u8; 32]> for HashValue {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<HashValue> for [u8; 32] {
    fn from(hash: HashValue) -> Self {
        hash.0
    }
}

impl AsRef<[u8]> for HashValue {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for HashValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for HashValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

// ============================================================================
// Content Hasher
// ============================================================================

/// Incremental hasher for computing Blake3 hashes.
///
/// This provides a simple API for incrementally hashing content.
/// It wraps Blake3's Hasher with a convenient interface.
///
/// # Example
///
/// ```rust
/// use verum_compiler::hash::ContentHash;
///
/// let mut hasher = ContentHash::new();
/// hasher.update(b"hello");
/// hasher.update(b" ");
/// hasher.update(b"world");
/// let hash = hasher.finalize();
/// ```
pub struct ContentHash {
    hasher: blake3::Hasher,
}

impl ContentHash {
    /// Create a new hasher.
    pub fn new() -> Self {
        Self {
            hasher: blake3::Hasher::new(),
        }
    }

    /// Update the hasher with bytes.
    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.hasher.update(data);
        self
    }

    /// Update the hasher with a string.
    pub fn update_str(&mut self, s: &str) -> &mut Self {
        self.hasher.update(s.as_bytes());
        self
    }

    /// Update the hasher with any hashable value.
    pub fn update_hashable<T: Hash>(&mut self, value: &T) -> &mut Self {
        let mut std_hasher = HashableAdapter(&mut self.hasher);
        value.hash(&mut std_hasher);
        self
    }

    /// Finalize and return the hash value.
    pub fn finalize(self) -> HashValue {
        HashValue::from_bytes(*self.hasher.finalize().as_bytes())
    }

    /// Finalize and return the hash as a hex string.
    pub fn finalize_hex(self) -> String {
        self.finalize().to_hex()
    }

    /// Reset the hasher for reuse.
    pub fn reset(&mut self) {
        self.hasher.reset();
    }
}

impl Default for ContentHash {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ContentHash {
    fn clone(&self) -> Self {
        Self {
            hasher: self.hasher.clone(),
        }
    }
}

/// Adapter to use Blake3 hasher with std::hash::Hash trait.
struct HashableAdapter<'a>(&'a mut blake3::Hasher);

impl<'a> Hasher for HashableAdapter<'a> {
    fn finish(&self) -> u64 {
        // Not used for our purposes
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Hash raw bytes and return the hash value.
#[inline]
pub fn hash_bytes(data: &[u8]) -> HashValue {
    HashValue::from_bytes(*blake3::hash(data).as_bytes())
}

/// Hash a string and return the hash value.
#[inline]
pub fn hash_str(s: &str) -> HashValue {
    hash_bytes(s.as_bytes())
}

/// Hash a file's contents and return the hash value.
///
/// Returns an error if the file cannot be read.
pub fn hash_file(path: &Path) -> std::io::Result<HashValue> {
    let data = std::fs::read(path)?;
    Ok(hash_bytes(&data))
}

/// Hash multiple items into a single hash.
///
/// This is useful for creating composite keys from multiple values.
pub fn hash_multiple<I, T>(items: I) -> HashValue
where
    I: IntoIterator<Item = T>,
    T: AsRef<[u8]>,
{
    let mut hasher = ContentHash::new();
    for item in items {
        hasher.update(item.as_ref());
    }
    hasher.finalize()
}

/// Hash a value that implements std::hash::Hash.
pub fn hash_hashable<T: Hash>(value: &T) -> HashValue {
    let mut hasher = ContentHash::new();
    hasher.update_hashable(value);
    hasher.finalize()
}

// ============================================================================
// Cache Key Generation
// ============================================================================

/// Generate a cache key from multiple components.
///
/// This creates a reproducible hash from multiple string components,
/// suitable for use as a cache key.
pub fn cache_key(components: &[&str]) -> HashValue {
    let mut hasher = ContentHash::new();
    for (i, component) in components.iter().enumerate() {
        if i > 0 {
            hasher.update(b"\x00"); // Separator
        }
        hasher.update_str(component);
    }
    hasher.finalize()
}

/// Generate a cache key from a type name and configuration.
///
/// This is useful for type-specific caches where the configuration
/// affects the cached result.
pub fn typed_cache_key<T: Hash>(type_name: &str, config: &T) -> HashValue {
    let mut hasher = ContentHash::new();
    hasher.update_str(type_name);
    hasher.update(b"\x00");
    hasher.update_hashable(config);
    hasher.finalize()
}

// ============================================================================
// Content-Addressable Storage Support
// ============================================================================

/// Compute a content-addressable storage key for source code.
///
/// This normalizes line endings and trims whitespace to ensure
/// consistent hashes across platforms.
pub fn source_hash(source: &str) -> HashValue {
    // Normalize line endings to LF
    let normalized = source.replace("\r\n", "\n");
    hash_str(&normalized)
}

/// Compute a hash for a module's signature (types, exports, etc.).
///
/// This is used for incremental compilation to detect API changes.
pub fn signature_hash(module_name: &str, exports: &[&str]) -> HashValue {
    let mut hasher = ContentHash::new();
    hasher.update_str(module_name);
    hasher.update(b"\x00");
    for export in exports {
        hasher.update_str(export);
        hasher.update(b"\x00");
    }
    hasher.finalize()
}

// ============================================================================
// Function Signature vs Body Hashing (Incremental Compilation)
// ============================================================================

/// Represents the change type for incremental compilation.
///
/// Distinguishes between signature changes (API-breaking) and body changes
/// (implementation-only), enabling more efficient recompilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// No change detected
    NoChange,
    /// Only the function body changed (implementation)
    /// Dependent modules need re-verification, not recompilation
    BodyOnly,
    /// Signature changed (API-breaking)
    /// Dependent modules need full recompilation
    Signature,
}

/// Hash data for a function, separating signature from body.
///
/// This enables fine-grained invalidation during incremental compilation:
/// - Signature change → all dependents need recompilation
/// - Body change → dependents only need re-verification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FunctionHashes {
    /// Hash of the function signature (name, params, return type, contexts, properties)
    pub signature: HashValue,
    /// Hash of the function body (bytecode, locals, registers)
    pub body: HashValue,
}

impl FunctionHashes {
    /// Create a new function hash pair.
    pub fn new(signature: HashValue, body: HashValue) -> Self {
        Self { signature, body }
    }

    /// Compare with another hash pair and determine the change kind.
    pub fn compare(&self, other: &FunctionHashes) -> ChangeKind {
        if self.signature != other.signature {
            ChangeKind::Signature
        } else if self.body != other.body {
            ChangeKind::BodyOnly
        } else {
            ChangeKind::NoChange
        }
    }

    /// Combine signature and body into a single content hash.
    pub fn combined(&self) -> HashValue {
        let mut hasher = ContentHash::new();
        hasher.update(self.signature.as_bytes());
        hasher.update(self.body.as_bytes());
        hasher.finalize()
    }
}

/// Builder for computing function hashes.
///
/// Separates signature components from body components to enable
/// fine-grained change detection.
///
/// # Example
///
/// ```rust
/// use verum_compiler::hash::FunctionHashBuilder;
///
/// let hashes = FunctionHashBuilder::new()
///     .with_name("my_function")
///     .with_param("x", "Int")
///     .with_param("y", "Float")
///     .with_return_type("Bool")
///     .with_bytecode(&[0x01, 0x02, 0x03])
///     .finish();
/// ```
pub struct FunctionHashBuilder {
    /// Hasher for signature components (name, params, return type, etc.)
    pub sig_hasher: ContentHash,
    /// Hasher for body components (bytecode, locals, etc.)
    pub body_hasher: ContentHash,
}

impl FunctionHashBuilder {
    /// Create a new function hash builder.
    pub fn new() -> Self {
        Self {
            sig_hasher: ContentHash::new(),
            body_hasher: ContentHash::new(),
        }
    }

    // ========================================================================
    // Signature Components
    // ========================================================================

    /// Add function name to signature.
    pub fn with_name(mut self, name: &str) -> Self {
        self.sig_hasher.update_str("name:");
        self.sig_hasher.update_str(name);
        self.sig_hasher.update(b"\x00");
        self
    }

    /// Add a parameter to signature.
    pub fn with_param(mut self, name: &str, type_str: &str) -> Self {
        self.sig_hasher.update_str("param:");
        self.sig_hasher.update_str(name);
        self.sig_hasher.update(b":");
        self.sig_hasher.update_str(type_str);
        self.sig_hasher.update(b"\x00");
        self
    }

    /// Add a parameter with mutability.
    pub fn with_param_mut(mut self, name: &str, type_str: &str, is_mut: bool) -> Self {
        self.sig_hasher.update_str("param:");
        self.sig_hasher.update_str(name);
        self.sig_hasher.update(b":");
        self.sig_hasher.update_str(type_str);
        self.sig_hasher.update(if is_mut { b":mut" } else { b":imm" });
        self.sig_hasher.update(b"\x00");
        self
    }

    /// Add type parameter to signature.
    pub fn with_type_param(mut self, name: &str, bounds: &[&str]) -> Self {
        self.sig_hasher.update_str("tparam:");
        self.sig_hasher.update_str(name);
        for bound in bounds {
            self.sig_hasher.update(b":");
            self.sig_hasher.update_str(bound);
        }
        self.sig_hasher.update(b"\x00");
        self
    }

    /// Add return type to signature.
    pub fn with_return_type(mut self, type_str: &str) -> Self {
        self.sig_hasher.update_str("ret:");
        self.sig_hasher.update_str(type_str);
        self.sig_hasher.update(b"\x00");
        self
    }

    /// Add context requirement to signature.
    pub fn with_context(mut self, ctx: &str) -> Self {
        self.sig_hasher.update_str("ctx:");
        self.sig_hasher.update_str(ctx);
        self.sig_hasher.update(b"\x00");
        self
    }

    /// Add computational property to signature.
    pub fn with_property(mut self, prop: &str) -> Self {
        self.sig_hasher.update_str("prop:");
        self.sig_hasher.update_str(prop);
        self.sig_hasher.update(b"\x00");
        self
    }

    /// Add visibility to signature.
    pub fn with_visibility(mut self, vis: &str) -> Self {
        self.sig_hasher.update_str("vis:");
        self.sig_hasher.update_str(vis);
        self.sig_hasher.update(b"\x00");
        self
    }

    // ========================================================================
    // Body Components
    // ========================================================================

    /// Add raw bytecode to body hash.
    pub fn with_bytecode(mut self, bytecode: &[u8]) -> Self {
        self.body_hasher.update_str("code:");
        self.body_hasher.update(bytecode);
        self.body_hasher.update(b"\x00");
        self
    }

    /// Add locals count to body hash.
    pub fn with_locals(mut self, count: u16) -> Self {
        self.body_hasher.update_str("locals:");
        self.body_hasher.update(&count.to_le_bytes());
        self.body_hasher.update(b"\x00");
        self
    }

    /// Add register count to body hash.
    pub fn with_registers(mut self, count: u16) -> Self {
        self.body_hasher.update_str("regs:");
        self.body_hasher.update(&count.to_le_bytes());
        self.body_hasher.update(b"\x00");
        self
    }

    /// Add max stack depth to body hash.
    pub fn with_max_stack(mut self, depth: u16) -> Self {
        self.body_hasher.update_str("stack:");
        self.body_hasher.update(&depth.to_le_bytes());
        self.body_hasher.update(b"\x00");
        self
    }

    /// Finish building and return the function hashes.
    pub fn finish(self) -> FunctionHashes {
        FunctionHashes {
            signature: self.sig_hasher.finalize(),
            body: self.body_hasher.finalize(),
        }
    }
}

impl Default for FunctionHashBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash data for a module item (function, type, constant).
///
/// Used for fine-grained incremental compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ItemHashes {
    /// Map from item name to its hashes
    pub functions: std::collections::HashMap<String, FunctionHashes>,
    /// Type definition hashes (signature-only, no body)
    pub types: std::collections::HashMap<String, HashValue>,
    /// Constant hashes
    pub constants: std::collections::HashMap<String, HashValue>,
}

impl ItemHashes {
    /// Create empty item hashes.
    pub fn new() -> Self {
        Self {
            functions: std::collections::HashMap::new(),
            types: std::collections::HashMap::new(),
            constants: std::collections::HashMap::new(),
        }
    }

    /// Add function hashes.
    pub fn add_function(&mut self, name: String, hashes: FunctionHashes) {
        self.functions.insert(name, hashes);
    }

    /// Add type hash.
    pub fn add_type(&mut self, name: String, hash: HashValue) {
        self.types.insert(name, hash);
    }

    /// Add constant hash.
    pub fn add_constant(&mut self, name: String, hash: HashValue) {
        self.constants.insert(name, hash);
    }

    /// Compare with another item hashes and return the overall change kind.
    ///
    /// Returns:
    /// - `NoChange` if all items are identical
    /// - `BodyOnly` if only function bodies changed
    /// - `Signature` if any signature, type, or constant changed
    pub fn compare(&self, other: &ItemHashes) -> ChangeKind {
        let mut has_body_change = false;

        // Check functions
        for (name, hashes) in &self.functions {
            match other.functions.get(name) {
                Some(other_hashes) => {
                    match hashes.compare(other_hashes) {
                        ChangeKind::Signature => return ChangeKind::Signature,
                        ChangeKind::BodyOnly => has_body_change = true,
                        ChangeKind::NoChange => {}
                    }
                }
                None => return ChangeKind::Signature, // New function added
            }
        }

        // Check for removed functions
        for name in other.functions.keys() {
            if !self.functions.contains_key(name) {
                return ChangeKind::Signature; // Function removed
            }
        }

        // Check types (any change is signature-breaking)
        if self.types != other.types {
            return ChangeKind::Signature;
        }

        // Check constants (any change is signature-breaking)
        if self.constants != other.constants {
            return ChangeKind::Signature;
        }

        if has_body_change {
            ChangeKind::BodyOnly
        } else {
            ChangeKind::NoChange
        }
    }

    /// Compute combined hash of all items.
    pub fn combined(&self) -> HashValue {
        let mut hasher = ContentHash::new();

        // Hash functions in sorted order for determinism
        let mut fn_names: Vec<_> = self.functions.keys().collect();
        fn_names.sort();
        for name in fn_names {
            let hashes = &self.functions[name];
            hasher.update_str(name);
            hasher.update(hashes.signature.as_bytes());
            hasher.update(hashes.body.as_bytes());
        }

        // Hash types in sorted order
        let mut type_names: Vec<_> = self.types.keys().collect();
        type_names.sort();
        for name in type_names {
            hasher.update_str(name);
            hasher.update(self.types[name].as_bytes());
        }

        // Hash constants in sorted order
        let mut const_names: Vec<_> = self.constants.keys().collect();
        const_names.sort();
        for name in const_names {
            hasher.update_str(name);
            hasher.update(self.constants[name].as_bytes());
        }

        hasher.finalize()
    }
}

impl Default for ItemHashes {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// AST-Based Item Hash Computation
// ============================================================================

use verum_ast::{
    decl::{
        ConstDecl, FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind, ItemKind,
        TypeDecl, TypeDeclBody,
    },
    ty::{GenericParam, GenericParamKind, Type},
    Module,
};
use verum_common::Maybe;

/// Compute item hashes for all items in a module.
///
/// This function extracts functions, types, and constants from a parsed
/// module and computes their signature and body hashes for fine-grained
/// incremental compilation.
///
/// # Example
///
/// ```ignore
/// use verum_compiler::hash::compute_item_hashes_from_module;
/// use verum_ast::Module;
///
/// let module: Module = parse("source.vr")?;
/// let hashes = compute_item_hashes_from_module(&module);
///
/// // Use hashes with IncrementalCompiler
/// incremental_compiler.update_item_hashes(path, hashes);
/// ```
pub fn compute_item_hashes_from_module(module: &Module) -> ItemHashes {
    let mut hashes = ItemHashes::new();

    for item in module.items.iter() {
        match &item.kind {
            ItemKind::Function(func) => {
                let func_hashes = compute_function_hashes(func);
                hashes.add_function(func.name.name.to_string(), func_hashes);
            }
            ItemKind::Type(type_decl) => {
                let type_hash = compute_type_hash(type_decl);
                hashes.add_type(type_decl.name.name.to_string(), type_hash);
            }
            ItemKind::Const(const_decl) => {
                let const_hash = compute_const_hash(const_decl);
                hashes.add_constant(const_decl.name.name.to_string(), const_hash);
            }
            ItemKind::Protocol(proto) => {
                // Protocols affect API, hash as type
                let proto_hash = compute_protocol_hash(proto);
                hashes.add_type(proto.name.name.to_string(), proto_hash);
            }
            ItemKind::Impl(impl_decl) => {
                // Implementation blocks affect both signature and potentially body
                let impl_hash = compute_impl_hash(impl_decl);
                let impl_name = format_impl_name(impl_decl);
                hashes.add_type(impl_name, impl_hash);
            }
            // Other item kinds (Module, Mount, Meta, etc.) don't participate
            // in fine-grained invalidation at this level
            _ => {}
        }
    }

    hashes
}

/// Compute hashes for a function declaration.
fn compute_function_hashes(func: &FunctionDecl) -> FunctionHashes {
    let mut builder = FunctionHashBuilder::new()
        .with_name(func.name.name.as_str())
        .with_visibility(func.visibility.as_str());

    // Hash function modifiers (part of signature)
    if func.is_async {
        builder = builder.with_property("async");
    }
    if func.is_meta {
        builder = builder.with_property("meta");
    }
    if func.is_pure {
        builder = builder.with_property("pure");
    }
    if func.is_unsafe {
        builder = builder.with_property("unsafe");
    }
    if func.is_generator {
        builder = builder.with_property("generator");
    }

    // Hash generic parameters (signature)
    for generic in func.generics.iter() {
        builder = hash_generic_param(builder, generic);
    }

    // Hash parameters (signature)
    for param in func.params.iter() {
        builder = hash_function_param(builder, param);
    }

    // Hash return type (signature)
    if let Maybe::Some(ref ret_ty) = func.return_type {
        builder = builder.with_return_type(&type_to_string(ret_ty));
    }

    // Hash context requirements (signature)
    for ctx in func.contexts.iter() {
        builder = builder.with_context(&ctx.path.to_string());
    }

    // Hash requires/ensures (signature - affects verification interface)
    for req in func.requires.iter() {
        builder.sig_hasher.update_str("requires:");
        builder.sig_hasher.update_str(&format!("{:?}", req));
        builder.sig_hasher.update(b"\x00");
    }
    for ens in func.ensures.iter() {
        builder.sig_hasher.update_str("ensures:");
        builder.sig_hasher.update_str(&format!("{:?}", ens));
        builder.sig_hasher.update(b"\x00");
    }

    // Hash body (implementation)
    if let Maybe::Some(ref body) = func.body {
        let body_str = match body {
            FunctionBody::Block(block) => format!("{:?}", block),
            FunctionBody::Expr(expr) => format!("{:?}", expr),
        };
        builder = builder.with_bytecode(body_str.as_bytes());
    }

    builder.finish()
}

/// Hash a generic parameter into the signature hasher.
fn hash_generic_param(builder: FunctionHashBuilder, param: &GenericParam) -> FunctionHashBuilder {
    match &param.kind {
        GenericParamKind::Type { name, bounds, default: _ } => {
            let bounds_strs: Vec<String> = bounds.iter().map(|b| format!("{:?}", b)).collect();
            let bounds_refs: Vec<&str> = bounds_strs.iter().map(|s| s.as_str()).collect();
            builder.with_type_param(name.name.as_str(), &bounds_refs)
        }
        GenericParamKind::HigherKinded { name, arity, bounds } => {
            let mut bounds_strs: Vec<String> = bounds.iter().map(|b| format!("{:?}", b)).collect();
            bounds_strs.push(format!("arity:{}", arity));
            let bounds_refs: Vec<&str> = bounds_strs.iter().map(|s| s.as_str()).collect();
            builder.with_type_param(name.name.as_str(), &bounds_refs)
        }
        GenericParamKind::Const { name, ty } => {
            builder.with_type_param(name.name.as_str(), &[&format!("const:{:?}", ty)])
        }
        GenericParamKind::Meta { name, ty, refinement } => {
            let refinement_str = match refinement {
                Maybe::Some(r) => format!("meta:{:?} where {:?}", ty, r),
                Maybe::None => format!("meta:{:?}", ty),
            };
            builder.with_type_param(name.name.as_str(), &[&refinement_str])
        }
        GenericParamKind::Lifetime { name } => {
            builder.with_type_param(name.name.as_str(), &["lifetime"])
        }
        GenericParamKind::Context { name } => {
            builder.with_type_param(name.name.as_str(), &["context"])
        }
        GenericParamKind::Level { name, .. } => {
            builder.with_type_param(name.name.as_str(), &["level"])
        }
    }
}

/// Hash a generic parameter into a content hasher.
/// Returns a string representation of the generic parameter for hashing.
fn generic_param_to_string(param: &GenericParam) -> String {
    match &param.kind {
        GenericParamKind::Type { name, bounds, default: _ } => {
            let bounds_str: String = bounds.iter().map(|b| format!("{:?}", b)).collect::<Vec<_>>().join("+");
            if bounds_str.is_empty() {
                name.name.to_string()
            } else {
                format!("{}: {}", name.name, bounds_str)
            }
        }
        GenericParamKind::HigherKinded { name, arity, bounds } => {
            let bounds_str: String = bounds.iter().map(|b| format!("{:?}", b)).collect::<Vec<_>>().join("+");
            format!("{}<{}>{}", name.name, "_,".repeat(*arity).trim_end_matches(','),
                    if bounds_str.is_empty() { String::new() } else { format!(": {}", bounds_str) })
        }
        GenericParamKind::Const { name, ty } => {
            format!("const {}: {:?}", name.name, ty)
        }
        GenericParamKind::Meta { name, ty, refinement } => {
            match refinement {
                Maybe::Some(r) => format!("{}: meta {:?} where {:?}", name.name, ty, r),
                Maybe::None => format!("{}: meta {:?}", name.name, ty),
            }
        }
        GenericParamKind::Lifetime { name } => {
            format!("'{}", name.name)
        }
        GenericParamKind::Context { name } => {
            format!("using {}", name.name)
        }
        GenericParamKind::Level { name, .. } => {
            format!("{}: Level", name.name)
        }
    }
}

/// Hash a function parameter into the signature hasher.
fn hash_function_param(builder: FunctionHashBuilder, param: &FunctionParam) -> FunctionHashBuilder {
    match &param.kind {
        FunctionParamKind::Regular { pattern, ty, default_value } => {
            let is_mut = false; // Regular params determine mutability from type
            let type_str = type_to_string(ty);
            let name = format!("{:?}", pattern);
            let mut b = builder.with_param_mut(&name, &type_str, is_mut);
            if let Maybe::Some(_) = default_value {
                b.sig_hasher.update_str("has_default");
                b.sig_hasher.update(b"\x00");
            }
            b
        }
        FunctionParamKind::SelfValue => builder.with_param("self", "Self"),
        FunctionParamKind::SelfValueMut => builder.with_param_mut("self", "Self", true),
        FunctionParamKind::SelfRef => builder.with_param("self", "&Self"),
        FunctionParamKind::SelfRefMut => builder.with_param_mut("self", "&mut Self", true),
        FunctionParamKind::SelfRefChecked => builder.with_param("self", "&checked Self"),
        FunctionParamKind::SelfRefCheckedMut => builder.with_param_mut("self", "&checked mut Self", true),
        FunctionParamKind::SelfRefUnsafe => builder.with_param("self", "&unsafe Self"),
        FunctionParamKind::SelfRefUnsafeMut => builder.with_param_mut("self", "&unsafe mut Self", true),
        FunctionParamKind::SelfOwn => builder.with_param("self", "%Self"),
        FunctionParamKind::SelfOwnMut => builder.with_param_mut("self", "%mut Self", true),
    }
}

/// Convert a type to a canonical string representation for hashing.
fn type_to_string(ty: &Type) -> String {
    // Use Debug representation for deterministic output
    // In a production system, we'd implement a proper canonical formatter
    format!("{:?}", ty.kind)
}

/// Compute hash for a type declaration.
fn compute_type_hash(type_decl: &TypeDecl) -> HashValue {
    let mut hasher = ContentHash::new();

    hasher.update_str("type:");
    hasher.update_str(type_decl.name.name.as_str());
    hasher.update(b"\x00");

    hasher.update_str("vis:");
    hasher.update_str(type_decl.visibility.as_str());
    hasher.update(b"\x00");

    // Hash generics
    for generic in type_decl.generics.iter() {
        hasher.update_str("generic:");
        hasher.update_str(&generic_param_to_string(generic));
        hasher.update(b"\x00");
    }

    // Hash resource modifier if present
    if let Maybe::Some(ref modifier) = type_decl.resource_modifier {
        hasher.update_str("resource:");
        hasher.update_str(modifier.as_str());
        hasher.update(b"\x00");
    }

    // Hash body
    hasher.update_str("body:");
    match &type_decl.body {
        TypeDeclBody::Alias(ty) => {
            hasher.update_str("alias:");
            hasher.update_str(&format!("{:?}", ty));
        }
        TypeDeclBody::Record(fields) => {
            hasher.update_str("record:");
            for field in fields.iter() {
                hasher.update_str(field.name.name.as_ref());
                hasher.update(b":");
                hasher.update_str(&format!("{:?}", field.ty));
                hasher.update(b"\x00");
            }
        }
        TypeDeclBody::Variant(variants) => {
            hasher.update_str("variant:");
            for variant in variants.iter() {
                hasher.update_str(variant.name.name.as_ref());
                hasher.update_str(&format!("{:?}", variant.data));
                hasher.update(b"\x00");
            }
        }
        TypeDeclBody::Protocol(proto_body) => {
            hasher.update_str("protocol:");
            hasher.update_str(&format!("{:?}", proto_body));
        }
        TypeDeclBody::Newtype(ty) => {
            hasher.update_str("newtype:");
            hasher.update_str(&format!("{:?}", ty));
        }
        TypeDeclBody::Tuple(types) => {
            hasher.update_str("tuple:");
            for ty in types.iter() {
                hasher.update_str(&format!("{:?}", ty));
                hasher.update(b"\x00");
            }
        }
        TypeDeclBody::SigmaTuple(types) => {
            hasher.update_str("sigma:");
            for ty in types.iter() {
                hasher.update_str(&format!("{:?}", ty));
                hasher.update(b"\x00");
            }
        }
        TypeDeclBody::Unit => {
            hasher.update_str("unit");
        }
        TypeDeclBody::Inductive(variants) => {
            hasher.update_str("inductive:");
            for variant in variants.iter() {
                hasher.update_str(variant.name.name.as_ref());
                hasher.update_str(&format!("{:?}", variant.data));
                hasher.update(b"\x00");
            }
        }
        TypeDeclBody::Coinductive(proto_body) => {
            hasher.update_str("coinductive:");
            hasher.update_str(&format!("{:?}", proto_body));
        }
    }

    hasher.finalize()
}

/// Compute hash for a const declaration.
fn compute_const_hash(const_decl: &ConstDecl) -> HashValue {
    let mut hasher = ContentHash::new();

    hasher.update_str("const:");
    hasher.update_str(const_decl.name.name.as_str());
    hasher.update(b"\x00");

    hasher.update_str("vis:");
    hasher.update_str(const_decl.visibility.as_str());
    hasher.update(b"\x00");

    hasher.update_str("type:");
    hasher.update_str(&format!("{:?}", const_decl.ty));
    hasher.update(b"\x00");

    hasher.update_str("value:");
    hasher.update_str(&format!("{:?}", const_decl.value));
    hasher.update(b"\x00");

    hasher.finalize()
}

/// Compute hash for a protocol declaration.
fn compute_protocol_hash(proto: &verum_ast::decl::ProtocolDecl) -> HashValue {
    let mut hasher = ContentHash::new();

    hasher.update_str("protocol:");
    hasher.update_str(proto.name.name.as_str());
    hasher.update(b"\x00");

    hasher.update_str("vis:");
    hasher.update_str(proto.visibility.as_str());
    hasher.update(b"\x00");

    // Hash generics
    for generic in proto.generics.iter() {
        hasher.update_str("generic:");
        hasher.update_str(&generic_param_to_string(generic));
        hasher.update(b"\x00");
    }

    // Hash bounds
    for bound in proto.bounds.iter() {
        hasher.update_str("bound:");
        hasher.update_str(&format!("{:?}", bound));
        hasher.update(b"\x00");
    }

    // Hash items
    for item in proto.items.iter() {
        hasher.update_str("item:");
        hasher.update_str(&format!("{:?}", item));
        hasher.update(b"\x00");
    }

    hasher.finalize()
}

/// Compute hash for an impl declaration.
fn compute_impl_hash(impl_decl: &verum_ast::decl::ImplDecl) -> HashValue {
    let mut hasher = ContentHash::new();

    hasher.update_str("impl:");
    hasher.update_str(&format!("{:?}", impl_decl.kind));
    hasher.update(b"\x00");

    // Hash generics
    for generic in impl_decl.generics.iter() {
        hasher.update_str("generic:");
        hasher.update_str(&generic_param_to_string(generic));
        hasher.update(b"\x00");
    }

    // Hash items
    for item in impl_decl.items.iter() {
        hasher.update_str("item:");
        hasher.update_str(&format!("{:?}", item));
        hasher.update(b"\x00");
    }

    hasher.finalize()
}

/// Format an impl block name for hash storage.
fn format_impl_name(impl_decl: &verum_ast::decl::ImplDecl) -> String {
    match &impl_decl.kind {
        verum_ast::decl::ImplKind::Inherent(ty) => format!("impl_{:?}", ty),
        verum_ast::decl::ImplKind::Protocol { protocol, for_type, .. } => {
            format!("impl_{}_{:?}", protocol, for_type)
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_bytes() {
        let hash = hash_bytes(b"hello world");
        assert!(!hash.is_zero());
        assert_eq!(hash.as_bytes().len(), 32);
    }

    #[test]
    fn test_hash_str() {
        let hash1 = hash_str("hello world");
        let hash2 = hash_bytes(b"hello world");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_deterministic() {
        let hash1 = hash_str("test");
        let hash2 = hash_str("test");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_different_inputs() {
        let hash1 = hash_str("hello");
        let hash2 = hash_str("world");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_incremental_hash() {
        let mut hasher = ContentHash::new();
        hasher.update(b"hello ");
        hasher.update(b"world");
        let incremental = hasher.finalize();

        let direct = hash_str("hello world");
        assert_eq!(incremental, direct);
    }

    #[test]
    fn test_hash_value_hex() {
        let hash = hash_str("test");
        let hex = hash.to_hex();
        let parsed = HashValue::from_hex(&hex).unwrap();
        assert_eq!(hash, parsed);
    }

    #[test]
    fn test_hash_value_short() {
        let hash = hash_str("test");
        let short = hash.short();
        assert_eq!(short.len(), 8); // 4 bytes = 8 hex chars
    }

    #[test]
    fn test_cache_key() {
        let key1 = cache_key(&["module", "function", "v1"]);
        let key2 = cache_key(&["module", "function", "v1"]);
        let key3 = cache_key(&["module", "function", "v2"]);

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_cache_key_separator() {
        // Ensure "a", "bc" produces different hash than "ab", "c"
        let key1 = cache_key(&["a", "bc"]);
        let key2 = cache_key(&["ab", "c"]);
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_source_hash_normalization() {
        let unix = "line1\nline2";
        let windows = "line1\r\nline2";

        let hash_unix = source_hash(unix);
        let hash_windows = source_hash(windows);

        assert_eq!(hash_unix, hash_windows);
    }

    #[test]
    fn test_hash_multiple() {
        let items = vec!["hello", "world"];
        let hash = hash_multiple(items.iter().map(|s| s.as_bytes()));
        assert!(!hash.is_zero());
    }

    #[test]
    fn test_signature_hash() {
        let sig1 = signature_hash("my_module", &["fn_a", "fn_b"]);
        let sig2 = signature_hash("my_module", &["fn_a", "fn_b"]);
        let sig3 = signature_hash("my_module", &["fn_a", "fn_c"]);

        assert_eq!(sig1, sig2);
        assert_ne!(sig1, sig3);
    }

    #[test]
    fn test_hash_value_zero() {
        let zero = HashValue::ZERO;
        assert!(zero.is_zero());
        assert_eq!(zero.to_hex(), "0".repeat(64));
    }

    #[test]
    fn test_hash_value_default() {
        let default: HashValue = Default::default();
        assert!(default.is_zero());
    }

    // ========================================================================
    // Function Hash Tests (Signature vs Body Invalidation)
    // ========================================================================

    #[test]
    fn test_function_hash_builder_basic() {
        let hashes = FunctionHashBuilder::new()
            .with_name("my_func")
            .with_param("x", "Int")
            .with_return_type("Bool")
            .with_bytecode(&[0x01, 0x02, 0x03])
            .finish();

        assert!(!hashes.signature.is_zero());
        assert!(!hashes.body.is_zero());
        assert_ne!(hashes.signature, hashes.body);
    }

    #[test]
    fn test_function_hash_signature_change() {
        let hashes1 = FunctionHashBuilder::new()
            .with_name("my_func")
            .with_param("x", "Int")
            .with_return_type("Bool")
            .with_bytecode(&[0x01, 0x02, 0x03])
            .finish();

        // Same body, different signature (return type changed)
        let hashes2 = FunctionHashBuilder::new()
            .with_name("my_func")
            .with_param("x", "Int")
            .with_return_type("Int") // Changed!
            .with_bytecode(&[0x01, 0x02, 0x03])
            .finish();

        assert_eq!(hashes1.compare(&hashes2), ChangeKind::Signature);
    }

    #[test]
    fn test_function_hash_body_change() {
        let hashes1 = FunctionHashBuilder::new()
            .with_name("my_func")
            .with_param("x", "Int")
            .with_return_type("Bool")
            .with_bytecode(&[0x01, 0x02, 0x03])
            .finish();

        // Same signature, different body
        let hashes2 = FunctionHashBuilder::new()
            .with_name("my_func")
            .with_param("x", "Int")
            .with_return_type("Bool")
            .with_bytecode(&[0x01, 0x02, 0x04]) // Changed!
            .finish();

        assert_eq!(hashes1.compare(&hashes2), ChangeKind::BodyOnly);
    }

    #[test]
    fn test_function_hash_no_change() {
        let hashes1 = FunctionHashBuilder::new()
            .with_name("my_func")
            .with_param("x", "Int")
            .with_return_type("Bool")
            .with_bytecode(&[0x01, 0x02, 0x03])
            .finish();

        let hashes2 = FunctionHashBuilder::new()
            .with_name("my_func")
            .with_param("x", "Int")
            .with_return_type("Bool")
            .with_bytecode(&[0x01, 0x02, 0x03])
            .finish();

        assert_eq!(hashes1.compare(&hashes2), ChangeKind::NoChange);
    }

    #[test]
    fn test_function_hash_param_mutability() {
        let hashes1 = FunctionHashBuilder::new()
            .with_name("my_func")
            .with_param_mut("x", "Int", false)
            .finish();

        let hashes2 = FunctionHashBuilder::new()
            .with_name("my_func")
            .with_param_mut("x", "Int", true) // Mutability changed!
            .finish();

        assert_eq!(hashes1.compare(&hashes2), ChangeKind::Signature);
    }

    #[test]
    fn test_item_hashes_module_comparison() {
        let mut items1 = ItemHashes::new();
        items1.add_function(
            "func_a".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_a")
                .with_return_type("Int")
                .with_bytecode(&[0x01])
                .finish(),
        );
        items1.add_function(
            "func_b".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_b")
                .with_return_type("Bool")
                .with_bytecode(&[0x02])
                .finish(),
        );

        // Same module, no changes
        let mut items2 = ItemHashes::new();
        items2.add_function(
            "func_a".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_a")
                .with_return_type("Int")
                .with_bytecode(&[0x01])
                .finish(),
        );
        items2.add_function(
            "func_b".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_b")
                .with_return_type("Bool")
                .with_bytecode(&[0x02])
                .finish(),
        );

        assert_eq!(items1.compare(&items2), ChangeKind::NoChange);
    }

    #[test]
    fn test_item_hashes_body_only_change() {
        let mut items1 = ItemHashes::new();
        items1.add_function(
            "func_a".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_a")
                .with_return_type("Int")
                .with_bytecode(&[0x01])
                .finish(),
        );

        let mut items2 = ItemHashes::new();
        items2.add_function(
            "func_a".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_a")
                .with_return_type("Int")
                .with_bytecode(&[0x02]) // Body changed!
                .finish(),
        );

        assert_eq!(items1.compare(&items2), ChangeKind::BodyOnly);
    }

    #[test]
    fn test_item_hashes_signature_change() {
        let mut items1 = ItemHashes::new();
        items1.add_function(
            "func_a".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_a")
                .with_return_type("Int")
                .with_bytecode(&[0x01])
                .finish(),
        );

        let mut items2 = ItemHashes::new();
        items2.add_function(
            "func_a".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_a")
                .with_return_type("Bool") // Signature changed!
                .with_bytecode(&[0x01])
                .finish(),
        );

        assert_eq!(items1.compare(&items2), ChangeKind::Signature);
    }

    #[test]
    fn test_item_hashes_function_added() {
        let mut items1 = ItemHashes::new();
        items1.add_function(
            "func_a".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_a")
                .finish(),
        );
        items1.add_function(
            "func_b".to_string(), // New function!
            FunctionHashBuilder::new()
                .with_name("func_b")
                .finish(),
        );

        let mut items2 = ItemHashes::new();
        items2.add_function(
            "func_a".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_a")
                .finish(),
        );

        assert_eq!(items1.compare(&items2), ChangeKind::Signature);
    }

    #[test]
    fn test_item_hashes_type_change() {
        let mut items1 = ItemHashes::new();
        items1.add_type("MyType".to_string(), hash_str("type_def_v1"));

        let mut items2 = ItemHashes::new();
        items2.add_type("MyType".to_string(), hash_str("type_def_v2")); // Changed!

        assert_eq!(items1.compare(&items2), ChangeKind::Signature);
    }

    #[test]
    fn test_item_hashes_combined() {
        let mut items = ItemHashes::new();
        items.add_function(
            "func_a".to_string(),
            FunctionHashBuilder::new()
                .with_name("func_a")
                .finish(),
        );
        items.add_type("MyType".to_string(), hash_str("type_def"));

        let combined = items.combined();
        assert!(!combined.is_zero());

        // Combined hash should be deterministic
        let combined2 = items.combined();
        assert_eq!(combined, combined2);
    }
}
