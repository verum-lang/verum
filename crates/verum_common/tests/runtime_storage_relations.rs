//! Retained payloads and inherited resource requirements are separate relations.
use verum_common::well_known_types::WellKnownType;

#[test]
fn allocation_handles_retain_payloads_without_inheriting_their_resource_mode() {
    for carrier in [WellKnownType::Heap, WellKnownType::Shared] {
        assert_eq!(carrier.retained_storage_parameters(), &[0]);
        assert!(carrier.owned_storage_parameters().is_empty());
    }
}

#[test]
fn owned_container_values_participate_in_both_relations() {
    for (carrier, parameters) in [
        (WellKnownType::List, &[0][..]), (WellKnownType::Map, &[0, 1][..]),
        (WellKnownType::Set, &[0][..]), (WellKnownType::Deque, &[0][..]),
    ] {
        assert_eq!(carrier.retained_storage_parameters(), parameters);
        assert_eq!(carrier.owned_storage_parameters(), parameters);
    }
}

#[test]
fn unregistered_handle_spelling_has_no_declaration_authority() {
    for name in ["Heap", "Shared"] {
        assert!(WellKnownType::from_declaration(name, "application").is_none());
        assert!(WellKnownType::from_declaration(name, "core.base.memory").is_some());
    }
}
