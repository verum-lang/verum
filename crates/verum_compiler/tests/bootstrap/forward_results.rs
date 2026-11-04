//! Forward calls retain source owners before the provider body is compiled.
use super::*;
use crate::Session;
use verum_common::Text;
use verum_fast_parser::Parser;

fn parsed() -> Vec<(String, Vec<OwnedSourceFile>)> {
    [
        ("core.base.result", "public type Result<T, E> is Ok(T) | Err(E);"),
        ("core.handles", "public type Descriptor is (Int); implement Descriptor { public fn raw(&self) -> Int { self.0 } }"),
        ("core.provider", "mount core.handles.Descriptor; public fn acquire() -> Result<Descriptor, Int> { Result.Ok(Descriptor(7)) }"),
        ("core.other", "public type Descriptor is (Int); implement Descriptor { public fn raw(&self) -> Int { 99 } } public fn acquire() -> Bool { false }"),
        ("core.consumer", "mount core.provider.acquire as open_handle; public fn probe() -> Int { match open_handle() { Result.Ok(value) => value.raw(), Result.Err(_) => 0 } }"),
    ].into_iter().map(|(owner, source)| {
        let ast = Parser::new(&format!("module {owner}; {source}")).parse_module().unwrap();
        (owner.to_owned(), vec![OwnedSourceFile {path: PathBuf::from(format!("{owner}.vr")), owner: owner.into(), ast}])
    }).collect()
}

#[test]
fn colliding_free_function_stubs_keep_both_source_owners() {
    let mut session = Session::new(Default::default());
    let mut pipeline = CompilationPipeline::new_core(&mut session, CoreConfig::new("."));
    pipeline
        .pre_register_public_free_functions(&parsed())
        .unwrap();
    let alpha = pipeline
        .global_function_registry
        .get("core.provider.acquire")
        .expect("source-qualified forward declaration");
    let beta = pipeline
        .global_function_registry
        .get("core.other.acquire")
        .expect("a colliding leaf does not erase the other declaration");
    assert_ne!(alpha.id, beta.id);
    assert!(!pipeline.global_function_registry.contains_key("acquire"));
}

#[test]
fn forward_result_payload_selects_its_foreign_nominal_method() {
    for serialized in [false, true] {
        for relative in [false, true] { forward_result_case(serialized, relative); }
    }
}

