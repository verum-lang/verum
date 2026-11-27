#![cfg(feature = "codegen")]

use verum_common::Text;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen,
    deserialize::deserialize_module_validated,
    interpreter::{Interpreter, InterpreterError},
    serialize::serialize_module,
};

#[test]
fn declared_argument_operations_read_the_invocation_and_preserve_source_functions() {
    let ast = Parser::new(
        r#"
        module application;
        @intrinsic("process_arg_count") fn count() -> Int;
        @intrinsic("process_arg_bytes") fn read(index: Int) -> &[Byte];
        type Field is { number: Int };
        fn arg(field: Field) -> Int { field.number }
        fn args() -> Int { 20 }
        fn args_count() -> Int { 12 }
        fn __arg_raw(index: Int) -> Int { index + 9 }
        fn ordinary() -> Int { arg(Field { number: 17 }) + args() + args_count() + __arg_raw(15) }
        fn supplied() -> Bool {
            count() == 4 && read(0).len() == 10 && read(0)[9] == 0x72 as Byte &&
                read(1).len() == 2 && read(1)[1] == 0x2D as Byte && read(2).len() == 0 &&
                read(3).len() == 22 && read(3)[0] == 0xCE as Byte && read(3)[1] == 0xB4 as Byte
        }
        fn too_high() -> &[Byte] { read(count()) }
        fn negative() -> &[Byte] { read(-1) }
    "#,
    )
    .parse_module()
    .unwrap();
    let source = VbcCodegen::new().compile_module(&ast).unwrap();
    let decoded = deserialize_module_validated(&serialize_module(&source).unwrap()).unwrap();
    for module in [source, decoded] {
        let function = |name: &str| {
            module
                .find_function_by_name(&format!("application.{name}"))
                .unwrap()
        };
        let (ordinary, count, supplied, too_high, negative) = (
            function("ordinary"),
            function("count"),
            function("supplied"),
            function("too_high"),
            function("negative"),
        );
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(
            vm.execute_function(ordinary).unwrap().try_as_i64(),
            Some(73)
        );
        assert_eq!(vm.execute_function(count).unwrap().try_as_i64(), Some(0));
        vm.state.config.program_arguments = ["project.vr", "--", "", "δ data.vr with spaces"]
            .map(Text::from)
            .into();
        assert!(vm.execute_function(supplied).unwrap().as_bool());
        for (id, invalid) in [(too_high, 4), (negative, -1)] {
            assert!(
                matches!(vm.execute_function(id), Err(InterpreterError::IndexOutOfBounds {
                index, length: 4
            }) if index == invalid)
            );
        }
    }
}

#[test]
fn process_argument_signatures_are_checked_before_lowering() {
    for decl in [
        r#"type List<T> is { fake: T }; @intrinsic("process_environment_bytes") fn snapshot() -> List<Byte>;"#,
        r#"@intrinsic("process_environment_bytes") fn snapshot() -> List<Int>;"#,
        r#"@intrinsic("process_environment_bytes") fn snapshot() -> &[Byte];"#,
        r#"@intrinsic("process_arg_count") fn count(n: Int) -> Int;"#,
        r#"@intrinsic("process_arg_count") fn count() -> Text;"#,
        r#"@intrinsic("process_arg_bytes") fn read(n: Bool) -> &[Byte];"#,
        r#"@intrinsic("process_arg_bytes") fn read(n: Int) -> Text;"#,
        r#"@intrinsic("process_arg_bytes") fn read(n: Int) -> &mut [Byte];"#,
        r#"@intrinsic("process_arg_bytes") fn read(n: Int) -> &[Int];"#,
    ] {
        let ast = Parser::new(decl).parse_module().unwrap();
        let error = VbcCodegen::new().compile_module(&ast).unwrap_err();
        assert!(error.to_string().contains("requires ("), "{error}");
    }
}

#[test]
fn argument_bytes_cannot_be_loaded_as_an_owned_text_result() {
    let ast = Parser::new(
        r#"module application; @intrinsic("process_arg_bytes") fn argument(i: Int) -> &[Byte];"#,
    )
    .parse_module()
    .unwrap();
    let valid = VbcCodegen::new().compile_module(&ast).unwrap();
    let id = valid.find_function_by_name("application.argument").unwrap();
    for alter_semantic in [false, true] {
        let mut module = valid.clone();
        let function = module.get_function_mut(id).unwrap();
        let text = verum_vbc::types::TypeRef::Concrete(verum_vbc::types::TypeId::TEXT);
        if alter_semantic {
            function.semantic_return = Some(text);
        } else {
            function.return_type = text;
        }
        let encoded = serialize_module(&module).unwrap();
        let error = deserialize_module_validated(&encoded).unwrap_err();
        assert!(error.to_string().contains("requires (Int) -> &[Byte]"), "{error}");
    }
}

#[test]
fn environment_bytes_read_an_isolated_child_process() {
    const CHILD: &str = "VERUM_ENVIRONMENT_SNAPSHOT_FIXTURE";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "environment_bytes_read_an_isolated_child_process"])
            .env_clear()
            .env(CHILD, "1")
            .env("A", "")
            .env("B", "x=y")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            Text::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let ast = Parser::new(include_str!("fixtures/process_environment.vr"))
        .parse_module()
        .unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let wire = deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let main = module.find_function_by_name("program.main").unwrap();
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(73));
    }
}
