//! Producer-selected value uses. These facts never grant cleanup authority.
use crate::Maybe;
use serde::{Deserialize, Serialize};

/// A declaration within one function body; independent of name and register reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BindingId(pub u32);
/// A syntactic production site within one body, not a runtime lifecycle ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValueUseId(pub u32);
/// The selected duplication operation; neither variant proves a fresh allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DuplicationOperation {
    /// Scalar/reference carrier copying, without copying the referent.
    Carrier,
    /// The existing language value-copy operation, emitted as VBC Clone.
    ValueCopy,
}
/// Semantic classification. Missing facts are not permission to move or duplicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueUseOperation {
    /// The producer cannot establish a semantic operation from its current facts.
    Unknown,
    /// A proven reference is forwarded without acquiring referent ownership.
    Borrow,
    /// A declared duplication operation was actually emitted.
    Copy(DuplicationOperation),
    /// Reserved for a producer that also transfers the cleanup obligation.
    /// The initial receipts producer deliberately never emits this variant.
    Transfer,
}
/// The syntactic consumer of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueUseSite {
    /// A local binding or assignment.
    Local,
    /// A direct call operand, in declaration order.
    Argument(u16),
    /// A direct function return operand.
    Return,
}
/// Shared CFG vocabulary. IDs are local to the owning function descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueUseEvent {
    /// Unique producer site.
    pub id: ValueUseId,
    /// Exact binding when the emitted operand directly names one.
    pub binding: BindingId,
    /// Receiving local declaration, where directly available.
    pub destination: Maybe<BindingId>,
    /// Consumer category.
    pub site: ValueUseSite,
    /// Selected operation, without destructor or freshness claims.
    pub operation: ValueUseOperation,
}
