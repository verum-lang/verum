#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;
use verum_vbc::types::{TypeId, TypeRef};

fn source(text: &str) -> verum_ast::Module {
    Parser::new(text).parse_module().expect("parse source")
}
fn producer(name: &str, text: &str) -> VbcModule {
    VbcCodegen::with_config(CodegenConfig::new(name))
        .compile_module(&source(text))
        .expect("producer")
}
fn import(text: &str, available: &[&VbcModule]) -> VbcModule {
    compile_boundary(text, available, true)
}
fn compile_boundary(text: &str, available: &[&VbcModule], carry_nominals: bool) -> VbcModule {
    let ast = source(text);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..400 {
        codegen
            .ctx_mut()
            .intern_string_raw(&format!("foreign_pool_{i}"));
    }
    if carry_nominals {
        codegen
            .import_bootstrap_nominal_dependencies(&[&ast], available)
            .expect("import dependencies");
    }
    codegen
        .collect_unit_declarations(&[&ast])
        .expect("collect source");
    codegen
        .compile_function_bodies(&ast)
        .expect("compile consumer")
}
fn descriptor<'a>(module: &'a VbcModule, name: &str) -> &'a verum_vbc::types::TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| module.strings.get(ty.name) == Some(name))
        .expect(name)
}
fn named_id(module: &VbcModule, name: &str) -> TypeId {
    descriptor(module, name).id
}

#[test]
fn nested_nominal_function_signature_survives_bootstrap_import() {
    let dependency = producer(
        "alpha",
        "module alpha; type Handle<T> is { item: T }; type Batch<T> is { head: T }; type Unused is { x: Int };",
    );
    let text = "module consumer; mount alpha.{Handle, Batch}; type Local is { value: Int }; fn accept(x: Batch<Handle<Local>>) -> Int { 7 }";
    // The previous bootstrap boundary carried layouts and names only. The
    // exact same public source then records PTR in both nominal positions.
    let before = compile_boundary(text, &[&dependency], false);
    let before_function = before
        .functions
        .iter()
        .find(|f| {
            before
                .strings
                .get(f.name)
                .is_some_and(|n| n.ends_with("accept"))
        })
        .unwrap();
    assert!(matches!(&before_function.params[0].type_ref,
        TypeRef::Instantiated { base, args } if *base == TypeId::PTR
            && matches!(&args[0], TypeRef::Instantiated { base, .. } if *base == TypeId::PTR)));
    let result = import(text, &[&dependency]);
    let function = result
        .functions
        .iter()
        .find(|f| {
            result
                .strings
                .get(f.name)
                .is_some_and(|name| name == "accept" || name.ends_with(".accept"))
        })
        .unwrap();
    assert_eq!(
        function.params[0].type_ref,
        TypeRef::Instantiated {
            base: named_id(&result, "Batch"),
            args: vec![TypeRef::Instantiated {
                base: named_id(&result, "Handle"),
                args: vec![TypeRef::Concrete(named_id(&result, "Local"))]
            }]
        }
    );
    assert!(
        !result
            .types
            .iter()
            .any(|ty| result.strings.get(ty.name) == Some("Unused"))
    );
    assert_eq!(
        result
            .strings
            .get(descriptor(&result, "Handle").fields[0].name),
        Some("item")
    );
}

#[test]
fn structural_dependency_closure_uses_source_ids_and_string_pool() {
    let dependency = producer(
        "alpha",
        "module alpha; type Leaf is { value: Int }; type Wrapper is { nested: Leaf }; type Other is { x: Int, y: Int };",
    );
    let result = import(
        "module consumer; mount alpha.Wrapper; fn accept(x: Wrapper) -> Int { 7 }",
        &[&dependency],
    );
    assert_eq!(
        descriptor(&result, "Wrapper").fields[0].type_ref,
        TypeRef::Concrete(named_id(&result, "Leaf"))
    );
    assert_eq!(
        result
            .strings
            .get(descriptor(&result, "Wrapper").fields[0].type_name),
        Some("Leaf")
    );
    assert_eq!(
        result
            .strings
            .get(descriptor(&result, "Leaf").fields[0].name),
        Some("value")
    );
}

