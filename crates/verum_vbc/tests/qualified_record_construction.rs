//! T2837: record construction and type positions share selected source identity.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn compile(provider: &str, consumer: &str, reverse: bool) -> Result<VbcModule, CodegenError> {
    let provider = format!("public module provider {{ {provider} }}");
    let decoy = "public module decoy { public type Token is { public foreign:Bool }; public type Missing is { public value:Int }; }";
    let modules = if reverse { format!("{decoy} {provider}") } else { format!("{provider} {decoy}") };
    let source = format!("@no_implicit_prelude module catalog; {modules} module consumer {{ {consumer} }}");
    let ast = Parser::new(&source).parse_module().unwrap();
    // No filesystem or glob discovery participates in this source unit.
    VbcCodegen::with_config(CodegenConfig::new("catalog").with_validation()).compile_module(&ast)
}

fn execute_source_and_wire(module: VbcModule) {
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let entry = module.find_function_by_name("catalog.consumer.probe").unwrap();
        let result = Interpreter::try_new_validated(Arc::new(module)).unwrap().execute_function(entry).unwrap();
        assert_eq!(result.try_as_i64(), Some(73));
    }
}

#[test]
fn named_namespace_alias_and_qualified_records_share_the_canonical_owner() {
    for reverse in [false, true] {
        for (mount, name) in [
            ("mount catalog.provider.Token;", "Token"),
            ("mount catalog.provider as selected;", "selected.Token"),
            ("", "catalog.provider.Token"),
        ] {
            let consumer = format!(r#"
                {mount}
                type Holder is {{ token:{name} }};
                public fn probe()->Int {{
                    let holder = Holder {{ token:{name} {{ value:41, tag:32 }} }};
                    holder.token.value + holder.token.tag
                }}
            "#);
            let module = compile("public type Token is { public value:Int, public tag:Int };", &consumer, reverse)
                .unwrap_or_else(|error| panic!("{name}, reverse={reverse}: {error}"));
            let token = module.types.iter().find(|ty|
                module.get_string(ty.name) == Some("Token")
                    && ty.origin_module.and_then(|id| module.get_string(id)) == Some("catalog.provider"))
                .unwrap();
            let holder = module.types.iter().find(|ty| module.get_string(ty.name) == Some("Holder")).unwrap();
            assert_eq!(holder.fields[0].declaration_type.as_ref().and_then(|ty| ty.base_type_id()), Some(token.id));
            execute_source_and_wire(module);
        }
    }
}

#[test]
fn qualified_record_paths_cannot_borrow_private_or_missing_declarations() {
    for (provider, mount, name) in [
        ("type Token is { public value:Int };", "mount catalog.provider as selected;", "selected.Token"),
        ("type Token is { public value:Int };", "", "catalog.provider.Token"),
        ("public type Token is { public value:Int };", "mount catalog.provider as selected;", "selected.Missing"),
        ("public type Token is { public value:Int };", "mount catalog.absent as selected;", "selected.Token"),
    ] {
        let consumer = format!("{mount} public fn probe()->Int {{ let token = {name} {{ value:73 }}; token.value }}");
        let error = compile(provider, &consumer, false).expect_err("unavailable source acquired a record owner");
        assert!(error.to_string().contains("source"), "{name}: {error}");
    }
}

#[test]
fn qualified_sum_record_variants_keep_their_existing_constructor_path() {
    let module = compile("public type Outcome is Ready { value:Int } | Empty;", r#"
        mount catalog.provider.Outcome;
        public fn probe()->Int {
            let result:Outcome = catalog.provider.Outcome.Ready { value:73 };
            match result { Outcome.Ready { value } => value, Outcome.Empty => 0 }
        }
    "#, false).unwrap();
    execute_source_and_wire(module);
}
