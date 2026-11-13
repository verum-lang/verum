use super::*;

#[test]
fn archive_loaders_preserve_declared_generic_slots() {
    let ast = verum_fast_parser::Parser::new(
        r#"
type Factory<T> is { value: T };
implement<T> Factory<T> { fn build<T, Output>(self) -> Output { Output.default() } }
"#,
    )
    .parse_module()
    .unwrap();
    let module = verum_vbc::codegen::VbcCodegen::with_config(
        verum_vbc::codegen::CodegenConfig::new("generic_archive"),
    )
    .compile_module(&ast)
    .unwrap();
    let desc = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .unwrap()
                .ends_with("Factory.build")
        })
        .unwrap();
    let explicit = desc.explicit_type_param_ids.clone();
    let roster: Vec<_> = desc.type_params.iter().map(|p| p.id).collect();
    assert_eq!(
        explicit,
        vec![
            Some(verum_vbc::types::TypeParamId(0x8000)),
            Some(verum_vbc::types::TypeParamId(1))
        ]
    );
    let mut builder = verum_vbc::archive::ArchiveBuilder::new();
    builder.add_module("generic_archive", &module, &[]).unwrap();
    let archive = builder.finish();
    let mut bytes = Vec::new();
    verum_vbc::archive::write_archive(&archive, &mut bytes).unwrap();
    let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes)).unwrap();
    let mut full = CodegenContext::new();
    populate_ctx_from_archive(&archive, &mut full, &mut 0).unwrap();
    let loaded = archive.load_module("generic_archive").unwrap();
    let mut filtered = CodegenContext::new();
    let wanted = ["Factory".to_owned(), "Factory.build".to_owned()]
        .into_iter()
        .collect();
    register_module_filtered(&loaded, "generic_archive", &mut filtered, &wanted, &mut 0);
    for ctx in [&full, &filtered] {
        let info = ctx
            .lookup_function("generic_archive.Factory.build")
            .expect("exact method import");
        assert_eq!(info.explicit_type_param_ids, explicit);
        assert_eq!(info.type_param_ids, roster);
    }
}

#[test]
fn archive_loaders_preserve_callable_parameter_declaration_identity() {
    let ast = verum_fast_parser::Parser::new(
        r#"
type Payload is { value: Int };
type Stage<F> is { value: F };
implement<F> Stage<F> { fn transform<F: fn(Payload) -> Payload>(self, f: F) -> F { f } }
"#,
    )
    .parse_module()
    .unwrap();
    let module = verum_vbc::codegen::VbcCodegen::with_config(
        verum_vbc::codegen::CodegenConfig::new("callable_archive"),
    )
    .compile_module(&ast)
    .unwrap();
    use verum_vbc::{TypeRef, TypeParamId};

    let method = module.functions.iter().position(|function| {
        module.get_string(function.name).is_some_and(|name| name.ends_with("Stage.transform"))
    }).unwrap();
    let signature = module.functions[method].semantic_callable_signature().unwrap();
    let TypeRef::Function { params, return_type, .. } = &signature else { panic!("function signature") };
    let shadowed = TypeRef::Generic(TypeParamId(0x8000));
    assert_eq!(params.len(), 2);
    assert_eq!(params[1], shadowed);
    assert_eq!(**return_type, shadowed);

    let round_trip = |module: &verum_vbc::VbcModule, expected: Option<&TypeRef>| {
        let mut builder = verum_vbc::archive::ArchiveBuilder::new();
        builder
            .add_module("callable_archive", module, &[])
            .unwrap();
        let mut bytes = Vec::new();
        verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
        let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes)).unwrap();
        let mut full = CodegenContext::new();
        populate_ctx_from_archive(&archive, &mut full, &mut 0).unwrap();
        let loaded = archive.load_module("callable_archive").unwrap();
        let mut filtered = CodegenContext::new();
        let wanted = ["Stage".to_owned(), "Stage.transform".to_owned()]
            .into_iter()
            .collect();
        register_module_filtered(&loaded, "callable_archive", &mut filtered, &wanted, &mut 0);
        for ctx in [&full, &filtered] {
            let info = ctx
                .lookup_function("callable_archive.Stage.transform")
                .expect("exact method import");
            assert_eq!(
                ctx.archive_fn_parameter_generics
                    .get(&info.id.0)
                    .unwrap()
                    .as_slice(),
                &[None, Some(verum_vbc::types::TypeParamId(0x8000))]
            );
            assert_eq!(info.callable_signature.as_ref(), expected);
        }
    };
    round_trip(&module, Some(&signature));
    // Keep the ABI and generic identity intact while removing one source witness.
    let mut missing_parameter = module.clone();
    missing_parameter.functions[method].semantic_params.as_mut().unwrap()[1] = None;
    round_trip(&missing_parameter, None);
    let mut missing_result = module;
    missing_result.functions[method].semantic_return = None;
    round_trip(&missing_result, None);
}
