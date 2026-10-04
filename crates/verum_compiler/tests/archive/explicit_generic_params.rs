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
    let mut builder = verum_vbc::archive::ArchiveBuilder::new();
    builder
        .add_module("callable_archive", &module, &[])
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
        // The wire ABI alone is not evidence of a proven local closure signature.
        assert!(info.callable_signature.is_none());
    }
}
