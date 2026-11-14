//! Language-owned aliases keep one declaration across source and archive paths.
//! Opaque pointer ABI admission does not manufacture an ownership witness.
use std::sync::Arc;
use verum_ast::{FileId, Module};
use verum_common::{List, Text};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_fast_parser::{FastParser, Parser};
use verum_types::{Type, TypeChecker};
use verum_vbc::{
    VbcModule,
    archive::{ArchiveBuilder, VbcArchive, read_archive, read_archive_from_file, write_archive},
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
    types::{FieldDescriptor, TypeDescriptor, TypeId, TypeRef},
    validate::validate_module,
};

fn fields<'a>(module: &'a VbcModule, owner: &str, name: &str) -> &'a TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| {
            ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
                && module.get_string(ty.name).is_some_and(|declared| {
                    declared == name || declared == format!("{owner}.{name}")
                })
        })
        .unwrap_or_else(|| panic!("missing {owner}.{name}"))
}

fn field<'a>(module: &'a VbcModule, ty: &'a TypeDescriptor, name: &str) -> &'a FieldDescriptor {
    ty.fields
        .iter()
        .find(|field| module.get_string(field.name) == Some(name))
        .unwrap()
}

fn interface(sources: &[(&str, &str)]) -> Result<VbcModule, Text> {
    let modules: List<Module> = sources
        .iter()
        .enumerate()
        .map(|(index, (_, source))| {
            FastParser::new()
                .parse_module_str(source, FileId::new(index as u32))
                .unwrap()
        })
        .collect();
    VbcCodegen::with_config(CodegenConfig::new("core"))
        .collect_declaration_interface(&modules.iter().collect::<List<_>>())
        .map_err(|error| error.to_string().into())
}

fn roundtrip(module: &VbcModule) -> VbcModule {
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core", module, &[]).unwrap();
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let module = archive.load_module("core").unwrap();
    validate_module(&module).unwrap();
    module
}

fn compile(sources: &[(&str, &str)]) -> Result<VbcArchive, Text> {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("core");
    std::fs::create_dir(&source).unwrap();
    for (path, text) in sources {
        std::fs::write(source.join(path), text).unwrap();
    }
    let output = root.path().join("aliases.vbca");
    let mut session = Session::new(CompilerOptions {
        input: source.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&source).with_output(&output))
            .compile_core()
            .map_err(|error| Text::from(error.to_string()))?;
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    read_archive_from_file(&output).map_err(|error| error.to_string().into())
}

fn execute(module: VbcModule, name: &str) {
    validate_module(&module).unwrap();
    let entry = module.find_function_by_name(name).unwrap();
    let mut vm = Interpreter::new(Arc::new(module));
    vm.run_global_ctors().unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
}

#[test]
fn direct_alias_layout_keeps_pointer_semantics_unclaimed() {
    let module = interface(&[(
        "mod.vr",
        "module core; public type Storage is {
        public pointer: RawPtr, public nested: (Int, RawPtr),
        public raw: *mut Int, public reference: &unsafe Int,
        public epoch: Epoch, public generation: u32
    }; public type Stored is Pointer(RawPtr) | Record {pointer:RawPtr};
    public type Wrapped is (RawPtr);
    public fn pointer(value: RawPtr) -> RawPtr;",
    )])
    .unwrap();
    for module in [module.clone(), roundtrip(&module)] {
        let storage = fields(&module, "core", "Storage");
        assert_eq!(
            field(&module, storage, "pointer").type_ref,
            TypeRef::Concrete(TypeId::PTR)
        );
        for name in ["pointer", "nested", "raw"] {
            assert_eq!(
                field(&module, storage, name).declaration_type,
                None,
                "{name}"
            );
        }
        assert!(
            matches!(&field(&module, storage, "reference").declaration_type,
            Some(TypeRef::Reference {tier:verum_vbc::types::CbgrTier::Tier2,inner,..})
            if **inner==TypeRef::Concrete(TypeId::INT))
        );
        let stored = fields(&module, "core", "Stored");
        for variant in &stored.variants {
            assert_eq!(variant.fields.len(), 1);
            assert_eq!(variant.fields[0].declaration_type, None);
        }
        assert_eq!(
            fields(&module, "core", "Wrapped").fields[0].declaration_type,
            None
        );
        assert_eq!(
            field(&module, storage, "epoch").declaration_type,
            Some(TypeRef::Concrete(TypeId::INT))
        );
        assert_eq!(
            field(&module, storage, "generation").declaration_type,
            Some(TypeRef::Concrete(TypeId::U32))
        );
        assert!(storage.source_declaration.is_some());
        let pointer = module
            .find_function_by_name("core.pointer")
            .or_else(|| module.find_function_by_name("pointer"))
            .unwrap_or_else(|| {
                panic!(
                    "interface functions: {:?}",
                    module
                        .functions
                        .iter()
                        .map(|function| module.get_string(function.name))
                        .collect::<List<_>>()
                )
            });
        let pointer = module.get_function(pointer).unwrap();
        assert_eq!(pointer.semantic_return, None);
        assert!(
            pointer
                .semantic_params
                .as_ref()
                .is_none_or(|parameters| parameters.iter().all(Option::is_none))
        );
    }
}

