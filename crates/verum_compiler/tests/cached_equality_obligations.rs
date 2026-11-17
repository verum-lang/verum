//! A legacy text cache cannot publish current metadata after erasing equations.
use verum_compiler::{
    core_cache::{CachedCoreMetadata, CachedFunctionEntry, CachedTypeEntry},
    pipeline::CompilationPipeline,
};
use verum_types::core_metadata::CORE_METADATA_VERSION;

fn cached(function: Option<&str>, ty: Option<&str>) -> CachedCoreMetadata {
    CachedCoreMetadata {
        functions: function
            .into_iter()
            .map(|signature| CachedFunctionEntry {
                path: "provider.choose".into(),
                signature: signature.into(),
                is_intrinsic: false,
                intrinsic_name: None,
            })
            .collect(),
        types: ty
            .into_iter()
            .map(|definition| CachedTypeEntry {
                path: "provider.Holder".into(),
                definition: definition.into(),
                kind: "record".into(),
            })
            .collect(),
        modules: Default::default(),
        context_declarations: Default::default(),
        meta_functions: Default::default(),
        macros: Default::default(),
        derives: Default::default(),
    }
}

#[test]
fn cached_callables_require_a_carrier_for_their_declared_equations() {
    let ordinary = cached(Some("public fn choose(value:Int)->Int { value }"), None);
    let metadata = CompilationPipeline::convert_cached_metadata_to_stdlib(&ordinary).unwrap();
    assert_eq!(metadata.version, CORE_METADATA_VERSION);
    assert_eq!(metadata.functions.len(), 1);
    for source in [
        "public fn choose<T>(value:T)->Int where T=Int { 0 }",
        "public fn choose<F:Family>(value:F)->Int where F.Item=Int { 0 }",
        "public fn choose(value:Int)->Int where Int=Bool { 0 }",
        "public fn choose<F:fn(Int)->Int>(value:F)->Int { 0 }",
        "public fn choose<F>(value:F)->Int where F=fn(Int)->Int { 0 }",
    ] {
        let error =
            CompilationPipeline::convert_cached_metadata_to_stdlib(&cached(Some(source), None))
                .unwrap_err()
                .to_string();
        assert!(
            error.contains("required source signature") && error.contains("provider.choose"),
            "{source}: {error}"
        );
    }
}

#[test]
fn cached_record_domains_still_require_their_source_declaration() {
    for source in [
        "public type Holder<T> where type T=Int is { value:T };",
        "public type Holder<T> where type T:Family is { value:T };",
    ] {
        let error =
            CompilationPipeline::convert_cached_metadata_to_stdlib(&cached(None, Some(source)))
                .unwrap_err()
                .to_string();
        assert!(
            error.contains("semantic") && error.contains("rebuild from source"),
            "{error}"
        );
    }
}
