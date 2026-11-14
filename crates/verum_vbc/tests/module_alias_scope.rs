//! T2464: namespace mounts belong to the actual source declaration scope.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    module::VbcModule,
};

fn wire(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).expect("serialize source"),
    ).expect("deserialize source")
}

fn run(module: &VbcModule, name: &str) -> i64 {
    let function = module.find_function_by_name(name).expect(name);
    Interpreter::new(Arc::new(wire(module))).execute_function(function)
        .expect("execute declared namespace target").as_i64()
}

fn pair(first: &str, second: &str, collect_reverse: bool, body_reverse: bool) -> Result<VbcModule, String> {
    let first = Parser::new(first).parse_module().expect("first source grammar");
    let second = Parser::new(second).parse_module().expect("second source grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("realm"));
    let declarations = if collect_reverse { [&second, &first] } else { [&first, &second] };
    let bodies = if body_reverse { [&second, &first] } else { [&first, &second] };
    codegen.collect_unit_declarations(&declarations).map_err(|e| e.to_string())?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&bodies, ItemFailurePolicy::Strict).map_err(|e| e.to_string())?;
    codegen.finalize_module().map_err(|e| e.to_string())
}

#[test]
fn each_file_keeps_its_alias_in_both_declaration_and_body_orders() {
    let alpha = "module alpha; module child { public type Cell is {a:Int}; public fn read()->Int{37} } mount child as short; fn layout()->Int{short.Cell.size} fn probe()->Int{short.read()}";
    let beta = "module beta; mount child as short; module child { public type Cell is {a:Int,b:Int}; public fn read()->Int{73} } fn layout()->Int{short.Cell.size} fn probe()->Int{short.read()}";
    for collect_reverse in [false, true] {
        for body_reverse in [false, true] {
            let module = pair(alpha, beta, collect_reverse, body_reverse).expect("independent aliases");
            for (name, value) in [("realm.alpha.layout",8),("realm.beta.layout",16),
                ("realm.alpha.probe",37),("realm.beta.probe",73)] {
                assert_eq!(run(&module, name), value, "{name}, declaration={collect_reverse}, body={body_reverse}");
            }
        }
    }
}

#[test]
fn inline_child_inherits_parent_mount_and_can_shadow_it() {
    for mounts_first in [false, true] {
        let mount = "mount data as selected;";
        let children = "module data {public type Cell is {a:Int};} module inherited {fn probe()->Int{selected.Cell.size}} module shadowed {module data {public type Cell is {a:Int,b:Int};} mount data as selected; fn probe()->Int{selected.Cell.size}}";
        let source = format!("module owner; {} {}", if mounts_first {mount} else {children}, if mounts_first {children} else {mount});
        let ast = Parser::new(&source).parse_module().expect("inline source grammar");
        let mut codegen = VbcCodegen::new();
        let module = codegen.compile_module(&ast).expect("lexical mount inheritance");
        assert_eq!(codegen.source_mount_binding("owner.inherited", "selected").flatten().map(|p| p.as_str()), Some("owner.data"));
        assert_eq!(codegen.source_mount_binding("owner.shadowed", "selected").flatten().map(|p| p.as_str()), Some("owner.shadowed.data"));
        assert_eq!(run(&module,"owner.inherited.probe"),8);
        assert_eq!(run(&module,"owner.shadowed.probe"),16);
    }
}

#[test]
fn dotted_file_owner_does_not_inherit_another_files_private_mount() {
    let alpha = "module alpha; public module data {public type Cell is {a:Int};} mount data as selected; fn probe()->Int{selected.Cell.size}";
    let beta = "module realm.alpha.other; fn probe()->Int{selected.Cell.size}";
    for collect_reverse in [false, true] {
        for body_reverse in [false, true] {
            let error = pair(alpha, beta, collect_reverse, body_reverse).expect_err("a namespace prefix is not lexical ancestry");
            assert!(error.contains("selected"), "{error}");
        }
    }
}

#[test]
fn inline_sibling_cannot_borrow_a_mount_from_its_neighbour() {
    let first = "module left {mount super.data as selected; fn probe()->Int{selected.Cell.size}}";
    let second = "module right {fn probe()->Int{selected.Cell.size}}";
    for reverse in [false, true] {
        let source = format!("module owner; public module data {{public type Cell is {{a:Int}};}} {} {}", if reverse {second} else {first}, if reverse {first} else {second});
        let ast = Parser::new(&source).parse_module().expect("sibling source grammar");
        let error = VbcCodegen::new().compile_module(&ast).expect_err("sibling alias must remain private");
        assert!(error.to_string().contains("selected"), "{error}");
    }
}

#[test]
fn an_explicit_foreign_mount_selects_a_public_namespace() {
    let alpha = "module alpha; public module child {public type Cell is {a:Int};} mount child as local; fn probe()->Int{local.Cell.size}";
    let beta = "module beta; mount realm.alpha.child as short; fn probe()->Int{short.Cell.size}";
    for collect_reverse in [false, true] {
        let module = pair(alpha, beta, collect_reverse, !collect_reverse).expect("explicit public module mount");
        assert_eq!(run(&module,"realm.alpha.probe"),8);
        assert_eq!(run(&module,"realm.beta.probe"),8);
    }
}

#[test]
fn a_local_value_shadows_a_namespace_alias_without_changing_its_owner() {
    let source = "module owner; module data {public type Cell is {size:Bool};} mount data as short; type Value is {size:Int}; type Holder is {Cell:Value}; fn value()->Int{let short=Holder{Cell:Value{size:37}}; short.Cell.size} fn layout()->Int{short.Cell.size}";
    let ast = Parser::new(source).parse_module().expect("value-shadow source grammar");
    let module = VbcCodegen::new().compile_module(&ast).expect("value binding wins");
    assert_eq!(run(&module,"owner.value"),37);
    assert_eq!(run(&module,"owner.layout"),8);
}

#[test]
fn a_count_query_uses_its_declaring_files_namespace_mount() {
    let alpha = "module alpha; module data {public const WIDTH:Int=3;} mount data as short; type Buffer is [Int;short.WIDTH]; fn probe()->Int{Buffer.size}";
    let beta = "module beta; module data {public const WIDTH:Int=5;} mount data as short; type Buffer is [Int;short.WIDTH]; fn probe()->Int{Buffer.size}";
    for collect_reverse in [false, true] {
        let module = pair(alpha, beta, collect_reverse, !collect_reverse).expect("count namespace authority");
        assert_eq!(run(&module,"realm.alpha.probe"),24);
        assert_eq!(run(&module,"realm.beta.probe"),40);
    }
}

#[test]
fn an_anonymous_units_mount_is_shared_by_layout_and_count_queries() {
    let source = "module data {public const WIDTH:Int=3; public type Cell is {a:Int};} mount data as short; type Buffer is [Int;short.WIDTH]; fn probe()->Int{Buffer.size+short.Cell.size}";
    let ast = Parser::new(source).parse_module().expect("anonymous source grammar");
    let module = VbcCodegen::new().compile_module(&ast).expect("anonymous owner mount");
    assert_eq!(run(&module,"probe"),32);
}
