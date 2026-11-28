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
const PARTIAL_EQ: &str = r#"module core.base.protocols;
    type PartialEq is protocol { fn eq(&self, other: &Self) -> Bool; };
    implement PartialEq for Int { fn eq(&self, other:&Self)->Bool { *self == *other } }
    implement PartialEq for Text { fn eq(&self, other:&Self)->Bool { *self == *other } }
"#;

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
    let owner = VbcCodegen::resolve_full_module_path(module, "projection")
        .unwrap_or_else(|| "projection".into());
    checker.set_current_module_path(owner.as_str());
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
        module core.base.protocols;
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
fn keyid_and_pair_equality_compares_both_operands_before_and_after_serialization() {
    let tuf = Parser::new(TUF_TYPES).parse_module().unwrap();
    let source = format!(
        r#"{PARTIAL_EQ}
        type Eq is protocol extends PartialEq {{}};
        implement Eq for Text {{}}
        type Pair is (Int, Int);
        fn compare_key(a:&KeyId, b:&KeyId)->Bool {{ a.eq(b) }}
        fn key_probe(which:Int)->Bool {{
            let key = KeyId("alpha");
            let same = KeyId("alpha");
            let other = KeyId("beta");
            if which == 0 {{ compare_key(&key, &same) }} else {{ compare_key(&key, &other) }}
        }}
    "#
    );
    let mut module = Parser::new(&source).parse_module().unwrap();
    let key = declaration(&tuf, "KeyId");
    let ItemKind::Type(decl) = &key.kind else {
        unreachable!()
    };
    let implementations = DeriveRegistry::new()
        .expand_all(decl, &["Eq".into()], key.span)
        .unwrap();
    module.items.push(key);
    module.items.extend(implementations);
    derive(&mut module, "Pair", "PartialEq");
    check(&module);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core.base.protocols"));
    let vbc = codegen.compile_module(&module).unwrap();
    let wire = verum_vbc::serialize::serialize_module(&vbc).unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    for module in [vbc, restored] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module
            .find_function_by_name("core.base.protocols.key_probe")
            .unwrap();
        let mut vm = Interpreter::new(Arc::new(module));
        for (input, expected) in [(0, true), (1, false)] {
            let result = vm
                .execute_function_with_args(entry, &[Value::from_i64(input)])
                .unwrap();
            assert_eq!(result.try_as_bool(), Some(expected));
        }
    }
}

#[test]
fn a_single_item_request_cannot_detach_the_eq_marker_from_its_comparison() {
    let tuf = Parser::new(TUF_TYPES).parse_module().unwrap();
    let item = declaration(&tuf, "KeyId");
    let ItemKind::Type(decl) = &item.kind else {
        unreachable!()
    };
    let registry = DeriveRegistry::new();
    assert!(matches!(
        registry.expand("Eq", decl, item.span),
        Err(verum_compiler::derives::DeriveError::RequiresGroup { .. })
    ));
    let items = registry
        .expand_all(
            decl,
            &["Eq".into(), "PartialEq".into(), "Hash".into()],
            item.span,
        )
        .unwrap();
    assert_eq!(items.len(), 3);
    for (index, item) in items.iter().enumerate() {
        assert!(items.iter().take(index).all(|prior| prior.span != item.span));
    }
}

fn assert_hash_body(decl: &str, constructor: &str) {
    let source = format!(
        r#"
        module core.base.protocols;
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
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core.base.protocols"));
    let vbc = codegen.compile_module(&module).unwrap();
    let entry = vbc.find_function_by_name("core.base.protocols.probe").unwrap();
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

#[test]
fn generated_equality_uses_the_implementing_type_in_its_signature() {
    let source = format!(r#"
        {PARTIAL_EQ}
        type Pair is {{ first: Int, second: Int }};
        fn probe(left: Int, right: Int) -> Bool {{
            let a = Pair {{ first: left, second: 7 }};
            let b = Pair {{ first: right, second: 7 }};
            a.eq(&b)
        }}
    "#);
    let mut module = Parser::new(&source).parse_module().unwrap();
    derive(&mut module, "Pair", "PartialEq");
    check(&module);
    let vbc = VbcCodegen::with_config(CodegenConfig::new("projection"))
        .compile_module(&module).expect("generated equality must lower with its real receiver");
    let entry = vbc.find_function_by_name("core.base.protocols.probe").unwrap();
    let mut vm = Interpreter::new(Arc::new(vbc));
    for (left, right, equal) in [(2, 2, true), (2, 9, false)] {
        let result = vm.execute_function_with_args(entry, &[Value::from_i64(left), Value::from_i64(right)]).unwrap();
        assert_eq!(result.try_as_bool(), Some(equal));
    }
}
