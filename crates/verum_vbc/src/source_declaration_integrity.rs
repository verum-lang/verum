//! Compatibility between a semantic declaration and its runtime descriptor.
//!
//! Universe terms, constraints, and method signatures live in the AST, not in
//! `TypeRef`. This check binds that AST to the runtime declaration's structural
//! slots; it does not establish the declaration's typing or proof obligations.
//! Incomplete import stubs must be enriched before carrying a semantic AST.

use crate::module::StringTable;
use crate::types::{FieldDescriptor, TypeDescriptor, TypeKind, TypeRef, VariantKind};
use verum_ast::{
    TypeDeclBody,
    decl::{FunctionParamKind, ProtocolItemKind, VariantData},
    ty::GenericParamKind,
};
use verum_common::{Set, Text};

/// Validate a declaration-bearing descriptor before publishing or importing it.
/// Ordinary ABI-only descriptors retain their existing validation path.
pub fn validate_type_descriptor(
    descriptor: &TypeDescriptor,
    strings: &StringTable,
) -> Result<(), Text> {
    if descriptor.kind != TypeKind::Protocol && descriptor.variants.iter().any(|v| v.method_dispatch.is_some()) {
        return Err("data variant cannot declare a protocol method receiver".into());
    }
    let Some(declaration) = &descriptor.source_declaration else {
        return Ok(());
    };
    verum_ast::source_signature::validate_source_type_declaration(declaration)?;
    let fail = |detail: &str| -> Text {
        format!("semantic type {}: {detail}", declaration.name.name).into()
    };
    let name = strings
        .get(descriptor.name)
        .ok_or_else(|| fail("descriptor has no name"))?;
    if name.rsplit('.').next() != Some(declaration.name.name.as_str()) {
        return Err(fail("declaration does not belong to its descriptor"));
    }
    if descriptor.resource_discipline != declaration.resource_discipline() {
        return Err(fail("runtime resource policy differs from declaration"));
    }

    // Type parameters alone occupy runtime type slots; compile-time value
    // parameters remain in the AST. Sum/alias lowering retains source indices,
    // while record/protocol lowering numbers the type-only slots densely.
    let source_indices = matches!(
        declaration.body,
        TypeDeclBody::Variant(_) | TypeDeclBody::Alias(_)
    );
    let mut actual_parameters = descriptor.type_params.iter();
    let mut dense_index = 0usize;
    for (source_index, parameter) in declaration.generics.iter().enumerate() {
        if let GenericParamKind::Type { name, .. } = &parameter.kind {
            let actual = actual_parameters
                .next()
                .ok_or_else(|| fail("missing runtime type parameter"))?;
            let expected_index = if source_indices {
                source_index
            } else {
                dense_index
            };
            let expected_id = u16::try_from(expected_index)
                .map_err(|_| fail("type parameter index exceeds runtime range"))?;
            if strings.get(actual.name) != Some(name.name.as_str()) || actual.id.0 != expected_id {
                return Err(fail(
                    "runtime type parameter order or identity differs from declaration",
                ));
            }
            dense_index += 1;
        }
    }
    if actual_parameters.next().is_some() {
        return Err(fail("extra runtime type parameter"));
    }

    let (kind, transparent) = match &declaration.body {
        TypeDeclBody::Record(_) => (TypeKind::Record, false),
        TypeDeclBody::Variant(_) => (TypeKind::Sum, false),
        TypeDeclBody::Protocol(_) => (TypeKind::Protocol, false),
        TypeDeclBody::Alias(_) => (TypeKind::Alias, false),
        // These are the explicit representation erasures used by codegen:
        // tuples/newtypes have record slots, with a single slot transparent.
        TypeDeclBody::Newtype(_) => (TypeKind::Record, true),
        TypeDeclBody::Tuple(elements) => (TypeKind::Record, elements.len() == 1),
        TypeDeclBody::Unit => (TypeKind::Unit, false),
        TypeDeclBody::Inductive(_)
        | TypeDeclBody::Coinductive(_)
        | TypeDeclBody::SigmaTuple(_)
        | TypeDeclBody::Quotient { .. } => {
            return Err(fail(
                "declaration has no supported runtime descriptor representation",
            ));
        }
    };
    if descriptor.kind != kind || descriptor.is_transparent_wrapper != transparent {
        return Err(fail(
            "runtime kind or representation differs from declaration",
        ));
    }
    if (kind == TypeKind::Alias) != descriptor.alias_target.is_some() {
        return Err(fail("runtime alias target does not match declaration kind"));
    }
    if !matches!(kind, TypeKind::Sum | TypeKind::Protocol) && !descriptor.variants.is_empty() {
        return Err(fail("non-variant declaration has runtime variants"));
    }
    match &declaration.body {
        TypeDeclBody::Protocol(body) => {
            if !descriptor.fields.is_empty() {
                return Err(fail("protocol declaration has runtime fields"));
            }
            // Protocol method signatures use the existing variant table as an
            // ordered vtable roster. They are not data-type payload variants.
            let mut actual_methods = descriptor.variants.iter();
            let mut index = 0u32;
            let mut names = Set::new();
            for item in &body.items {
                if let ProtocolItemKind::Function { decl, .. } = &item.kind {
                    let actual = actual_methods
                        .next()
                        .ok_or_else(|| fail("missing runtime protocol method"))?;
                    if actual.method_dispatch.is_some_and(|dispatch|
                        dispatch != verum_ast::method_dispatch::MethodDispatch::from_declaration(decl))
                    {
                        return Err(fail("protocol method dispatch differs from declaration"));
                    }
                    let regular_params = decl
                        .params
                        .iter()
                        .filter(|param| matches!(param.kind, FunctionParamKind::Regular { .. }))
                        .count();
                    if strings.get(actual.name) != Some(decl.name.name.as_str())
                        || actual.tag != index
                        || actual.kind != VariantKind::Unit
                        || actual.arity != 0
                        || !actual.fields.is_empty()
                        || !names.insert(decl.name.name.clone())
                    {
                        return Err(fail(
                            "runtime protocol method name or order differs from declaration",
                        ));
                    }
                    if !matches!(&actual.payload, Some(TypeRef::Function { params, .. }) if params.len() == regular_params)
                    {
                        return Err(fail(
                            "runtime protocol method parameter shape differs from declaration",
                        ));
                    }
                    index = index
                        .checked_add(1)
                        .ok_or_else(|| fail("protocol method index exceeds runtime range"))?;
                }
            }
            if actual_methods.next().is_some() {
                return Err(fail("extra runtime protocol method"));
            }
        }
        TypeDeclBody::Record(fields) => {
            check_fields(
                &descriptor.fields,
                fields.iter().map(|field| field.name.name.to_string()),
                strings,
            )
            .map_err(|message| fail(message.as_str()))?;
        }
        TypeDeclBody::Newtype(_) => {
            check_fields(
                &descriptor.fields,
                std::iter::once("_0".to_owned()),
                strings,
            )
            .map_err(|message| fail(message.as_str()))?;
        }
        TypeDeclBody::Tuple(elements) => {
            check_fields(
                &descriptor.fields,
                (0..elements.len()).map(|index| format!("_{index}")),
                strings,
            )
            .map_err(|message| fail(message.as_str()))?;
        }
        TypeDeclBody::Variant(variants) => {
            if !descriptor.fields.is_empty() || descriptor.variants.len() != variants.len() {
                return Err(fail("runtime variant roster differs from declaration"));
            }
            let mut names = Set::new();
            for (index, (actual, expected)) in descriptor.variants.iter().zip(variants).enumerate()
            {
                let tag =
                    u32::try_from(index).map_err(|_| fail("variant tag exceeds runtime range"))?;
                if strings.get(actual.name) != Some(expected.name.name.as_str())
                    || actual.tag != tag
                    || !names.insert(expected.name.name.clone())
                {
                    return Err(fail(
                        "runtime variant name, order, or tag differs from declaration",
                    ));
                }
                // Current producers describe each payload slot in `fields`;
                // the optional aggregate payload is deliberately absent.
                if actual.payload.is_some() {
                    return Err(fail("unexpected aggregate variant payload"));
                }
                match expected.data.as_ref() {
                    None => {
                        if actual.kind != VariantKind::Unit
                            || actual.arity != 0
                            || !actual.fields.is_empty()
                        {
                            return Err(fail("unit variant has runtime payload slots"));
                        }
                    }
                    Some(VariantData::Tuple(elements)) => {
                        let arity = u8::try_from(elements.len())
                            .map_err(|_| fail("variant arity exceeds runtime range"))?;
                        let kind = if elements.is_empty() {
                            VariantKind::Unit
                        } else {
                            VariantKind::Tuple
                        };
                        if actual.kind != kind || actual.arity != arity {
                            return Err(fail(
                                "runtime tuple variant kind or arity differs from declaration",
                            ));
                        }
                        check_fields(
                            &actual.fields,
                            (0..elements.len()).map(|index| format!("_{index}")),
                            strings,
                        )
                        .map_err(|message| fail(message.as_str()))?;
                    }
                    Some(VariantData::Record(fields)) => {
                        if actual.kind != VariantKind::Record || actual.arity != 0 {
                            return Err(fail(
                                "runtime record variant kind or arity differs from declaration",
                            ));
                        }
                        check_fields(
                            &actual.fields,
                            fields.iter().map(|field| field.name.name.to_string()),
                            strings,
                        )
                        .map_err(|message| fail(message.as_str()))?;
                    }
                }
            }
        }
        _ => {
            if !descriptor.fields.is_empty() {
                return Err(fail("declaration has unexpected runtime fields"));
            }
        }
    }
    Ok(())
}

fn check_fields(
    actual: &[FieldDescriptor],
    expected: impl Iterator<Item = String>,
    strings: &StringTable,
) -> Result<(), Text> {
    let mut actual = actual.iter();
    let mut names = Set::new();
    for name in expected {
        let field = actual
            .next()
            .ok_or_else(|| Text::from("missing runtime field"))?;
        if strings.get(field.name) != Some(name.as_str()) || !names.insert(Text::from(name)) {
            return Err("runtime field name or order differs from declaration".into());
        }
    }
    if actual.next().is_some() {
        return Err("extra runtime field".into());
    }
    Ok(())
}