#[test]
fn actual_source_archive_and_vm_share_registered_alias_targets() {
    let archive = compile(&[
        (
            "mod.vr",
            "module core; public module provider; public module consumer; public module decoy;",
        ),
        (
            "decoy.vr",
            "module core.decoy; public type RawPtr is Bool; public type Epoch is Bool;",
        ),
        (
            "provider.vr",
            "module core.provider; public type Storage is {
            public pointer:RawPtr, public epoch:Epoch, public generation:u32
        }; public fn epoch(value:Epoch)->Epoch {value}
        public fn generation(value:u32)->u32 {value}",
        ),
        (
            "consumer.vr",
            "module core.consumer; mount core.provider.{epoch,generation};
            public fn probe()->Int {let value:Epoch=73; let count:u32=19;
                if generation(count)==19 {epoch(value)} else {0}}",
        ),
    ])
    .unwrap();
    let produced = archive.load_module("core").unwrap();
    let storage = fields(&produced, "core.provider", "Storage");
    assert_eq!(
        field(&produced, storage, "pointer").type_ref,
        TypeRef::Concrete(TypeId::PTR)
    );
    assert_eq!(
        field(&produced, storage, "epoch").type_ref,
        TypeRef::Concrete(TypeId::INT)
    );
    execute(produced, "core.consumer.probe");
    let metadata = verum_compiler::archive_metadata::archive_to_core_metadata(&archive).unwrap();
    for eager in [false, true] {
        for (mount, body, valid) in [
            (
                "mount core.provider.Storage;",
                "fn read(value:&Storage)->Epoch {value.epoch}",
                true,
            ),
            (
                "mount core.provider.Storage;",
                "fn read(value:&Storage)->Bool {value.epoch}",
                false,
            ),
            (
                "mount core.provider.Storage;",
                "fn read(value:&Storage)->RawPtr {value.pointer}",
                true,
            ),
            (
                "mount core.provider.Storage;",
                "fn read(value:&Storage)->&unsafe Bool {value.pointer}",
                false,
            ),
            (
                "mount core.decoy.Epoch;",
                "fn read(value:Epoch)->Bool {value}",
                true,
            ),
            (
                "mount core.decoy.Epoch;",
                "fn read(value:Epoch)->Int {value}",
                false,
            ),
            (
                "type Epoch is Bool;",
                "fn read(value:Epoch)->Bool {value}",
                true,
            ),
        ] {
            let mut checker = TypeChecker::with_minimal_context();
            checker.register_builtins();
            let metadata = Arc::new(metadata.clone());
            if eager {
                checker.load_core_eager(metadata);
            } else {
                checker.set_core_metadata(metadata);
            }
            checker.set_current_module_path("reader");
            let ast = Parser::new(&format!("{mount} {body}"))
                .parse_module()
                .unwrap();
            let mut errors = checker.prepare_module_declarations(&ast);
            errors.extend(
                ast.items
                    .iter()
                    .filter_map(|item| checker.check_item(item).err()),
            );
            errors.extend(checker.take_deferred_errors());
            if mount == "mount core.provider.Storage;" {
                let Some(Type::Record(fields)) =
                    checker.resolved_type("__struct_fields_core.provider.Storage")
                else {
                    panic!("missing source-owned fields: {errors:?}");
                };
                assert!(
                    matches!(fields.get(&Text::from("pointer")),
                        Some(Type::Pointer { mutable: true, inner }) if **inner == Type::Int),
                    "eager={eager}: {fields:?}"
                );
            }
            assert_eq!(
                errors.is_empty() && checker.diagnostic_sources().is_empty(),
                valid,
                "eager={eager} {body}: {errors:?} {:?}",
                checker.diagnostic_sources()
            );
        }
    }
}

