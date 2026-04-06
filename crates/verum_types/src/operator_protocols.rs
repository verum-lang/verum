//! Operator Protocol System
//!
//! Maps operators to their corresponding protocols for stdlib-agnostic resolution.
//!
//! ## Architecture
//!
//! Instead of hardcoding operator behavior for specific types (like Int, Float),
//! the type system resolves operators through protocol implementations:
//!
//! ```verum
//! // User code
//! let sum = a + b;
//!
//! // Type checker resolves:
//! // 1. Get protocol for `+` operator -> "Add"
//! // 2. Check if type of `a` implements `Add`
//! // 3. Resolve method signature from Add.add
//! // 4. Type check arguments and return type
//! ```
//!
//! ## Protocol Mappings
//!
//! | Operator | Protocol | Method |
//! |----------|----------|--------|
//! | `+`      | Add      | add    |
//! | `-`      | Sub      | sub    |
//! | `*`      | Mul      | mul    |
//! | `/`      | Div      | div    |
//! | `%`      | Rem      | rem    |
//! | `==`     | Eq       | eq     |
//! | `!=`     | Eq       | ne     |
//! | `<`      | Ord      | lt     |
//! | `<=`     | Ord      | le     |
//! | `>`      | Ord      | gt     |
//! | `>=`     | Ord      | ge     |
//! | `&`      | BitAnd   | bitand |
//! | `\|`     | BitOr    | bitor  |
//! | `^`      | BitXor   | bitxor |
//! | `<<`     | Shl      | shl    |
//! | `>>`     | Shr      | shr    |
//! | `!`      | Not      | not    |
//! | `-` (unary) | Neg   | neg    |
//! | `[idx]`  | Index    | index  |
//! | `[idx]=` | IndexMut | index_mut |
//! | `*ref`   | Deref    | deref  |
//! | `*mut_ref` | DerefMut | deref_mut |

use verum_ast::expr::{BinOp, UnOp};
use verum_common::{List, Map, Maybe, Text};

/// Operator to protocol mapping configuration
///
/// This struct holds the complete mapping from operators to their protocols.
/// The mappings can be customized or extended for different language configurations.
#[derive(Debug, Clone)]
pub struct OperatorProtocols {
    /// Binary operator -> (Protocol name, Method name)
    binary_ops: Map<BinOpKey, OperatorMapping>,

    /// Unary operator -> (Protocol name, Method name)
    unary_ops: Map<UnOpKey, OperatorMapping>,

    /// Special protocol for iteration (for loops)
    into_iterator_protocol: Text,

    /// Special protocol for async/await
    future_protocol: Text,

    /// Special protocol for `?` operator
    try_protocol: Text,

    /// Special protocol for index operator `[]`
    index_protocol: Text,

    /// Special protocol for mutable index `[]=`
    index_mut_protocol: Text,

    /// Special protocol for deref `*`
    deref_protocol: Text,

    /// Special protocol for mutable deref
    deref_mut_protocol: Text,
}

/// Key for binary operator lookup (avoids issues with BinOp not implementing Hash)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOpKey {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    // Comparison
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    // Logical
    And,
    Or,
    Imply,
    // Bitwise
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    // Assignment
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    RemAssign,
    BitAndAssign,
    BitOrAssign,
    BitXorAssign,
    ShlAssign,
    ShrAssign,
}

