//! Callable spellings must establish their contract before metadata is admitted.
use super::CoreMetadata;
use verum_ast::{
    FileId, TypeKind,
    ty::GenericParamKind,
    visitor::{Visitor, walk_type},
};
use verum_common::{List, Text};

/// The descriptor grammar shares the source parser; internal projections are
/// its explicit archive mode, not a second callable grammar.
pub fn validate_archive_callable_spelling(spelling: &str, has_source: bool) -> Result<(), Text> {
    if !spelling.contains("fn") {
        return Ok(());
    }
    let tokens = verum_lexer::Lexer::new(spelling, FileId::new(0))
        .collect::<Result<List<_>, _>>()
        .map_err(|_| Text::from("invalid token in archive callable spelling"))?;
    if !tokens
        .iter()
        .any(|token| matches!(token.kind, verum_lexer::TokenKind::Fn))
    {
        return Ok(());
    }
    let ty = verum_fast_parser::FastParser::new()
        .parse_descriptor_type_str(spelling, FileId::new(0))
        .map_err(|_| Text::from("invalid archive callable type; rebuild its metadata"))?;
    struct Contracts {
        needs_source: bool,
        unsupported_abi: bool,
    }
    impl Visitor for Contracts {
        fn visit_type(&mut self, ty: &verum_ast::Type) {
            match &ty.kind {
                TypeKind::Function {
                    contexts,
                    calling_convention,
                    ..
                } => {
                    self.needs_source |= !contexts.is_empty();
                    self.unsupported_abi |= calling_convention.is_some();
                }
                TypeKind::Rank2Function {
                    contexts,
                    calling_convention,
                    type_params,
                    where_clause,
                    ..
                } => {
                    self.unsupported_abi |= calling_convention.is_some();
                    self.needs_source |= !contexts.is_empty()
                        || where_clause.is_some()
                        || type_params.iter().any(|parameter| {
                            !matches!(
                            &parameter.kind, GenericParamKind::Type { bounds, default, .. }
                            if bounds.is_empty() && default.is_none())
                        });
                }
                _ => (),
            }
            walk_type(self, ty);
        }
    }
    let mut contracts = Contracts {
        needs_source: false,
        unsupported_abi: false,
    };
    contracts.visit_type(&ty);
    if contracts.unsupported_abi {
        return Err("archive callable ABI requires a typed calling-convention carrier".into());
    }
    if contracts.needs_source && !has_source {
        return Err(
            "archive callable contract requires its source declaration and declaring scope".into(),
        );
    }
    Ok(())
}

pub(super) fn validate_archive_callables(metadata: &CoreMetadata) -> Result<(), Text> {
    super::array_extents::visit_archive_type_spellings(metadata, validate_archive_callable_spelling)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_metadata::{
        CORE_METADATA_VERSION, StaticDescriptor, decode_metadata, encode_metadata,
    };

    fn metadata(spelling: &str) -> CoreMetadata {
        let mut metadata = CoreMetadata {
            version: CORE_METADATA_VERSION,
            ..Default::default()
        };
        metadata.statics.insert(
            "sample.callback".into(),
            StaticDescriptor {
                name: "callback".into(),
                module_path: "sample".into(),
                ty: spelling.into(),
                is_mut: false,
            },
        );
        metadata
    }

    #[test]
    fn nested_eager_callables_and_internal_projections_use_the_same_grammar() {
        for spelling in [
            "async fn(Int) -> Bool",
            "(Int, &async fn() -> Int)",
            "Maybe<fn(async fn(Int) -> Bool) -> Int>",
            "async fn(__generic_0) -> ::Item<__generic_0>",
        ] {
            let bytes = encode_metadata(&metadata(spelling)).unwrap();
            assert!(decode_metadata(&bytes).is_ok(), "{spelling}");
        }
    }

    #[test]
    fn malformed_nested_callables_never_become_nominal_names() {
        for spelling in [
            "async fn(Int",
            "Maybe<async fn(Int) ->>",
            "async fn() -> Int trailing",
            "fn() -> async fn(,)",
        ] {
            assert!(encode_metadata(&metadata(spelling)).is_err(), "{spelling}");
            assert!(
                crate::infer::parse_descriptor_type_string(spelling).is_err(),
                "{spelling}"
            );
        }
    }

    #[test]
    fn scope_dependent_contracts_require_source_authority() {
        for spelling in [
            "async fn() -> Int using [Clock]",
            "Maybe<fn<T: Bound>(T) -> T>",
        ] {
            let error = encode_metadata(&metadata(spelling)).unwrap_err();
            assert!(error.contains("source declaration"), "{error}");
            assert!(validate_archive_callable_spelling(spelling, true).is_ok());
        }
        assert!(validate_archive_callable_spelling("core.fn_like.Type", false).is_ok());
    }

    #[test]
    fn source_presence_cannot_authorize_erasing_a_foreign_calling_convention() {
        for spelling in [
            "extern \"C\" fn(Int) -> Int",
            "Maybe<extern \"system\" fn(Int) -> Int>",
        ] {
            for has_source in [false, true] {
                let error = validate_archive_callable_spelling(spelling, has_source).unwrap_err();
                assert!(error.contains("calling-convention carrier"), "{error}");
            }
            assert!(crate::infer::parse_descriptor_type_string(spelling).is_err());
        }
    }
}
