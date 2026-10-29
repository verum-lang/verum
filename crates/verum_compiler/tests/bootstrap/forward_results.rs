//! Forward calls retain source owners before the provider body is compiled.
use super::*;
use crate::Session;
use verum_common::Text;
use verum_fast_parser::Parser;

fn parsed() -> Vec<(String, Vec<(PathBuf, verum_ast::Module)>)> {
    [
        ("core.base.result", "public type Result<T, E> is Ok(T) | Err(E);"),
        ("core.handles", "public type Descriptor is (Int); implement Descriptor { public fn raw(&self) -> Int { self.0 } }"),
        ("core.provider", "mount core.handles.Descriptor; public fn acquire() -> Result<Descriptor, Int> { Result.Ok(Descriptor(7)) }"),
        ("core.other", "public type Descriptor is (Int); implement Descriptor { public fn raw(&self) -> Int { 99 } } public fn acquire() -> Bool { false }"),
        ("core.consumer", "mount core.provider.acquire as open_handle; public fn probe() -> Int { match open_handle() { Result.Ok(value) => value.raw(), Result.Err(_) => 0 } }"),
    ].into_iter().map(|(owner, source)| {
        let ast = Parser::new(&format!("module {owner}; {source}")).parse_module().unwrap();
        (owner.to_owned(), vec![(PathBuf::from(format!("{owner}.vr")), ast)])
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
        forward_result_case(serialized);
    }
}

fn forward_result_case(serialized: bool) {
    let parsed = parsed();
    let mut session = Session::new(Default::default());
    {
        let registry = session.module_registry();
        let mut registry = registry.write();
        for (owner, files) in &parsed {
            let ast = &files[0].1;
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
        .pre_register_canonical_variant_constructors(&parsed)
        .unwrap();
    pipeline
        .pre_register_public_free_functions(&parsed)
        .unwrap();
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
                &[&files[0].1],
                &config,
                &target,
                &Default::default(),
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
