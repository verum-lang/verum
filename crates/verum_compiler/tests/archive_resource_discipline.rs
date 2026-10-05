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
