//! Declaration-owned existential protocol constraints shared by wire consumers.

use crate::types::{
    DynamicProtocolType, ProtocolApplication, TypeDescriptor, TypeId, TypeKind, TypeRef,
};
use verum_ast::{TypeDeclBody, decl::ProtocolItemKind, ty::GenericParamKind};
use verum_common::{Set, Text};

/// Check the declaration identities and associated members of one existential
/// constraint set. Binder scope and nested types are checked by the caller.
/// This establishes a semantic contract, never an owning allocation descriptor.
pub(crate) fn validate<'a>(
    object: &DynamicProtocolType,
    lookup: &impl Fn(TypeId) -> Option<&'a TypeDescriptor>,
) -> Result<(), Text> {
    if object.bounds.is_empty() {
        return Err("dynamic subject has no declared protocol bounds".into());
    }
    if object.bounds.len().saturating_add(object.bindings.len()) > crate::types::MAX_TYPE_REF_NODES
    {
        return Err("dynamic protocol exceeds structural resource limit".into());
    }
    let mut applications = Set::new();
    for bound in &object.bounds {
        declaration(bound, lookup)?;
        if !applications.insert(bound) {
            return Err("dynamic subject repeats the same protocol application".into());
        }
    }
    let mut members = Set::new();
    for binding in &object.bindings {
        if !applications.contains(&&binding.owner) {
            return Err(
                "dynamic associated equality belongs to an unbound protocol application".into(),
            );
        }
        let body = declaration(&binding.owner, lookup)?;
        let mut declarations = body.items.iter().filter_map(|item| match &item.kind {
            ProtocolItemKind::Type {
                name, type_params, ..
            } if name.name == binding.member => Some(type_params),
            _ => None,
        });
        let Some(parameters) = declarations.next() else {
            return Err("dynamic associated equality has no declared member".into());
        };
        if !parameters.is_empty() || declarations.next().is_some() {
            return Err("dynamic associated equality requires one non-family type member".into());
        }
        if !members.insert((&binding.owner, &binding.member)) {
            return Err("dynamic associated equality repeats a member".into());
        }
        if matches!(
            binding.value,
            TypeRef::ConstValue(_) | TypeRef::ConstSequence(_)
        ) {
            return Err("dynamic associated type equality cannot contain a value argument".into());
        }
    }
    Ok(())
}

fn declaration<'a>(
    application: &ProtocolApplication,
    lookup: &impl Fn(TypeId) -> Option<&'a TypeDescriptor>,
) -> Result<&'a verum_ast::decl::ProtocolBody, Text> {
    let descriptor = lookup(TypeId(application.protocol.0))
        .filter(|descriptor| descriptor.kind == TypeKind::Protocol)
        .ok_or_else(|| Text::from("dynamic bound requires an exact protocol declaration"))?;
    let source = descriptor
        .source_declaration
        .as_ref()
        .ok_or_else(|| Text::from("dynamic bound lacks its declaration-owned signature"))?;
    let TypeDeclBody::Protocol(body) = &source.body else {
        return Err("dynamic bound descriptor disagrees with its source declaration".into());
    };
    let parameters = source
        .generics
        .iter()
        .filter(|parameter| !matches!(parameter.kind, GenericParamKind::Lifetime { .. }));
    if parameters.clone().count() != application.args.len() {
        return Err("dynamic protocol application has incomplete declaration arguments".into());
    }
    for (parameter, argument) in parameters.zip(&application.args) {
        match &parameter.kind {
            GenericParamKind::Type { .. } | GenericParamKind::HigherKinded { .. } => {
                if matches!(argument, TypeRef::ConstValue(_) | TypeRef::ConstSequence(_)) {
                    return Err("dynamic protocol type parameter received a value".into());
                }
            }
            GenericParamKind::Const { .. } | GenericParamKind::Meta { .. } => {
                // Every application is scoped under this object's type subject.
                // Generic(0) therefore cannot denote a compile-time value.
                if matches!(argument, TypeRef::Generic(id) if id.0 == 0) {
                    return Err("dynamic type subject cannot occupy a value parameter".into());
                }
                if !matches!(
                    argument,
                    TypeRef::Generic(_) | TypeRef::ConstValue(_) | TypeRef::ConstSequence(_)
                ) {
                    return Err("dynamic protocol value parameter received a type".into());
                }
            }
            _ => {
                return Err(
                    "dynamic protocol application has an unsupported parameter kind".into(),
                );
            }
        }
    }
    Ok(body)
}