impl From<BinOp> for BinOpKey {
    fn from(op: BinOp) -> Self {
        match op {
            BinOp::Add | BinOp::Concat => BinOpKey::Add,
            BinOp::Sub => BinOpKey::Sub,
            BinOp::Mul => BinOpKey::Mul,
            BinOp::Div => BinOpKey::Div,
            BinOp::Rem => BinOpKey::Rem,
            BinOp::Pow => BinOpKey::Pow,
            BinOp::Eq => BinOpKey::Eq,
            BinOp::Ne => BinOpKey::Ne,
            BinOp::Lt => BinOpKey::Lt,
            BinOp::Le => BinOpKey::Le,
            BinOp::Gt => BinOpKey::Gt,
            BinOp::Ge => BinOpKey::Ge,
            BinOp::In => BinOpKey::In,
            BinOp::And => BinOpKey::And,
            BinOp::Or => BinOpKey::Or,
            BinOp::Imply => BinOpKey::Imply,
            BinOp::Iff => BinOpKey::Imply, // <-> is treated as implication for operator lookup
            BinOp::BitAnd => BinOpKey::BitAnd,
            BinOp::BitOr => BinOpKey::BitOr,
            BinOp::BitXor => BinOpKey::BitXor,
            BinOp::Shl => BinOpKey::Shl,
            BinOp::Shr => BinOpKey::Shr,
            BinOp::Assign => BinOpKey::Assign,
            BinOp::AddAssign => BinOpKey::AddAssign,
            BinOp::SubAssign => BinOpKey::SubAssign,
            BinOp::MulAssign => BinOpKey::MulAssign,
            BinOp::DivAssign => BinOpKey::DivAssign,
            BinOp::RemAssign => BinOpKey::RemAssign,
            BinOp::BitAndAssign => BinOpKey::BitAndAssign,
            BinOp::BitOrAssign => BinOpKey::BitOrAssign,
            BinOp::BitXorAssign => BinOpKey::BitXorAssign,
            BinOp::ShlAssign => BinOpKey::ShlAssign,
            BinOp::ShrAssign => BinOpKey::ShrAssign,
        }
    }
}

/// Key for unary operator lookup
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnOpKey {
    Not,
    Neg,
    BitNot,
    Deref,
    Ref,
    RefMut,
    RefChecked,
    RefCheckedMut,
    RefUnsafe,
    RefUnsafeMut,
    Own,
    OwnMut,
}

impl From<UnOp> for UnOpKey {
    fn from(op: UnOp) -> Self {
        match op {
            UnOp::Not => UnOpKey::Not,
            UnOp::Neg => UnOpKey::Neg,
            UnOp::BitNot => UnOpKey::BitNot,
            UnOp::Deref => UnOpKey::Deref,
            UnOp::Ref => UnOpKey::Ref,
            UnOp::RefMut => UnOpKey::RefMut,
            UnOp::RefChecked => UnOpKey::RefChecked,
            UnOp::RefCheckedMut => UnOpKey::RefCheckedMut,
            UnOp::RefUnsafe => UnOpKey::RefUnsafe,
            UnOp::RefUnsafeMut => UnOpKey::RefUnsafeMut,
            UnOp::Own => UnOpKey::Own,
            UnOp::OwnMut => UnOpKey::OwnMut,
        }
    }
}

/// Mapping from operator to protocol and method
#[derive(Debug, Clone)]
pub struct OperatorMapping {
    /// Protocol that provides this operator
    pub protocol: Text,
    /// Method name to call
    pub method: Text,
    /// Whether the operator is commutative (a op b == b op a)
    pub commutative: bool,
    /// Output type strategy
    pub output_strategy: OutputStrategy,
}

/// Strategy for determining operator output type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStrategy {
    /// Output type is same as operand type (a + b -> typeof(a))
    SameAsOperand,
    /// Output type is Bool (comparison operators)
    Bool,
    /// Output type comes from protocol associated type
    Associated,
    /// Output type is explicitly defined
    Custom,
}

impl Default for OperatorProtocols {
    fn default() -> Self {
        Self::standard()
    }
}

