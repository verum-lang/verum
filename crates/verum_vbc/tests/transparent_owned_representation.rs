//! A transparent declaration owns the inner runtime carrier, never a record header.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use verum_fast_parser::Parser;
use verum_vbc::{
    TypeRef, VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("transparent"));
    canonical_drop::initialize(&mut compiler);
    compiler
        .compile_additional_module(&ast)
        .expect("complete source body")
}
fn execute(source: &str) {
    let module = compile(source);
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        verum_vbc::validate::validate_module(&module).expect("valid source metadata");
        let entry = module.find_function_by_name("transparent.main").unwrap();
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(
            vm.execute_function(entry)
                .expect("source execution")
                .try_as_i64(),
            Some(73)
        );
    }
}

#[test]
fn scalar_newtype_copy_preserves_its_inner_value() {
    execute(
        "type Number is (Int); fn main()->Int {let first=Number(73);let second=first;second.0}",
    );
}
#[test]
fn newtype_copy_preserves_nested_owned_storage() {
    execute(
        r#"
        type Leaf is {value:Int}; type Branch is {leaf:Leaf}; type Wrapped is (Branch);
        fn main()->Int {
            let mut first=Wrapped(Branch{leaf:Leaf{value:73}});
            let second=first;
            first.0.leaf.value=0;
            second.0.leaf.value
        }
    "#,
    );
}
#[test]
fn generic_newtype_copy_substitutes_its_inner_declaration() {
    execute(
        r#"
        type Leaf is {value:Int}; type Wrapped<T> is (T);
        fn main()->Int {
            let mut first:Wrapped<Leaf> =Wrapped(Leaf{value:73});
            let second=first;first.0.value=0;second.0.value
        }
    "#,
    );
}
#[test]
fn transparent_structural_cleanup_reaches_its_owned_payload_once() {
    execute(
        r#"
        mount core.base.protocols.Drop;
        type Ticket is {count:&mut Int};
        implement Drop for Ticket {fn drop(&mut self) {*self.count=*self.count+1;}}
        type Wrapped<T> is (T);
        fn main()->Int {let mut count=0;{let value:Wrapped<Ticket> =Wrapped(Ticket{count:&mut count});}if count==1 {73}else{0}}
    "#,
    );
}
#[test]
fn an_ordinary_single_field_record_keeps_its_record_carrier() {
    execute(
        "type Record is {value:Int}; fn main()->Int {let first=Record{value:73};let second=first;second.value}",
    );
}
#[test]
fn forged_transparency_cannot_replace_a_record_declaration() {
    let mut module = compile("type Record is {value:Int}; fn main()->Int{73}");
    let owner = module
        .types
        .iter()
        .position(|d| module.get_string(d.name) == Some("Record"))
        .unwrap();
    module.types[owner].is_transparent_wrapper = true;
    let ty = TypeRef::Concrete(module.types[owner].id);
    assert!(verum_vbc::validate::validate_module(&module).is_err());
    assert!(
        verum_vbc::copy_plan::build(&module, &ty).is_err(),
        "copy must not trust a forged flag"
    );
    assert!(
        verum_vbc::drop_plan::build(&module, &ty).is_err(),
        "drop must not trust a forged flag"
    );
}

#[test]
fn annotated_scalar_newtype_copy_uses_the_declaration_carrier() {
    execute(
        "type Number is (Int); fn main()->Int {let first:Number=Number(73);let second=first;second.0}",
    );
}
#[test]
fn annotated_newtype_copy_preserves_nested_owned_storage() {
    execute(
        r#"
        type Leaf is {value:Int}; type Branch is {leaf:Leaf}; type Wrapped is (Branch);
        fn main()->Int {
            let mut first:Wrapped=Wrapped(Branch{leaf:Leaf{value:73}});
            let second=first; first.0.leaf.value=0; second.0.leaf.value
        }
    "#,
    );
}

#[test]
fn borrowed_newtype_copy_preserves_nested_owned_storage() {
    execute(
        r#"
        type Leaf is {value:Int}; type Branch is {leaf:Leaf}; type Wrapped is (Branch);
        fn main()->Int {
            let mut first=Wrapped(Branch{leaf:Leaf{value:73}});
            let second=*(&first);
            first.0.leaf.value=0;
            second.0.leaf.value
        }
    "#,
    );
}

#[test]
fn wrapper_argument_scope_cannot_publish_into_an_enclosing_call() {
    execute(
        r#"
        type Number is (Int);
        type Other is {value:Int};
        fn receive(value:Number)->Other {Other{value:value.0}}
        fn main()->Int {let first=receive(Number(73));let second=first;second.value}
    "#,
    );
}

#[test]
fn a_lexical_callable_does_not_inherit_a_same_named_constructor_result() {
    execute(
        r#"
        type Number is (Int);
        type Other is {value:Int};
        fn receive(Number:fn(Int)->Other)->Int {let first=Number(73);let second=first;second.value}
        fn build(n:Int)->Other {Other{value:n}}
        fn main()->Int {receive(build)}
    "#,
    );
}

