//! The selected structural operation survives source checking and VBC wire.
use verum_ast::{Module, expr::ResolvedCallTarget};
use verum_common::List;
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::{
    Instruction, bytecode::decode_instructions, codegen::VbcCodegen,
    deserialize::deserialize_module, interpreter::Interpreter, serialize::serialize_module,
};

fn checked(source: &str) -> Module {
    let mut ast = Parser::new(source)
        .parse_module()
        .expect("grammar-valid source");
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
    assert!(
        checker.diagnostic_sources().is_empty(),
        "{:?}",
        checker.diagnostic_sources()
    );
    checker.commit_resolved_call_targets(&mut ast);
    ast
}

fn execute(source: &str, selected_copy: bool) -> i64 {
    struct Selections(usize);
    impl verum_ast::Visitor for Selections {
        fn visit_expr(&mut self, expression: &verum_ast::Expr) {
            if matches!(
                expression.resolved_call_target,
                Some(ResolvedCallTarget::ValueCopy)
            ) {
                self.0 += 1;
            }
            verum_ast::visitor::walk_expr(self, expression);
        }
    }
    use verum_ast::Visitor;
    let ast = checked(source);
    let mut selections = Selections(0);
    for item in &ast.items {
        selections.visit_item(item);
    }
    assert_eq!(
        selections.0 > 0,
        selected_copy,
        "selected operation differs"
    );
    let produced = VbcCodegen::new()
        .compile_module(&ast)
        .expect("source lowering");
    let module = deserialize_module(&serialize_module(&produced).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&module).expect("valid serialized program");
    if selected_copy {
        let copy = module
            .get_function(module.find_function_by_name("duplicate").unwrap())
            .unwrap();
        let code = &module.bytecode
            [copy.bytecode_offset as usize..(copy.bytecode_offset + copy.bytecode_length) as usize];
        let instructions = decode_instructions(code).unwrap();
        assert!(
            instructions
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CloneTyped { .. })),
            "{instructions:?}"
        );
        assert!(
            !instructions.iter().any(|instruction| matches!(
                instruction,
                Instruction::CallM { .. } | Instruction::Clone { .. }
            )),
            "selected copy reverted to legacy dispatch: {instructions:?}"
        );
    }
    let main = module.find_function_by_name("main").unwrap();
    let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
    vm.execute_function(main).unwrap().try_as_i64().unwrap()
}

#[test]
fn selected_copy_preserves_a_borrowed_receiver_and_owns_its_result() {
    assert_eq!(
        execute(
            r#"
        type Leaf is { value: Int };
        fn duplicate(source: &Leaf) -> Leaf { source.clone() }
        fn main() -> Int {
            let original = Leaf { value: 0 };
            let mut copied = duplicate(&original);
            copied.value = 73;
            original.value + copied.value
        }
    "#,
            true
        ),
        73
    );
}

#[test]
fn selected_copy_keeps_the_instantiated_declaration() {
    assert_eq!(
        execute(
            r#"
        type Cell<T> is { value: T };
        type Alias is Cell<Int>;
        fn duplicate(source: &Alias) -> Alias { source.clone() }
        fn main() -> Int {
            let original = Cell { value: 73 };
            let copied = duplicate(&original);
            copied.value
        }
    "#,
            true
        ),
        73
    );
}

#[test]
fn an_explicit_clone_implementation_keeps_its_declared_behavior() {
    assert_eq!(
        execute(
            r#"
        type Leaf is { value: Int };
        implement Leaf { fn clone(&self) -> Leaf { Leaf { value: self.value + 1 } } }
        fn duplicate(source: &Leaf) -> Leaf { source.clone() }
        fn main() -> Int { let original = Leaf { value: 73 }; duplicate(&original).value }
    "#,
            false
        ),
        74
    );
}

