//! Typed copies preserve nested ownership and reject absent declaration authority.
#![cfg(feature = "codegen")]
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction, Reg, TypeId, TypeRef,
    bytecode::{decode_instructions, encode_instruction},
    bytecode_remap::{IdRemap, rewrite_instruction_ids},
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn wire(source: &str) -> verum_vbc::VbcModule {
    let ast = Parser::new(source).parse_module().expect("valid source");
    let module = VbcCodegen::new().compile_module(&ast).expect("source VBC");
    let module = deserialize_module(&serialize_module(&module).expect("serialize")).expect("reload");
    verum_vbc::validate::validate_module(&module).expect("validated wire");
    module
}
fn execute(source: &str) -> i64 {
    let module = wire(source);
    assert!(decode_instructions(&module.bytecode).unwrap().iter().any(|instruction|
        matches!(instruction, Instruction::CloneTyped { .. })), "the actual source copy uses declaration evidence");
    let entry = module.find_function_by_name("main").unwrap();
    let mut vm = Interpreter::try_new_validated(module.into()).expect("validated interpreter entry");
    vm.execute_function(entry).expect("executed source").try_as_i64().unwrap()
}

#[test]
fn owned_record_fields_survive_an_independent_nested_mutation() {
    assert_eq!(execute(r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        fn main() -> Int {
            let mut first = Branch { leaf: Leaf { value: 73 } };
            let second = first;
            first.leaf.value = 0;
            second.leaf.value
        }
    "#), 73);
}

#[test]
fn borrowed_fields_remain_references_to_the_original_scalar() {
    assert_eq!(execute(r#"
        type Borrowed is { value: &Int };
        fn observe(value: &Int) -> Int {
            let first = Borrowed { value: value };
            let second = first;
            *second.value
        }
        fn main() -> Int { let original = 73; observe(&original) }
    "#), 73);
}

#[test]
fn repeated_owned_children_are_copied_independently() {
    assert_eq!(execute(r#"
        type Leaf is { value: Int };
        type Pair is { left: Leaf, right: Leaf };
        fn main() -> Int {
            let leaf = Leaf { value: 73 };
            let first = Pair { left: leaf, right: leaf };
            let mut second = first;
            second.left.value = 0;
            second.right.value
        }
    "#), 73);
}

#[test]
fn array_elements_have_independent_nested_value_storage() {
    assert_eq!(execute(r#"
        type Leaf is { value: Int };
        fn main() -> Int {
            let mut first = [Leaf { value: 73 }];
            let second = first;
            first[0].value = 0;
            second[0].value
        }
    "#), 73);
}

#[test]
fn copy_wire_preserves_borrowed_qualifiers_and_truncation_is_refused() {
    let instruction = Instruction::CloneTyped {
        dst: Reg(300), src: Reg(301),
        type_ref: TypeRef::Reference {
            inner: Box::new(TypeRef::Concrete(TypeId(9000))),
            mutability: verum_vbc::types::Mutability::Immutable,
            tier: verum_vbc::types::CbgrTier::Tier1,
        },
    };
    let mut bytes = List::<u8>::new().into();
    encode_instruction(&instruction, &mut bytes);
    assert_eq!(&bytes[..2], &[0x1f, 0x0f]);
    for end in 1..bytes.len() { assert!(decode_instructions(&bytes[..end]).is_err(), "accepted truncation {end}"); }
    assert_eq!(decode_instructions(&bytes).unwrap(), [instruction.clone()]);
    struct Shift;
    impl IdRemap for Shift { fn map_type_id(&self, id: TypeId) -> TypeId { TypeId(id.0 + 50) } }
    let mut remapped = instruction;
    rewrite_instruction_ids(&mut remapped, &Shift);
    let Instruction::CloneTyped { type_ref: TypeRef::Reference { inner, .. }, .. } = remapped else { panic!("lost reference carrier") };
    assert_eq!(*inner, TypeRef::Concrete(TypeId(9050)));
}

#[test]
fn affine_owner_and_unknown_slot_never_grant_copy_authority() {
    let module = wire("type affine Ticket is { value: Int }; fn main() -> Int { 73 }");
    let ticket = module.types.iter().find(|declaration| module.get_string(declaration.name).is_some_and(|name| name == "Ticket" || name.ends_with(".Ticket"))).unwrap();
    assert!(verum_vbc::copy_plan::build(&module, &TypeRef::Concrete(ticket.id)).is_err());
    assert!(verum_vbc::copy_plan::build(&module, &TypeRef::Concrete(TypeId(9000))).is_err());
}
