//! Check source-owned array dimensions against any concrete dimensions in their ABI.
use super::CoreMetadata;
use super::{
    FieldDescriptor, FunctionDescriptor, TypeDescriptor, TypeDescriptorKind, VariantPayload,
};
use crate::ty::Type;
use verum_ast::{
    decl::{FunctionParamKind, RecordField, TypeDeclBody, VariantData},
    ty::{GenericArg, TypeKind},
    visitor::{Visitor, walk_type},
};
use verum_common::Text;

fn source_count(count: &verum_ast::Expr) -> Result<Option<usize>, Text> {
    use verum_ast::checked_const::{EvalError, Scalar, evaluate};
    match evaluate(count, 0, &mut |_, _| Ok::<_, ()>(None)) {
        Ok(Scalar::Int(n)) => usize::try_from(n)
            .map(Some)
            .map_err(|_| "source array extent must be nonnegative and representable".into()),
        // A name or layout expression requires the declaration's lexical context.
        Err(EvalError::Unresolved(_)) => Ok(None),
        _ => Err("source array extent is not a supported checked integer expression".into()),
    }
}

struct SourceArrays<'a> {
    scope: &'a Scope<'a>,
    error: Option<Text>,
}

impl Visitor for SourceArrays<'_> {
    fn visit_type(&mut self, ty: &verum_ast::Type) {
        if self.error.is_some() {
            return;
        }
        if let TypeKind::Array {
            size: Some(count), ..
        } = &ty.kind
            && let Err(error) = source_count(count)
        {
            self.error = Some(error);
            return;
        }
        if let TypeKind::Generic { base, args } = &ty.kind
            && let TypeKind::Path(head) = &base.kind
        {
            for (i, arg) in args.iter().enumerate() {
                if let Some(projected) = self.scope.bracket_type(head, args.len(), i, arg) {
                    self.visit_type(&projected);
                }
            }
        }
        walk_type(self, ty);
    }
}

struct Scope<'a> {
    metadata: &'a CoreMetadata,
    module: &'a str,
}

impl Scope<'_> {
    // Bracket syntax in a generic argument can also denote a repeated value.
    // Only the selected declaration's parameter kind can grant it a type role.
    fn bracket_type(
        &self,
        head: &verum_ast::ty::Path,
        count: usize,
        i: usize,
        arg: &GenericArg,
    ) -> Option<verum_ast::Type> {
        if !matches!(arg, GenericArg::Const(expr) if matches!(expr.kind, verum_ast::ExprKind::Array(_)))
        {
            return None;
        }
        let name = head.to_string().replace("::", ".");
        let key = if let Some(bindings) = self
            .metadata
            .declaration_type_bindings
            .get(&Text::from(self.module))
            && let Some(binding) = bindings.get(&Text::from(name.as_str()))
        {
            binding.clone()?
        } else {
            verum_common::qualified_name::qualify_module_name(self.module, &name)
        };
        let decl = crate::TypeChecker::metadata_type_with_declaring_key(self.metadata, &key)?
            .source_declaration
            .as_ref()?;
        if decl.generics.len() == count
            && matches!(
                decl.generics[i].kind,
                verum_ast::ty::GenericParamKind::Type { .. }
            )
        {
            arg.type_for_declared_slot()
        } else {
            None
        }
    }
}

