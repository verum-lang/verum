//! Runs the actual bootstrap publication boundary on a small in-memory source unit.
use super::*;
use crate::Session;
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::codegen::FunctionInfo;
use verum_vbc::module::FunctionId;

#[test]
fn source_publication_replaces_stubs_and_keeps_existing_real_entries() {
    let mut session = Session::new(Default::default());
    let config = CoreConfig::new(".");
    let mut pipeline = CompilationPipeline::new_core(&mut session, config.clone());
    let preserved = FunctionInfo {
        id: FunctionId(50000),
        param_count: 1,
        param_names: List::from_iter(["original_binding".into()]).into(),
        ..Default::default()
    };
    pipeline
        .global_function_registry
        .insert("select".into(), preserved.clone());
    pipeline
        .global_function_registry
        .insert("prior.untouched".into(), preserved.clone());
    pipeline.global_function_registry.insert(
        "bundle.alpha.select".into(),
        FunctionInfo {
            id: FunctionId(verum_vbc::stub_ranges::STAGE1_BASE),
            param_count: 1,
            ..Default::default()
        },
    );
    let module = StdlibModule {
        name: "bundle".into(),
        source_files: List::new().into(),
        dependencies: List::new().into(),
    };
    let alpha = Parser::new("module alpha; public fn select<T>(value: T)->T {value}")
        .parse_module()
        .unwrap();
    let beta = Parser::new(
        "module beta; public fn apply<T,F:fn(T)->T>(value:T, callback:F)->T {callback(value)}",
    )
    .parse_module()
    .unwrap();
    let (artifact, count) = pipeline
        .compile_core_module_from_ast(
            &module,
            &[&alpha, &beta],
            &config,
            &verum_ast::cfg::TargetConfig::host(),
            &Default::default(),
        )
        .expect("small source-driven bootstrap unit");
    assert!(count >= 2);
    let selected = &pipeline.global_function_registry["bundle.alpha.select"];
    assert!(!verum_vbc::stub_ranges::is_stub_id(selected.id.0));
    assert_eq!(selected.param_names.as_slice(), &["value"]);
    assert_eq!(selected.explicit_type_param_ids.len(), 1);
    assert_eq!(
        format!("{:?}", pipeline.global_function_registry["select"]),
        format!("{preserved:?}")
    );
    assert_eq!(
        format!("{:?}", pipeline.global_function_registry["prior.untouched"]),
        format!("{preserved:?}")
    );
    let callable = &pipeline.global_function_registry["bundle.beta.apply"];
    assert_eq!(callable.param_count, 2);
    assert_eq!(callable.explicit_type_param_ids.len(), 2);
    let bytes = verum_vbc::serialize::serialize_module(&artifact).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    assert!(decoded.functions.iter().any(|function| {
        decoded
            .get_string(function.name)
            .is_some_and(|name| name.ends_with("select"))
    }));
}