#[test]
fn lexical_shadowing_preserves_the_language_alias_definition_scope() {
    let archive = compile(&[(
        "mod.vr",
        "module core;
        public type UInt32 is {public foreign:Bool};
        public type RawPtr is {public flag:Bool};
        public type Storage is {public pointer:RawPtr, public generation:u32};
        public type Bound<RawPtr> is {public value:RawPtr};
        public fn probe()->Int {
            let value=Storage{pointer:RawPtr{flag:true},generation:7};
            let bound: Bound<Bool> = Bound {value:true};
            if value.pointer.flag && value.generation==7 && bound.value {73} else {0}
        }",
    )])
    .unwrap();
    let module = archive.load_module("core").unwrap();
    let storage = fields(&module, "core", "Storage");
    assert_eq!(
        field(&module, storage, "pointer").type_ref,
        TypeRef::Concrete(fields(&module, "core", "RawPtr").id)
    );
    assert_eq!(
        field(&module, storage, "generation").type_ref,
        TypeRef::Concrete(TypeId::U32)
    );
    assert_eq!(
        fields(&module, "core", "Bound").fields[0].declaration_type,
        Some(TypeRef::Generic(verum_vbc::types::TypeParamId(0)))
    );
    execute(module, "core.probe");
}

#[test]
fn callable_binders_shadow_language_aliases() {
    let archive = compile(&[(
        "mod.vr",
        "module core;
        public fn identity<RawPtr>(value:RawPtr)->RawPtr {value}
        public fn probe()->Int {if identity<Bool>(true) {73} else {0}}",
    )])
    .unwrap();
    execute(archive.load_module("core").unwrap(), "core.probe");
}

#[test]
fn explicit_imported_homonym_owns_its_spelling() {
    let sources = [
        (
            "mod.vr",
            "module core; public module provider; public module consumer;",
        ),
        (
            "provider.vr",
            "module core.provider; public type Handle is {public flag:Bool};",
        ),
        (
            "consumer.vr",
            "module core.consumer; mount core.provider.Handle as RawPtr;
            public type Storage is {public pointer:RawPtr};
            public fn probe()->Int {let value=Storage{pointer:RawPtr{flag:true}};
                if value.pointer.flag {73} else {0}}",
        ),
    ];
    let archive = compile(&sources).unwrap();
    for module in [
        interface(&sources).unwrap(),
        archive.load_module("core").unwrap(),
    ] {
        let selected = fields(&module, "core.consumer", "Storage");
        assert_eq!(
            field(&module, selected, "pointer").type_ref,
            TypeRef::Concrete(fields(&module, "core.provider", "Handle").id)
        );
    }
    execute(archive.load_module("core").unwrap(), "core.consumer.probe");
}

#[test]
fn unknown_or_refused_source_bindings_never_acquire_language_authority() {
    for (consumer, expected, written) in [
        (
            "mount core.provider.Hidden as RawPtr; public type Storage is {pointer:RawPtr};",
            "Hidden",
            "RawPtr",
        ),
        (
            "mount core.provider.Missing as Epoch; public type Storage is {epoch:Epoch};",
            "Missing",
            "Epoch",
        ),
        (
            "public type Storage is {epoch:LooksScalar};",
            "LooksScalar",
            "LooksScalar",
        ),
        (
            "public type Storage is {pointer:unbound.RawPtr};",
            "unbound",
            "unbound.RawPtr",
        ),
    ] {
        let consumer = format!("module core.consumer; {consumer}");
        let sources = [
            (
                "mod.vr",
                "module core; public module provider; public module consumer;",
            ),
            (
                "provider.vr",
                "module core.provider; type Hidden is Int; public type LooksScalar is Int;",
            ),
            ("consumer.vr", consumer.as_str()),
        ];
        let raw = interface(&sources).expect_err("invalid VBC source alias binding accepted");
        assert!(
            raw.contains(expected) || raw.contains(&format!("'{written}'")),
            "{raw}"
        );
        let ordinary = compile(&sources).expect_err("invalid source alias binding accepted");
        assert!(ordinary.contains(expected), "{ordinary}");
    }
}
