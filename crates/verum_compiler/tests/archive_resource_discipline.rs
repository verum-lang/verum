//! T1594: the actual archive converter preserves declaration-owned constraints.
#[path = "fixtures/resource_metadata_probe.rs"]
mod probe;
use verum_compiler::archive_metadata::archive_to_core_metadata;
#[test]
fn exact_owners_survive_eager_and_lazy_metadata_loading() {
    probe::exact_owners(archive_to_core_metadata);
}
#[test]
fn imported_aliases_and_siblings_keep_consumption_rules() {
    probe::imported_consumption(archive_to_core_metadata);
}
#[test]
fn archive_field_metadata_preserves_borrow_qualifiers() {
    probe::borrowed_field(archive_to_core_metadata);
}
#[test]
fn promoted_descriptor_does_not_duplicate_declaring_owner() {
    probe::promoted_owner(archive_to_core_metadata);
}

#[test]
fn generic_imported_aliases_preserve_owner_and_substitution() {
    probe::generic_aliases(archive_to_core_metadata);
}

#[test]
fn imported_generic_constructor_keeps_its_declaring_owner() {
    probe::imported_generic_constructor_owner(archive_to_core_metadata);
}

#[test]
fn embedded_collection_constructor_keeps_its_declaring_owner() {
    let metadata = verum_compiler::embedded_stdlib_metadata::get_runtime_metadata()
        .expect("normal compiler build embeds the standard-library metadata");
    probe::stdlib_collection_constructor_owner(metadata);
}

#[test]
fn constructor_reconnection_preserves_qualified_siblings_and_generic_names() {
    probe::constructor_owner_boundaries(archive_to_core_metadata);
}