fn compare(
    source: &verum_ast::Type,
    abi: &Type,
    matched: &mut usize,
    scope: &Scope,
) -> Result<(), Text> {
    match (&source.kind, abi) {
        (
            TypeKind::Array { element, size },
            Type::Array {
                element: other,
                size: Some(actual),
            },
        ) => {
            let expected = size
                .as_ref()
                .ok_or_else(|| Text::from("source array is missing an extent"))?;
            match source_count(expected)? {
                Some(n) if n == *actual => *matched += 1,
                Some(n) => {
                    return Err(format!(
                        "source array extent {n} disagrees with archive extent {actual}"
                    )
                    .into());
                }
                None => {
                    return Err(
                        "source array extent cannot establish the archive's fixed dimension".into(),
                    );
                }
            }
            compare(element, other, matched, scope)?;
        }
        (TypeKind::Slice(_), Type::Array { size: Some(_), .. }) => {
            return Err("source slice disagrees with a fixed archive array".into());
        }
        (
            TypeKind::Array { element, .. } | TypeKind::Slice(element),
            Type::Slice { element: other },
        ) => {
            // The legacy slice spelling provides no evidence about this dimension.
            compare(element, other, matched, scope)?;
        }
        (TypeKind::Reference { inner, .. }, Type::Reference { inner: other, .. })
        | (TypeKind::CheckedReference { inner, .. }, Type::CheckedReference { inner: other, .. })
        | (TypeKind::UnsafeReference { inner, .. }, Type::UnsafeReference { inner: other, .. })
        | (TypeKind::Pointer { inner, .. }, Type::Pointer { inner: other, .. }) => {
            compare(inner, other, matched, scope)?
        }
        (TypeKind::Tuple(items), Type::Tuple(others)) if items.len() == others.len() => {
            for (item, other) in items.iter().zip(others) {
                compare(item, other, matched, scope)?;
            }
        }
        (
            TypeKind::Function {
                params,
                return_type,
                ..
            },
            Type::Function {
                params: others,
                return_type: result,
                ..
            },
        ) if params.len() == others.len() => {
            for (item, other) in params.iter().zip(others) {
                compare(item, other, matched, scope)?;
            }
            compare(return_type, result, matched, scope)?;
        }
        (TypeKind::Generic { base, args }, Type::Named { path, args: others })
            if let TypeKind::Path(head) = &base.kind
                && head.to_string() == path.to_string()
                && args.len() == others.len() =>
        {
            for (i, (arg, other)) in args.iter().zip(others).enumerate() {
                if let GenericArg::Type(ty) = arg {
                    compare(ty, other, matched, scope)?;
                } else if let Some(ty) = scope.bracket_type(head, args.len(), i, arg) {
                    compare(&ty, other, matched, scope)?;
                }
            }
        }
        (TypeKind::Refined { base, .. }, _) => compare(base, abi, matched, scope)?,
        _ => (),
    }
    Ok(())
}

fn compare_spelling(
    source: Option<&verum_ast::Type>,
    spelling: &str,
    scope: &Scope,
) -> Result<(), Text> {
    if !spelling.contains(';') {
        return Ok(());
    }
    let mut claimed = 0;
    super::array_extents::visit_archive_extents(spelling, |_| claimed += 1)?;
    if claimed == 0 {
        return Ok(());
    }
    let mut matched = 0;
    if let Some(source) = source {
        compare(
            source,
            &crate::infer::parse_descriptor_type_string(spelling)?,
            &mut matched,
            scope,
        )?;
    }
    if matched != claimed {
        return Err("fixed archive array dimension has no corresponding source array".into());
    }
    Ok(())
}

fn callable(
    source: &verum_ast::FunctionDecl,
    source_params: &[verum_ast::decl::FunctionParam],
    params: &[super::ParamDescriptor],
    result: &str,
    scope: &Scope,
) -> Result<(), Text> {
    if !result.contains(';')
        && !params
            .iter()
            .any(|p| p.ty.contains(";") || p.declared_ty.contains(";"))
    {
        return Ok(());
    }
    if source_params.len() != params.len() {
        return Err("source argument count disagrees with its archive descriptor".into());
    }
    for (param, abi) in source_params.iter().zip(params) {
        let ty = match &param.kind {
            FunctionParamKind::Regular { ty, .. } => Some(ty),
            _ => None,
        };
        compare_spelling(ty, &abi.ty, scope)?;
        compare_spelling(ty, &abi.declared_ty, scope)?;
    }
    compare_spelling(source.return_type.as_ref(), result, scope)
}

pub(super) fn function(
    descriptor: &FunctionDescriptor,
    metadata: &CoreMetadata,
) -> Result<(), Text> {
    let Some(source) = &descriptor.source_signature else {
        return Ok(());
    };
    verum_ast::source_signature::validate_source_signature(
        source,
        descriptor.parent_type.is_some(),
    )?;
    let scope = Scope {
        metadata,
        module: descriptor
            .origin_module_path
            .as_ref()
            .filter(|p| !p.is_empty())
            .unwrap_or(&descriptor.module_path),
    };
    let mut arrays = SourceArrays {
        scope: &scope,
        error: None,
    };
    arrays.visit_function(source);
    if let Some(err) = arrays.error {
        return Err(err);
    }
    callable(
        source,
        &source.params,
        &descriptor.params,
        &descriptor.return_type,
        &scope,
    )
}

fn fields(source: &[RecordField], abi: &[FieldDescriptor], scope: &Scope) -> Result<(), Text> {
    for field in abi {
        let ty = source
            .iter()
            .find(|f| f.name.name == field.name)
            .map(|f| &f.ty);
        compare_spelling(ty, &field.ty, scope)?;
    }
    Ok(())
}

