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
    let archive = try_compile_archive(provider, consumer)?;
    let module = archive.load_module("core").unwrap();
    validate_module(&module).unwrap();
    Ok(module)
}

fn try_compile_archive(
    provider: &str,
    consumer: &str,
) -> Result<verum_vbc::archive::VbcArchive, verum_common::Text> {
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
    Ok(read_archive_from_file(&output).unwrap())
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

#[test]
fn record_field_named_alias_keeps_its_declaring_predicate() {
    let module = compile(
        "module core.provider;
        fn allowed(value:Int)->Bool {value>0}
        public type Positive is Int {allowed(it)};
        public type Holder is {public value:Positive};",
        "module core.api; mount core.provider.Holder;
        fn allowed(value:Int)->Bool {value<0}
        public fn inspect(value:Int)->Int {let holder=Holder{value};holder.value}",
    );
    decision(&module, "core.api.inspect", &[73], true);
    decision(&module, "core.api.inspect", &[-73], false);
}

#[test]
fn record_field_inline_predicate_calls_its_private_owner_helper() {
    let module = compile(
        "module core.provider;
        fn allowed(value:Int)->Bool {value>0}
        public type Holder is {public value:Int{allowed(it)}};",
        "module core.api; mount core.provider.Holder;
        fn allowed(value:Int)->Bool {value<0}
        public fn inspect(value:Int)->Int {let holder=Holder{value};holder.value}",
    );
    decision(&module, "core.api.inspect", &[73], true);
    decision(&module, "core.api.inspect", &[-73], false);
}

#[test]
fn record_field_assignment_checks_the_selected_alias() {
    let module = compile(
        "module core.provider;
        fn allowed(value:Int)->Bool {value>0}
        public type Positive is Int {allowed(it)};
        public type Holder is {public value:Positive};",
        "module core.api; mount core.provider.Holder;
        public fn inspect(value:Int)->Int {
            let mut holder=Holder{value:73}; holder.value=value; holder.value
        }",
    );
    decision(&module, "core.api.inspect", &[73], true);
    decision(&module, "core.api.inspect", &[-73], false);
}

#[test]
fn record_field_compound_assignment_checks_the_selected_alias() {
    let module = compile(
        "module core.provider;
        fn allowed(value:Int)->Bool {value>0}
        public type Positive is Int {allowed(it)};
        public type Holder is {public value:Positive};",
        "module core.api; mount core.provider.Holder;
        public fn inspect(value:Int)->Int {
            let mut holder=Holder{value:73}; holder.value+=value-73; holder.value
        }",
    );
    decision(&module, "core.api.inspect", &[73], true);
    decision(&module, "core.api.inspect", &[-73], false);
}

#[test]
fn record_field_uses_the_full_declared_value_application() {
    let module = compile(
        "module core.provider;
        public type Holder<MIN:meta Int> is {public value:Int{it>MIN}};",
        "module core.api; mount core.provider.Holder;
        public fn inspect(MIN:Int,value:Int)->Int {
            let holder:Holder<0> = Holder{value};holder.value
        }",
    );
    decision(&module, "core.api.inspect", &[100, 73], true);
    decision(&module, "core.api.inspect", &[-100, -73], false);
}

#[test]
fn record_field_call_form_requires_a_declared_callable() {
    let error = try_compile(
        "module core.provider;
        fn allowed(value:Int)->Bool {value>0}
        public type Positive is Int{allowed(it)};
        public type Holder is {public value:Positive};",
        "module core.api; mount core.provider.Holder;
        public fn inspect(value:Int)->Int {let holder=Holder(value);holder.value}",
    ).expect_err("a record declaration is not a callable constructor");
    assert!(error.contains("E412") && error.contains("Holder"), "{error}");
}

#[test]
fn record_field_new_archive_consumer_replays_the_original_predicate() {
    let archive = try_compile_archive(
        "module core.provider;
        fn allowed(value:Int)->Bool {value>0}
        public type Positive is Int{allowed(it)};
        public type Holder is {public value:Positive};",
        "module core.api;",
    )
    .unwrap();
    let module = compile_archive_consumer(&archive,
        "module app; mount core.provider.Holder;
        fn allowed(value:Int)->Bool {value<0}
        public fn inspect(value:Int)->Int {let holder=Holder{value};holder.value}
        public fn write(value:Int)->Int {
            let mut holder=Holder{value:73};holder.value=value;holder.value
        }
        public fn compound(value:Int)->Int {
            let mut holder=Holder{value:73};holder.value+=value-73;holder.value
        }",
    ).unwrap();
    for entry in ["app.inspect", "app.write", "app.compound"] {
        decision(&module, entry, &[73], true);
        decision(&module, entry, &[-73], false);
    }
}

#[test]
fn record_field_keeps_all_owner_value_arguments() {
    let module = compile(
        "module core.provider;
        public type Holder<LOW:meta Int,HIGH:meta Int> is {public value:Int{it>LOW && it<HIGH}};",
        "module core.api; mount core.provider.Holder;
        public fn inspect(LOW:Int,HIGH:Int,value:Int)->Int {
            let holder:Holder<0,100> = Holder{value}; holder.value
        }",
    );
    decision(&module, "core.api.inspect", &[100,-100,73], true);
    decision(&module, "core.api.inspect", &[-100,1000,-73], false);
    decision(&module, "core.api.inspect", &[-100,1000,173], false);
}

#[test]
fn record_field_closed_type_argument_retains_its_refinement() {
    let module = compile(
        "module core.provider;
        public type Positive is Int{it>0};
        public type Holder<T> is {public value:T};",
        "module core.api; mount core.provider.{Holder,Positive};
        public fn inspect(value:Int)->Int {
            let holder:Holder<Positive> = Holder{value}; holder.value
        }",
    );
    decision(&module, "core.api.inspect", &[73], true);
    decision(&module, "core.api.inspect", &[-73], false);
}

fn compile_archive_consumer(
    archive: &verum_vbc::archive::VbcArchive, source: &str,
) -> Result<VbcModule, verum_common::Text> {
    use verum_compiler::archive_ctx_loader::ArchiveCtxCache;
    use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen};
    let app = verum_fast_parser::Parser::new(source).parse_module().unwrap();
    let mut generator = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new(archive).apply_lazy_with_types(&mut generator, &app, &[])
        .map_err(|error| error.to_string())?;
    generator.collect_unit_declarations(&[&app]).map_err(|error| error.to_string())?;
    generator.resolve_pending_imports();
    generator.compile_unit_items(&[&app], ItemFailurePolicy::Strict)
        .map_err(|error| error.to_string())?;
    let module = generator.finalize_module_from_state().map_err(|error| error.to_string())?;
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    validate_module(&module).unwrap();
    Ok(module)
}

