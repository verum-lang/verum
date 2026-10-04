//! T1584: a root declaration owns its body and bare calls among inline siblings.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionId, VbcModule};

const ROOT: &str = "fn choose(value: Int) -> Int { value + 30 }";
const ALPHA: &str = "module alpha { public fn choose(value: Int) -> Int { value + 50 } public fn own() -> Int { choose(7) } }";
const BETA: &str = "module beta { public fn choose(value: Int) -> Int { value + 70 } public fn own() -> Int { choose(7) } }";
const CALLERS: &str = "fn probe() -> Int { choose(7) } fn left() -> Int { alpha.own() } fn right() -> Int { beta.own() } fn main() { probe(); }";

fn ast(source: &str) -> verum_ast::Module {
    Parser::new(source).parse_module().expect("source grammar")
}
fn compile(source: &str) -> VbcModule {
    VbcCodegen::new()
        .compile_module(&ast(source))
        .expect("source VBC")
}
fn loaded(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap()
}
fn run(module: &VbcModule, name: &str) -> i64 {
    let id = module.find_function_by_name(name).expect(name);
    Interpreter::new(Arc::new(module.clone()))
        .execute_function(id)
        .expect("execute source body")
        .as_i64()
}
fn call_targets(module: &VbcModule, name: &str) -> List<FunctionId> {
    let f = module
        .get_function(module.find_function_by_name(name).expect(name))
        .unwrap();
    let mut pc = f.bytecode_offset as usize;
    let end = pc + f.bytecode_length as usize;
    let mut targets = List::new();
    while pc < end {
        if let Instruction::Call { func_id, .. } =
            verum_vbc::bytecode::decode_instruction(&module.bytecode, &mut pc).unwrap()
        {
            targets.push(FunctionId(func_id));
        }
    }
    targets
}
fn assert_family(module: &VbcModule) {
    let root = module
        .find_function_by_name("choose")
        .expect("root body retained");
    let alpha = module.find_function_by_name("alpha.choose").unwrap();
    let beta = module.find_function_by_name("beta.choose").unwrap();
    assert_ne!(root, alpha);
    assert_ne!(root, beta);
    assert_ne!(alpha, beta);
    assert_eq!(call_targets(module, "probe").as_slice(), &[root]);
    assert_eq!(call_targets(module, "alpha.own").as_slice(), &[alpha]);
    assert_eq!(call_targets(module, "beta.own").as_slice(), &[beta]);
    for m in [module.clone(), loaded(module)] {
        assert_eq!(run(&m, "probe"), 37);
        assert_eq!(run(&m, "left"), 57);
        assert_eq!(run(&m, "right"), 77);
        assert!(matches!(
            m.entry_main(),
            verum_vbc::module::EntryMain::Unique { .. }
        ));
    }
}
#[test]
fn root_before_inline_siblings_retains_body_and_call_identity() {
    assert_family(&compile(&format!("{ROOT} {ALPHA} {BETA} {CALLERS}")));
}
#[test]
fn root_after_inline_siblings_retains_body_and_call_identity() {
    assert_family(&compile(&format!("{BETA} {ALPHA} {ROOT} {CALLERS}")));
}
#[test]
fn root_between_inline_siblings_retains_body_and_call_identity() {
    assert_family(&compile(&format!("{ALPHA} {ROOT} {BETA} {CALLERS}")));
}
#[test]
fn declaration_arity_is_scoped_before_an_ambient_bare_alternative() {
    let m = compile(
        r#"
        fn choose(value:Int)->Int {value+30}
        fn choose()->Int {91}
        module alpha { public fn choose(value:Int, extra:Int)->Int {value+extra} public fn own()->Int {choose(50,7)} }
        module beta { public fn choose()->Int {77} public fn own()->Int {choose()} }
        fn probe()->Int {choose(7)+choose()}
        fn left()->Int {alpha.own()}
        fn right()->Int {beta.own()}
    "#,
    );
    for m in [m.clone(), loaded(&m)] {
        assert_eq!(run(&m, "probe"), 128);
        assert_eq!(run(&m, "left"), 57);
        assert_eq!(run(&m, "right"), 77);
    }
}
#[test]
fn repeated_declaration_collection_reuses_each_source_body_identity() {
    let source = ast(&format!("{ROOT} {ALPHA} {BETA} {CALLERS}"));
    let mut cg = VbcCodegen::new();
    cg.collect_unit_declarations(&[&source]).unwrap();
    cg.collect_unit_declarations(&[&source]).unwrap();
    assert_family(&cg.compile_function_bodies(&source).unwrap());
}
#[test]
fn renamed_mount_and_local_root_declaration_keep_distinct_targets() {
    for declarations in [
        format!("{ROOT} {ALPHA} mount alpha.{{choose as other}};"),
        format!("{ALPHA} mount alpha.{{choose as other}}; {ROOT}"),
    ] {
        let m = compile(&format!(
            "{declarations} fn probe()->Int {{choose(7)+other(7)}}"
        ));
        assert_eq!(run(&loaded(&m), "probe"), 94);
    }
}
#[test]
fn root_identity_is_cleared_when_reusing_codegen_for_another_module() {
    let mut cg = VbcCodegen::new();
    let m = cg
        .compile_module(&ast(&format!("{ROOT} fn probe()->Int{{choose(7)}}")))
        .unwrap();
    assert_eq!(run(&m, "probe"), 37);
    let m = cg
        .compile_module(&ast(
            "module beta {public fn choose(value:Int)->Int {value+70}} fn probe()->Int {choose(7)}",
        ))
        .unwrap();
    assert_eq!(run(&m, "probe"), 77);
}
#[test]
fn configured_root_and_explicit_header_keep_their_own_declarations() {
    for header in ["", "module consumer;"] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        let m = cg
            .compile_module(&ast(&format!(
                "{header} {ROOT} {ALPHA} fn probe()->Int {{choose(7)}}"
            )))
            .unwrap();
        assert_eq!(run(&m, "consumer.probe"), 37);
    }
}

