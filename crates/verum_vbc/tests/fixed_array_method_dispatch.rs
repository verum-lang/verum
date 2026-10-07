//! T1615: integer and Float64 fixed-array method receivers preserve their layout.
//! Float32 slice encoding remains tracked separately in T1621.
#![cfg(feature = "codegen")]
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
    ty::TypeKind,
};
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;

fn execute(source: &str) -> verum_vbc::interpreter::InterpreterResult<verum_vbc::Value> {
    // The declared fixed-array surface comes from the actual slice methods.
    let mut ast = Parser::new(source)
        .parse_module()
        .expect("inline array grammar");
    let mut core = Parser::new(include_str!("../../../core/collections/slice.vr"))
        .parse_module()
        .expect("actual slice grammar");
    core.items.retain_mut(|item| {
        let ItemKind::Impl(decl) = &mut item.kind else {
            return false;
        };
        if !matches!(&decl.kind, ImplKind::Inherent(ty) if matches!(&ty.kind, TypeKind::Slice(_))) {
            return false;
        }
        decl.items.retain(|item| {
            matches!(&item.kind, ImplItemKind::Function(function)
            if ["len", "swap", "reverse"].contains(&function.name.name.as_str()))
        });
        true
    });
    ast.items.extend(core.items);
    let mut memory = Parser::new(include_str!("../../../core/intrinsics/memory.vr"))
        .parse_module()
        .expect("actual slice intrinsic grammar");
    memory.items.retain(|item| matches!(&item.kind, ItemKind::Function(function) if function.name.name == "slice_len"));
    ast.items.extend(memory.items);
    let mut codegen = VbcCodegen::new();
    codegen.register_stdlib_intrinsics();
    let module = codegen.compile_module(&ast).expect("inline array VBC");
    let wire = verum_vbc::serialize::serialize_module(&module).expect("inline array wire");
    let module = verum_vbc::deserialize::deserialize_module(&wire).expect("inline array roundtrip");
    let entry = module
        .functions
        .iter()
        .find(|function| module.get_string(function.name) == Some("probe"))
        .unwrap()
        .id;
    verum_vbc::interpreter::Interpreter::new(verum_common::Shared::new(module).into_arc())
        .execute_function(entry)
}

#[test]
fn fixed_array_methods_preserve_packed_element_widths() {
    for ty in ["Int", "Byte", "UInt16", "UInt32", "UInt64"] {
        let values = if ty == "Byte" {
            "11_u8,37_u8,99_u8".to_string()
        } else {
            format!("11 as {ty},37 as {ty},99 as {ty}")
        };
        let source = format!(
            "fn probe()->Int {{let mut values:[{ty};3]=[{values}]; values.swap(0,1); values.reverse(); (values[0] as Int)*1000000+(values[1] as Int)*1000+(values[2] as Int)}}"
        );
        assert_eq!(execute(&source).expect(&source).as_i64(), 99011037, "{ty}");
    }
}

#[test]
fn fixed_array_methods_preserve_float64_elements() {
    for ty in ["Float"] {
        let source = format!(
            "fn probe()->Int {{let mut values:[{ty};3]=[1.5 as {ty},3.25 as {ty},9.75 as {ty}]; values.swap(0,1); values.reverse(); (values[0]*100.0) as Int+(values[1]*10.0) as Int+values[2] as Int}}"
        );
        assert_eq!(execute(&source).expect(&source).as_i64(), 993, "{ty}");
    }
}

#[test]
fn parenthesized_array_receiver_has_the_same_borrowed_layout() {
    let source = "fn probe()->Int {let mut values:[Int;3]=[11,37,99]; ((values)).swap(0,1); (values).reverse(); values[0]*1000000+values[1]*1000+values[2]}";
    assert_eq!(execute(source).expect("parentheses").as_i64(), 99011037);
}

#[test]
fn mutable_slice_methods_write_through_existing_references() {
    for declaration in ["&mut [Int]", "&mut [Int;3]"] {
        let source = format!(
            "fn change(values:{declaration}) {{values.swap(0,1);values.reverse();}} fn probe()->Int {{let mut values:[Int;3]=[11,37,99]; change(&mut values); values[0]*1000000+values[1]*1000+values[2]}}"
        );
        assert_eq!(execute(&source).expect(&source).as_i64(), 99011037);
    }
}

#[test]
fn explicit_array_borrow_retains_slice_method_identity() {
    for calls in [
        "let borrowed=&mut values; borrowed.swap(0,1); borrowed.reverse();",
        "(&mut values).swap(0,1); (&mut values).reverse();",
    ] {
        let source = format!(
            "fn probe()->Int {{let mut values:[Int;3]=[11,37,99]; {calls} values[0]*1000000+values[1]*1000+values[2]}}"
        );
        assert_eq!(execute(&source).expect(&source).as_i64(), 99011037);
    }
}

#[test]
fn empty_reverse_and_self_swap_preserve_array_storage() {
    for ty in ["Byte", "Int"] {
        let source = format!(
            "fn probe()->Int {{let mut empty:[{ty};0]=[]; empty.reverse(); let mut single:[{ty};1]=[255]; single.swap(0,0); single.reverse(); empty.len()+single.len()*1000+single[0] as Int}}"
        );
        assert_eq!(execute(&source).expect(&source).as_i64(), 1255);
    }
}

