//! Source declaration headers available before layout and method collection.
//! A binder roster is a fact about one selected nominal declaration. It neither
//! invents a record layout nor imports a same-spelled foreign type's parameters.
use std::borrow::Cow;

use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::types::{ProtocolId, StringId, TypeId, TypeParamDescriptor, TypeParamId, TypeRef};
use verum_ast::{FunctionDecl, Type, TypeDecl, TypeKind, decl::ImplKind, ty::GenericParamKind};

#[derive(Debug, Clone)]
pub(super) struct DeclaredGenericParameter {
    pub(super) name: String,
    pub(super) is_value: bool,
}

pub(super) type GenericRoster = Vec<DeclaredGenericParameter>;

pub(super) struct DeclaredGenericParameters {
    pub(super) scope: std::collections::HashMap<String, u16>,
    pub(super) parameters: smallvec::SmallVec<[TypeParamDescriptor; 2]>,
}

impl VbcCodegen {
    /// Every executable descriptor uses the same dense slots as its claimed
    /// source header. Lifetimes have no runtime slot; value parameters do.
    pub(super) fn declared_generic_parameters(
        &mut self,
        id: TypeId,
        declaration: &TypeDecl,
    ) -> CodegenResult<DeclaredGenericParameters> {
        let invalid = |message: String| {
            CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(message),
                declaration.span,
            )
        };
        let names: Vec<_> = self
            .declaration_generic_roster(id)
            .ok_or_else(|| {
                invalid(format!(
                    "type '{}' has an unsupported generic parameter roster",
                    declaration.name.name
                ))
            })?
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect();
        let mut scope = std::collections::HashMap::new();
        for (index, name) in names.iter().enumerate() {
            let index = u16::try_from(index).map_err(|_| {
                invalid(format!(
                    "type '{}' exceeds the VBC generic index space",
                    declaration.name.name
                ))
            })?;
            if scope.insert(name.clone(), index).is_some() {
                return Err(invalid(format!(
                    "type '{}' repeats generic parameter '{name}'",
                    declaration.name.name
                )));
            }
        }
        let mut parameters = smallvec::SmallVec::new();
        for generic in &declaration.generics {
            let (name, bounds, default) = match &generic.kind {
                GenericParamKind::Lifetime { .. } => continue,
                GenericParamKind::Type {
                    name,
                    bounds,
                    default,
                } => (name, bounds.as_slice(), default.as_ref()),
                GenericParamKind::HigherKinded { name, bounds, .. } => {
                    (name, bounds.as_slice(), None)
                }
                GenericParamKind::Const { name, .. } | GenericParamKind::Meta { name, .. } => {
                    (name, &[][..], None)
                }
                _ => {
                    return Err(invalid(format!(
                        "type '{}' has an unsupported generic parameter",
                        declaration.name.name
                    )));
                }
            };
            let index = *scope.get(name.name.as_str()).ok_or_else(|| {
                invalid(format!(
                    "generic parameter '{}' has no claimed slot",
                    name.name
                ))
            })?;
            if names.get(parameters.len()).map(String::as_str) != Some(name.name.as_str()) {
                return Err(invalid(format!(
                    "type '{}' disagrees with its claimed generic roster",
                    declaration.name.name
                )));
            }
            let mut descriptor = TypeParamDescriptor {
                name: StringId(self.ctx.intern_string_raw(name.name.as_str())),
                id: TypeParamId(index),
                default: default
                    .map(|ty| self.resolve_signature_type_ref(ty, &scope))
                    .transpose()?,
                ..Default::default()
            };
            for bound in bounds {
                match &bound.kind {
                    verum_ast::ty::TypeBoundKind::Protocol(path) => {
                        let name = Self::format_type_path(path);
                        let id = self.nominal_type_id(&name).ok_or_else(|| {
                            invalid(format!(
                                "unresolved protocol bound '{name}' on '{}'",
                                declaration.name.name
                            ))
                        })?;
                        descriptor.bounds.push(ProtocolId(id.0));
                    }
                    verum_ast::ty::TypeBoundKind::Equality(ty)
                        if matches!(ty.kind, TypeKind::Function { .. }) =>
                    {
                        descriptor
                            .type_bounds
                            .push(self.resolve_signature_type_ref(ty, &scope)?);
                    }
                    _ => {}
                }
            }
            parameters.push(descriptor);
        }
        Ok(DeclaredGenericParameters { scope, parameters })
    }

    pub(super) fn claim_generic_roster(&mut self, id: TypeId, declaration: &TypeDecl) {
        let roster = declaration
            .generics
            .iter()
            .filter(|parameter| !matches!(parameter.kind, GenericParamKind::Lifetime { .. }))
            .map(|parameter| {
                let (name, is_value) = match &parameter.kind {
                    GenericParamKind::Type { name, .. }
                    | GenericParamKind::HigherKinded { name, .. } => (name, false),
                    GenericParamKind::Const { name, .. } | GenericParamKind::Meta { name, .. } => {
                        (name, true)
                    }
                    _ => return None,
                };
                Some(DeclaredGenericParameter {
                    name: name.name.to_string(),
                    is_value,
                })
            })
            .collect();
        self.source_generic_rosters.insert(id, roster);
    }

    /// A known unsupported source header remains unknown. It cannot fall back
    /// to an erased descriptor and accidentally reinterpret value/level kinds.
    pub(super) fn declaration_generic_roster(
        &self,
        id: TypeId,
    ) -> Option<Cow<'_, [DeclaredGenericParameter]>> {
        if let Some(roster) = self.source_generic_rosters.get(&id) {
            return roster
                .as_ref()
                .map(|roster| Cow::Borrowed(roster.as_slice()));
        }
        // Imported executable descriptors declare only their actual type slots.
        // No absent source value parameter is reconstructed from a name.
        self.type_by_id(id)?
            .type_params
            .iter()
            .map(|parameter| {
                Some(DeclaredGenericParameter {
                    name: self.ctx.strings.get(parameter.name.0 as usize)?.clone(),
                    is_value: false,
                })
            })
            .collect::<Option<Vec<_>>>()
            .map(Cow::Owned)
    }

    pub(super) fn implementation_nominal_id(&self, implementation: &ImplKind) -> Option<TypeId> {
        let target = match implementation {
            ImplKind::Inherent(target)
            | ImplKind::Protocol {
                for_type: target, ..
            } => target,
        };
        self.implementation_type_nominal_id(target)
    }

    fn implementation_type_nominal_id(&self, target: &Type) -> Option<TypeId> {
        match &target.kind {
            TypeKind::Generic { base, .. }
            | TypeKind::Reference { inner: base, .. }
            | TypeKind::CheckedReference { inner: base, .. }
            | TypeKind::UnsafeReference { inner: base, .. }
            | TypeKind::Pointer { inner: base, .. } => self.implementation_type_nominal_id(base),
            TypeKind::Path(path) => self.nominal_type_id(&Self::format_type_path(path)),
            _ => self
                .extract_impl_type_name_from_type(target)
                .and_then(|name| self.nominal_type_id(&name)),
        }
    }

    /// Entering an implementation selects its own declaration before any Self
    /// witness is installed. An enclosing method's carried receiver cannot
    /// supply the new implementation's parameter order.
    pub(super) fn declared_impl_generic_param_maps(
        &self,
        function: &FunctionDecl,
        implementation: &ImplKind,
        generics: &[String],
    ) -> (
        std::collections::HashMap<String, u16>,
        std::collections::HashMap<String, u16>,
    ) {
        let parent_names = self
            .implementation_nominal_id(implementation)
            .and_then(|id| self.declaration_generic_roster(id))
            .map(|roster| {
                roster
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self::declaration_generic_param_maps(function, &parent_names, generics)
    }

    /// Exact carried Self takes precedence; otherwise source lexical lookup
    /// selects the declaration. A missing binding never consults a global leaf.
    pub(super) fn method_parent_generic_names(&self, name: &str) -> Option<Vec<String>> {
        let id = self
            .current_impl_receiver
            .as_ref()
            .and_then(|receiver| match &receiver.ty {
                TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => Some(*id),
                _ => None,
            })
            .or_else(|| self.nominal_type_id(name))?;
        Some(
            self.declaration_generic_roster(id)?
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect(),
        )
    }
}
