#![cfg(feature = "codegen")]
//! T2869: old source AST cannot decide whether a callable was ':' or '='.
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    format::SOURCE_CONSTRAINT_CLASSIFICATION_VERSION,
    serialize::{serialize_archive_member, serialize_module},
};

fn interface(source: &str) -> VbcModule {
    let ast = Parser::new(&format!("module schema; {source}"))
        .parse_module()
        .expect("source grammar");
    VbcCodegen::with_config(CodegenConfig::new("schema"))
        .collect_declaration_interface(&[&ast])
        .expect("actual declaration producer")
}

fn current_and_old(module: &VbcModule, requires_distinction: bool) {
    for mut bytes in [
        serialize_module(module).unwrap(),
        serialize_archive_member(module).unwrap(),
    ] {
        let current = deserialize_module(&bytes).expect("current source schema");
        assert_eq!(
            current.header.version_minor,
            SOURCE_CONSTRAINT_CLASSIFICATION_VERSION
        );
        // 2.63 and 2.64 have the same physical layout. This intentionally
        // removes only the producer's claim about the source AST's meaning.
        bytes[6..8].copy_from_slice(&(SOURCE_CONSTRAINT_CLASSIFICATION_VERSION - 1).to_le_bytes());
        let old = deserialize_module(&bytes);
        if requires_distinction {
            let error = old.unwrap_err().to_string();
            assert!(
                error.contains("constraint classification") && error.contains("rebuild"),
                "{error}"
            );
        } else {
            old.expect("unambiguous source carriers remain readable");
        }
    }
}

#[test]
fn callable_function_bounds_and_equations_require_the_distinguishing_schema() {
    for source in [
        "public fn accept<F:fn(Int)->Int>(item:F)->Int {0}",
        "public fn accept<F>(item:F)->Int where F=fn(Int)->Int {0}",
        "public fn accept<F:fn<T>(T)->T>(item:F)->Int {0}",
        "public fn accept<F>(item:F)->Int where F=fn<T>(T)->T {0}",
    ] {
        let module = interface(source);
        assert!(
            module
                .functions
                .iter()
                .any(|function| function.source_signature.is_some())
        );
        current_and_old(&module, true);
    }
}

#[test]
fn nominal_headers_and_nested_protocol_methods_need_the_same_source_schema() {
    for source in [
        "public type Slot<F:fn(Int)->Int> is {value:F};",
        "public type Source is protocol {fn accept<F>(&self,item:F)->Int where F=fn(Int)->Int;};",
    ] {
        let module = interface(source);
        assert!(
            module
                .types
                .iter()
                .any(|descriptor| descriptor.source_declaration.is_some())
        );
        current_and_old(&module, true);
    }
}

#[test]
fn inherent_and_protocol_implementation_headers_cannot_bypass_the_schema() {
    for declaration in [
        "implement<F:fn(Int)->Int> Slot<F> {}",
        "implement<F:fn(Int)->Int> Marker for Slot<F> {}",
    ] {
        let module = interface(&format!(
            "public type Slot<F> is {{value:F}}; public type Marker is protocol {{}}; {declaration}"
        ));
        assert!(module.types.iter().any(|descriptor| {
            !descriptor.source_inherent_implementations.is_empty()
                || descriptor
                    .protocols
                    .iter()
                    .any(|implementation| implementation.source_implementation.is_some())
        }));
        current_and_old(&module, true);
    }
}

#[test]
fn old_unambiguous_source_declarations_are_not_rejected_by_a_blanket_floor() {
    let module = interface(
        "public type Slot<T> is {value:T}; public type Marker is protocol {}; \
         implement<T> Marker for Slot<T> {} \
         public fn accept<T>(item:T)->Int where T=Int {0}",
    );
    current_and_old(&module, false);
}

#[test]
fn generic_value_predicates_require_the_classified_source_schema() {
    let module = interface("public fn accept(item:Int)->Int where item>0 {item}");
    let signature = module
        .functions
        .iter()
        .find_map(|function| function.source_signature.as_ref())
        .expect("actual source signature");
    assert!(
        signature
            .generic_where_clause
            .as_ref()
            .unwrap()
            .predicates
            .iter()
            .any(|predicate| matches!(
                predicate.kind,
                verum_ast::ty::WherePredicateKind::Value { .. }
            ))
    );
    current_and_old(&module, true);
    // An explicitly classified type equation did not have this old ambiguity.
    current_and_old(
        &interface("public fn accept<T>(item:T)->Int where type T=Int {0}"),
        false,
    );
}
