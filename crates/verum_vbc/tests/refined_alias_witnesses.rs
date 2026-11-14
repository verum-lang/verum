//! Refinement predicates and runtime representation have distinct authorities.
#![cfg(feature = "codegen")]

use verum_ast::{TypeDeclBody, ty::TypeKind as SourceTypeKind};
use verum_common::ResourceDiscipline;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
    types::{TypeDescriptor, TypeId, TypeKind, TypeRef},
    validate::validate_module,
};

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source)
        .parse_module()
        .expect("valid source grammar");
    let module = VbcCodegen::new()
        .compile_module(&ast)
        .expect("strict source compilation");
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    validate_module(&module).expect("declared carrier survives the wire");
    module
}

fn declaration<'a>(module: &'a VbcModule, owner: &str, name: &str) -> &'a TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| {
            module.get_string(ty.name) == Some(name)
                && ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
        })
        .expect("exact nominal owner")
}

#[test]
fn refined_alias_retains_the_base_and_original_predicate_after_wire() {
    let module = compile(
        "module contracts;
        public type Positive is Int { it > 0 };
        fn main()->Int {let value:Positive=73; value}",
    );
    let positive = declaration(&module, "contracts", "Positive");
    assert_eq!(positive.kind, TypeKind::Alias);
    assert_eq!(positive.alias_target, Some(TypeRef::Concrete(TypeId::INT)));
    let source = positive
        .source_declaration
        .as_ref()
        .expect("predicate authority");
    assert!(matches!(&source.body, TypeDeclBody::Alias(target)
        if matches!(target.kind, SourceTypeKind::Refined { .. })));
    assert_eq!(
        module.resource_discipline(&TypeRef::Concrete(positive.id)),
        ResourceDiscipline::Unrestricted
    );
    assert!(
        module
            .type_identity(&TypeRef::Concrete(positive.id))
            .is_err(),
        "a runtime representation is not a predicate-bearing semantic identity"
    );
    let main = module.find_function_by_name("contracts.main").unwrap();
    let mut vm = Interpreter::new(module.into());
    vm.run_global_ctors().unwrap();
    assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(73));
}

#[test]
fn refined_alias_cannot_admit_a_value_that_violates_its_predicate() {
    for value in [0, -1] {
        let module = compile(&format!(
            "module contracts;
            public type Positive is Int {{ it > 0 }};
            fn main()->Int {{let value:Positive={value}; value}}"
        ));
        let main = module.find_function_by_name("contracts.main").unwrap();
        let mut vm = Interpreter::new(module.into());
        vm.run_global_ctors().unwrap();
        let error = vm
            .execute_function(main)
            .expect_err("invalid refined value");
        assert!(
            matches!(
                error,
                verum_vbc::interpreter::InterpreterError::AssertionFailed { .. }
            ),
            "unexpected failure: {error:?}"
        );
    }
}

#[test]
fn refined_generic_base_preserves_its_declared_binder_and_resource_mode() {
    let module = compile(
        "module contracts;
        public type Checked<T> is T { true };
        public type affine Token is {value:Int};",
    );
    let checked = declaration(&module, "contracts", "Checked");
    let token = declaration(&module, "contracts", "Token");
    assert_eq!(checked.type_params.len(), 1);
    assert_eq!(
        checked.alias_target,
        Some(TypeRef::Generic(checked.type_params[0].id))
    );
    assert!(checked.source_declaration.is_some());
    for (argument, expected) in [
        (TypeId::INT, ResourceDiscipline::Unrestricted),
        (token.id, ResourceDiscipline::Affine),
    ] {
        assert_eq!(
            module.resource_discipline(&TypeRef::Instantiated {
                base: checked.id,
                args: vec![TypeRef::Concrete(argument)],
            }),
            expected
        );
    }
}

#[test]
fn same_named_refined_bases_keep_their_owning_declaration() {
    let module = compile(
        "module first {
            public type Carrier is {value:Int};
            public type Valid is Carrier { it.value > 0 };
        }
        module second {
            public type affine Carrier is {value:Int};
            public type Valid is Carrier { it.value > 0 };
        }",
    );
    for (owner, mode) in [
        ("first", ResourceDiscipline::Unrestricted),
        ("second", ResourceDiscipline::Affine),
    ] {
        let base = declaration(&module, owner, "Carrier");
        let alias = declaration(&module, owner, "Valid");
        assert_eq!(alias.alias_target, Some(TypeRef::Concrete(base.id)));
        assert_eq!(
            module.resource_discipline(&TypeRef::Concrete(alias.id)),
            mode
        );
    }
}

#[test]
fn unimported_refined_base_cannot_borrow_another_modules_same_name() {
    let ast = Parser::new(
        "module other {public type Carrier is {value:Int};}
        module contracts {public type Valid is Carrier { it.value > 0 };}",
    )
    .parse_module()
    .unwrap();
    let error = VbcCodegen::new()
        .compile_module(&ast)
        .expect_err("no source binding for Carrier");
    assert!(error.to_string().contains("Carrier"), "{error}");
}

#[test]
fn generic_shadow_does_not_inherit_a_same_named_nominal_predicate() {
    let module = compile(
        "module contracts;
        public type Positive is Int { it > 0 };
        fn retain<Positive>(value:Positive)->Positive {value}
        fn main()->Int {retain(-73)}",
    );
    let main = module.find_function_by_name("contracts.main").unwrap();
    let mut vm = Interpreter::new(module.into());
    vm.run_global_ctors().unwrap();
    assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(-73));
}
