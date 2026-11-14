//! Refinement evaluation belongs to its declaration, not the use site's names.
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{
    Value, VbcModule,
    archive::read_archive_from_file,
    interpreter::{Interpreter, InterpreterError},
    validate::validate_module,
};

fn compile(provider: &str, consumer: &str) -> VbcModule {
    try_compile(provider, consumer).expect("valid declaration-owned refinement")
}

fn try_compile(provider: &str, consumer: &str) -> Result<VbcModule, verum_common::Text> {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("core");
    std::fs::create_dir(&source).unwrap();
    for (file, text) in [
        (
            "mod.vr",
            "module core; public module provider; public module api;",
        ),
        ("provider.vr", provider),
        ("api.vr", consumer),
    ] {
        std::fs::write(source.join(file), text).unwrap();
    }
    let output = root.path().join("scoped.vbca");
    let mut session = Session::new(CompilerOptions {
        input: source.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&source).with_output(&output))
            .compile_core()
            .map_err(|error| error.to_string())?;
    if !result.errors.is_empty() {
        return Err(format!("{:?}", result.errors).into());
    }
    let archive = read_archive_from_file(&output).unwrap();
    let module = archive.load_module("core").unwrap();
    validate_module(&module).unwrap();
    Ok(module)
}

fn decision(module: &VbcModule, entry: &str, args: &[i64], accepted: bool) {
    let entry_id = module.find_function_by_name(entry).unwrap();
    let mut vm = Interpreter::new(module.clone().into());
    vm.run_global_ctors().unwrap();
    let args: verum_common::List<_> = args.iter().copied().map(Value::from_i64).collect();
    let result = vm.execute_function_with_args(entry_id, &args);
    if accepted {
        assert_eq!(
            result.unwrap().try_as_i64(),
            args.last().unwrap().try_as_i64(),
            "{entry}"
        );
    } else {
        assert!(
            matches!(result, Err(InterpreterError::AssertionFailed { .. })),
            "{entry} {args:?}: {result:?}"
        );
    }
}

#[test]
fn imported_predicate_calls_its_declaring_helper() {
    let module = compile(
        "module core.provider;
        fn allowed(value:Int)->Bool { value > 0 }
        public type Positive is Int { allowed(it) };",
        "module core.api;
        mount core.provider.Positive as Selected;
        fn allowed(value:Int)->Bool { value < 0 }
        public fn inspect(value:Selected)->Int {value}",
    );
    decision(&module, "core.api.inspect", &[-73], false);
    decision(&module, "core.api.inspect", &[73], true);
}

#[test]
fn imported_predicate_keeps_constants_separate_from_consumer_locals() {
    let module = compile(
        "module core.provider;
        const LIMIT:Int = 0;
        public type Positive is Int { it > LIMIT };",
        "module core.api;
        mount core.provider.Positive as Selected;
        const LIMIT:Int = -100;
        public fn inspect(value:Selected)->Int {value}
        public fn local(LIMIT:Int,value:Selected)->Int {value}",
    );
    decision(&module, "core.api.inspect", &[-73], false);
    decision(&module, "core.api.inspect", &[73], true);
    decision(&module, "core.api.local", &[-100, -73], false);
    decision(&module, "core.api.local", &[100, 73], true);
}

#[test]
fn alias_chain_preserves_the_original_predicate() {
    let module = compile(
        "module core.provider;
        public type Positive is Int { it > 0 };
        public type First is Positive;",
        "module core.api;
        mount core.provider.First;
        public type Second is First;
        public fn inspect(value:Second)->Int {value}",
    );
    decision(&module, "core.api.inspect", &[-73], false);
    decision(&module, "core.api.inspect", &[73], true);
}

#[test]
fn each_alias_constraint_retains_its_own_helper() {
    let module = compile(
        "module core.provider;
        fn allowed(value:Int)->Bool {value>0}
        public type Positive is Int {allowed(it)};
        public type First is Positive;",
        "module core.api;
        mount core.provider.First;
        fn allowed(value:Int)->Bool {value<100}
        public type Limited is First {allowed(it)};
        public type Last is Limited;
        public fn inspect(value:Last)->Int {value}",
    );
    for (value, accepted) in [(-73, false), (73, true), (173, false)] {
        decision(&module, "core.api.inspect", &[value], accepted);
    }
}

#[test]
fn generic_value_arguments_are_not_caller_locals() {
    let module = compile(
        "module core.provider;
        public type Above<MIN:meta Int> is Int {it>MIN};",
        "module core.api;
        mount core.provider.Above;
        const MIN:Int=-100;
        public fn inspect(MIN:Int,value:Above<0>)->Int {value}",
    );
    decision(&module, "core.api.inspect", &[-100, -73], false);
    decision(&module, "core.api.inspect", &[100, 73], true);
}

