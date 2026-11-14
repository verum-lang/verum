//! Declaration authority for retain/copy of a reference-counted shallow handle.
//!
//! This operation does not discover a counter inside a record. Its unsafe caller
//! supplies the live UInt32 counter belonging to that handle's release contract.
use crate::{
    module::VbcModule,
    mono::TypeSubstitution,
    types::{MAX_TYPE_REF_NODES, TypeId, TypeKind, TypeRef},
};
use verum_common::{List, Text};

/// Exact fixed layout copied before the atomic retain is attempted.
#[derive(Clone, Copy, Debug)]
pub struct RetainLayout {
    /// Nominal owner, preserved in the newly allocated wrapper.
    pub owner: TypeId,
    /// Number of Value-width slots in the handle itself.
    pub fields: usize,
}

/// Validate a shallow record. References are copied as references; an owned
/// aggregate, closure, erased dynamic payload, or unbound slot never grants
/// permission to copy ownership bits. Alias expansion is bounded and iterative.
pub fn layout(module: &VbcModule, ty: &TypeRef) -> Result<RetainLayout, Text> {
    let checked = TypeSubstitution::empty()
        .apply(ty)
        .map_err(|e| Text::from(e.to_string()))?;
    let ty = &checked;
    let (owner, args) = match ty {
        TypeRef::Concrete(owner) => (*owner, &[][..]),
        TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
        _ => return Err("retain_copy requires a declared shallow record".into()),
    };
    let declaration = module
        .get_type(owner)
        .ok_or_else(|| Text::from("retain_copy has no declaring owner"))?;
    if declaration.kind != TypeKind::Record || declaration.type_params.len() != args.len() {
        return Err("retain_copy requires the exact record parameter roster".into());
    }
    if declaration.fields.len() > MAX_TYPE_REF_NODES {
        return Err("retain_copy exceeds its declaration budget".into());
    }
    let substitution = TypeSubstitution::new(&declaration.type_params, args);
    let mut work = List::new();
    for field in &declaration.fields {
        let ty = field
            .declaration_type
            .as_ref()
            .ok_or_else(|| Text::from("retain_copy field lacks its declaration"))?;
        work.push(
            substitution
                .apply(ty)
                .map_err(|e| Text::from(e.to_string()))?,
        );
    }
    let mut remaining = MAX_TYPE_REF_NODES;
    while let Some(field) = work.pop() {
        remaining = remaining
            .checked_sub(1)
            .ok_or_else(|| Text::from("retain_copy exceeds its declaration budget"))?;
        match field {
            TypeRef::Reference { .. } => (),
            TypeRef::Concrete(id)
                if matches!(
                    id,
                    TypeId::UNIT
                        | TypeId::BOOL
                        | TypeId::I8
                        | TypeId::I16
                        | TypeId::I32
                        | TypeId::I64
                        | TypeId::U8
                        | TypeId::U16
                        | TypeId::U32
                        | TypeId::U64
                        | TypeId::F32
                        | TypeId::F64
                        | TypeId::CHAR
                ) =>
            {
                ()
            }
            TypeRef::Concrete(id) => {
                let alias = module
                    .get_type(id)
                    .filter(|d| d.kind == TypeKind::Alias && d.type_params.is_empty())
                    .and_then(|d| d.alias_target.as_ref())
                    .ok_or_else(|| Text::from("retain_copy refuses an owning or unknown field"))?;
                work.push(
                    TypeSubstitution::empty()
                        .apply(alias)
                        .map_err(|e| Text::from(e.to_string()))?,
                );
            }
            _ => return Err("retain_copy refuses an owning or unknown field".into()),
        }
    }
    Ok(RetainLayout {
        owner,
        fields: declaration.fields.len(),
    })
}
