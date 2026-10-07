//! Registry publication must finish owner-alias selection before first-wins import.
#![cfg(feature = "codegen")]
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    module::FunctionId,
};

fn source_bundle(reverse: bool) -> VbcCodegen {
    let alpha = Parser::new(
        "module alpha; public fn select<T>(value: T)->T {value} public fn result()->Int {11}",
    )
    .parse_module()
    .unwrap();
    let beta = Parser::new(
        "module beta; public fn select<T>(value: T)->T {value} public fn result()->Int {22}",
    )
    .parse_module()
    .unwrap();
    let units = if reverse {
        [&beta, &alpha]
    } else {
        [&alpha, &beta]
    };
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("bundle"));
    codegen.collect_unit_declarations(&units).unwrap();
    codegen
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    codegen.finalize_module_from_state().unwrap();
    codegen
}

#[test]
fn source_owned_aliases_replace_conflicting_registry_keys() {
    for reverse in [false, true] {
        let mut codegen = source_bundle(reverse);
        let selected = codegen
            .export_functions()
            .get("bundle.alpha.select")
            .unwrap()
            .clone();
        let sibling = codegen
            .export_functions()
            .get("bundle.beta.select")
            .unwrap()
            .clone();
        assert_ne!(selected.id, sibling.id);
        // Preserve an existing reference to the source owner while a conflicting
        // qualified registry key exercises the exporter alias overwrite boundary.
        codegen
            .ctx_mut()
            .register_function("saved_alpha_select".into(), selected.clone());
        codegen
            .ctx_mut()
            .register_function("bundle.alpha.select".into(), sibling);
        let raw = codegen.ctx_mut().export_functions();
        assert_ne!(
            raw["bundle.alpha.select"].id, selected.id,
            "control: consuming base keys first would select the sibling"
        );
        let published = codegen.export_functions();
        let borrowed = codegen.export_function_view();
        assert_eq!(
            format!(
                "{:?}",
                borrowed.get(&Text::from("bundle.alpha.select")).unwrap()
            ),
            format!("{selected:?}")
        );
        let actual = published
            .get("bundle.alpha.select")
            .expect("source owner alias");
        assert_eq!(format!("{actual:?}"), format!("{selected:?}"));
        assert_eq!(actual.explicit_type_param_ids.len(), 1);
        assert_eq!(actual.param_count, 1);
    }
}

#[test]
fn registry_exports_keep_existing_imports_and_exact_source_owners() {
    let mut codegen = source_bundle(false);
    let mut imported = codegen
        .export_functions()
        .get("bundle.alpha.select")
        .unwrap()
        .clone();
    imported.id = FunctionId(123456);
    imported.param_names = List::from_iter([Text::from("imported_value").into_string()]).into();
    codegen
        .ctx_mut()
        .register_function("prior.module.select".into(), imported.clone());
    let published = codegen.export_functions();
    let borrowed = codegen.export_function_view();
    for name in [
        "bundle.alpha.select",
        "bundle.beta.select",
        "bundle.alpha.result",
        "bundle.beta.result",
    ] {
        assert!(published.contains_key(name), "{name}");
        assert!(borrowed.contains_key(&Text::from(name)), "{name}");
    }
    assert_eq!(
        format!("{:?}", published["prior.module.select"]),
        format!("{imported:?}")
    );
    assert_eq!(
        format!("{:?}", borrowed[&Text::from("prior.module.select")]),
        format!("{imported:?}")
    );
}

#[test]
fn borrowed_exports_match_owned_metadata_in_both_source_orders() {
    for reverse in [false, true] {
        let codegen = source_bundle(reverse);
        let owned = codegen.export_functions();
        let borrowed = codegen.export_function_view();
        assert_eq!(borrowed.len(), owned.len());
        for (name, info) in &owned {
            let actual = borrowed
                .get(&Text::from(name.as_str()))
                .expect("same final export keys");
            assert_eq!(format!("{actual:?}"), format!("{info:?}"), "{name}");
        }
    }
}