#[test]
fn non_boolean_predicate_cannot_produce_an_archive() {
    for predicate in ["73", "wrong(it)", "it && true", "!it"] {
        let source = format!(
            "module core.provider;
            fn wrong(value:Int)->Int {{value}}
            public type Invalid is Int {{{predicate}}};"
        );
        let error = try_compile(
            &source,
            "module core.api; mount core.provider.Invalid;
            public fn inspect(value:Invalid)->Int {value}",
        )
        .expect_err("only Bool authorizes a predicate decision");
        assert!(error.contains("Bool") || error.contains("bool"), "{error}");
    }
}

#[test]
fn contextual_numeric_payloads_keep_the_selected_carrier() {
    for (carrier, literal, valid) in [
        ("UInt32", "73", true),
        ("Byte", "73", true),
        ("Byte", "300", false),
        ("UInt32", "-1", false),
    ] {
        let source = format!(
            "module core.api; mount core.provider.Payload;
            public fn inspect()->{carrier} {{
                let item: Payload<{carrier}> = Payload.Some({literal});
                match item {{Payload.Some(value)=>value,Payload.Empty=>0}}
            }}"
        );
        let result = try_compile(
            "module core.provider; public type Payload<T> is Some(T)|Empty;",
            &source,
        );
        if valid {
            let module = result.unwrap();
            let entry = module.find_function_by_name("core.api.inspect").unwrap();
            let mut vm = Interpreter::new(module.into());
            vm.run_global_ctors().unwrap();
            assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
        } else {
            assert!(
                result.is_err(),
                "invalid contextual {carrier} literal {literal}"
            );
        }
    }
}

#[test]
fn finite_repeated_generic_alias_is_not_a_dependency_cycle() {
    let module = compile(
        "module core.provider; public type Checked<T> is T {true};",
        "module core.api; mount core.provider.Checked;
        public type Twice is Checked<Checked<Int>>;
        public fn inspect(value:Twice)->Int {value}",
    );
    decision(&module, "core.api.inspect", &[73], true);
}

#[test]
fn non_generic_variant_predicate_uses_its_declaring_helper() {
    let module = compile(
        "module core.provider;
        fn allowed(value:Int)->Bool {value>0}
        public type Payload is Item(Int{allowed(it)})|Empty;",
        "module core.api; mount core.provider.Payload;
        fn allowed(value:Int)->Bool {value<0}
        public fn inspect(value:Int)->Int {let item=Payload.Item(value); value}",
    );
    decision(&module, "core.api.inspect", &[73], true);
    decision(&module, "core.api.inspect", &[-73], false);
}

#[test]
fn generic_handoff_preserves_the_refined_argument_identity() {
    let module = compile(
        "module core.provider;
        public type Positive is Int{it>0};",
        "module core.api; mount core.provider.Positive;
        fn retain<T>(value:T)->T {value}
        public fn inspect(value:Positive)->Positive {retain(value)}",
    );
    let positive = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Positive"))
        .unwrap()
        .id;
    let entry = module.find_function_by_name("core.api.inspect").unwrap();
    let function = module
        .functions
        .iter()
        .find(|function| function.id == entry)
        .unwrap();
    let start = function.bytecode_offset as usize;
    let end = start + function.bytecode_length as usize;
    let instructions =
        verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end]).unwrap();
    assert!(instructions.iter().any(|instruction|matches!(instruction,
        verum_vbc::Instruction::CallG {type_args,..} if type_args.as_slice()==[verum_vbc::types::TypeRef::Concrete(positive)])));
    decision(&module, "core.api.inspect", &[73], true);
    decision(&module, "core.api.inspect", &[-73], false);
}

#[test]
fn successive_applications_do_not_reuse_another_predicate_substitution() {
    let module = compile(
        "module core.provider;
        public type Above<MIN:meta Int> is Int {it>MIN};",
        "module core.api; mount core.provider.Above;
        public fn inspect(first:Above<0>,second:Above<100>)->Int {second}",
    );
    decision(&module, "core.api.inspect", &[73, 173], true);
    decision(&module, "core.api.inspect", &[73, 73], false);
    decision(&module, "core.api.inspect", &[-73, 173], false);
}

#[test]
fn a_private_helper_does_not_become_a_consumer_binding() {
    let error = try_compile(
        "module core.provider;
        fn allowed(value:Int)->Bool {value>0}
        public type Positive is Int {allowed(it)};",
        "module core.api; mount core.provider.Positive;
        public fn inspect(value:Positive)->Bool {core.provider.allowed(value)}",
    )
    .expect_err("the predicate owner cannot grant the consumer private access");
    assert!(error.contains("allowed"), "{error}");
}
