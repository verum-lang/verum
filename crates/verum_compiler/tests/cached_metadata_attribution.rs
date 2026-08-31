//! A type recovered from the metadata cache must keep its DECLARER
//! (T1004).
//!
//! `convert_cached_metadata_to_stdlib` wrote
//! `origin_module_path: Maybe::None` for every type, unconditionally —
//! not a missing arm but a policy, and the exact defect T1002 fixed on
//! the codegen side: the type loses its declarer and is attributed to the
//! entry path instead. Nothing calls this conversion today, which is why
//! it went unnoticed; the next caller would have inherited the regression
//! whole, and T1002's guard watches the codegen path, not this one.
//!
//! The declarer is recoverable and always was: the cache builds each
//! entry's path as `<declaring module>.<name>`, so the prefix IS the
//! declarer — the same string the conversion already derived one line
//! above for `module_path`.

use verum_compiler::core_cache::{CachedCoreMetadata, CachedTypeEntry};
use verum_compiler::pipeline::CompilationPipeline;

fn cached_type(path: &str, kind: &str) -> CachedTypeEntry {
    CachedTypeEntry {
        path: path.to_string(),
        definition: format!("public type {} is {{ x: Int }};", path.rsplit('.').next().unwrap()),
        kind: kind.to_string(),
    }
}

fn convert(entries: Vec<CachedTypeEntry>) -> verum_types::core_metadata::CoreMetadata {
    // Every field spelled out: `CachedCoreMetadata` has no `Default`, and
    // adding one to reach for `..Default::default()` would widen a dead
    // subsystem's API for a test's convenience.
    let cached = CachedCoreMetadata {
        types: entries,
        functions: Vec::new(),
        modules: Vec::new(),
        context_declarations: Vec::new(),
        meta_functions: Vec::new(),
        macros: Vec::new(),
        derives: Vec::new(),
    };
    CompilationPipeline::convert_cached_metadata_to_stdlib(&cached)
}

#[test]
fn a_cached_type_keeps_its_declaring_module() {
    let meta = convert(vec![cached_type("core.base.text.Rope", "struct")]);
    let ty = meta
        .types
        .values()
        .find(|t| t.name.as_str() == "Rope")
        .expect("the type survives the conversion");
    assert_eq!(
        ty.origin_module_path.as_ref().map(|m| m.as_str()),
        Some("core.base.text"),
        "the declarer is the path's prefix; `None` here is what attributes \
         the type to whatever module mounts it"
    );
}

#[test]
fn the_declarer_and_the_module_path_agree() {
    // They are derived from the same string, so a change that moves one
    // and not the other is the drift this pins.
    let meta = convert(vec![cached_type("core.security.tuf.types.KeyId", "struct")]);
    let ty = meta
        .types
        .values()
        .find(|t| t.name.as_str() == "KeyId")
        .expect("the type survives the conversion");
    assert_eq!(ty.module_path.as_str(), "core.security.tuf.types");
    assert_eq!(
        ty.origin_module_path.as_ref().map(|m| m.as_str()),
        Some("core.security.tuf.types")
    );
}

#[test]
fn a_bare_name_with_no_module_claims_no_declarer() {
    // The control for the fix's own edge: with nothing before the name
    // there is no declarer to record, and inventing an empty one would be
    // worse than admitting none.
    let meta = convert(vec![cached_type("Standalone", "struct")]);
    let ty = meta
        .types
        .values()
        .find(|t| t.name.as_str() == "Standalone")
        .expect("the type survives the conversion");
    assert!(
        ty.origin_module_path.is_none(),
        "no prefix means no declarer, not an empty one"
    );
}
