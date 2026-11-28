//! Source declarations and native ABI declarations have independent identities.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::context::Context;
use verum_llvm::targets::{InitializationConfig, Target};
use verum_types::TypeChecker;
use verum_vbc::{VbcModule, codegen::VbcCodegen};

#[cfg(target_os = "macos")]
#[path = "support/source_image.rs"]
mod source_image;

const FLOAT_READ: &str = r#"
    type Number is (Float);
    type Holder is { inner:Number };
    fn read(value:&Holder)->Float { value.inner.0 }
    fn main()->Int {
        let value=Holder{inner:Number(72.5)};
        if read(&value)+0.5==73.0 {73}else{0}
    }
"#;

fn checked_wire(source: &str) -> VbcModule {
    let mut ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    let mut errors = checker.prepare_module_declarations(&ast);
    errors.extend(
        ast.items
            .iter()
            .filter_map(|item| checker.check_item(item).err()),
    );
    errors.extend(checker.take_deferred_errors());
    assert!(errors.is_empty(), "{errors:?}");
    assert!(checker.diagnostic_sources().is_empty());
    checker.commit_resolved_call_targets(&mut ast);
    let original = VbcCodegen::new()
        .compile_module(&ast)
        .expect("checked source VBC");
    let wire = verum_vbc::serialize::serialize_module(&original).expect("serialize");
    let module = verum_vbc::deserialize::deserialize_module(&wire).expect("reload");
    verum_vbc::validate::validate_module(&module).expect("valid loaded module");
    module
}

fn assert_source_and_native_are_distinct(source: &str, name: &str, equal_abi: bool) {
    let mut module = checked_wire(source);
    let selected = module
        .find_function_by_name(name)
        .expect("source declaration");
    for function in &mut module.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut body = verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end])
            .expect("loaded body");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
        function.instructions = Some(body);
    }
    Target::initialize_all(&InitializationConfig::default());
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("source_native_identity")
            .with_target("aarch64-apple-darwin")
            .with_debug_info(false),
    );
    lowering.lower_module(&module).expect("native lowering");
    lowering
        .module()
        .verify()
        .expect("valid complete native IR");
    let native = lowering
        .module()
        .get_function(name)
        .expect("native ABI owner");
    let source = lowering
        .get_function_by_id(selected.0)
        .expect("selected source owner");
    assert_ne!(source, native, "source body replaced native symbol {name}");
    if equal_abi {
        assert_eq!(source.get_type(), native.get_type(), "fixture must compare equal ABIs");
    }
    assert!(
        source.count_basic_blocks() > 0,
        "source body must be emitted"
    );
}

#[test]
fn source_function_cannot_replace_a_native_forward_declaration() {
    assert_source_and_native_are_distinct(FLOAT_READ, "read", false);
}

#[test]
fn equal_signature_is_not_native_symbol_ownership() {
    assert_source_and_native_are_distinct(
        "fn sched_yield()->Int {73} fn main()->Int {sched_yield()}",
        "sched_yield",
        true,
    );
}

#[test]
fn defined_native_helpers_keep_their_own_bodies() {
    assert_source_and_native_are_distinct(
        "fn verum_strlen_export()->Int {73} fn main()->Int {verum_strlen_export()}",
        "verum_strlen_export",
        false,
    );
}

#[test]
fn runtime_system_calls_keep_native_identity() {
    assert_source_and_native_are_distinct(
        "fn getpid()->Int {73} fn main()->Int {getpid()}",
        "getpid",
        false,
    );
}

#[test]
fn selected_source_calls_keep_their_result_after_wire_reload() {
    for source in [
        FLOAT_READ,
        "fn getpid()->Int {73} fn main()->Int {getpid()}",
        "fn verum_main()->Int {1} fn main()->Int {verum_main()+72}",
        "fn main()->Int {verum_main()+72} fn verum_main()->Int {1}",
    ] {
        let module = checked_wire(source);
        let entry = module.find_function_by_name("main").unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(module.into())
            .expect("validated VM");
        assert_eq!(
            vm.execute_function(entry)
                .expect("source execution")
                .try_as_i64(),
            Some(73)
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn selected_source_calls_execute_in_the_native_application() {
    for source in [
        FLOAT_READ,
        "fn getpid()->Int {73} fn main()->Int {getpid()}",
        "fn verum_main()->Int {1} fn main()->Int {verum_main()+72}",
        "fn main()->Int {verum_main()+72} fn verum_main()->Int {1}",
    ] {
        source_image::run_module(checked_wire(source), |_| {});
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_static_calls_bind_the_selected_implementations_receiver() {
    source_image::run(include_str!("../../verum_vbc/tests/fixtures/static_receiver_application.vr"));
}

#[cfg(target_os = "macos")]
#[test]
fn native_bound_calls_select_the_receivers_applied_protocol() {
    source_image::run_module(
        checked_wire(include_str!(
            "../../verum_vbc/tests/fixtures/applied_receiver_witness.vr"
        )),
        |_| {},
    );
}
