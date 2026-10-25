//! A real bootstrap archive must retain witnesses across forward module calls.
use std::fs;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{
    Instruction,
    archive::read_archive_from_file,
    bytecode::decode_instructions,
    types::{TypeId, TypeRef},
};

#[test]
fn forward_generic_constructor_retains_nested_unit_witness_in_archive() {
    check_forward_constructor(false, false);
}

#[test]
fn reexported_forward_constructor_retains_nested_unit_witness_in_archive() {
    check_forward_constructor(true, false);
}

#[test]
fn file_module_forward_constructor_retains_nested_unit_witness_in_archive() {
    check_forward_constructor(false, true);
}

#[test]
fn reexported_file_module_constructor_retains_nested_unit_witness_in_archive() {
    check_forward_constructor(true, true);
}

fn check_forward_constructor(reexported: bool, file_module: bool) {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir_all(core.join("provider")).unwrap();
    let input = core.join("mod.vr");
    fs::write(
        &input,
        r#"
        module core;
        public module provider;
        mount core.provider.Holder;
        type Choice<T> is Empty | Value(T);
        static ROOT: Holder<Choice<()>> = Holder.new(Choice.Empty);
    "#,
    )
    .unwrap();
    let provider = if reexported || file_module {
        fs::write(core.join("provider/mod.vr"),
            "module core.provider; public module inner; public mount .inner.Holder;").unwrap();
        if file_module {
            if !reexported {
                let source = fs::read_to_string(&input).unwrap();
                fs::write(&input, source.replace("mount core.provider.Holder", "mount core.provider.inner.Holder")).unwrap();
            }
            core.join("provider/inner.vr")
        } else {
            fs::create_dir_all(core.join("provider/inner")).unwrap();
            core.join("provider/inner/mod.vr")
        }
    } else {
        core.join("provider/mod.vr")
    };
    let owner = if reexported || file_module { "core.provider.inner" } else { "core.provider" };
    fs::write(
        provider,
        format!("module {owner};") + r#"
        public type Holder<T> is { item: T };
        implement<T> Holder<T> {
            public fn new(item: T) -> Holder<T> { Holder { item: item } }
        }
    "#,
    )
    .unwrap();
    let output = root.path().join("runtime.vbca");
    let config = CoreConfig::new(&core).with_output(&output);
    let mut session = Session::new(CompilerOptions {
        input,
        ..Default::default()
    });
    let result = CompilationPipeline::new_core(&mut session, config)
        .compile_core()
        .unwrap();
    assert_eq!(result.modules_compiled, if reexported && !file_module { 3 } else { 2 });
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&output).unwrap();
    let module = archive.load_module("core").unwrap();
    let init = module
        .functions
        .iter()
        .find(|f| module.strings.get(f.name) == Some("__tls_init_ROOT"))
        .unwrap();
    let body = decode_instructions(
        &module.bytecode
            [init.bytecode_offset as usize..(init.bytecode_offset + init.bytecode_length) as usize],
    )
    .unwrap();
    let args: Vec<_> = body
        .iter()
        .filter_map(|op| match op {
            Instruction::CallG { type_args, .. } => Some(type_args),
            _ => None,
        })
        .collect();
    assert_eq!(args.len(), 1, "{body:?}");
    assert!(
        matches!(&args[0][0], TypeRef::Instantiated { args, .. }
        if args == &[TypeRef::Concrete(TypeId::UNIT)]),
        "{args:?}"
    );
}
