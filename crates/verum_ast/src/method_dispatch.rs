//! Declaration facts needed after a protocol method's runtime ABI erases its receiver.
use crate::{
    FunctionDecl,
    visitor::{Visitor, walk_type},
};
use serde::{Deserialize, Serialize};

/// Receiver permission independently of the argument types that follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReceiverKind {
    /// No receiver — associated function (static method, no
    /// self parameter).  Aliased as `Static` in the previous
    /// `protocol::ReceiverKind` form.
    None,
    /// Takes self by value (consumes ownership).  Aliased as
    /// `ByValue` in `method_resolution` and `Value` in
    /// `protocol`.
    SelfValue,
    /// Takes `&self` (immutable borrow).  Aliased as `ByRef`
    /// in `method_resolution` and `Ref` in `protocol`.
    SelfRef,
    /// Takes `&mut self` (mutable borrow).  Aliased as
    /// `ByMutRef` in `method_resolution` and `RefMut` in
    /// `protocol`.
    SelfMut,
}

/// Per-variant projection for [`ReceiverKind`] — the classifier
/// flags partition the four receiver kinds along three
/// orthogonal axes (presence-of-self / by-value vs. borrowed /
/// mutability).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverKindMeta {
    /// Lower-snake-case wire form for telemetry surfaces.
    pub name: &'static str,
    /// Whether the method takes a `self` parameter at all.
    /// `None` (static method) is the unique non-self kind;
    /// the other three flip `has_self`.
    pub has_self: bool,
    /// Whether `self` is taken *by value* (consumes
    /// ownership).  `SelfValue` singleton.
    pub is_by_value: bool,
    /// Whether `self` is taken *by reference* (borrowed).
    /// `SelfRef` + `SelfMut`.
    pub is_borrowed: bool,
    /// Whether the receiver is *mutable* (the method may
    /// modify `self`).  `SelfValue` (owns it, can mutate
    /// freely) + `SelfMut` (borrowed mutably).  Pinned so
    /// codegen mutability analysis branches on this single
    /// flag rather than per-variant matching.
    pub allows_mutation: bool,
}

impl ReceiverKind {
    /// Source receiver permission before method parameters are lowered.
    pub fn from_declaration(function: &crate::FunctionDecl) -> Self {
        use crate::decl::FunctionParamKind::*;
        match function.params.first().map(|p| &p.kind) {
            Some(SelfRef | SelfRefChecked | SelfRefUnsafe) => Self::SelfRef,
            Some(SelfRefMut | SelfRefCheckedMut | SelfRefUnsafeMut) => Self::SelfMut,
            Some(SelfValue | SelfValueMut | SelfOwn | SelfOwnMut) => Self::SelfValue,
            Some(Regular { .. }) | None => Self::None,
        }
    }

    /// All variants in declaration order.
    pub const ALL: &'static [Self] = &[Self::None, Self::SelfValue, Self::SelfRef, Self::SelfMut];

    /// Static fact-pack — the partition table behind every
    /// receiver-kind dispatch surface (codegen, vtable layout,
    /// method-resolution priority).
    pub const fn meta(self) -> ReceiverKindMeta {
        match self {
            ReceiverKind::None => ReceiverKindMeta {
                name: "none",
                has_self: false,
                is_by_value: false,
                is_borrowed: false,
                allows_mutation: false,
            },
            ReceiverKind::SelfValue => ReceiverKindMeta {
                name: "self_value",
                has_self: true,
                is_by_value: true,
                is_borrowed: false,
                allows_mutation: true,
            },
            ReceiverKind::SelfRef => ReceiverKindMeta {
                name: "self_ref",
                has_self: true,
                is_by_value: false,
                is_borrowed: true,
                allows_mutation: false,
            },
            ReceiverKind::SelfMut => ReceiverKindMeta {
                name: "self_mut",
                has_self: true,
                is_by_value: false,
                is_borrowed: true,
                allows_mutation: true,
            },
        }
    }
}

/// Source facts that participate in vtable checks or method registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodDispatch {
    pub receiver: ReceiverKind,
    pub is_async: bool,
    pub has_generics: bool,
    pub returns_self: bool,
}

impl MethodDispatch {
    pub fn from_declaration(method: &FunctionDecl) -> Self {
        struct SelfInResult(bool);
        impl Visitor for SelfInResult {
            fn visit_type(&mut self, ty: &crate::Type) {
                use crate::ty::{PathSegment, TypeKind};
                if let TypeKind::Path(path) = &ty.kind
                    && (matches!(path.segments.as_slice(), [PathSegment::SelfValue])
                        || matches!(path.segments.as_slice(), [PathSegment::Name(name)] if name.as_str() == "Self"))
                {
                    self.0 = true;
                } else if !self.0 {
                    walk_type(self, ty);
                }
            }
        }
        let mut result = SelfInResult(false);
        if let Some(ty) = &method.return_type {
            result.visit_type(ty);
        }
        Self {
            receiver: ReceiverKind::from_declaration(method),
            is_async: method.is_async,
            has_generics: !method.generics.is_empty(),
            returns_self: result.0,
        }
    }
}
