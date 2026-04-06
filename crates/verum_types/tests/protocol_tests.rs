#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// NOTE: These tests have been superseded by protocol_comprehensive_tests.rs
// which uses the current stable public API of ProtocolChecker.
//
// The tests below were written against an older internal API that has since changed.
// Rather than rewriting them, we've consolidated protocol testing in the comprehensive
// test suite which provides better coverage and uses only public methods.

// use verum_types::protocol::*;
// use verum_std::collections::Map;
// use verum_std::core::Text;

// #[test]
// fn test_protocol_registration() {
//     let mut checker = ProtocolChecker::new();
//
//     assert!(checker.protocols.get(&Text::from("Eq")).is_some());
//     assert!(checker.protocols.get(&Text::from("Ord")).is_some());
//     assert!(checker.protocols.get(&Text::from("Show")).is_some());
// }

// #[test]
// fn test_protocol_impl() {
//     let mut checker = ProtocolChecker::new();
//
//     let impl_ = ProtocolImpl {
//         protocol: Text::from("Eq"),
//         for_type: Text::from("Int"),
//         methods: Map::new(),
//     };
//
//     checker.register_impl(impl_);
//     assert!(checker.implements(&"Int", &"Eq"));
//     assert!(!checker.implements(&"Int", &"NonExistent"));
// }
