//! Attribute target definitions for the Verum AST.
//!
//! This module defines [`AttributeTarget`], a bitflag type that specifies
//! which syntactic positions an attribute can be applied to.
//!
//! # Overview
//!
//! Verum supports attributes on many syntactic elements:
//!
//! ```verum
//! @profile(application)                    // Module
//! module server {
//!     @derive(Clone, Serialize)            // Type
//!     type User is {
//!         @serialize(rename = "user_id")   // Field
//!         @validate(min = 1)
//!         id: Int,
//!     };
//!
//!     @inline(always)                       // Function
//!     fn process(
//!         @unused _ctx: &Context,           // Parameter
//!     ) -> Result<User> {
//!         match result {
//!             @cold Err(e) => handle(e),    // Match arm
//!             Ok(v) => v,
//!         }
//!     }
//! }
//! ```
//!
//! # Design
//!
//! `AttributeTarget` uses bitflags to allow efficient combination and checking
//! of valid targets. Common combinations are provided as constants.
//!
//! # Target Validation
//!
//! Each registered attribute declares which syntactic positions it can appear on
//! using `AttributeTarget` bitflags. The compiler validates at parse time that
//! attributes only appear on their declared targets (e.g., @inline only on functions,
//! @derive only on types, @serialize on fields, etc.).

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use verum_common::Text;

bitflags! {
    /// Bitflags representing valid attribute targets.
    ///
    /// Multiple targets can be combined using bitwise OR:
    /// ```rust
    /// use verum_ast::attr::AttributeTarget;
    ///
    /// let targets = AttributeTarget::Function | AttributeTarget::Type;
    /// assert!(targets.contains(AttributeTarget::Function));
    /// ```
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[repr(transparent)]
    pub struct AttributeTarget: u16 {
        // =================================================================
        // ITEM TARGETS (top-level declarations)
        // =================================================================

        /// Functions: `@inline fn foo() { }`
        const Function      = 1 << 0;

        /// Type definitions: `@derive(Clone) type Point is { ... }`
        const Type          = 1 << 1;

        /// Modules: `@profile(systems) module kernel { }`
        const Module        = 1 << 2;

        /// Implementation blocks: `@specialize implement Trait for T { }`
        const Impl          = 1 << 3;

        /// Constants: `@deprecated const OLD_VALUE = 42`
        const Const         = 1 << 4;

        /// Statics: `@used static BUFFER: [u8; 1024] = [0; 1024]`
        const Static        = 1 << 5;

        /// Context definitions: `@std(io) context FileSystem { }`
        const Context       = 1 << 6;

        /// Protocol definitions: `@marker type Iterator is protocol { }`
        const Protocol      = 1 << 7;

        // =================================================================
        // MEMBER TARGETS (inside type definitions)
        // =================================================================

        /// Record fields: `type T is { @skip_serialize cache: Cache }`
        const Field         = 1 << 8;

        /// Sum type variants: `type Status is @default Ok | Error`
        const Variant       = 1 << 9;

        // =================================================================
        // CODE ELEMENT TARGETS
        // =================================================================

        /// Function parameters: `fn f(@unused x: Int) { }`
        const Param         = 1 << 10;

        /// Match arms: `match x { @cold Err(e) => ... }`
        const MatchArm      = 1 << 11;

        /// Loop expressions: `@parallel for x in xs { }`
        const Loop          = 1 << 12;

        /// Block expressions: `@optimize_barrier { ... }`
        const Expr          = 1 << 13;

        /// Statements: `@assume(x > 0) let y = compute(x)`
        const Stmt          = 1 << 14;

        // =================================================================
        // COMMON COMBINATIONS
        // =================================================================

        /// All item-level targets (top-level declarations)
        const Item = Self::Function.bits()
                   | Self::Type.bits()
                   | Self::Module.bits()
                   | Self::Impl.bits()
                   | Self::Const.bits()
                   | Self::Static.bits()
                   | Self::Context.bits()
                   | Self::Protocol.bits();

        /// Type member targets (fields and variants)
        const TypeMember = Self::Field.bits() | Self::Variant.bits();

        /// Callable targets (functions and closures)
        const Callable = Self::Function.bits();

        /// Value-producing targets (expressions)
        const ValueProducing = Self::Expr.bits() | Self::Loop.bits();

        /// All possible targets
        const All = 0b0111_1111_1111_1111;
    }
}