#[test]
fn distinct_qualified_dependencies_remain_distinct_in_both_orders() {
    let alpha = producer("alpha", "module alpha; type Envelope is { left: Int };");
    let beta = producer(
        "beta",
        "module beta; type Envelope is { right: Text, extra: Int };",
    );
    for available in [&[&alpha, &beta][..], &[&beta, &alpha][..]] {
        let result = import(
            "module consumer; fn accept(a: alpha.Envelope, b: beta.Envelope) -> Int { 7 }",
            available,
        );
        let function = result
            .functions
            .iter()
            .find(|f| {
                result
                    .strings
                    .get(f.name)
                    .is_some_and(|name| name == "accept" || name.ends_with(".accept"))
            })
            .unwrap();
        assert_ne!(function.params[0].type_ref, function.params[1].type_ref);
        let count = result
            .types
            .iter()
            .filter(|ty| {
                result
                    .strings
                    .get(ty.name)
                    .is_some_and(|name| name.ends_with("Envelope"))
            })
            .count();
        assert_eq!(count, 2);
    }
}

#[test]
fn declared_generic_shadows_a_foreign_nominal_with_the_same_name() {
    let dependency = producer(
        "alpha",
        "module alpha; type Handle<T> is { item: T }; type T is { value: Int };",
    );
    let result = import(
        "module consumer; mount alpha.Handle; fn accept<T>(x: Handle<T>) -> T { x.item }",
        &[&dependency],
    );
    let function = result
        .functions
        .iter()
        .find(|f| {
            result
                .strings
                .get(f.name)
                .is_some_and(|name| name == "accept" || name.ends_with(".accept"))
        })
        .unwrap();
    assert_eq!(
        function.params[0].type_ref,
        TypeRef::Instantiated {
            base: named_id(&result, "Handle"),
            args: vec![TypeRef::Generic(verum_vbc::types::TypeParamId(0))]
        }
    );
}

