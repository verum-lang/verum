//! T2178: an unreadable module cannot become a successful partial import.
use super::{ArchiveCtxCache, CtxLoadError, SymbolGraph, populate_ctx_from_archive};
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcError, VbcModule,
    archive::{ArchiveBuilder, VbcArchive},
    codegen::{CodegenConfig, CodegenContext, VbcCodegen},
    module::FunctionDescriptor,
    serialize::serialize_module,
    types::{TypeParamId, TypeRef},
};

#[derive(Clone, Copy, Debug)]
enum Damage {
    LegacyRank2,
    Truncated,
}

fn good_module(name: &str) -> VbcModule {
    let ast = Parser::new("public type Token is { value: Int }; public fn value()->Int { 7 }")
        .parse_module()
        .unwrap();
    VbcCodegen::with_config(CodegenConfig::new(name))
        .compile_module(&ast)
        .unwrap()
}

fn archive(damage: Option<Damage>, compressed: bool, broken_first: bool) -> VbcArchive {
    let mut builder = ArchiveBuilder::new();
    if compressed {
        builder = builder.with_compression();
    }
    let mut broken = VbcModule::new("broken".into());
    let mut callable = FunctionDescriptor::new(broken.intern_string("value"));
    callable.return_type = TypeRef::Rank2Function {
        type_param_count: 1,
        params: vec![TypeRef::Generic(TypeParamId(0))],
        return_type: Box::new(TypeRef::Generic(TypeParamId(0))),
        contexts: Default::default(),
    };
    broken.add_function(callable);
    let mut bytes = serialize_module(&broken).unwrap();
    if let Some(damage) = damage {
        // The outer container and older module header remain readable; the
        // semantic payload is either ambiguous or incomplete.
        bytes[6..8].copy_from_slice(&34u16.to_le_bytes());
        match damage {
            Damage::LegacyRank2 => {
                let prefix = [0x0c, 1, 0, 1, 0x02, 0, 0];
                let positions: Vec<_> = bytes
                    .windows(prefix.len())
                    .enumerate()
                    .filter_map(|(i, window)| (window == prefix).then_some(i))
                    .collect();
                assert_eq!(positions.len(), 1);
                bytes[positions[0]] = 0x09;
            }
            Damage::Truncated => bytes.truncate(8),
        }
    }
    if !broken_first {
        builder
            .add_module("good", &good_module("good"), &[])
            .unwrap();
    }
    builder.add_module_data("broken", bytes, &[]).unwrap();
    if broken_first {
        builder
            .add_module("good", &good_module("good"), &[])
            .unwrap();
    }
    builder.finish()
}

fn check_error(error: CtxLoadError, damage: Damage) {
    let CtxLoadError::ModuleDecode {
        ref module,
        archive_schema,
        module_schema,
        ref source,
    } = error;
    assert_eq!(module, "broken");
    assert_eq!(archive_schema, (1, 0));
    assert_eq!(module_schema, Some((2, 34)));
    if matches!(damage, Damage::LegacyRank2) {
        assert!(matches!(source.as_ref(), VbcError::InvalidTypeRefTag(0x09)));
    }
    let diagnostic = error.to_string();
    assert!(diagnostic.contains("broken"));
    assert!(diagnostic.contains("VBCA 1.0"));
    assert!(diagnostic.contains("VBC 2.34"));
    assert!(std::error::Error::source(&error).is_some());
}

fn consumer() -> verum_ast::Module {
    Parser::new(
        "mount good.value; mount broken.value; fn main()->Int { good.value() + broken.value() }",
    )
    .parse_module()
    .unwrap()
}

#[test]
fn every_import_route_propagates_the_actual_module_decode_failure() {
    for damage in [Damage::LegacyRank2, Damage::Truncated] {
        for compressed in [false, true] {
            for broken_first in [false, true] {
                let archive = archive(Some(damage), compressed, broken_first);
                let ast = consumer();
                let cache = ArchiveCtxCache::new();
                let mut ctx = CodegenContext::new();
                let mut next_id = 41;
                check_error(
                    populate_ctx_from_archive(&archive, &mut ctx, &mut next_id).unwrap_err(),
                    damage,
                );
                // Direct preload owns a disposable compilation context. The
                // shared cache below must not publish its partial state.
                let mut ctx = CodegenContext::new();
                check_error(cache.get_or_build(&archive).unwrap_err(), damage);
                check_error(cache.apply(&archive, &mut ctx).unwrap_err(), damage);
                check_error(
                    cache
                        .apply_lazy(&archive, &mut ctx, &ast, &mut next_id)
                        .unwrap_err(),
                    damage,
                );
                let mut codegen = VbcCodegen::new();
                check_error(
                    ArchiveCtxCache::import_types_for_module(&archive, &mut codegen, &ast)
                        .unwrap_err(),
                    damage,
                );
                check_error(
                    cache
                        .apply_lazy_with_types(&archive, &mut codegen, &ast, &[])
                        .unwrap_err(),
                    damage,
                );
                check_error(
                    cache
                        .apply_lazy_supplemental(
                            &archive,
                            &mut codegen,
                            ["broken.value".to_owned()].into_iter().collect(),
                        )
                        .unwrap_err(),
                    damage,
                );
                check_error(
                    SymbolGraph::scan_and_encode(&archive)
                        .err()
                        .expect("graph must refuse"),
                    damage,
                );
            }
        }
    }
}

#[test]
fn failed_cache_initialization_never_publishes_a_partial_table_or_graph() {
    let archive = archive(Some(Damage::LegacyRank2), false, false);
    let cache = ArchiveCtxCache::new();
    for _ in 0..2 {
        check_error(
            cache.get_or_build(&archive).unwrap_err(),
            Damage::LegacyRank2,
        );
        check_error(
            cache.graph(&archive).err().expect("no partial graph"),
            Damage::LegacyRank2,
        );
        assert!(matches!(cache.table.get(), Some(Err(_))));
        assert!(matches!(cache.graph.get(), Some(Err(_))));
        let mut ctx = CodegenContext::new();
        check_error(
            cache.apply(&archive, &mut ctx).unwrap_err(),
            Damage::LegacyRank2,
        );
        assert!(ctx.lookup_function("good.value").is_none());
    }
}

#[test]
fn valid_scoped_modules_keep_the_cached_fast_path() {
    for compressed in [false, true] {
        let archive = archive(None, compressed, false);
        let cache = ArchiveCtxCache::new();
        let table = cache.get_or_build(&archive).unwrap();
        assert!(table.contains_key("good.value"));
        assert!(table.contains_key("broken.value"));
        assert!(std::ptr::eq(table, cache.get_or_build(&archive).unwrap()));
        let graph = cache.graph(&archive).unwrap();
        assert!(std::ptr::eq(graph, cache.graph(&archive).unwrap()));
    }
}