fn forward_result_case(serialized: bool, relative: bool) {
    let mut parsed = parsed();
    if relative {
        parsed[2].1[0].ast = Parser::new("module core.provider; mount super.handles.Descriptor; public fn acquire() -> Result<Descriptor, Int> { Result.Ok(Descriptor(7)) }").parse_module().unwrap();
        parsed[4].1[0].ast = Parser::new("module core.consumer; mount super.provider.acquire as open_handle; public fn probe() -> Int { match open_handle() { Result.Ok(value) => value.raw(), Result.Err(_) => 0 } }").parse_module().unwrap();
    }
    let mut session = Session::new(Default::default());
    {
        let registry = session.module_registry();
        let mut registry = registry.write();
        for (owner, files) in &parsed {
            let ast = &files[0].ast;
            let id = registry.allocate_id();
            let path = ModulePath::from_str(owner);
            let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, Text::new());
            info.exports = extract_exports_from_module(ast, id, &path).unwrap();
            registry.register(info);
        }
        resolve_specific_reexport_kinds(&mut registry).unwrap();
    }
    let config = CoreConfig::new(".");
    let target = verum_ast::cfg::TargetConfig::host();
    let mut pipeline = CompilationPipeline::new_core(&mut session, config.clone());
    pipeline
        .pre_register_canonical_static_methods(&parsed)
        .unwrap();
    pipeline
        .pre_register_public_free_functions(&parsed)
        .unwrap();
    // The declaring interfaces now publish constructor identities. Follow the
    // production bootstrap path instead of the removed spelling-based pass.
    let interfaces = pipeline
        .collect_core_declaration_interfaces(&parsed, &config, &target)
        .unwrap();
    let interfaces: Vec<_> = interfaces
        .iter()
        .map(|module| {
            if serialized {
                let bytes = verum_vbc::serialize::serialize_module(module).unwrap();
                verum_vbc::deserialize::deserialize_module(&bytes).unwrap()
            } else {
                module.clone()
            }
        })
        .collect();
    // The consumer deliberately precedes both free-function bodies.
    for index in [0, 3, 1, 4] {
        let (name, files) = &parsed[index];
        let module = StdlibModule {
            name: name.clone(),
            source_files: vec![],
            dependencies: vec![],
        };
        let (compiled, _) = pipeline
            .compile_core_module_from_ast(
                &module,
                &[(files[0].owner.as_str(), &files[0].ast)],
                &config,
                &target,
                &interfaces,
            )
            .unwrap();
        if index == 4 {
            let function = compiled
                .find_function_by_name("core.consumer.probe")
                .unwrap();
            let descriptor = compiled.get_function(function).unwrap();
            let mut pc = descriptor.bytecode_offset as usize;
            let end = pc + descriptor.bytecode_length as usize;
            let mut owner_selected = false;
            let mut observed = Vec::new();
            while pc < end {
                let instruction =
                    verum_vbc::bytecode::decode_instruction(&compiled.bytecode, &mut pc).unwrap();
                let method = match instruction {
                    verum_vbc::Instruction::CallM { method_id, .. } => {
                        compiled.get_string(verum_vbc::types::StringId(method_id))
                    }
                    verum_vbc::Instruction::Call { func_id, .. } => compiled
                        .external_function_names
                        .iter()
                        .find(|(id, _)| id.0 == func_id)
                        .and_then(|(_, name)| compiled.get_string(*name))
                        .or_else(|| {
                            compiled
                                .functions
                                .iter()
                                .find(|function| function.id.0 == func_id)
                                .and_then(|function| compiled.get_string(function.name))
                        }),
                    _ => None,
                };
                if let Some(method) = method {
                    observed.push(method.to_owned());
                    assert_ne!(
                        method, "raw",
                        "an integer carrier cannot recover the nominal owner"
                    );
                    assert_ne!(method, "core.other.Descriptor.raw");
                    owner_selected |= method == "core.handles.Descriptor.raw";
                }
            }
            assert!(
                owner_selected,
                "payload method must retain its source owner"
            );
        }
        pipeline
            .compiled_stdlib_modules
            .insert(name.clone(), compiled);
    }
}


#[test]
fn protocol_method_forward_ids_belong_to_the_selected_declarations() {
    let mut parsed = Vec::new();
    for owner in ["core.first", "core.second"] {
        let source = format!(
            "module {owner}; type Reader is protocol {{ fn read(&self) -> Int; }}; type Handle is {{ number: Int }}; implement Reader for Handle {{ fn read(&self) -> Int {{ self.number }} }}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        parsed.push((
            owner.to_owned(),
            vec![OwnedSourceFile {path: PathBuf::from(format!("{owner}.vr")), owner: owner.into(), ast}],
        ));
    }
    let mut session = Session::new(Default::default());
    let config = CoreConfig::new(".");
    let mut pipeline = CompilationPipeline::new_core(&mut session, config.clone());
    pipeline
        .pre_register_canonical_static_methods(&parsed)
        .unwrap();
    let _interfaces = pipeline
        .collect_core_declaration_interfaces(
            &parsed,
            &config,
            &verum_ast::cfg::TargetConfig::host(),
        )
        .unwrap();
    let mut ids = std::collections::HashSet::new();
    // Source owners, not the interface bundle partition, define identity.
    for (owner, _) in &parsed {
        let key = format!("{owner}.Handle.read");
        let forward = pipeline.global_function_registry.get(&key).unwrap_or_else(|| {
            let mut keys: Vec<_> = pipeline.global_function_registry.keys().collect();
            keys.sort();
            panic!("missing declaration {key}; registered: {keys:?}");
        });
        assert!(verum_vbc::stub_ranges::in_stage1(forward.id.0));
        assert!(ids.insert(forward.id));
        assert!(forward.has_source_body);
        assert!(
            forward.callable_signature.is_none(),
            "the consumer imports pool-local types"
        );
    }
    assert!(
        !pipeline
            .global_function_registry
            .contains_key("Handle.read")
    );
}