impl OperatorProtocols {
    /// Create standard Verum operator->protocol mappings
    pub fn standard() -> Self {
        let mut binary_ops = Map::new();

        // Arithmetic operators
        binary_ops.insert(
            BinOpKey::Add,
            OperatorMapping {
                protocol: "Add".into(),
                method: "add".into(),
                commutative: true,
                output_strategy: OutputStrategy::Associated,
            },
        );
        binary_ops.insert(
            BinOpKey::Sub,
            OperatorMapping {
                protocol: "Sub".into(),
                method: "sub".into(),
                commutative: false,
                output_strategy: OutputStrategy::Associated,
            },
        );
        binary_ops.insert(
            BinOpKey::Mul,
            OperatorMapping {
                protocol: "Mul".into(),
                method: "mul".into(),
                commutative: true,
                output_strategy: OutputStrategy::Associated,
            },
        );
        binary_ops.insert(
            BinOpKey::Div,
            OperatorMapping {
                protocol: "Div".into(),
                method: "div".into(),
                commutative: false,
                output_strategy: OutputStrategy::Associated,
            },
        );
        binary_ops.insert(
            BinOpKey::Rem,
            OperatorMapping {
                protocol: "Rem".into(),
                method: "rem".into(),
                commutative: false,
                output_strategy: OutputStrategy::Associated,
            },
        );
        binary_ops.insert(
            BinOpKey::Pow,
            OperatorMapping {
                protocol: "Pow".into(),
                method: "pow".into(),
                commutative: false,
                output_strategy: OutputStrategy::Associated,
            },
        );

        // Comparison operators
        binary_ops.insert(
            BinOpKey::Eq,
            OperatorMapping {
                protocol: "Eq".into(),
                method: "eq".into(),
                commutative: true,
                output_strategy: OutputStrategy::Bool,
            },
        );
        binary_ops.insert(
            BinOpKey::Ne,
            OperatorMapping {
                protocol: "Eq".into(),
                method: "ne".into(),
                commutative: true,
                output_strategy: OutputStrategy::Bool,
            },
        );
        binary_ops.insert(
            BinOpKey::Lt,
            OperatorMapping {
                protocol: "Ord".into(),
                method: "lt".into(),
                commutative: false,
                output_strategy: OutputStrategy::Bool,
            },
        );
        binary_ops.insert(
            BinOpKey::Le,
            OperatorMapping {
                protocol: "Ord".into(),
                method: "le".into(),
                commutative: false,
                output_strategy: OutputStrategy::Bool,
            },
        );
        binary_ops.insert(
            BinOpKey::Gt,
            OperatorMapping {
                protocol: "Ord".into(),
                method: "gt".into(),
                commutative: false,
                output_strategy: OutputStrategy::Bool,
            },
        );
        binary_ops.insert(
            BinOpKey::Ge,
            OperatorMapping {
                protocol: "Ord".into(),
                method: "ge".into(),
                commutative: false,
                output_strategy: OutputStrategy::Bool,
            },
        );

        // Logical operators (these are built-in for Bool, but can be extended)
        binary_ops.insert(
            BinOpKey::And,
            OperatorMapping {
                protocol: "Bool".into(), // Built-in, short-circuiting
                method: "and".into(),
                commutative: true,
                output_strategy: OutputStrategy::Bool,
            },
        );
        binary_ops.insert(
            BinOpKey::Or,
            OperatorMapping {
                protocol: "Bool".into(), // Built-in, short-circuiting
                method: "or".into(),
                commutative: true,
                output_strategy: OutputStrategy::Bool,
            },
        );

        // Bitwise operators
        binary_ops.insert(
            BinOpKey::BitAnd,
            OperatorMapping {
                protocol: "BitAnd".into(),
                method: "bitand".into(),
                commutative: true,
                output_strategy: OutputStrategy::Associated,
            },
        );
        binary_ops.insert(
            BinOpKey::BitOr,
            OperatorMapping {
                protocol: "BitOr".into(),
                method: "bitor".into(),
                commutative: true,
                output_strategy: OutputStrategy::Associated,
            },
        );
        binary_ops.insert(
            BinOpKey::BitXor,
            OperatorMapping {
                protocol: "BitXor".into(),
                method: "bitxor".into(),
                commutative: true,
                output_strategy: OutputStrategy::Associated,
            },
        );
        binary_ops.insert(
            BinOpKey::Shl,
            OperatorMapping {
                protocol: "Shl".into(),
                method: "shl".into(),
                commutative: false,
                output_strategy: OutputStrategy::Associated,
            },
        );
        binary_ops.insert(
            BinOpKey::Shr,
            OperatorMapping {
                protocol: "Shr".into(),
                method: "shr".into(),
                commutative: false,
                output_strategy: OutputStrategy::Associated,
            },
        );

        // Logical implication (for formal proofs)
        binary_ops.insert(
            BinOpKey::Imply,
            OperatorMapping {
                protocol: "Imply".into(),
                method: "imply".into(),
                commutative: false,
                output_strategy: OutputStrategy::Bool,
            },
        );

        // Unary operators
        let mut unary_ops = Map::new();

        unary_ops.insert(
            UnOpKey::Neg,
            OperatorMapping {
                protocol: "Neg".into(),
                method: "neg".into(),
                commutative: false,
                output_strategy: OutputStrategy::SameAsOperand,
            },
        );
        unary_ops.insert(
            UnOpKey::Not,
            OperatorMapping {
                protocol: "Not".into(),
                method: "not".into(),
                commutative: false,
                output_strategy: OutputStrategy::SameAsOperand,
            },
        );
        unary_ops.insert(
            UnOpKey::BitNot,
            OperatorMapping {
                protocol: "BitNot".into(),
                method: "bitnot".into(),
                commutative: false,
                output_strategy: OutputStrategy::SameAsOperand,
            },
        );
        unary_ops.insert(
            UnOpKey::Deref,
            OperatorMapping {
                protocol: "Deref".into(),
                method: "deref".into(),
                commutative: false,
                output_strategy: OutputStrategy::Associated,
            },
        );

        Self {
            binary_ops,
            unary_ops,
            into_iterator_protocol: "IntoIterator".into(),
            future_protocol: "Future".into(),
            try_protocol: "Try".into(),
            index_protocol: "Index".into(),
            index_mut_protocol: "IndexMut".into(),
            deref_protocol: "Deref".into(),
            deref_mut_protocol: "DerefMut".into(),
        }
    }