// Manual Serialize/Deserialize implementations for bitflags
impl Serialize for AttributeTarget {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.bits().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AttributeTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bits = u16::deserialize(deserializer)?;
        Self::from_bits(bits).ok_or_else(|| {
            serde::de::Error::custom(format!("invalid AttributeTarget bits: {:#06x}", bits))
        })
    }
}

impl AttributeTarget {
    /// Get a human-readable name for this target.
    ///
    /// For single targets, returns the specific name.
    /// For combinations, returns a generic description.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_ast::attr::AttributeTarget;
    ///
    /// assert_eq!(AttributeTarget::Function.display_name(), "function");
    /// assert_eq!(AttributeTarget::Field.display_name(), "field");
    /// ```
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match *self {
            Self::Function => "function",
            Self::Type => "type",
            Self::Module => "module",
            Self::Impl => "implementation block",
            Self::Const => "constant",
            Self::Static => "static",
            Self::Context => "context",
            Self::Protocol => "protocol",
            Self::Field => "field",
            Self::Variant => "variant",
            Self::Param => "parameter",
            Self::MatchArm => "match arm",
            Self::Loop => "loop",
            Self::Expr => "expression",
            Self::Stmt => "statement",
            _ => "item", // For combinations
        }
    }

    /// Format as a human-readable list for error messages.
    ///
    /// Produces output like "function, type, or field" for use in diagnostics.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_ast::attr::AttributeTarget;
    ///
    /// let targets = AttributeTarget::Function | AttributeTarget::Type | AttributeTarget::Field;
    /// assert_eq!(targets.format_list(), "function, type, or field");
    /// ```
    #[must_use]
    pub fn format_list(&self) -> Text {
        let mut parts: Vec<&'static str> = Vec::new();

        // Collect all individual targets that are set
        if self.contains(Self::Function) {
            parts.push("function");
        }
        if self.contains(Self::Type) {
            parts.push("type");
        }
        if self.contains(Self::Module) {
            parts.push("module");
        }
        if self.contains(Self::Impl) {
            parts.push("implementation block");
        }
        if self.contains(Self::Const) {
            parts.push("constant");
        }
        if self.contains(Self::Static) {
            parts.push("static");
        }
        if self.contains(Self::Context) {
            parts.push("context");
        }
        if self.contains(Self::Protocol) {
            parts.push("protocol");
        }
        if self.contains(Self::Field) {
            parts.push("field");
        }
        if self.contains(Self::Variant) {
            parts.push("variant");
        }
        if self.contains(Self::Param) {
            parts.push("parameter");
        }
        if self.contains(Self::MatchArm) {
            parts.push("match arm");
        }
        if self.contains(Self::Loop) {
            parts.push("loop");
        }
        if self.contains(Self::Expr) {
            parts.push("expression");
        }
        if self.contains(Self::Stmt) {
            parts.push("statement");
        }

        format_list_with_or(&parts)
    }

    /// Check if this target represents exactly one syntactic position.
    ///
    /// Returns `true` if exactly one bit is set.
    #[must_use]
    pub const fn is_single(&self) -> bool {
        self.bits().count_ones() == 1
    }

    /// Check if this target represents multiple syntactic positions.
    ///
    /// Returns `true` if more than one bit is set.
    #[must_use]
    pub const fn is_multiple(&self) -> bool {
        self.bits().count_ones() > 1
    }

    /// Get the number of individual targets in this combination.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.bits().count_ones()
    }

    /// Iterate over individual targets in this combination.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_ast::attr::AttributeTarget;
    ///
    /// let targets = AttributeTarget::Function | AttributeTarget::Type;
    /// let individual: Vec<_> = targets.iter_individual().collect();
    /// assert_eq!(individual.len(), 2);
    /// ```
    pub fn iter_individual(self) -> impl Iterator<Item = AttributeTarget> {
        AttributeTargetIter {
            remaining: self,
            current_bit: 0,
        }
    }
}