#[test]
fn same_leaf_file_types_keep_stage1_constants_methods_and_generic_slots() {
    for reverse in [false, true] {
        // The archive group is intentionally shared. Only the per-file owner
        // distinguishes these declarations; both parser ASTs have FileId(0).
        let mut files: Vec<_> = [
            ("fixture.cells.alpha", "alpha.vr", "public type Cell<T> is { value:T }; implement Cell<T> { public const DEFAULT:Int=11; public fn read(&self)->T {self.value} public fn convert<V>(&self,value:V)->V {value} }"),
            ("fixture.cells.beta", "beta.vr", "public type Cell<U> is { value:U }; implement Cell<U> { public const DEFAULT:Bool=false; public fn read(&self)->U {self.value} public fn convert<V>(&self,value:V)->V {value} }"),
        ].into_iter().map(|(owner, path, source)| OwnedSourceFile {
            path: PathBuf::from(path),
            owner: owner.into(),
            ast: Parser::new(source).parse_module().expect("headerless generic source"),
        }).collect();
        if reverse { files.reverse(); }
        let sources = vec![("fixture.cells".to_owned(), files)];
        let mut session = Session::new(Default::default());
        let mut pipeline = CompilationPipeline::new_core(&mut session, CoreConfig::new("."));
        pipeline.pre_register_canonical_static_methods(&sources).unwrap();
        let mut ids = std::collections::HashSet::new();
        for (owner, parameter, constant_type) in [
            ("fixture.cells.alpha", "T", "Int"),
            ("fixture.cells.beta", "U", "Bool"),
        ] {
            let constant_key = format!("{owner}.Cell.DEFAULT");
            let constant = pipeline.global_function_registry.get(&constant_key)
                .unwrap_or_else(|| panic!("missing source-owned constant {constant_key}"));
            assert!(constant.is_const);
            assert_eq!(constant.return_type_name.as_deref(), Some(constant_type));
            assert!(verum_vbc::stub_ranges::in_stage1(constant.id.0));
            assert!(ids.insert(constant.id), "source-owned constants must not share a stub");

            let read_key = format!("{owner}.Cell.read");
            let read = pipeline.global_function_registry.get(&read_key)
                .unwrap_or_else(|| panic!("missing source-owned method {read_key}"));
            assert!(!read.is_const);
            assert!(read.has_source_body);
            assert_eq!(read.return_type_name.as_deref(), Some(parameter));
            assert_eq!(read.type_param_ids, vec![verum_vbc::types::TypeParamId(0)],
                "each owner's parent generic must survive the same-leaf collision");
            assert!(read.explicit_type_param_ids.is_empty());
            assert!(verum_vbc::stub_ranges::in_stage1(read.id.0));
            assert!(ids.insert(read.id));

            let convert_key = format!("{owner}.Cell.convert");
            let convert = pipeline.global_function_registry.get(&convert_key)
                .unwrap_or_else(|| panic!("missing source-owned method {convert_key}"));
            assert_eq!(convert.type_param_ids,
                vec![verum_vbc::types::TypeParamId(0), verum_vbc::types::TypeParamId(1)]);
            assert_eq!(convert.explicit_type_param_ids,
                vec![Some(verum_vbc::types::TypeParamId(1))],
                "the method argument follows its own retained parent slot");
            assert!(verum_vbc::stub_ranges::in_stage1(convert.id.0));
            assert!(ids.insert(convert.id));
        }
        assert_eq!(ids.len(), 6);
        for short in ["Cell.DEFAULT", "Cell.read", "Cell.convert"] {
            assert!(!pipeline.global_function_registry.contains_key(short),
                "non-prelude same-leaf declarations cannot publish {short}");
        }
    }
}

#[test]
fn file_mounted_archive_publication_refuses_an_invalid_dependency_name() {
    let mut session = Session::new(Default::default());
    let config = CoreConfig::new(".");
    let mut pipeline = CompilationPipeline::new_core(&mut session, config.clone());
    let mut module = verum_vbc::VbcModule::new("mounted".into());
    module.dependencies.push(verum_vbc::module::ModuleDependency {
        name: verum_vbc::types::StringId(u32::MAX), hash: 0,
    });
    pipeline.compiled_stdlib_modules.insert("mounted".into(), module);
    let error = pipeline.build_stdlib_archive(&config).unwrap_err();
    assert!(format!("{error:#}").contains("module mounted has an invalid dependency string index"), "{error:#}");
}
