#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

#[test]
fn inline_factories_keep_their_file_owner_in_all_compile_paths() {
    let source = r#"
module app.factory;
public type Factory is { value: Int };
public module Factory {
    public fn make(value: Int) -> Int { value + 1 }
    public module Nested { public fn make(value: Int) -> Int { value + 2 } }
}
public fn after() -> Int { Factory.make(40) + Factory.Nested.make(1) }
"#;
    let ast = Parser::new(source).parse_module().expect("grammar");
    for lenient in [false, true] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("bundle"));
        codegen
            .collect_unit_declarations(&[&ast])
            .expect("declarations");
        if lenient {
            codegen
                .compile_module_items_lenient(&ast)
                .expect("lenient bodies");
        } else {
            codegen.compile_module_items(&ast).expect("strict bodies");
        }
        let module = codegen.finalize_module().expect("module");
        for (name, owner) in [
            ("app.factory.Factory.make", "app.factory.Factory"),
            (
                "app.factory.Factory.Nested.make",
                "app.factory.Factory.Nested",
            ),
            ("app.factory.after", "app.factory"),
        ] {
            let function = module
                .functions
                .iter()
                .find(|function| module.get_string(function.name) == Some(name))
                .unwrap_or_else(|| panic!("missing {name}, lenient={lenient}"));
            assert_eq!(
                function.origin_module.and_then(|id| module.get_string(id)),
                Some(owner)
            );
        }
    }
}
