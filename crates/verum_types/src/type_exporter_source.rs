//! Admission before the legacy source extractor can discard a contract.
use super::{ExportError, ExportResult, MAX_TYPE_DEPTH};
use verum_ast::decl::{FunctionDecl, FunctionParamKind, RecordField, TypeDeclBody, VariantData};
use verum_ast::ty::{GenericArg, TypeKind};

fn unsupported(what: &str) -> ExportError {
    ExportError(format!("{what} requires declaration-owned semantic replay"))
}

pub(super) fn validate_module(module: &verum_ast::Module) -> ExportResult<()> {
    for item in &module.items {
        match &item.kind {
            verum_ast::ItemKind::Function(function) if function.visibility.is_public() => {
                validate_function(function)?;
            }
            verum_ast::ItemKind::Function(_) => {}
            verum_ast::ItemKind::Type(declaration) => {
                if !declaration.generics.is_empty()
                    || !declaration.value_params.is_empty()
                    || declaration.resource_modifier.is_some()
                    || declaration.generic_where_clause.is_some()
                    || declaration.meta_where_clause.is_some()
                    || declaration
                        .attributes
                        .iter()
                        .any(|attribute| attribute.name.as_str() != "doc")
                {
                    return Err(unsupported("nominal declaration contracts"));
                }
                match &declaration.body {
                    TypeDeclBody::Record(fields) => {
                        for field in fields {
                            validate_field(field)?;
                        }
                    }
                    TypeDeclBody::Variant(variants) => {
                        for variant in variants {
                            if !variant.generic_params.is_empty()
                                || variant.where_clause.is_some()
                                || !variant.attributes.is_empty()
                                || variant.path_endpoints.is_some()
                            {
                                return Err(unsupported("variant contracts"));
                            }
                            match &variant.data {
                                None => {}
                                Some(VariantData::Tuple(types)) => {
                                    for ty in types {
                                        validate_type(ty, 0)?;
                                    }
                                }
                                Some(VariantData::Record(fields)) => {
                                    // The extractor's variant payload schema assumes public fields.
                                    for field in fields {
                                        if !field.visibility.is_public() {
                                            return Err(unsupported("private variant fields"));
                                        }
                                        validate_field(field)?;
                                    }
                                }
                            }
                        }
                    }
                    TypeDeclBody::Alias(ty) | TypeDeclBody::Newtype(ty) => validate_type(ty, 0)?,
                    TypeDeclBody::Unit => {}
                    _ => return Err(unsupported("nominal definition shape")),
                }
            }
            verum_ast::ItemKind::Module(declaration)
                if declaration.items.is_none()
                    && declaration.contexts.is_empty()
                    && declaration.profile.is_none()
                    && declaration.features.is_none() => {}
            // This exporter does not carry protocol/default/implementation obligations,
            // mounted declaration identities, or the remaining declaration forms.
            _ => return Err(unsupported("module declaration")),
        }
    }
    Ok(())
}

fn validate_field(field: &RecordField) -> ExportResult<()> {
    if !field.attributes.is_empty() || field.default_value.is_some() || field.bit_spec.is_some() {
        return Err(unsupported("field contracts"));
    }
    validate_type(&field.ty, 0)
}

fn validate_function(function: &FunctionDecl) -> ExportResult<()> {
    // This source-only snapshot has no checked body result. An omitted return
    // annotation may infer a value type and must never be fabricated as Unit.
    if function.return_type.is_none() {
        return Err(unsupported("inferred function result"));
    }
    if !function.generics.is_empty()
        || !function.contexts.is_empty()
        || function.generic_where_clause.is_some()
        || function.meta_where_clause.is_some()
        || function.throws_clause.is_some()
        || function.std_attr.is_some()
        || !function.requires.is_empty()
        || !function.ensures.is_empty()
        || !function.decreases.is_empty()
        || function.is_meta
        || function.stage_level != 0
        || function.is_pure
        || function.is_generator
        || function.is_cofix
        || function.is_unsafe
        || function.is_transparent
        || function.extern_abi.is_some()
        || function.is_variadic
        || function
            .attributes
            .iter()
            .any(|attribute| attribute.name.as_str() != "doc")
    {
        return Err(unsupported("function contracts"));
    }
    for parameter in &function.params {
        if !parameter.attributes.is_empty() {
            return Err(unsupported("parameter attributes"));
        }
        match &parameter.kind {
            FunctionParamKind::Regular {
                pattern,
                ty,
                default_value,
            } if default_value.is_none() => {
                if !matches!(
                    pattern.kind,
                    verum_ast::PatternKind::Ident { .. } | verum_ast::PatternKind::Wildcard
                ) {
                    return Err(unsupported("destructured parameter"));
                }
                validate_type(ty, 0)?;
            }
            _ => return Err(unsupported("receiver or default parameter")),
        }
    }
    if let Some(ty) = &function.return_type {
        validate_type(ty, 0)?;
    }
    Ok(())
}

fn validate_type(ty: &verum_ast::Type, depth: usize) -> ExportResult<()> {
    if depth >= MAX_TYPE_DEPTH {
        return Err(unsupported("deeply nested annotation"));
    }
    match &ty.kind {
        TypeKind::Unit
        | TypeKind::Never
        | TypeKind::Bool
        | TypeKind::Int
        | TypeKind::Float
        | TypeKind::Char
        | TypeKind::Text
        | TypeKind::Path(_) => {}
        TypeKind::Generic { base, args } => {
            validate_type(base, depth + 1)?;
            for argument in args {
                match argument {
                    GenericArg::Type(ty) => validate_type(ty, depth + 1)?,
                    _ => return Err(unsupported("non-type generic argument")),
                }
            }
        }
        TypeKind::Reference { inner, .. } => validate_type(inner, depth + 1)?,
        TypeKind::Tuple(types) => {
            for ty in types {
                validate_type(ty, depth + 1)?;
            }
        }
        TypeKind::Array { element, size } if size.is_none() => validate_type(element, depth + 1)?,
        TypeKind::Function {
            params,
            return_type,
            calling_convention,
            contexts,
            ..
        } if calling_convention.is_none() && contexts.is_empty() => {
            for parameter in params {
                validate_type(parameter, depth + 1)?;
            }
            validate_type(return_type, depth + 1)?;
        }
        _ => return Err(unsupported("annotation contract or type shape")),
    }
    Ok(())
}