#[test]
fn missing_user_id_is_rejected_instead_of_reused_in_consumer_pool() {
    let mut dependency = producer("alpha", "module alpha; type Wrapper is { value: Int };");
    let ty = dependency
        .types
        .iter_mut()
        .find(|ty| dependency.strings.get(ty.name) == Some("Wrapper"))
        .unwrap();
    ty.fields[0].type_ref = TypeRef::Concrete(TypeId(87654));
    let ast =
        source("module consumer; mount alpha.Wrapper; fn accept(value: Wrapper) -> Int { 1 }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let error = codegen
        .import_bootstrap_nominal_dependencies(&[&ast], &[&dependency])
        .expect_err("unknown id");
    assert!(
        error.to_string().contains("unknown source TypeId 87654"),
        "{error}"
    );
}

#[test]
fn source_function_ids_are_never_copied_as_consumer_function_ids() {
    let mut dependency = producer(
        "alpha",
        "module alpha; type Wrapper is { value: Int }; fn foreign() -> Int { 1 }",
    );
    let foreign = dependency
        .functions
        .iter()
        .find(|f| {
            dependency
                .strings
                .get(f.name)
                .is_some_and(|n| n.ends_with("foreign"))
        })
        .unwrap()
        .id
        .0;
    let wrapper = dependency
        .types
        .iter_mut()
        .find(|ty| dependency.strings.get(ty.name) == Some("Wrapper"))
        .unwrap();
    wrapper.drop_fn = Some(foreign);
    wrapper.clone_fn = Some(foreign);
    let ast = source("module consumer; mount alpha.Wrapper; fn accept(x: Wrapper) -> Int { 7 }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    // Same raw number and bare name deliberately belong to a different owner.
    codegen.ctx_mut().register_function(
        "foreign".to_owned(),
        verum_vbc::codegen::context::FunctionInfo {
            callable_signature: None,
            id: verum_vbc::module::FunctionId(foreign),
            ..Default::default()
        },
    );
    codegen
        .import_bootstrap_nominal_dependencies(&[&ast], &[&dependency])
        .unwrap();
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    let result = codegen.compile_function_bodies(&ast).unwrap();
    assert_eq!(descriptor(&result, "Wrapper").drop_fn, None);
    assert_eq!(descriptor(&result, "Wrapper").clone_fn, None);
}

#[test]
fn method_signature_closure_and_registry_return_use_consumer_type_ids() {
    let dependency = producer(
        "alpha",
        "module alpha; type Hidden is { value: Int }; type Wrapper is { value: Int }; implement Wrapper { fn reveal(&self) -> Hidden { Hidden { value: self.value } } }",
    );
    let ast = source("module consumer; mount alpha.Wrapper; fn accept(x: Wrapper) -> Int { 7 }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let original = dependency
        .functions
        .iter()
        .find(|f| {
            dependency
                .strings
                .get(f.name)
                .is_some_and(|n| n.ends_with("Wrapper.reveal"))
        })
        .unwrap();
    codegen.ctx_mut().register_function(
        "alpha.Wrapper.reveal".to_owned(),
        verum_vbc::codegen::context::FunctionInfo {
            callable_signature: None,
            id: verum_vbc::module::FunctionId(9123),
            return_type: Some(original.return_type.clone()),
            ..Default::default()
        },
    );
    codegen
        .import_bootstrap_nominal_dependencies(&[&ast], &[&dependency])
        .unwrap();
    let carried = codegen.ctx_mut().functions["alpha.Wrapper.reveal"]
        .return_type
        .clone()
        .unwrap();
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    let result = codegen.compile_function_bodies(&ast).unwrap();
    assert_eq!(carried, TypeRef::Concrete(named_id(&result, "Hidden")));
}

#[test]
fn missing_explicit_mount_does_not_import_a_same_leaf_ancestor() {
    let dependency = producer("alpha", "module alpha; type Wrapper is { value: Int };");
    let result = import(
        "module consumer; mount alpha.child.Wrapper; fn accept(x: Wrapper) -> Int { 7 }",
        &[&dependency],
    );
    assert!(
        !result
            .types
            .iter()
            .any(|ty| result.strings.get(ty.name) == Some("Wrapper"))
    );
}

#[test]
fn mount_aliases_preserve_both_qualified_nominal_owners() {
    let alpha = producer("alpha", "module alpha; type Envelope is { left: Int };");
    let beta = producer("beta", "module beta; type Envelope is { right: Text };");
    let result = import(
        "module consumer; mount alpha.Envelope as A; mount beta.Envelope as B; fn accept(a: A, b: B) -> Int { 7 }",
        &[&alpha, &beta],
    );
    let function = result
        .functions
        .iter()
        .find(|f| {
            result
                .strings
                .get(f.name)
                .is_some_and(|name| name.ends_with("accept"))
        })
        .unwrap();
    assert_ne!(function.params[0].type_ref, function.params[1].type_ref);
    assert!(!matches!(
        function.params[0].type_ref,
        TypeRef::Concrete(TypeId::PTR)
    ));
    assert!(!matches!(
        function.params[1].type_ref,
        TypeRef::Concrete(TypeId::PTR)
    ));
}

#[test]
fn promoted_names_and_originless_descriptors_keep_identity_on_reimport() {
    let mut dependency = producer("alpha", "module alpha; type Wrapper is { value: Int };");
    let promoted = dependency.strings.intern("alpha.Wrapper");
    let ty = dependency
        .types
        .iter_mut()
        .find(|ty| dependency.strings.get(ty.name) == Some("Wrapper"))
        .unwrap();
    ty.name = promoted;
    ty.origin_module = None;
    let source = "module consumer; mount alpha.Wrapper; fn accept(x: Wrapper) -> Int { 7 }";
    let first = import(source, &[&dependency]);
    let second = import(source, &[&first]);
    let wrapper = descriptor(&second, "Wrapper");
    assert_eq!(
        wrapper.origin_module.and_then(|id| second.strings.get(id)),
        Some("alpha")
    );
    let function = second
        .functions
        .iter()
        .find(|f| {
            second
                .strings
                .get(f.name)
                .is_some_and(|name| name.ends_with("accept"))
        })
        .unwrap();
    assert_eq!(function.params[0].type_ref, TypeRef::Concrete(wrapper.id));
}

#[test]
fn explicit_nested_hole_binds_the_imported_nominal_in_the_call_witness() {
    use verum_vbc::{bytecode::decode_instruction, instruction::Instruction};
    let dependency = producer(
        "alpha",
        "module alpha; type Handle<T> is { item: T }; type Batch<T> is { head: T };",
    );
    let text = r#"
module consumer;
mount alpha.{Handle, Batch};
type Local is { value: Int };
type Factory is { marker: Int };
implement Factory { fn make<C>(self) -> C { C.default() } }
fn probe(factory: Factory) -> Int {
    let result: Batch<Handle<Local>> = factory.make<Batch<_>>();
    7
}
"#;
    let result = import(text, &[&dependency]);
    let function = result
        .functions
        .iter()
        .find(|f| {
            result
                .strings
                .get(f.name)
                .is_some_and(|name| name.ends_with("probe"))
        })
        .unwrap();
    let mut pc = function.bytecode_offset as usize;
    let end = pc + function.bytecode_length as usize;
    let expected = TypeRef::Instantiated {
        base: named_id(&result, "Batch"),
        args: vec![TypeRef::Instantiated {
            base: named_id(&result, "Handle"),
            args: vec![TypeRef::Concrete(named_id(&result, "Local"))],
        }],
    };
    let mut witnesses = Vec::new();
    while pc < end {
        if let Instruction::CallG { type_args, .. } =
            decode_instruction(&result.bytecode, &mut pc).unwrap()
        {
            witnesses.push(type_args);
        }
    }
    assert_eq!(witnesses, vec![vec![expected]]);
}
