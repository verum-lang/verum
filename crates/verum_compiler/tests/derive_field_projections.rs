use std::sync::Arc;
use verum_ast::{Item, ItemKind, Module};
use verum_compiler::derives::DeriveRegistry;
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::{
    Value,
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::{Interpreter, InterpreterError},
};

const TUF_TYPES: &str = include_str!("../../../core/security/tuf/types.vr");
const PARTIAL_EQ: &str = "type PartialEq is protocol { fn eq(&self, other: &Self) -> Bool; };";

fn declaration(module: &Module, name: &str) -> Item {
    module
        .items
        .iter()
        .find(|item| matches!(&item.kind, ItemKind::Type(decl) if decl.name.as_str() == name))
        .unwrap()
        .clone()
}

fn derive(module: &mut Module, name: &str, protocol: &str) {
    let item = declaration(module, name);
    let ItemKind::Type(decl) = &item.kind else {
        unreachable!()
    };
    let implementation = DeriveRegistry::new()
        .expand(protocol, decl, item.span)
        .unwrap();
    module.items.push(implementation);
}

fn check(module: &Module) {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("projection");
    let mut errors = checker.prepare_module_declarations(module);
    checker.with_module_declarations(module, |checker| {
        for item in &module.items {
            if let Err(err) = checker.check_item(item) {
                errors.push(err);
                break;
            }
        }
    });
    errors.extend(checker.take_deferred_errors());
    assert!(errors.is_empty(), "{errors:?}");
    let diagnostics = checker.diagnostic_sources();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn keyid_hash_projects_the_declared_text() {
    let tuf = Parser::new(TUF_TYPES).parse_module().unwrap();
    let mut module = Parser::new(
        r#"
        type Hasher is protocol {};
        type Hash is protocol { fn hash(&self, hasher: &mut Hasher); };
        implement Hash for Text { fn hash(&self, hasher: &mut Hasher) {} }
    "#,
    )
    .parse_module()
    .unwrap();
    module.items.push(declaration(&tuf, "KeyId"));
    derive(&mut module, "KeyId", "Hash");
    check(&module);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("projection"));
    let vbc = codegen.compile_module(&module).unwrap();
    assert!(vbc.functions.iter().any(|f| {
        vbc.get_string(f.name)
            .is_some_and(|name| name.ends_with("KeyId.hash"))
    }));
}

#[test]
fn keyid_and_pair_equality_typecheck_both_operands() {
    let tuf = Parser::new(TUF_TYPES).parse_module().unwrap();
    let source = format!("{PARTIAL_EQ} type Pair is (Int, Int);");
    let mut module = Parser::new(&source).parse_module().unwrap();
    module.items.push(declaration(&tuf, "KeyId"));
    derive(&mut module, "KeyId", "PartialEq");
    derive(&mut module, "Pair", "PartialEq");
    check(&module);
}

fn assert_hash_body(decl: &str, constructor: &str) {
    let source = format!(
        r#"
        type Hasher is {{ state: Int }};
        type Hash is protocol {{ fn hash(&self, hasher: &mut Hasher); }};
        type Leaf is {{ value: Int }};
        implement Hash for Leaf {{
            fn hash(&self, hasher: &mut Hasher) {{
                if self.value == 93 {{ panic("visited second field"); }}
                hasher.state = hasher.state * 100 + self.value;
            }}
        }}
        {decl}
        fn probe(first: Int, second: Int) -> Int {{
            let a = Leaf {{ value: first }};
            let b = Leaf {{ value: second }};
            let wrapper = {constructor};
            let mut sink = Hasher {{ state: 0 }};
            wrapper.hash(&mut sink);
            sink.state
        }}
    "#
    );
    let mut module = Parser::new(&source).parse_module().unwrap();
    derive(&mut module, "Wrapper", "Hash");
    check(&module);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("projection"));
    let vbc = codegen.compile_module(&module).unwrap();
    let entry = vbc.find_function_by_name("projection.probe").unwrap();
    let mut vm = Interpreter::new(Arc::new(vbc));
    let run = |vm: &mut Interpreter, first, second| {
        vm.execute_function_with_args(entry, &[Value::from_i64(first), Value::from_i64(second)])
    };
    assert_eq!(run(&mut vm, 2, 7).unwrap().as_i64(), 207);
    assert_eq!(run(&mut vm, 7, 2).unwrap().as_i64(), 702);
    assert!(matches!(run(&mut vm, 2, 93),
        Err(InterpreterError::Panic { message }) if message == "visited second field"));
}

#[test]
fn generated_hash_visits_tuple_fields_in_source_order() {
    assert_hash_body("type Wrapper is (Leaf, Leaf);", "Wrapper(a, b)");
}

#[test]
fn generated_hash_visits_named_fields_in_source_order() {
    assert_hash_body(
        "type Wrapper is { first: Leaf, second: Leaf };",
        "Wrapper { first: a, second: b }",
    );
}