    /// Create empty operator protocols (for testing)
    pub fn empty() -> Self {
        Self {
            binary_ops: Map::new(),
            unary_ops: Map::new(),
            into_iterator_protocol: "IntoIterator".into(),
            future_protocol: "Future".into(),
            try_protocol: "Try".into(),
            index_protocol: "Index".into(),
            index_mut_protocol: "IndexMut".into(),
            deref_protocol: "Deref".into(),
            deref_mut_protocol: "DerefMut".into(),
        }
    }

    /// Get protocol mapping for a binary operator
    pub fn get_binary_protocol(&self, op: BinOp) -> Option<&OperatorMapping> {
        self.binary_ops.get(&BinOpKey::from(op))
    }

    /// Get protocol mapping for a unary operator
    pub fn get_unary_protocol(&self, op: UnOp) -> Option<&OperatorMapping> {
        self.unary_ops.get(&UnOpKey::from(op))
    }

    /// Get the IntoIterator protocol name (for for-loops)
    pub fn into_iterator_protocol(&self) -> &Text {
        &self.into_iterator_protocol
    }

    /// Get the Future protocol name (for async/await)
    pub fn future_protocol(&self) -> &Text {
        &self.future_protocol
    }

    /// Get the Try protocol name (for ? operator)
    pub fn try_protocol(&self) -> &Text {
        &self.try_protocol
    }

    /// Get the Index protocol name (for [] operator)
    pub fn index_protocol(&self) -> &Text {
        &self.index_protocol
    }

    /// Get the IndexMut protocol name (for []= operator)
    pub fn index_mut_protocol(&self) -> &Text {
        &self.index_mut_protocol
    }

    /// Get the Deref protocol name (for * operator)
    pub fn deref_protocol(&self) -> &Text {
        &self.deref_protocol
    }

    /// Get the DerefMut protocol name (for *mut operator)
    pub fn deref_mut_protocol(&self) -> &Text {
        &self.deref_mut_protocol
    }

    /// Check if a binary operator is defined
    pub fn has_binary_protocol(&self, op: BinOp) -> bool {
        self.binary_ops.contains_key(&BinOpKey::from(op))
    }

    /// Check if a unary operator is defined
    pub fn has_unary_protocol(&self, op: UnOp) -> bool {
        self.unary_ops.contains_key(&UnOpKey::from(op))
    }

    /// Get all binary operators and their protocols
    pub fn all_binary_operators(&self) -> impl Iterator<Item = (&BinOpKey, &OperatorMapping)> {
        self.binary_ops.iter()
    }