#[test]
fn invalid_swap_is_rejected_by_the_declared_slice_body() {
    for index in ["-1", "3"] {
        let source = format!(
            "fn probe()->Int {{let mut values:[Int;3]=[11,37,99]; values.swap(0,{index}); 0}}"
        );
        let error = execute(&source).expect_err("invalid index");
        assert!(
            error.to_string().contains("swap indices out of bounds"),
            "{error}"
        );
    }
}

#[test]
fn nominal_owner_with_same_method_names_keeps_its_own_body() {
    let source = "type Cell is {value:Int}; implement Cell {fn swap(&mut self,a:Int,b:Int) {self.value=a+b;} fn reverse(&mut self) {self.value=self.value*10;}} fn probe()->Int {let mut cell=Cell {value:99};cell.swap(3,4);cell.reverse();cell.value}";
    assert_eq!(execute(source).expect("nominal owner").as_i64(), 70);
}

#[test]
fn shadowing_an_array_name_does_not_transfer_its_storage_identity() {
    let source = "type Cell is {value:Int}; implement Cell {fn swap(&mut self,a:Int,b:Int) {self.value=a+b;}} fn probe()->Int {let mut values:[Int;3]=[11,37,99]; let mut result=0; {let mut values=Cell {value:99};values.swap(3,4); result=values.value;} values.swap(0,1); result*100+values[0]}";
    assert_eq!(execute(source).expect("shadowed receiver").as_i64(), 737);
}

#[test]
fn nested_fixed_arrays_restore_outer_width_and_count() {
    let source = "fn probe()->Int {let mut values:[Int;3]=[11,37,99]; let mut result=0; {let mut values:[Byte;2]=[255_u8,42_u8];values.swap(0,1);result=values[1] as Int;} values.swap(0,1);values.reverse();result+values.len()*1000+values[0]*10000+values[1]*100+values[2]}";
    assert_eq!(execute(source).expect("nested widths").as_i64(), 994392);
}

#[test]
fn reordering_borrows_the_same_data_allocation_and_preserves_length() {
    let source = "fn probe()->Int {let mut values:[Int;3]=[11,37,99];let before=values.as_mut_ptr() as Int;values.swap(0,1);values.reverse();let after=values.as_mut_ptr() as Int;if before==after {values.len()*1000+values[0]} else {-1}}";
    assert_eq!(execute(source).expect("borrowed allocation").as_i64(), 3099);
}

#[test]
fn closure_compilation_restores_the_enclosing_array_layout() {
    let source = "fn probe()->Int {let mut values:[Int;3]=[11,37,99];let work=|| -> Int {let mut child:[Byte;2]=[5_u8,7_u8];child.swap(0,1);child[0] as Int};let result=work();values.swap(0,1);values.reverse();result*1000+values[0]}";
    assert_eq!(
        execute(source).expect("closure layout scope").as_i64(),
        7099
    );
}

#[test]
fn array_element_identity_survives_shadow_and_reference_forwarding() {
    let source = "implement<T> [T] {fn element_width(&self)->Int {T.size}} fn forward(values:&[Int])->Int {values.element_width()} fn probe()->Int {let values:[Int;3]=[11,37,99];let before=values.element_width();let mut middle=0;{let values:[Byte;2]=[5_u8,7_u8];middle=values.element_width();}let borrowed=&values;before*1000+middle*100+values.element_width()*10+forward(borrowed)}";
    assert_eq!(
        execute(source).expect("declared element identity").as_i64(),
        8188
    );
}

#[test]
fn captured_fixed_array_preserves_its_layout_and_element_identity() {
    for (binding, body) in [
        (
            "let",
            "values.element_width()*1000+values.len()*100+values[2]",
        ),
        (
            "let mut",
            "values.swap(0,1);values.reverse();values.element_width()*1000+values.len()*100+values[0]",
        ),
    ] {
        let source = format!(
            "implement<T> [T] {{fn element_width(&self)->Int {{T.size}}}} fn probe()->Int {{{binding} values:[Int;3]=[11,37,99];let work=|| -> Int {{{body}}};work()}}"
        );
        assert_eq!(execute(&source).expect(&source).as_i64(), 8399);
    }
}

#[test]
fn returned_fixed_array_preserves_its_declared_element_identity() {
    for (ty, values, width) in [("Byte", "11_u8,37_u8,99_u8", 1), ("UInt32", "11,37,99", 4)] {
        let source = format!(
            "implement<T> [T] {{fn element_width(&self)->Int {{T.size}}}} fn create()->[{ty};3] {{let values:[{ty};3]=[{values}];values}} fn probe()->Int {{let mut values=create();values.swap(0,1);values.reverse();values.element_width()*1000+values.len()*100+values[0] as Int}}"
        );
        assert_eq!(
            execute(&source).expect(&source).as_i64(),
            width * 1000 + 399
        );
    }
}

#[test]
fn structural_slice_keeps_the_whole_generic_element_witness() {
    let source = "type Cell<T> is {left:T,right:T}; implement<T> [T] {fn element_width(&self)->Int {T.size}} fn forward(values:&[Cell<Int>])->Int {values.element_width()} fn probe()->Int {let values:[Cell<Int>;0]=[];forward(&values[..])}";
    assert_eq!(
        execute(source)
            .expect("whole generic slice element")
            .as_i64(),
        16
    );
}
