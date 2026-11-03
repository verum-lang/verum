//! T2179: archive ownership is fixed before any table or graph can be cached.
use super::ArchiveCtxCache;
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    archive::{ArchiveBuilder, VbcArchive},
    codegen::{CodegenConfig, VbcCodegen},
};

fn archive(owner: &str, value: i64) -> VbcArchive {
    let source = format!("public fn value()->Int {{ {value} }}");
    let ast = Parser::new(&source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast)
        .unwrap();
    let mut builder = ArchiveBuilder::new();
    builder.add_module(owner, &module, &[]).unwrap();
    builder.finish()
}

fn execute(cache: &ArchiveCtxCache<'_>, owner: &str) -> i64 {
    let ast = Parser::new(&format!("fn main()->Int {{ {owner}.value() }}"))
        .parse_module()
        .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    cache
        .apply_lazy_with_types(&mut codegen, &ast, &[])
        .unwrap();
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&ast], verum_vbc::codegen::ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = Arc::new(verum_vbc::deserialize::deserialize_module(&bytes).unwrap());
    let id = module.find_function_by_name("consumer.main").unwrap();
    let value = verum_vbc::interpreter::Interpreter::new(module)
        .execute_function(id)
        .unwrap();
    assert!(value.is_int());
    value.as_i64()
}

#[test]
fn borrowed_and_shared_sessions_keep_distinct_archive_owners() {
    let alpha = archive("alpha", 17);
    let beta = Arc::new(archive("beta", 29));
    let a = ArchiveCtxCache::new(&alpha);
    let b = ArchiveCtxCache::from_shared(beta.clone());
    assert!(std::ptr::eq(a.archive(), &alpha));
    assert!(std::ptr::eq(b.archive(), beta.as_ref()));
    assert_eq!(Arc::strong_count(&beta), 2);
    drop(beta);
    for _ in 0..2 {
        assert!(a.get_or_build().unwrap().contains_key("alpha.value"));
        assert!(!a.get_or_build().unwrap().contains_key("beta.value"));
        assert!(b.get_or_build().unwrap().contains_key("beta.value"));
        assert!(!b.get_or_build().unwrap().contains_key("alpha.value"));
        assert_eq!(execute(&a, "alpha"), 17);
        assert_eq!(execute(&b, "beta"), 29);
    }
}

#[test]
fn failed_shared_session_cannot_inherit_another_sessions_success() {
    let good = archive("good", 37);
    let a = ArchiveCtxCache::new(&good);
    a.get_or_build().unwrap();
    a.graph().unwrap();
    let mut broken = archive("broken", 41);
    broken.module_data[0].truncate(8);
    let b = ArchiveCtxCache::from_shared(Arc::new(broken));
    for _ in 0..2 {
        assert!(b.get_or_build().is_err());
        assert!(b.graph().is_err());
        assert!(a.get_or_build().unwrap().contains_key("good.value"));
        assert_eq!(execute(&a, "good"), 37);
    }
}

#[test]
fn project_revisions_with_identical_names_keep_their_own_bodies() {
    let first = archive("project", 41);
    let second = Arc::new(archive("project", 73));
    let a = ArchiveCtxCache::new(&first);
    let b = ArchiveCtxCache::from_shared(second);
    for _ in 0..2 {
        assert!(a.get_or_build().unwrap().contains_key("project.value"));
        assert!(b.get_or_build().unwrap().contains_key("project.value"));
        assert_eq!(execute(&a, "project"), 41);
        assert_eq!(execute(&b, "project"), 73);
    }
}