fn descriptor_index(module: &VbcModule, name: &str) -> usize {
    module
        .types
        .iter()
        .position(|d| module.get_string(d.name) == Some(name))
        .unwrap()
}
fn refuse_both(module: &VbcModule, ty: &TypeRef) {
    assert!(
        verum_vbc::copy_plan::build(module, ty).is_err(),
        "copy must reject forged representation"
    );
    assert!(
        verum_vbc::drop_plan::build(module, ty).is_err(),
        "drop must reject forged representation"
    );
    // Admission can reject the forgery before execution. If it reaches a wire
    // consumer, neither ownership plan may accept it there either.
    if let Ok(bytes) = serialize_module(module)
        && let Ok(wire) = deserialize_module(&bytes)
    {
        assert!(verum_vbc::copy_plan::build(&wire, ty).is_err());
        assert!(verum_vbc::drop_plan::build(&wire, ty).is_err());
    }
}
#[test]
fn transparent_plans_require_source_and_exact_payload_slots() {
    let module = compile("type Number is (Int); fn main()->Int{73}");
    let owner = descriptor_index(&module, "Number");
    let ty = TypeRef::Concrete(module.types[owner].id);
    for corruption in 0..6 {
        let mut module = module.clone();
        let descriptor = &mut module.types[owner];
        match corruption {
            0 => descriptor.source_declaration = None,
            1 => descriptor.fields.clear(),
            2 => descriptor.fields.push(descriptor.fields[0].clone()),
            3 => descriptor.fields[0].declaration_type = None,
            4 => descriptor.fields[0].offset = 8,
            5 => descriptor.is_transparent_wrapper = false,
            _ => unreachable!(),
        }
        refuse_both(&module, &ty);
    }
}
#[test]
fn transparent_generic_plans_refuse_wrong_or_missing_owner_arguments() {
    let module = compile("type Wrapped<T> is (T); fn main()->Int{73}");
    let owner = module.types[descriptor_index(&module, "Wrapped")].id;
    refuse_both(&module, &TypeRef::Concrete(owner));
    refuse_both(
        &module,
        &TypeRef::Instantiated {
            base: owner,
            args: vec![
                TypeRef::Concrete(verum_vbc::TypeId::INT),
                TypeRef::Concrete(verum_vbc::TypeId::INT),
            ],
        },
    );
}
#[test]
fn explicit_drop_on_a_transparent_wrapper_is_not_silently_skipped() {
    let module = compile(
        r#"
        mount core.base.protocols.Drop;
        type Number is (Int);
        implement Drop for Number {fn drop(&mut self) {}}
        fn main()->Int{73}
    "#,
    );
    let owner = &module.types[descriptor_index(&module, "Number")];
    assert!(owner.drop_fn.is_some(), "actual declared Drop was attached");
    for module in [
        module.clone(),
        deserialize_module(&serialize_module(&module).unwrap()).unwrap(),
    ] {
        let error = verum_vbc::drop_plan::build(&module, &TypeRef::Concrete(owner.id))
            .expect_err("unsupported transparent Drop carrier must refuse");
        assert!(
            error.contains("carrier-aware explicit Drop call"),
            "{error}"
        );
    }
}

#[test]
fn constructor_in_a_closure_keeps_its_own_result_scope() {
    execute(
        r#"
        type Number is (Int);
        fn main()->Int {
            let build=|| {let first=Number(73);let second=first;second.0};
            let value=build();
            let copy=value;
            copy
        }
    "#,
    );
}
#[test]
fn an_affine_payload_cannot_gain_copy_permission_through_transparency() {
    let module =
        compile("type affine Ticket is {value:Int}; type Wrapped<T> is (T); fn main()->Int{73}");
    let ticket = module.types[descriptor_index(&module, "Ticket")].id;
    let wrapper = module.types[descriptor_index(&module, "Wrapped")].id;
    let ty = TypeRef::Instantiated {
        base: wrapper,
        args: vec![TypeRef::Concrete(ticket)],
    };
    assert!(verum_vbc::copy_plan::build(&module, &ty).is_err());
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    assert!(verum_vbc::copy_plan::build(&wire, &ty).is_err());
}

#[test]
fn qualified_same_named_record_and_newtype_keep_distinct_carriers() {
    let root = Parser::new(
        r#"
        module cog;
        public type Wrapped is (Int);
        fn main()->Int {
            let first=cog.Wrapped(37);
            let second=cog.foreign.Wrapped(73);
            let copy=second;
            if first.0==37 {copy.value}else{0}
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let foreign = Parser::new("module cog.foreign; public type Wrapped is {value:Int};")
        .parse_module()
        .unwrap();
    for reverse in [false, true] {
        let units = if reverse {
            [&foreign, &root]
        } else {
            [&root, &foreign]
        };
        let mut generator = VbcCodegen::with_config(CodegenConfig::new("cog"));
        generator.collect_unit_declarations(&units).unwrap();
        generator
            .compile_unit_items(&units, verum_vbc::codegen::ItemFailurePolicy::Strict)
            .unwrap();
        let module = generator.finalize_module_from_state().unwrap();
        let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        for module in [module, wire] {
            let entry = module.find_function_by_name("cog.main").unwrap();
            let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
            assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
        }
    }
}