    /// Get all unary operators and their protocols
    pub fn all_unary_operators(&self) -> impl Iterator<Item = (&UnOpKey, &OperatorMapping)> {
        self.unary_ops.iter()
    }

    /// Register a custom binary operator protocol
    pub fn register_binary(&mut self, op: BinOp, mapping: OperatorMapping) {
        self.binary_ops.insert(BinOpKey::from(op), mapping);
    }

    /// Register a custom unary operator protocol
    pub fn register_unary(&mut self, op: UnOp, mapping: OperatorMapping) {
        self.unary_ops.insert(UnOpKey::from(op), mapping);
    }
}

/// Operator resolution result
#[derive(Debug, Clone)]
pub struct OperatorResolution {
    /// The resolved method signature type
    pub method_signature: crate::ty::Type,
    /// The output type of the operation
    pub output_type: crate::ty::Type,
    /// Whether the operator is commutative
    pub commutative: bool,
    /// Source of the method (protocol name)
    pub protocol: Text,
}

/// Error when operator cannot be resolved
#[derive(Debug, Clone)]
pub enum OperatorError {
    /// No protocol defined for this operator
    NoProtocolForOperator { op: Text },
    /// Type does not implement the required protocol
    TypeDoesNotImplement {
        ty: crate::ty::Type,
        protocol: Text,
    },
    /// Operator types are incompatible
    IncompatibleTypes {
        left: crate::ty::Type,
        right: crate::ty::Type,
        op: Text,
    },
}

impl std::fmt::Display for OperatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OperatorError::NoProtocolForOperator { op } => {
                write!(f, "no protocol defined for operator '{}'", op)
            }
            OperatorError::TypeDoesNotImplement { ty, protocol } => {
                write!(f, "type '{}' does not implement protocol '{}'", ty, protocol)
            }
            OperatorError::IncompatibleTypes { left, right, op } => {
                write!(
                    f,
                    "incompatible types for '{}' operator: '{}' and '{}'",
                    op, left, right
                )
            }
        }
    }
}

impl std::error::Error for OperatorError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_binary_operators() {
        let ops = OperatorProtocols::standard();

        // Arithmetic
        let add = ops.get_binary_protocol(BinOp::Add).unwrap();
        assert_eq!(add.protocol.as_str(), "Add");
        assert_eq!(add.method.as_str(), "add");
        assert!(add.commutative);

        // Comparison
        let eq = ops.get_binary_protocol(BinOp::Eq).unwrap();
        assert_eq!(eq.protocol.as_str(), "Eq");
        assert_eq!(eq.output_strategy, OutputStrategy::Bool);

        // Order
        let lt = ops.get_binary_protocol(BinOp::Lt).unwrap();
        assert_eq!(lt.protocol.as_str(), "Ord");
        assert!(!lt.commutative);
    }

    #[test]
    fn test_standard_unary_operators() {
        let ops = OperatorProtocols::standard();

        let neg = ops.get_unary_protocol(UnOp::Neg).unwrap();
        assert_eq!(neg.protocol.as_str(), "Neg");

        let not = ops.get_unary_protocol(UnOp::Not).unwrap();
        assert_eq!(not.protocol.as_str(), "Not");
    }

    #[test]
    fn test_special_protocols() {
        let ops = OperatorProtocols::standard();

        assert_eq!(ops.into_iterator_protocol().as_str(), "IntoIterator");
        assert_eq!(ops.future_protocol().as_str(), "Future");
        assert_eq!(ops.try_protocol().as_str(), "Try");
        assert_eq!(ops.index_protocol().as_str(), "Index");
    }

    #[test]
    fn test_custom_operator_registration() {
        let mut ops = OperatorProtocols::empty();

        ops.register_binary(
            BinOp::Add,
            OperatorMapping {
                protocol: "CustomAdd".into(),
                method: "custom_add".into(),
                commutative: false,
                output_strategy: OutputStrategy::Custom,
            },
        );

        let add = ops.get_binary_protocol(BinOp::Add).unwrap();
        assert_eq!(add.protocol.as_str(), "CustomAdd");
    }
}