#[test]
fn structural_copy_does_not_grant_affine_or_mutable_reference_duplication() {
    for declaration in [
        "type affine Ticket is { value: Int };",
        "type Ticket is { value: &mut Int };",
        "type Ticket is { value: &unsafe Int };",
    ] {
        let source =
            format!("{declaration} fn duplicate(source:&Ticket)->Ticket {{source.clone()}}");
        let ast = Parser::new(&source).parse_module().unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        let mut errors: List<_> = checker.prepare_module_declarations(&ast);
        errors.extend(
            ast.items
                .iter()
                .filter_map(|item| checker.check_item(item).err()),
        );
        errors.extend(checker.take_deferred_errors());
        assert!(
            errors
                .iter()
                .any(|error| error.to_diagnostic().code() == Some("E400")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn ordinary_stdlib_admission_keeps_the_selected_operation_in_the_archive() {
    use verum_compiler::{
        CompilationPipeline, CompilerOptions, CoreConfig, Session,
        archive_ctx_loader::ArchiveCtxCache,
    };
    use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("core");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("mod.vr"),
        r#"
        module core;
        public type Leaf is { value: Int };
        public fn duplicate(source: &Leaf) -> Leaf { source.clone() }
        public fn probe() -> Int {
            let original = Leaf { value: 0 };
            let mut copied = duplicate(&original);
            copied.value = 73;
            original.value + copied.value
        }
    "#,
    )
    .unwrap();
    let output = directory.path().join("stdlib.vbca");
    let mut session = Session::new(CompilerOptions {
        input: root.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&root).with_output(&output))
            .compile_core()
            .unwrap_or_else(|error| panic!("{error}\n{}", session.format_diagnostics()));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = verum_vbc::archive::read_archive_from_file(&output).unwrap();
    let app = Parser::new("module app; mount core.probe; fn main()->Int {probe()}")
        .parse_module()
        .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new(&archive)
        .apply_lazy_with_types(&mut codegen, &app, &[])
        .unwrap();
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], ItemFailurePolicy::Strict)
        .unwrap();
    let produced = codegen.finalize_module_from_state().unwrap();
    let module = deserialize_module(&serialize_module(&produced).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    let copy = module
        .get_function(module.find_function_by_name("core.duplicate").unwrap())
        .unwrap();
    let code = &module.bytecode
        [copy.bytecode_offset as usize..(copy.bytecode_offset + copy.bytecode_length) as usize];
    let instructions = decode_instructions(code).unwrap();
    assert!(
        instructions
            .iter()
            .any(|i| matches!(i, Instruction::CloneTyped { .. })),
        "{instructions:?}"
    );
    assert!(
        !instructions
            .iter()
            .any(|i| matches!(i, Instruction::CallM { .. })),
        "{instructions:?}"
    );
    let main = module.find_function_by_name("app.main").unwrap();
    let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
    assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(73));
}

#[test]
fn selected_scalar_copy_preserves_declared_width_and_payload() {
    for (ty, literal) in [
        ("Int64", "-9223372036854775807"),
        ("UInt64", "18446744073709551615"),
        ("Byte", "231"),
        ("UInt8", "231"),
        ("UInt16", "65000"),
    ] {
        let source = format!(
            "type Word is {{ value:{ty} }}; \
             fn duplicate(source:&Word)->Word{{source.clone()}} \
             fn main()->Int{{let original=Word {{value:{literal}}}; \
             let copied=duplicate(&original); if copied.value == original.value {{73}} else {{0}}}}"
        );
        assert_eq!(execute(&source, true), 73, "{source}");
    }
}

#[test]
fn source_homonyms_do_not_inherit_registered_scalar_copy_permission() {
    for declaration in [
        "type affine Int64 is { value: Int };",
        "type UInt64 is { value: &mut Int };",
        "type Byte is { value: &unsafe Int };",
    ] {
        let name = declaration.split(" is ").next().unwrap().split_whitespace().last().unwrap();
        let source = format!("module foreign; {declaration} fn duplicate(source:&{name})->{name}{{source.clone()}}");
        let ast = Parser::new(&source).parse_module().unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_primitives();
        checker.set_current_module_path("foreign");
        let mut errors = checker.prepare_module_declarations(&ast);
        errors.extend(ast.items.iter().filter_map(|item| checker.check_item(item).err()));
        errors.extend(checker.take_deferred_errors());
        assert!(errors.iter().any(|error| error.to_diagnostic().code() == Some("E400")), "{source}: {errors:?}");
    }
}


fn execute_nested_archive(source: &str, entry: &str, expected: i64) {
    use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), source).unwrap();
    let output = directory.path().join("selected.vbca");
    let mut session = Session::new(CompilerOptions { input: core.join("mod.vr"), ..Default::default() });
    let result = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core().unwrap_or_else(|error| panic!("{error}\n{}", session.format_diagnostics()));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = verum_vbc::archive::read_archive_from_file(&output).unwrap();
    let produced = archive.load_module("core").unwrap();
    let wire = deserialize_module(&serialize_module(&produced).unwrap()).unwrap();
    for (route, module) in [("archive", produced), ("wire", wire)] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let function=module.find_function_by_name(entry).expect("declared archive entry");
        let mut vm=Interpreter::try_new_validated(module.into()).unwrap();
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(function).unwrap_or_else(|error| panic!("{route}: {error:?}")).try_as_i64(), Some(expected), "{route}");
    }
}