#[test]
fn unannotated_root_function_value_keeps_its_identity_in_a_higher_order_call() {
    for source in [
        "fn choose(value:Int) {value+30} module alpha {public fn choose(value:Int)->Int {value+50}}",
        "module alpha {public fn choose(value:Int)->Int {value+50}} fn choose(value:Int) {value+30}",
    ] {
        let m = compile(&format!(
            "{source} fn apply(f:fn(Int)->Int)->Int {{f(7)}} fn probe()->Int {{apply(choose)}}"
        ));
        assert_eq!(run(&loaded(&m), "probe"), 37);
    }
}
#[test]
fn scoped_identity_reads_refined_live_metadata_for_the_same_function_id() {
    let mut cg = VbcCodegen::new();
    cg.collect_unit_declarations(&[&ast(ROOT)]).unwrap();
    let id = cg.ctx_mut().lookup_function("choose").unwrap().id;
    cg.collect_unit_declarations(&[&ast(ALPHA)]).unwrap();
    for info in cg
        .ctx_mut()
        .functions
        .values_mut()
        .filter(|info| info.id == id)
    {
        info.return_type_name = Some("RefinedResult".into());
    }
    let selected = cg.ctx_mut().lookup_function_in_scope("choose").unwrap();
    assert_eq!(selected.id, id);
    assert_eq!(selected.return_type_name.as_deref(), Some("RefinedResult"));
}

#[test]
fn durable_cli_source_preserves_all_six_declared_call_targets() {
    let module = compile(include_str!(
        "../../../vcs/specs/L0-critical/vbc/root_inline_function_identity.vr"
    ));
    let expected: List<_> = [
        "choose",
        "alpha.choose",
        "beta.choose",
        "late",
        "alpha.late",
        "beta.late",
    ]
    .into_iter()
    .map(|name| module.find_function_by_name(name).expect(name))
    .collect();
    assert_eq!(call_targets(&module, "main"), expected);
}

#[test]
fn inline_main_is_a_child_declaration_scope_not_the_unnamed_root() {
    let child = "module main { public fn choose(value:Int)->Int {value+70} public fn inside()->Int {choose(7)} }";
    for declarations in [format!("{ROOT} {child}"), format!("{child} {ROOT}")] {
        let module = compile(&format!("{declarations} fn probe()->Int {{choose(7)}}"));
        // The legacy descriptor renderer does not qualify a module literally
        // named main. Even there its source-body IDs must remain distinct.
        let choices: List<_> = module
            .functions
            .iter()
            .filter(|f| module.get_string(f.name) == Some("choose"))
            .map(|f| f.id)
            .collect();
        assert_eq!(choices.len(), 2);
        assert_ne!(choices[0], choices[1]);
        assert_eq!(run(&module, "probe"), 37);
        assert_eq!(run(&module, "inside"), 77);
    }
}
