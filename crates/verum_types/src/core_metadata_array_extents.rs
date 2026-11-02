//! Admission of fixed extents in textual archive type descriptors.
use self::validate_archive_array_spelling as check;
use super::{
    AssociatedTypeDescriptor, CoreMetadata, GenericParam, MethodSignature, ParamDescriptor,
    TypeDescriptorKind, VariantPayload,
};
use verum_common::Text;

/// The archive writer emits decimal lengths from the checked signed count domain.
/// An unresolved source expression cannot establish a fixed ABI extent.
pub fn parse_archive_array_extent(extent: &str) -> Result<usize, Text> {
    let extent = extent.trim();
    if extent.is_empty() || !extent.bytes().all(|b| b.is_ascii_digit()) {
        return Err("archive array extent requires a nonnegative decimal constant".into());
    }
    extent
        .parse::<i64>()
        .ok()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| "archive array extent exceeds the supported count range".into())
}

/// Check every fixed dimension in an archive spelling before parsing its type.
pub fn validate_archive_array_spelling(spelling: &str) -> Result<(), Text> {
    if spelling
        .bytes()
        .any(|b| b.is_ascii_control() && !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
    {
        return Err("control byte in archive type spelling".into());
    }
    if !spelling.contains(['[', ']']) {
        return Ok(());
    }
    check_brackets(spelling).map_err(|error| {
        let preview: String = spelling.chars().take(160).collect();
        Text::from(format!("{error} in `{preview}`"))
    })
}

fn check_brackets(spelling: &str) -> Result<(), Text> {
    // Bound the stack and visit every dimension once, including those inside
    // generic arguments, function signatures and other fixed arrays.
    let mut brackets = Vec::new();
    let mut string = false;
    let mut escaped = false;
    let mut delimiters = 0usize;
    let mut chars = spelling.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                string = false;
            }
            continue;
        }
        match ch {
            '"' => string = true,
            '\'' => {
                // A character literal may contain a delimiter. A lifetime
                // has no closing quote and remains part of the type.
                let mut literal = chars.clone();
                if let Some((_, c)) = literal.next() {
                    if c == '\\' {
                        literal.next();
                    }
                    if literal.next().is_some_and(|(_, c)| c == '\'') {
                        chars = literal;
                    }
                }
            }
            '(' | '{' => delimiters += 1,
            ')' | '}' => delimiters = delimiters.saturating_sub(1),
            '[' => {
                if brackets.len() >= 128 {
                    return Err("archive array nesting exceeds its depth budget".into());
                }
                brackets.push((i + 1, delimiters, None));
            }
            ';' => {
                if let Some((_, depth, separator)) = brackets.last_mut()
                    && *depth == delimiters
                    && separator.replace(i).is_some()
                {
                    return Err("archive array has more than one extent separator".into());
                }
            }
            ']' => {
                let Some((start, depth, separator)) = brackets.pop() else {
                    return Err("unmatched closing bracket in archive type".into());
                };
                if delimiters != depth {
                    return Err("unbalanced delimiters within archive array type".into());
                }
                if let Some(split) = separator {
                    if spelling[start..split].trim().is_empty() {
                        return Err("archive array has no element type".into());
                    }
                    parse_archive_array_extent(&spelling[split + 1..i])?;
                }
            }
            _ => (),
        }
    }
    if !brackets.is_empty() {
        return Err("unclosed bracket in archive type".into());
    }
    Ok(())
}

fn generics(params: &[GenericParam]) -> Result<(), Text> {
    for param in params {
        for ty in param
            .bounds
            .iter()
            .chain(&param.type_bounds)
            .chain(&param.default)
        {
            check(ty)?;
        }
    }
    Ok(())
}

fn signature(params: &[ParamDescriptor], result: &str) -> Result<(), Text> {
    for param in params {
        check(&param.ty)?;
        check(&param.declared_ty)?;
    }
    check(result)
}

fn methods(declarations: &[MethodSignature]) -> Result<(), Text> {
    for method in declarations {
        signature(&method.params, &method.return_type)?;
    }
    Ok(())
}

fn associated(declarations: &[AssociatedTypeDescriptor]) -> Result<(), Text> {
    for decl in declarations {
        for ty in decl.bounds.iter().chain(&decl.default) {
            check(ty)?;
        }
    }
    Ok(())
}

