//! Count paths select a declared module owner before looking up a member.
#![cfg(feature = "codegen")]
use verum_common::Shared;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter};

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
    for count in ["ns.CAP", "ns.deep.CAP", "cog.outer.inner.ns.CAP"] {
        let source = format!(
            "module outer {{ module ns {{ public const CAP: Int = 3; module deep {{ public const CAP: Int = 7; }} }} module inner {{ module ns {{}} fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}} }} }}"
        );
        let ast = Parser::new(&source).parse_module().expect("source grammar");
        assert!(
            VbcCodegen::new().compile_module(&ast).is_err(),
            "wrong owner accepted: {source}"
        );
    }
}
