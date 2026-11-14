//! Preserve dynamic constraints before selecting an erased execution carrier.

use super::{
    CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen,
    source_substitution::SourceValueScope,
};
use crate::types::{
    DynamicAssociatedBinding, DynamicProtocolType, ProtocolApplication, ProtocolId, TypeId,
    TypeParamId, TypeRef,
};
use verum_ast::{
    Type, TypeDeclBody,
    decl::ProtocolItemKind,
    ty::{TypeBoundKind, TypeKind},
};
use verum_common::List;

impl VbcCodegen {
    pub(super) fn dynamic_type_name(
        &self,
        object: &DynamicProtocolType,
        scope: Option<&std::collections::HashMap<String, u16>>,
    ) -> Option<verum_common::Text> {
        let mut inner = std::collections::HashMap::new();
        for (name, index) in scope.into_iter().flatten() {
            inner.insert(name.clone(), index.checked_add(1)?);
        }
        inner.insert("Self".into(), 0);
        let render = |ty: &TypeRef| self.scoped_callable_type_name(ty, Some(&inner));
        let mut parts = List::new();
        for application in &object.bounds {
            let name = self.scoped_callable_type_name(
                &TypeRef::Concrete(TypeId(application.protocol.0)),
                Some(&inner),
            )?;
            let mut args = application
                .args
                .iter()
                .map(render)
                .collect::<Option<List<_>>>()?;
            for binding in object
                .bindings
                .iter()
                .filter(|binding| &binding.owner == application)
            {
                args.push(format!("{}={}", binding.member, render(&binding.value)?).into());
            }
            parts.push(if args.is_empty() {
                name
            } else {
                format!("{name}<{}>", args.join(",")).into()
            });
        }
        Some(format!("dyn:{}", parts.join("+")).into())
    }

    pub(super) fn resolve_dynamic_type(
        &self,
        ty: &Type,
        scope: &std::collections::HashMap<String, u16>,
        declaration_scope: Option<&str>,
        array_count: &mut dyn FnMut(&verum_ast::Expr) -> Option<u64>,
        values: &SourceValueScope,
        nominal_defaults: &[TypeId],
    ) -> CodegenResult<TypeRef> {
        let TypeKind::DynProtocol { bounds, bindings } = &ty.kind else {
            return Err(CodegenError::internal(
                "dynamic elaborator requires a dynamic source node",
            ));
        };
        let invalid = |message: &str| {
            CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(message.into()),
                ty.span,
            )
        };
        let mut inner = std::collections::HashMap::new();
        for (name, index) in scope {
            inner.insert(
                name.clone(),
                index
                    .checked_add(1)
                    .ok_or_else(|| invalid("dynamic subject exceeds the binder index space"))?,
            );
        }
        let values = values.lifted(1, ty.span)?;
        let subject = TypeRef::Generic(TypeParamId(0));
        let mut applications = List::new();
        for bound in bounds {
            let application = match &bound.kind {
                TypeBoundKind::Protocol(path) => {
                    Type::new(TypeKind::Path(path.clone()), bound.span)
                }
                TypeBoundKind::GenericProtocol(application) => application.clone(),
                _ => {
                    return Err(invalid(
                        "dynamic bound requires an exact protocol application",
                    ));
                }
            };
            let resolved = self.resolve_type_ref_in_scope(
                &application,
                &inner,
                true,
                declaration_scope,
                array_count,
                Some(&subject),
                &values,
                nominal_defaults,
            )?;
            let (id, args) = match resolved {
                TypeRef::Concrete(id) => (id, List::new()),
                TypeRef::Instantiated { base, args } => (base, args.into()),
                _ => return Err(invalid("dynamic bound has no selected protocol identity")),
            };
            let descriptor = self
                .type_by_id(id)
                .filter(|descriptor| descriptor.kind == crate::types::TypeKind::Protocol)
                .ok_or_else(|| invalid("dynamic bound requires an exact protocol declaration"))?;
            if !descriptor
                .source_declaration
                .as_ref()
                .is_some_and(|source| matches!(source.body, TypeDeclBody::Protocol(_)))
            {
                return Err(invalid(
                    "dynamic bound lacks its declaration-owned signature",
                ));
            }
            let args = self.complete_nominal_arguments(
                id,
                args.into(),
                ty.span,
                nominal_defaults,
                Some(&subject),
            )?;
            let application = ProtocolApplication {
                protocol: ProtocolId(id.0),
                args,
            };
            if !applications.contains(&application) {
                applications.push(application);
            }
        }
        let mut equalities = List::new();
        for binding in bindings.iter().flatten() {
            let mut owners = applications.iter().filter(|application| {
                self.type_by_id(TypeId(application.protocol.0))
                    .and_then(|descriptor| descriptor.source_declaration.as_ref())
                    .is_some_and(|source| match &source.body {
                        TypeDeclBody::Protocol(body) => body.items.iter().any(|item| {
                            matches!(&item.kind,
                            ProtocolItemKind::Type {name, ..} if name.name == binding.name.name)
                        }),
                        _ => false,
                    })
            });
            let owner = owners
                .next()
                .ok_or_else(|| invalid("dynamic associated equality has no declaring protocol"))?;
            if owners.next().is_some() {
                return Err(invalid(
                    "dynamic associated equality has ambiguous declaration owners",
                ));
            }
            let value = self.resolve_type_ref_in_scope(
                &binding.ty,
                &inner,
                true,
                declaration_scope,
                array_count,
                Some(&subject),
                &values,
                nominal_defaults,
            )?;
            equalities.push(DynamicAssociatedBinding {
                owner: owner.clone(),
                member: binding.name.name.clone(),
                value,
            });
        }
        let object = DynamicProtocolType {
            bounds: applications,
            bindings: equalities,
        };
        crate::dynamic_protocol::validate(&object, &|id| self.type_by_id(id))
            .map_err(|message| invalid(message.as_str()))?;
        Ok(TypeRef::Dynamic(object))
    }
}