/// Validate all descriptor type strings before a consumer can cache a scheme.
pub fn validate_archive_array_extents(metadata: &CoreMetadata) -> Result<(), Text> {
    for decl in metadata.types.values() {
        generics(&decl.generic_params)?;
        for ty in &decl.implements {
            check(ty)?;
        }
        match &decl.kind {
            TypeDescriptorKind::Record { fields } => {
                for field in fields {
                    check(&field.ty)?;
                }
            }
            TypeDescriptorKind::Variant { cases } => {
                for payload in cases.iter().filter_map(|case| case.payload.as_ref()) {
                    match payload {
                        VariantPayload::Tuple(items) => {
                            for ty in items {
                                check(ty)?;
                            }
                        }
                        VariantPayload::Record(fields) => {
                            for field in fields {
                                check(&field.ty)?;
                            }
                        }
                    }
                }
            }
            TypeDescriptorKind::Protocol {
                super_protocols,
                associated_types,
                required_methods,
                ..
            } => {
                for ty in super_protocols {
                    check(ty)?;
                }
                associated(associated_types)?;
                methods(required_methods)?;
            }
            TypeDescriptorKind::Alias { target } => check(target)?,
            TypeDescriptorKind::Opaque => (),
        }
    }
    for protocol in metadata.protocols.values() {
        generics(&protocol.generic_params)?;
        for ty in &protocol.super_protocols {
            check(ty)?;
        }
        associated(&protocol.associated_types)?;
        methods(&protocol.required_methods)?;
        methods(&protocol.default_methods)?;
    }
    for function in metadata.functions.values() {
        generics(&function.generic_params)?;
        signature(&function.params, &function.return_type)?;
    }
    for decl in &metadata.implementations {
        for ty in decl
            .protocol_args
            .iter()
            .chain(decl.associated_types.values())
        {
            check(ty)?;
        }
        check(&decl.protocol)?;
        check(&decl.target_type)?;
        generics(&decl.generic_params)?;
    }
    for decl in metadata.statics.values() {
        check(&decl.ty)?;
    }
    for mono in metadata.monomorphizations.values() {
        for ty in &mono.type_args {
            check(ty)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_metadata::{StaticDescriptor, decode_metadata, encode_metadata};

    fn with_static(ty: &str) -> CoreMetadata {
        let mut metadata = CoreMetadata {
            version: super::super::CORE_METADATA_VERSION,
            ..CoreMetadata::default()
        };
        metadata.statics.insert(
            "sample.BLOCK".into(),
            StaticDescriptor {
                name: "BLOCK".into(),
                module_path: "sample".into(),
                ty: ty.into(),
                is_mut: false,
            },
        );
        metadata
    }

    #[test]
    fn checked_array_spellings_validate_at_each_dimension() {
        for spelling in [
            "[Int]",
            "[Int; 0]",
            "fn([[Byte; 2]; 4]) -> Maybe<&[Byte; 8]>",
        ] {
            let bytes = encode_metadata(&with_static(spelling)).unwrap();
            assert!(decode_metadata(&bytes).is_ok(), "{spelling}");
        }
        for spelling in [
            "[Int; -1]",
            "[Int; ]",
            "[; 9]",
            "[Int; 1; 2]",
            "[Int; 9x]",
            "[Byte; 9223372036854775808]",
            "Maybe<[Int; N]>",
            "&[[Int; 1]; 18446744073709551616]",
            "Tuple<[[Byte; 2], [Byte; -1]]>",
            "[Int; 8",
            "[Int(; 2]",
        ] {
            assert!(
                encode_metadata(&with_static(spelling)).is_err(),
                "{spelling}"
            );
        }
    }

    #[test]
    fn raw_installs_and_prepared_snapshots_refuse_a_poisoned_impl_bound() {
        use crate::core_metadata::ImplementationDescriptor;
        use crate::{AnalysisControl, CoreMetadataSnapshot, TypeChecker};
        use std::sync::Arc;

        let mut metadata = with_static("[Int; 2]");
        metadata.implementations.push(ImplementationDescriptor {
            protocol: "P".into(),
            target_type: "Record".into(),
            generic_params: vec![GenericParam {
                name: "T".into(),
                bounds: Default::default(),
                default: None,
                type_bounds: vec!["\x1b[?1003h".into()].into(),
                pid: None,
            }]
            .into(),
            where_clause: Default::default(),
            associated_types: Default::default(),
            methods: Default::default(),
            protocol_args: Default::default(),
        });
        assert!(encode_metadata(&metadata).is_err());
        let metadata = Arc::new(metadata);
        for mut checker in [
            TypeChecker::new_with_core(metadata.clone()),
            TypeChecker::new_with_core_eager(metadata.clone()),
        ] {
            assert!(
                checker
                    .take_deferred_errors()
                    .iter()
                    .any(|e| e.to_string().contains("archive type"))
            );
        }
        assert!(CoreMetadataSnapshot::prepare(metadata, &AnalysisControl::default()).is_err());
    }

    #[test]
    fn decoded_bytes_cannot_supply_a_poisoned_inner_extent() {
        let needle = b"[Byte; 19]";
        let clean = encode_metadata(&with_static("&[[Byte; 19]; 4]")).unwrap();
        let i = clean
            .windows(needle.len())
            .position(|w| w == needle)
            .unwrap();
        for replacement in [b"-1", b"NN", b"1x"] {
            let mut bytes = clean.clone();
            bytes[i + 7..i + 9].copy_from_slice(replacement);
            assert!(decode_metadata(&bytes).unwrap_err().contains("extent"));
        }
    }
}
