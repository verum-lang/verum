//! Nominal constructors retain the same selected type and binder telescope in
//! source compilation, archive loading and both call syntaxes.
use super::{CodegenError, CodegenErrorKind, CodegenResult, FunctionInfo, VbcCodegen};
use crate::{
    module::FunctionId,
    types::{StringId, TypeDescriptor, TypeId, TypeRef},
};
use verum_ast::ty::GenericParamKind;
use verum_common::{List, Map, Text};

impl FunctionInfo {
    /// Construct callable evidence directly from one declared nominal descriptor.
    /// Synthetic constructors have no executable body; their shared sentinel is
    /// never an identity from which parameter or representation facts can be read.
    pub fn for_nominal_constructor<'a>(
        declaration: &TypeDescriptor,
        name: Text,
        string: impl Fn(StringId) -> Option<&'a str>,
    ) -> Self {
        let type_param_ids: List<_> = declaration
            .type_params
            .iter()
            .map(|parameter| parameter.id)
            .collect();
        let result = if type_param_ids.is_empty() {
            TypeRef::Concrete(declaration.id)
        } else {
            TypeRef::Instantiated {
                base: declaration.id,
                args: type_param_ids
                    .iter()
                    .copied()
                    .map(TypeRef::Generic)
                    .collect(),
            }
        };
        let explicit_type_param_ids = declaration
            .source_declaration
            .as_ref()
            .map(|source| {
                source
                    .generics
                    .iter()
                    .filter(|parameter| !parameter.is_implicit)
                    .map(|parameter| {
                        let name = match &parameter.kind {
                            GenericParamKind::Type { name, .. }
                            | GenericParamKind::Meta { name, .. }
                            | GenericParamKind::Const { name, .. }
                            | GenericParamKind::HigherKinded { name, .. }
                            | GenericParamKind::Context { name }
                            | GenericParamKind::KindAnnotated { name, .. }
                            | GenericParamKind::Level { name } => name.name.as_str(),
                            GenericParamKind::Lifetime { .. } => return None,
                        };
                        declaration
                            .type_params
                            .iter()
                            .find(|parameter| string(parameter.name) == Some(name))
                            .map(|parameter| parameter.id)
                    })
                    .collect()
            })
            .unwrap_or_else(|| type_param_ids.iter().copied().map(Some).collect());
        let params: Option<List<_>> = declaration
            .fields
            .iter()
            .map(|field| field.declaration_type.clone())
            .collect();
        let callable_signature = params.map(|params| TypeRef::Function {
            is_async: false,
            params: params.into(),
            return_type: Box::new(result.clone()),
            contexts: Default::default(),
        });
        Self {
            has_source_body: false,
            foreign_declaration: None,
            callable_signature,
            id: FunctionId(u32::MAX / 2),
            param_count: declaration.fields.len(),
            param_names: declaration
                .fields
                .iter()
                .map(|field| string(field.name).unwrap_or("").to_owned())
                .collect(),
            param_type_names: declaration
                .fields
                .iter()
                .map(|field| string(field.type_name).unwrap_or("").to_owned())
                .collect(),
            is_async: false,
            is_generator: false,
            contexts: List::new().into(),
            return_type: Some(result),
            yield_type: None,
            intrinsic_name: None,
            variant_tag: None,
            parent_type_name: None,
            variant_payload_types: None,
            is_partial_pattern: false,
            takes_self_mut_ref: false,
            return_type_name: Some(name.into_string()),
            return_type_inner: None,
            is_const: false,
            is_transparent_wrapper: declaration.is_transparent_wrapper,
            param_closure_return_type_names: List::new().into(),
            explicit_type_param_ids,
            type_param_ids: type_param_ids.into(),
        }
    }

    pub(super) fn nominal_constructor_type(&self) -> Option<TypeId> {
        if self.has_source_body || self.variant_tag.is_some() || self.id != FunctionId(u32::MAX / 2)
        {
            return None;
        }
        match self.return_type.as_ref()? {
            TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => Some(*id),
            _ => None,
        }
    }
}

impl VbcCodegen {
    pub(super) fn register_nominal_constructor(
        &mut self,
        id: TypeId,
        local_name: &str,
    ) -> CodegenResult<()> {
        let descriptor = self.type_by_id(id).ok_or_else(|| {
            CodegenError::internal("declared constructor has no nominal descriptor")
        })?;
        let owner = self
            .type_declaration_identity(descriptor)
            .ok_or_else(|| CodegenError::internal("declared constructor has no owner"))?;
        let info = FunctionInfo::for_nominal_constructor(descriptor, owner.clone(), |id| {
            self.ctx
                .strings
                .get(id.0 as usize)
                .map(|name| name.as_str())
        });
        self.ctx
            .register_function(owner.into_string(), info.clone());
        self.ctx.register_function(local_name.to_owned(), info);
        Ok(())
    }

    /// Callable witnesses use the nominal's own source environment. The cache is
    /// keyed by its TypeId, so unrelated synthetic constructors cannot share a
    /// roster merely because they have the same non-executable FunctionId.
    pub(super) fn prepare_nominal_constructor(
        &mut self,
        function: &FunctionInfo,
    ) -> CodegenResult<()> {
        let Some(id) = function.nominal_constructor_type() else {
            return Ok(());
        };
        if self.constructor_generic_rosters.contains_key(&id) {
            return Ok(());
        }
        let invalid = || {
            CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                "nominal constructor lacks declaration-owned parameter evidence".into(),
            ))
        };
        let descriptor = self.type_by_id(id).ok_or_else(invalid)?;
        let owner = self
            .type_declaration_identity(descriptor)
            .ok_or_else(invalid)?;
        let module = owner
            .rsplit_once('.')
            .map(|(module, _)| module.to_owned())
            .ok_or_else(invalid)?;
        let scope: Map<_, _> = descriptor
            .type_params
            .iter()
            .map(|parameter| {
                self.ctx
                    .strings
                    .get(parameter.name.0 as usize)
                    .map(|name| (name.clone(), parameter.id.0))
                    .ok_or_else(invalid)
            })
            .collect::<CodegenResult<_>>()?;
        let mut roster = self
            .declaration_generic_roster(id)
            .ok_or_else(invalid)?
            .into_owned();
        for parameter in &mut roster {
            let slot = scope
                .get(&parameter.name)
                .copied()
                .map(crate::types::TypeParamId)
                .ok_or_else(invalid)?;
            parameter.witness_slot = Some(slot);
            parameter.explicit_slot = function
                .explicit_type_param_ids
                .iter()
                .position(|id| *id == Some(slot));
        }
        let scope = scope.into();
        self.with_protocol_source_scope(Some(module), Default::default(), None, |generator| {
            for parameter in &mut roster {
                if let Some(source) = parameter.value_parameter() {
                    let ty = match &source.kind {
                        GenericParamKind::Meta { ty, .. } | GenericParamKind::Const { ty, .. } => {
                            ty
                        }
                        _ => return Err(invalid()),
                    };
                    let domain = generator.resolve_signature_type_ref(ty, &scope)?;
                    parameter.value_carrier = generator.runtime_value_carrier(&domain);
                }
            }
            Ok(())
        })?;
        self.constructor_generic_rosters.insert(id, roster);
        Ok(())
    }
}