pub(super) fn nominal(descriptor: &TypeDescriptor, metadata: &CoreMetadata) -> Result<(), Text> {
    let Some(source) = &descriptor.source_declaration else {
        return Ok(());
    };
    verum_ast::source_signature::validate_source_type_declaration(source)?;
    let scope = &Scope {
        metadata,
        module: descriptor
            .origin_module_path
            .as_ref()
            .filter(|p| !p.is_empty())
            .unwrap_or(&descriptor.module_path),
    };
    let mut arrays = SourceArrays { scope, error: None };
    arrays.visit_type_decl(source);
    if let Some(err) = arrays.error {
        return Err(err);
    }
    match &descriptor.kind {
        TypeDescriptorKind::Alias { target } => {
            let ty = match &source.body {
                TypeDeclBody::Alias(ty) => Some(ty),
                _ => None,
            };
            compare_spelling(ty, target, scope)?;
        }
        TypeDescriptorKind::Record { fields: abi } => {
            let original = match &source.body {
                TypeDeclBody::Record(tys) => tys.as_slice(),
                _ => &[],
            };
            fields(original, abi, scope)?;
        }
        TypeDescriptorKind::Variant { cases } => {
            for case in cases {
                let data = match &source.body {
                    TypeDeclBody::Variant(original) => original
                        .iter()
                        .find(|v| v.name.name == case.name)
                        .and_then(|v| v.data.as_ref()),
                    _ => None,
                };
                match &case.payload {
                    Some(VariantPayload::Tuple(abi)) => {
                        let original = match data {
                            Some(VariantData::Tuple(tys)) if tys.len() == abi.len() => {
                                tys.as_slice()
                            }
                            _ => &[],
                        };
                        for (i, text) in abi.iter().enumerate() {
                            compare_spelling(original.get(i), text, scope)?;
                        }
                    }
                    Some(VariantPayload::Record(abi)) => {
                        let original = match data {
                            Some(VariantData::Record(tys)) => tys.as_slice(),
                            _ => &[],
                        };
                        fields(original, abi, scope)?;
                    }
                    None => (),
                }
            }
        }
        TypeDescriptorKind::Protocol {
            required_methods, ..
        } => {
            protocol_methods(source, required_methods, scope)?;
        }
        TypeDescriptorKind::Opaque => (),
    }
    Ok(())
}

fn protocol_methods<'a>(
    source: &verum_ast::TypeDecl,
    methods: impl IntoIterator<Item = &'a super::MethodSignature>,
    scope: &Scope,
) -> Result<(), Text> {
    for method in methods {
        let original = match &source.body {
            TypeDeclBody::Protocol(body) => body.items.iter().find_map(|item| match &item.kind {
                verum_ast::decl::ProtocolItemKind::Function { decl, .. }
                    if decl.name.name == method.name =>
                {
                    Some(decl)
                }
                _ => None,
            }),
            _ => None,
        };
        if let Some(original) = original {
            // Keep an explicit ABI receiver paired with its source slot so its
            // own type is checked too. A value-only ABI starts after source self.
            let params = match original.params.split_first() {
                Some((first, rest)) if first.is_self() && method.dispatch.receiver.meta().has_self => {
                    if method.stores_receiver() { original.params.as_slice() } else { rest }
                }
                _ if method.stores_receiver() => {
                    return Err("stored protocol receiver has no corresponding source receiver".into());
                }
                _ => original.params.as_slice(),
            };
            callable(original, params, &method.params, &method.return_type, scope)?;
        } else {
            for param in &method.params {
                compare_spelling(None, &param.ty, scope)?;
                compare_spelling(None, &param.declared_ty, scope)?;
            }
            compare_spelling(None, &method.return_type, scope)?;
        }
    }
    Ok(())
}

pub(super) fn protocol(
    desc: &super::ProtocolDescriptor,
    metadata: &CoreMetadata,
) -> Result<(), Text> {
    let owner = desc
        .origin_module_path
        .as_ref()
        .filter(|p| !p.is_empty())
        .unwrap_or(&desc.module_path);
    let key = verum_common::qualified_name::qualify_module_name(owner, &desc.name);
    if let Some(source) = crate::TypeChecker::metadata_type_with_declaring_key(metadata, &key)
        .and_then(|d| d.source_declaration.as_ref())
    {
        let scope = Scope {
            metadata,
            module: owner,
        };
        protocol_methods(
            source,
            desc.required_methods.iter().chain(&desc.default_methods),
            &scope,
        )?;
    }
    Ok(())
}