#[test]
fn record_field_missing_archive_helper_cannot_borrow_a_consumer_function() {
    use verum_vbc::archive::{ArchiveBuilder,read_archive,write_archive};
    let archive = try_compile_archive(
        "module core.provider; fn allowed(value:Int)->Bool {value>0}
         public type Positive is Int{allowed(it)};
         public type Holder is {public value:Positive};",
        "module core.api;",
    ).unwrap();
    let mut module = archive.load_module("core").unwrap();
    let names = module.strings.clone();
    module.functions.retain(|function| names.get(function.name) != Some("core.provider.allowed"));
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core", &module, &[]).unwrap();
    let mut bytes = verum_common::List::new();
    write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = read_archive(std::io::Cursor::new(bytes)).unwrap();
    let error = compile_archive_consumer(&archive,
        "module app; mount core.provider.Holder;
        fn allowed(value:Int)->Bool {true}
        public fn inspect(value:Int)->Int {let holder=Holder{value};holder.value}",
    ).expect_err("a missing owner must never select an ambient predicate");
    assert!(error.contains("allowed"), "{error}");
}

#[test]
fn record_field_legacy_hint_cannot_replace_the_source_declaration() {
    use verum_vbc::archive::{ArchiveBuilder, read_archive, write_archive};
    let archive = try_compile_archive(
        "module core.provider;
         public type Holder is {public value:Int{it>0}};",
        "module core.api;",
    ).unwrap();
    let consumer = "module app; mount core.provider.Holder;
        public fn inspect(value:Int)->Int {let holder=Holder{value};holder.value}";
    let complete = compile_archive_consumer(&archive, consumer).unwrap();
    decision(&complete, "app.inspect", &[73], true);
    decision(&complete, "app.inspect", &[-73], false);

    let mut module = archive.load_module("core").unwrap();
    let predicate = module.strings.intern("true");
    let binding = module.strings.intern("it");
    let names = module.strings.clone();
    let holder = module.types.iter_mut().find(|declaration|
        declaration.origin_module.and_then(|owner| names.get(owner)) == Some("core.provider")
            && declaration.source_declaration.as_ref()
                .is_some_and(|source| source.name.name == "Holder")
    ).expect("the actual record declaration");
    holder.source_declaration = None;
    holder.fields[0].refinement_src = predicate;
    holder.fields[0].refinement_binding = binding;
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core", &module, &[]).unwrap();
    let mut bytes = verum_common::List::new();
    write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = read_archive(std::io::Cursor::new(bytes)).unwrap();
    let error = compile_archive_consumer(&archive, consumer)
        .expect_err("a display hint cannot authorize an unchecked field write");
    assert!(error.contains("source declaration"), "{error}");
}

#[test]
fn record_field_predicate_does_not_enter_the_consumer_nested_function() {
    let module = compile(
        "module core.provider; fn allowed(value:Int)->Bool {value>0}
         public type Positive is Int{allowed(it)};
         public type Holder is {public value:Positive};",
        "module core.api; mount core.provider.Holder;
         public fn inspect(value:Int)->Int {
             fn allowed(value:Int)->Bool {value<0}
             let holder=Holder{value};holder.value
         }",
    );
    decision(&module, "core.api.inspect", &[73], true);
    decision(&module, "core.api.inspect", &[-73], false);
}

