//! Rigid identities for type-valued fields of a dependent record.

use super::{Type, UniverseLevel};
use std::fmt;
use verum_common::{List, Text};

/// An ordinal in one declared record. A lexical field name alone carries no
/// information about which introduction rule bound it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordFieldSlot {
    pub owner: Text,
    pub ordinal: usize,
}

/// A stable binding selected by the checker, possibly followed by field reads.
/// The binding identity distinguishes equally spelled identifiers in nested scopes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordReceiver {
    pub binding: u64,
    pub fields: List<Text>,
}

impl fmt::Display for RecordReceiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "binding#{}", self.binding)?;
        for field in &self.fields {
            write!(f, ".{field}")?;
        }
        Ok(())
    }
}

// The immutable dependency query and mutable instantiation must reach exactly
// the same children. The stack holds references, so neither walk consumes the
// Rust call stack and a read-only query never clones a field type.
macro_rules! push_type_children {
    ($ty:expr, $stack:ident $(, $m:ident)?) => {
        match $ty {
            Type::Function { params, return_type, type_params, .. } => {
                $stack.push(return_type);
                $stack.extend(params);
                for param in type_params {
                    if let Some(default) = & $($m)? param.default {
                        $stack.push(default);
                    }
                    for bound in & $($m)? param.bounds {
                        $stack.extend(& $($m)? bound.args);
                        for (_, ty) in & $($m)? bound.associated_types {
                            $stack.push(ty);
                        }
                    }
                }
            }
            Type::Tuple(types) | Type::Named { args: types, .. }
            | Type::Generic { args: types, .. } => $stack.extend(types),
            Type::TypeApp { constructor, args } => {
                $stack.push(constructor);
                $stack.extend(args);
            }
            Type::Array { element, .. } | Type::Slice { element }
            | Type::Tensor { element, .. } => $stack.push(element),
            Type::Record(fields) | Type::Variant(fields)
            | Type::ExtensibleRecord { fields, .. } => {
                for (_, field) in fields {
                    $stack.push(field);
                }
            }
            Type::DynProtocol { bindings, .. } => {
                for (_, ty) in bindings {
                    $stack.push(ty);
                }
            }
            Type::Reference { inner, .. } | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. } | Type::VolatilePointer { inner, .. }
            | Type::GenRef { inner } | Type::Quantified { inner, .. } => $stack.push(inner),
            Type::Refined { base, .. } | Type::CapabilityRestricted { base, .. } => $stack.push(base),
            Type::Exists { body, .. } | Type::Forall { body, .. } => $stack.push(body),
            Type::Meta { ty, .. } | Type::Eq { ty, .. } => $stack.push(ty),
            Type::Future { output } => $stack.push(output),
            Type::Generator { yield_ty, return_ty } => {
                $stack.push(yield_ty);
                $stack.push(return_ty);
            }
            Type::Pi { param_type, return_type, .. } => {
                $stack.push(param_type);
                $stack.push(return_type);
            }
            Type::Sigma { fst_type, snd_type, .. } => {
                $stack.push(fst_type);
                $stack.push(snd_type);
            }
            Type::PathType { space, .. } => $stack.push(space),
            Type::Partial { element_type, .. } => $stack.push(element_type),
            Type::Inductive { params, indices, constructors, .. } => {
                for (_, ty) in params { $stack.push(ty); }
                for (_, ty) in indices { $stack.push(ty); }
                for constructor in constructors {
                    for (_, ty) in & $($m)? constructor.type_params { $stack.push(ty); }
                    for ty in & $($m)? constructor.args { $stack.push(ty); }
                    $stack.push(& $($m)? constructor.return_type);
                }
            }
            Type::Coinductive { params, destructors, .. } => {
                for (_, ty) in params { $stack.push(ty); }
                for destructor in destructors { $stack.push(& $($m)? destructor.result_type); }
            }
            Type::HigherInductive { params, point_constructors, path_constructors, .. } => {
                for (_, ty) in params { $stack.push(ty); }
                for constructor in point_constructors {
                    for (_, ty) in & $($m)? constructor.type_params { $stack.push(ty); }
                    for ty in & $($m)? constructor.args { $stack.push(ty); }
                    $stack.push(& $($m)? constructor.return_type);
                }
                for constructor in path_constructors {
                    for (_, ty) in & $($m)? constructor.type_params { $stack.push(ty); }
                    for ty in & $($m)? constructor.args { $stack.push(ty); }
                    $stack.push(& $($m)? constructor.path_type.ty);
                }
            }
            Type::Unit | Type::Never | Type::Unknown | Type::Bool | Type::Int | Type::Float
            | Type::Char | Type::Text | Type::Var(_) | Type::Lifetime { .. } | Type::Interval
            | Type::Universe { .. } | Type::TypeConstructor { .. } | Type::Prop
            | Type::Placeholder { .. } | Type::RecordField { .. } => {}
        }
    };
}

impl Type {
    /// Whether any nested type depends on a record slot or instance.
    /// A backend without a faithful representation must decline the whole
    /// type, including opaque containers whose translation skips arguments.
    pub fn contains_record_field(&self) -> bool {
        self.contains_type(|ty| matches!(ty, Type::RecordField { .. }))
    }

    pub(crate) fn contains_capability_restriction(&self) -> bool {
        self.contains_type(|ty| matches!(ty, Type::CapabilityRestricted { .. }))
    }

    fn contains_type(&self, predicate: impl Fn(&Type) -> bool) -> bool {
        let mut stack = vec![self];
        while let Some(ty) = stack.pop() {
            if predicate(ty) {
                return true;
            }
            push_type_children!(ty, stack);
        }
        false
    }

    /// Collect just the declaration slots; previously projected types are closed.
    pub fn record_field_slots(&self) -> List<RecordFieldSlot> {
        let mut slots = List::new();
        let mut stack = vec![self];
        while let Some(ty) = stack.pop() {
            if let Type::RecordField {
                slot,
                receiver: None,
                ..
            } = ty
            {
                if !slots.contains(slot.as_ref()) {
                    slots.push(slot.as_ref().clone());
                }
            } else {
                push_type_children!(ty, stack);
            }
        }
        slots
    }

    /// Substitute in parallel. Already projected fields and every replacement
    /// are rigid under this walk. A missing owner/ordinal must be refused by
    /// the caller rather than substituted from an unrelated telescope.
    pub fn try_instantiate_record_fields(
        &self,
        mut resolve: impl FnMut(&RecordFieldSlot, &UniverseLevel) -> Result<Type, Text>,
    ) -> Result<Type, Text> {
        let mut result = self.clone();
        let mut stack = vec![&mut result];
        while let Some(ty) = stack.pop() {
            if let Type::RecordField {
                slot,
                receiver: None,
                level,
            } = ty
            {
                *ty = resolve(slot, level)?;
            } else {
                push_type_children!(ty, stack, mut);
            }
        }
        Ok(result)
    }
}
