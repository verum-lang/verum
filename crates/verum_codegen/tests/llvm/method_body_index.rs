//! T1523: indexed body availability preserves T1214 declaration-order semantics.
use super::method_will_have_body;
use crate::llvm::context::FuncNameIndex;
use verum_llvm::context::Context;
use verum_vbc::instruction::Instruction;
use verum_vbc::module::{FunctionDescriptor, VbcModule};

fn add(
    vbc: &mut VbcModule,
    name: &str,
    instructions: Option<Vec<Instruction>>,
    bytecode_length: u32,
) {
    let mut descriptor = FunctionDescriptor::new(vbc.strings.intern(name));
    descriptor.instructions = instructions;
    descriptor.bytecode_length = bytecode_length;
    vbc.functions.push(descriptor);
}

fn check(vbc: &VbcModule, name: &str, expected: bool) {
    let llvm = Context::create();
    let module = llvm.create_module("method_body_index");
    let function = module.add_function(name, llvm.void_type().fn_type(&[], false), None);
    let index = FuncNameIndex::build(vbc);
    assert_eq!(function.count_basic_blocks(), 0, "body has not lowered yet");
    assert_eq!(
        method_will_have_body(function, vbc, Some(&index), name),
        expected
    );
    assert_eq!(method_will_have_body(function, vbc, None, name), expected);
}

#[test]
fn a_later_vbc_body_is_eligible_before_llvm_lowers_it() {
    let mut vbc = VbcModule::new("later_body".into());
    add(&mut vbc, "List.from_iter", Some(vec![Instruction::RetV]), 0);
    add(&mut vbc, "Range.next", Some(vec![Instruction::RetV]), 0);
    // T1214: a forward LLVM declaration is not evidence that a body is absent.
    check(&vbc, "Range.next", true);
}

#[test]
fn duplicate_body_order_does_not_change_eligibility() {
    for bodies in [
        vec![None, Some(vec![])],
        vec![Some(vec![]), None],
        vec![None, Some(vec![Instruction::RetV])],
        vec![Some(vec![Instruction::RetV]), None],
    ] {
        let mut vbc = VbcModule::new("duplicates".into());
        for body in bodies {
            add(&mut vbc, "Range.next", body, 0);
        }
        check(&vbc, "Range.next", true);
    }
}

#[test]
fn the_single_name_winner_cannot_hide_an_empty_duplicate_body() {
    for bytecode_length in [0, 1] {
        let mut vbc = VbcModule::new("name_winner".into());
        add(&mut vbc, "Range.next", None, bytecode_length);
        add(&mut vbc, "Range.next", Some(vec![]), 0);
        assert_eq!(
            FuncNameIndex::build(&vbc)
                .find_by_name("Range.next")
                .unwrap()
                .index,
            0
        );
        check(&vbc, "Range.next", true);
    }
}

#[test]
fn absent_instructions_and_foreign_owners_remain_ineligible() {
    let mut vbc = VbcModule::new("exact_names".into());
    add(&mut vbc, "alpha.Range.next", None, 1);
    add(
        &mut vbc,
        "beta.Range.next",
        Some(vec![Instruction::RetV]),
        0,
    );
    check(&vbc, "alpha.Range.next", false);
    check(&vbc, "beta.Range.next", true);
    check(&vbc, "Range.next", false);
    check(&vbc, "missing.Range.next", false);
    // This spelling hits the suffix bucket, but must not adopt another name.
    check(&vbc, ".next", false);
}

#[test]
fn an_already_emitted_llvm_body_does_not_need_a_vbc_descriptor() {
    let llvm = Context::create();
    let module = llvm.create_module("emitted_body");
    let function = module.add_function("Range.next", llvm.void_type().fn_type(&[], false), None);
    let builder = llvm.create_builder();
    builder.position_at_end(llvm.append_basic_block(function, "entry"));
    builder.build_return(None).unwrap();
    module.verify().unwrap();
    let vbc = VbcModule::new("no_descriptors".into());
    let index = FuncNameIndex::build(&vbc);
    assert!(method_will_have_body(
        function,
        &vbc,
        Some(&index),
        "Range.next"
    ));
    assert!(method_will_have_body(function, &vbc, None, "Range.next"));
}