impl Default for AttributeTarget {
    /// Default to no targets (empty set).
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Display for AttributeTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.format_list())
    }
}

/// Iterator over individual targets in an `AttributeTarget` combination.
struct AttributeTargetIter {
    remaining: AttributeTarget,
    current_bit: u8,
}

impl Iterator for AttributeTargetIter {
    type Item = AttributeTarget;

    fn next(&mut self) -> Option<Self::Item> {
        while self.current_bit < 16 {
            let bit = 1u16 << self.current_bit;
            self.current_bit += 1;

            if self.remaining.bits() & bit != 0 {
                return AttributeTarget::from_bits(bit);
            }
        }
        None
    }
}

/// Format a list of items with proper English grammar.
///
/// - Empty: ""
/// - One: "item"
/// - Two: "item1 or item2"
/// - Three+: "item1, item2, or item3"
fn format_list_with_or(items: &[&str]) -> Text {
    match items.len() {
        0 => Text::new(),
        1 => Text::from(items[0]),
        2 => Text::from(format!("{} or {}", items[0], items[1])),
        _ => {
            let (last, rest) = items.split_last().unwrap();
            Text::from(format!("{}, or {}", rest.join(", "), last))
        }
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_target() {
        assert!(AttributeTarget::Function.is_single());
        assert!(AttributeTarget::Field.is_single());
        assert!(!AttributeTarget::Item.is_single());
    }

    #[test]
    fn test_display_name() {
        assert_eq!(AttributeTarget::Function.display_name(), "function");
        assert_eq!(AttributeTarget::Field.display_name(), "field");
        assert_eq!(AttributeTarget::MatchArm.display_name(), "match arm");
    }

    #[test]
    fn test_format_list() {
        let single = AttributeTarget::Function;
        assert_eq!(single.format_list().as_str(), "function");

        let two = AttributeTarget::Function | AttributeTarget::Type;
        assert_eq!(two.format_list().as_str(), "function or type");

        let three = AttributeTarget::Function | AttributeTarget::Type | AttributeTarget::Field;
        assert_eq!(three.format_list().as_str(), "function, type, or field");
    }

    #[test]
    fn test_contains() {
        let targets = AttributeTarget::Function | AttributeTarget::Type;
        assert!(targets.contains(AttributeTarget::Function));
        assert!(targets.contains(AttributeTarget::Type));
        assert!(!targets.contains(AttributeTarget::Field));
    }

    #[test]
    fn test_item_combination() {
        assert!(AttributeTarget::Item.contains(AttributeTarget::Function));
        assert!(AttributeTarget::Item.contains(AttributeTarget::Type));
        assert!(AttributeTarget::Item.contains(AttributeTarget::Module));
        assert!(!AttributeTarget::Item.contains(AttributeTarget::Field));
    }

    #[test]
    fn test_iter_individual() {
        let targets = AttributeTarget::Function | AttributeTarget::Type | AttributeTarget::Field;
        let individual: Vec<_> = targets.iter_individual().collect();

        assert_eq!(individual.len(), 3);
        assert!(individual.contains(&AttributeTarget::Function));
        assert!(individual.contains(&AttributeTarget::Type));
        assert!(individual.contains(&AttributeTarget::Field));
    }

    #[test]
    fn test_serialization() {
        let target = AttributeTarget::Function | AttributeTarget::Type;
        let json = serde_json::to_string(&target).unwrap();
        let restored: AttributeTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(target, restored);
    }
}
