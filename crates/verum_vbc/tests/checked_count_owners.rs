//! Count paths select a declared module owner before looking up a member.
#![cfg(feature = "codegen")]
use verum_common::Shared;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
fn selected_count_owners_survive_source_and_wire() {
    for (inner, count, expected) in [
        ("", "ns.CAP", 3),
        ("module ns { public const CAP: Int = 5; }", "ns.CAP", 5),
        ("module ns {}", "cog.outer.ns.CAP", 3),
        ("const ns: Int = 9;", "ns.CAP", 3),
    ] {
        let source = format!(
            "module outer {{ module ns {{ public const CAP: Int = 3; }} module inner {{ {inner} fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}} }} }}"
        );
        let ast = Parser::new(&source).parse_module().expect("source grammar");
        let module = VbcCodegen::new().compile_module(&ast).expect(&source);
        let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
        let id = module
            .functions
            .iter()
            .find(|f| {
                module
                    .get_string(f.name)
                    .is_some_and(|name| name == "probe" || name.ends_with(".probe"))
            })
            .unwrap()
            .id;
        assert_eq!(
            Interpreter::new(Shared::new(module).into_arc())
                .execute_function(id)
                .unwrap()
                .as_i64(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn empty_nearer_module_refuses_outer_count_and_missing_intermediate_owner() {
    for (count, declaration_after) in [
        ("ns.CAP", false),
        ("ns.deep.CAP", false),
        ("cog.outer.inner.ns.CAP", false),
        ("ns.CAP", true),
    ] {
        // Independent source files put the refusing function directly in the
        // strict unit producer; legacy nested-item error swallowing is separate.
        let origin = Parser::new("module outer; module ns { public const CAP: Int = 3; module deep { public const CAP: Int = 7; } }").parse_module().unwrap();
        let (before, after) = if declaration_after {
            ("", "module ns {}")
        } else {
            ("module ns {}", "")
        };
        let caller = Parser::new(&format!("module outer.inner; {before} fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}} {after}")).parse_module().unwrap();
        let mut codegen = VbcCodegen::new();
        codegen
            .collect_unit_declarations(&[&origin, &caller])
            .unwrap();
        let error = codegen
            .compile_unit_items(&[&origin, &caller], ItemFailurePolicy::Strict)
            .expect_err("nearest empty owner must refuse its missing member");
        assert!(
            error.to_string().contains("constant") || error.to_string().contains("array"),
            "{count}: {error}"
        );
    }
}

#[test]
fn module_namespace_precedes_type_layout_properties() {
    for member in ["public const size: Int = 3;", ""] {
        let source = format!(
            "type ns is {{ value: Int }}; module ns {{ {member} }} fn measure<T>()->Int {{T.size}} fn probe()->Int {{measure<[Byte; ns.size]>()}}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let result = VbcCodegen::new().compile_module(&ast);
        if member.is_empty() {
            assert!(
                result.is_err(),
                "empty module owner borrowed same-name type layout"
            );
        } else {
            let module = result.expect("module constant");
            let id = module.find_function_by_name("probe").unwrap();
            assert_eq!(
                Interpreter::new(Shared::new(module).into_arc())
                    .execute_function(id)
                    .unwrap()
                    .as_i64(),
                3
            );
        }
    }
}

#[test]
fn selected_explicit_constant_import_keeps_its_lexical_owner() {
    for count in ["CAP", "ALIAS"] {
        let source = format!(
            "module remote {{ public const CAP: Int=3; }} module first {{ const CAP: Int=5; mount remote.{{CAP}}; const ALIAS: Int=CAP; fn measure<T>()->Int {{T.size}} fn imported_probe()->Int {{measure<[Byte; {count}]>()}} }} module second {{ const CAP: Int=7; fn measure<T>()->Int {{T.size}} fn local_probe()->Int {{measure<[Byte; CAP]>()}} }}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let module = VbcCodegen::new().compile_module(&ast).unwrap();
        let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
        for (name, expected) in [("imported_probe", 3), ("local_probe", 7)] {
            let id = module
                .functions
                .iter()
                .find(|function| {
                    module.get_string(function.name).is_some_and(|actual| {
                        actual == name || actual.ends_with(&format!(".{name}"))
                    })
                })
                .unwrap()
                .id;
            assert_eq!(
                Interpreter::new(Shared::new(module.clone()).into_arc())
                    .execute_function(id)
                    .unwrap()
                    .as_i64(),
                expected,
                "{source}: {name}"
            );
        }
    }
}

#[test]
fn a_module_type_member_keeps_its_layout_property() {
    let source = "module ns { public type Item is { value: Int }; } fn measure<T>()->Int {T.size} fn probe()->Int {measure<[Byte; ns.Item.size]>()}";
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let id = module.find_function_by_name("probe").unwrap();
    assert_eq!(
        Interpreter::new(Shared::new(module).into_arc())
            .execute_function(id)
            .unwrap()
            .as_i64(),
        8
    );
}
