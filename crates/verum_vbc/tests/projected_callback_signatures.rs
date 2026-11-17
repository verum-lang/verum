#![cfg(feature = "codegen")]

use verum_vbc::{
    codegen::{CodegenConfig, CodegenResult, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    module::VbcModule,
};

fn compile(source: &str) -> CodegenResult<VbcModule> {
    let ast = verum_fast_parser::Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.collect_unit_declarations(&[&ast])?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}

#[test]
fn a_declared_callback_keeps_its_callers_associated_parameter() {
    let module = compile(r#"
        module consumer;
        type Family is protocol { type Item; };
        type Number is { value: Int };
        type Owner is ();
        implement Family for Owner { type Item = Number; }
        fn apply<A, F: fn(A) -> A>(value: A, transform: F) -> A { transform(value) }
        fn through<T: Family>(value: T.Item) -> T.Item { apply(value, |item| item) }
        fn run() -> Int { through<Owner>(Number { value: 43 }).value }
    "#).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    for module in [module, wire] {
        let closure = module.functions.iter().find(|function| function.closure_capture_count.is_some()).unwrap();
        let parameter = closure.semantic_parameter_types().unwrap()[1].as_ref().unwrap();
        assert!(matches!(parameter, verum_vbc::types::TypeRef::AssociatedProjection { base, assoc, .. }
            if matches!(base.as_ref(), verum_vbc::types::TypeRef::Generic(_)) && assoc == "Item"), "{parameter:?}");
        assert_eq!(closure.semantic_return.as_ref(), Some(parameter));
        let entry = module.find_function_by_name("consumer.run").unwrap();
        let actual = Interpreter::new(module.into()).execute_function(entry).unwrap();
        assert_eq!(actual.try_as_i64(), Some(43));
    }
}

#[test]
fn an_unbounded_callback_cannot_inherit_a_siblings_projected_parameter() {
    let error = compile(r#"
        module consumer;
        type Family is protocol { type Item; };
        fn ignore<A, F>(value: A, transform: F) -> A { value }
        fn through<T: Family>(value: T.Item) -> T.Item { ignore(value, |item| item) }
    "#).err().expect("the callback has no parameter declaration").to_string();
    assert!(error.contains("owned closure parameter requires a complete declaration and binding inventory"), "{error}");
}

#[test]
fn a_projected_callback_transports_distinct_callee_and_caller_binder_ids() {
    let module = compile(r#"
        module consumer;
        type Family is protocol { type Item; };
        type Number is { value: Int };
        type Owner is { tag: Int };
        implement Family for Owner { type Item = Number; }
        fn apply<T: Family, F: fn(T.Item) -> T.Item>(owner: T, value: T.Item, transform: F) -> T.Item {
            transform(value)
        }
        fn through<Unused, T: Family>(owner: T, value: T.Item) -> T.Item {
            apply(owner, value, |item| item)
        }
        fn run() -> Int { through<Bool, Owner>(Owner { tag: 0 }, Number { value: 47 }).value }
    "#).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    for module in [module, wire] {
        let entry = module.find_function_by_name("consumer.run").unwrap();
        assert_eq!(Interpreter::new(module.into()).execute_function(entry).unwrap().try_as_i64(), Some(47));
    }
}

#[test]
fn a_callback_cannot_infer_an_owner_from_a_same_leaf_projection() {
    for (parameters, arguments) in [
        ("owner: T, value: T.Item, transform: F", "owner, value, |item| item"),
        ("value: T.Item, transform: F", "value, |item| item"),
    ] {
        let source = format!(r#"
            module consumer;
            type Family is protocol {{ type Item; }};
            fn apply<T: Family, F: fn(T.Item) -> T.Item>({parameters}) -> T.Item {{ transform(value) }}
            fn through<A: Family, B: Family>(owner: A, value: B.Item) -> A.Item {{ apply({arguments}) }}
        "#);
        let error = compile(&source).err().expect("different or absent receiver evidence must fail").to_string();
        assert!(error.contains("complete declaration and binding inventory")
            || error.contains("cannot infer generic argument")
            || error.contains("source declaration"), "{error}");
    }
}