#[test]
fn nested_namespaces_preserve_selected_copy_through_archive_and_wire() {
    execute_nested_archive(r#"
        module core;
        public module outer { public module inner {
            type Leaf is {value:Int};
            fn duplicate(source:&Leaf)->Leaf {source.clone()}
            public fn probe()->Int {
                let mut original=Leaf {value:73};
                let copied=duplicate(&original);
                original.value=0;
                copied.value+original.value
            }
        }}
    "#, "core.outer.inner.probe", 73);
}

#[test]
fn generic_protocol_default_preserves_selected_copy_through_archive_and_wire() {
    execute_nested_archive(r#"
        module core;
        type Leaf is {value:Int};
        type Factory<A> is protocol {
            fn duplicate(&self, source:&Leaf)->Leaf {source.clone()}
            fn marker(&self, value:A)->A;
        };
        type Maker is {seed:Int};
        implement Factory<Int> for Maker {fn marker(&self, value:Int)->Int {value}}
        public fn probe()->Int {
            let maker=Maker {seed:0};
            let mut original=Leaf {value:73};
            let copied=maker.duplicate(&original);
            original.value=0;
            copied.value+original.value
        }
    "#, "core.probe", 73);
}

#[test]
fn explicit_protocol_override_preserves_declared_behavior_through_archive_and_wire() {
    execute_nested_archive(r#"
        module core;
            type Leaf is {value:Int};
            implement Leaf {fn next(&self)->Leaf {Leaf {value:self.value+1}}}
            type Factory<A> is protocol {
                fn duplicate(&self, source:&Leaf)->Leaf {source.clone()}
                fn marker(&self, value:A)->A;
            };
            type Maker is {seed:Int};
            implement Factory<Int> for Maker {
                fn duplicate(&self, source:&Leaf)->Leaf {source.next()}
                fn marker(&self, value:Int)->Int {value}
            }
            public fn probe()->Int {
                let maker=Maker {seed:0};
                let original=Leaf {value:72};
                maker.duplicate(&original).value
            }
    "#, "core.probe", 73);
}

#[test]
fn selected_nested_copy_isolates_every_owned_component_after_wire_reload() {
    for source in [
        r#"
        type Leaf is { value:Int };
        type Wrap<T> is { item:T };
        fn duplicate(source:&Wrap<Leaf>)->Wrap<Leaf> { source.clone() }
        fn main()->Int {
            let mut original=Wrap { item:Leaf { value:73 } };
            let mut copied=duplicate(&original);
            original.item.value=0;
            copied.item.value + original.item.value
        }
        "#,
        r#"
        type Leaf is { value:Int };
        type Batch is { items:[Leaf;2] };
        fn duplicate(source:&Batch)->Batch { source.clone() }
        fn main()->Int {
            let mut original=Batch { items:[Leaf {value:40}, Leaf {value:33}] };
            let copied=duplicate(&original);
            original.items[0].value=0;
            original.items[1].value=0;
            copied.items[0].value+copied.items[1].value
        }
        "#,
        r#"
        type Leaf is { value:Int };
        type Choice is Empty | Data(Leaf);
        fn duplicate(source:&Choice)->Choice { source.clone() }
        fn main()->Int {
            let mut original=Choice.Data(Leaf {value:73});
            let copied=duplicate(&original);
            match original { Data(ref mut leaf)=>leaf.value=0, Empty=>() };
            match copied { Data(leaf)=>leaf.value, Empty=>0 }
        }
        "#,
    ] {
        assert_eq!(execute(source, true), 73, "{source}");
    }
}