#[test]
fn record_field_predicate_does_not_capture_a_consumer_local_constant() {
    let module = compile(
        "module core.provider; const LIMIT:Int=0;
         public type Positive is Int{it>LIMIT};
         public type Holder is {public value:Positive};",
        "module core.api; mount core.provider.Holder;
         public fn inspect(value:Int)->Int {
             const LIMIT:Int=-100;
             let holder=Holder{value};holder.value
         }",
    );
    decision(&module, "core.api.inspect", &[73], true);
    decision(&module, "core.api.inspect", &[-73], false);
}

#[test]
fn record_field_archive_predicate_keeps_its_mounted_helper() {
    let archive = try_compile_archive(
        "module core.provider;
         public module rules {public fn allowed(value:Int)->Bool {value>0}}
         mount core.provider.rules.allowed as accept;
         public type Positive is Int{accept(it)};
         public type Holder is {public value:Positive};",
        "module core.api;",
    ).unwrap();
    let module = compile_archive_consumer(&archive,
        "module app; mount core.provider.Holder;
         fn accept(value:Int)->Bool {value<0}
         public fn inspect(value:Int)->Int {let holder=Holder{value};holder.value}",
    ).unwrap();
    decision(&module, "app.inspect", &[73], true);
    decision(&module, "app.inspect", &[-73], false);
}

#[test]
fn record_field_private_member_remains_inaccessible() {
    let error = try_compile(
        "module core.provider; public type Holder is {value:Int{it>0}};",
        "module core.api; mount core.provider.Holder;
         public fn inspect(value:Int)->Holder {Holder{value}}",
    ).expect_err("retained predicate source cannot grant private field access");
    assert!(error.contains("E202") && error.contains("value") && error.contains("core.provider"), "{error}");
}

#[test]
fn record_field_archive_predicate_keeps_its_qualified_sibling_helper() {
    let archive = try_compile_archive(
        "module core.provider;
         public type Positive is Int{core.api.allowed(it)};
         public type Holder is {public value:Positive};",
        "module core.api; public fn allowed(value:Int)->Bool {value>0}",
    ).unwrap();
    let module = compile_archive_consumer(&archive,
        "module app; mount core.provider.Holder;
         fn allowed(value:Int)->Bool {value<0}
         public fn inspect(value:Int)->Int {let holder=Holder{value};holder.value}",
    ).unwrap();
    decision(&module, "app.inspect", &[73], true);
    decision(&module, "app.inspect", &[-73], false);
}

#[test]
fn record_field_archive_owner_survives_a_consumer_homonym() {
    let archive = try_compile_archive(
        "module core.provider;
         fn allowed(value:Int)->Bool {value>0}
         public type Positive is Int{allowed(it)};
         public type Holder is {public value:Positive};",
        "module core.api;",
    ).unwrap();
    let module = compile_archive_consumer(&archive,
        "module app; mount core.provider.Holder as Selected;
         type Holder is {value:Int{it<0}};
         fn allowed(value:Int)->Bool {value<0}
         public fn inspect(value:Int)->Int {let holder=Selected{value};holder.value}",
    ).unwrap();
    decision(&module, "app.inspect", &[73], true);
    decision(&module, "app.inspect", &[-73], false);
}

#[test]
fn generic_record_method_return_preserves_its_value_arguments() {
    for return_type in ["Holder<N>", "Self"] {
        let provider = format!(
            "module core.provider;
             public type Holder<N:meta Int> is {{public value:Int{{it<N}}}};
             implement<N:meta Int> Holder<N> {{
                 public fn new(value:Int)->{return_type} {{Holder{{value}}}}
             }}",
        );
        let module = compile(&provider,
            "module core.api; mount core.provider.Holder;
             const N:Int = 1000;
             public fn inspect(value:Int)->Int {let holder=Holder<74>.new(value);holder.value}",
        );
        decision(&module, "core.api.inspect", &[73], true);
        decision(&module, "core.api.inspect", &[74], false);
    }
}

#[test]
fn scalar_field_homonym_does_not_remove_the_private_archive_predicate() {
    let archive = try_compile_archive(
        "module core.provider;
         fn allowed(value:Int)->Bool {value>0}
         public type Holder is {public allowed:Int, public value:Int{allowed(it)}};",
        "module core.api;",
    )
    .unwrap();
    let module = compile_archive_consumer(
        &archive,
        "module app; mount core.provider.Holder;
         fn allowed(value:Int)->Bool {value<0}
         public fn inspect(value:Int)->Int {let holder=Holder{allowed:7,value};holder.value}",
    )
    .unwrap();
    decision(&module, "app.inspect", &[73], true);
    decision(&module, "app.inspect", &[-73], false);
}
