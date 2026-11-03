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

fn compile_mounted(
    source: &str,
    surface: &str,
    reverse: bool,
) -> Result<verum_vbc::VbcModule, String> {
    let mut files = [
        "module catalog.a; public fn fetch() -> Int { 79 }",
        "module catalog.b; public fn expected() -> Int { 41 }",
        surface,
        source,
    ]
    .map(|source| Parser::new(source).parse_module().unwrap());
    if reverse {
        files.reverse();
    }
    let sources: Vec<_> = files.iter().collect();
    let mut codegen = VbcCodegen::new();
    codegen
        .collect_unit_declarations(&sources)
        .map_err(|e| e.to_string())?;
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&sources, ItemFailurePolicy::Strict)
        .map_err(|e| e.to_string())?;
    codegen.finalize_module().map_err(|e| e.to_string())
}

#[test]
fn a_named_mount_cannot_publish_a_sibling_implementation() {
    for reverse in [false, true] {
        for (import, missing) in [
            (
                "mount catalog.surface.{fetch}; public fn probe()->Int { fetch() }",
                "fetch",
            ),
            (
                "mount catalog.surface.{fetch as selected}; public fn probe()->Int { selected() }",
                "fetch",
            ),
            (
                "mount catalog.surface.{absent as fetch}; public fn probe()->Int { fetch() }",
                "absent",
            ),
        ] {
            let caller = format!("module catalog.client; {import}");
            let error = compile_mounted(
                &caller,
                "module catalog.surface; public fn other()->Int {4}",
                reverse,
            )
            .expect_err("the missing member has no implementation in this module");
            assert!(
                error.contains(&format!("catalog.surface.{missing}")),
                "{error}"
            );
        }
    }
}

#[test]
fn public_callable_reexports_preserve_the_declaring_body() {
    use std::sync::Arc;
    use verum_vbc::interpreter::Interpreter;
    let source = "module catalog.client; mount catalog.surface.{fetch as selected}; public fn probe()->Int {selected()}";
    for reverse in [false, true] {
        let module = compile_mounted(
            source,
            "module catalog.surface; public mount catalog.b.{expected as fetch};",
            reverse,
        )
        .unwrap();
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        for module in [module, wire] {
            let id = module
                .find_function_by_name("catalog.client.probe")
                .unwrap();
            let mut vm = Interpreter::new(Arc::new(module));
            assert_eq!(vm.execute_function(id).unwrap().as_i64(), 41);
        }
    }
}

#[test]
fn callable_globs_and_inline_facades_preserve_only_public_reexport_edges() {
    let caller = "module catalog.client; mount catalog.surface.{expected as f}; public fn probe()->Int {f()}";
    for surface in [
        "module catalog.surface; public mount catalog.b.*;",
        "module catalog.surface; public module facade { public mount catalog.b.{expected}; }",
    ] {
        for reverse in [false, true] {
            let m = compile_mounted(caller, surface, reverse).unwrap();
            let id = m.find_function_by_name("catalog.client.probe").unwrap();
            let mut vm = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(m));
            assert_eq!(vm.execute_function(id).unwrap().as_i64(), 41);
        }
    }
    for surface in [
        "module catalog.surface; mount catalog.b.*;",
        "module catalog.surface; public module facade { public fn expected()->Int{12} }",
        "module catalog.surface; public mount catalog.surface.*;",
    ] {
        assert!(
            compile_mounted(caller, surface, true)
                .unwrap_err()
                .contains("catalog.surface.expected")
        );
    }
}

#[test]
fn a_module_variant_export_uses_its_own_sum_and_discriminant() {
    let sources = [
        "module catalog.a; public type Early is Pick(Int) | Other;",
        "module catalog.b; public type Late is Other | Pick(Int);",
        "module catalog.client; mount catalog.b.{Late, Pick as requested}; fn selected()->Late { requested(41) } public fn probe()->Int { match selected() { Late.Other => 0, Late.Pick(n) => n } }",
    ].map(|s| Parser::new(s).parse_module().unwrap());
    for order in [[0, 1, 2], [2, 1, 0]] {
        let files = order.map(|i| &sources[i]);
        let mut cg = VbcCodegen::new();
        cg.collect_unit_declarations(&files).unwrap();
        cg.resolve_pending_imports();
        cg.compile_unit_items(&files, ItemFailurePolicy::Strict)
            .unwrap();
        let module = cg.finalize_module().unwrap();
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        let probe = wire.find_function_by_name("catalog.client.probe").unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(wire));
        assert_eq!(vm.execute_function(probe).unwrap().as_i64(), 41);
    }
}

#[test]
fn competing_public_callable_reexports_must_agree_on_the_declaration() {
    let surface = "module catalog.surface; public mount catalog.a.{fetch}; public mount catalog.b.{expected as fetch};";
    let caller = "module catalog.client; mount catalog.surface.{fetch as selected}; public fn probe()->Int{selected()}";
    for reverse in [false, true] {
        let error = compile_mounted(caller, surface, reverse).unwrap_err();
        assert!(
            error.contains("ambiguous callable mount")
                && error.contains("catalog.a.fetch")
                && error.contains("catalog.b.expected"),
            "{error}"
        );
    }
}

#[test]
fn ordinary_single_file_with_inline_modules_refuses_an_unowned_mount() {
    let source = "module catalog; module a { public fn fetch()->Int {79} } module b { public fn other()->Int {4} } module client { mount catalog.b.{fetch as chosen}; public fn probe()->Int { chosen() } }";
    let ast = Parser::new(source).parse_module().unwrap();
    let error = VbcCodegen::new().compile_module(&ast).unwrap_err();
    assert!(
        matches!(
            error.kind,
            verum_vbc::codegen::CodegenErrorKind::InvalidSourceMount(_)
        ),
        "{error}"
    );
}

#[test]
fn a_foreign_mount_selects_its_declared_link_symbol_in_either_order() {
    let sources = [
        r#"module host.native; extern "C" { @link_name("real_symbol") public fn fetch() -> Int; }"#,
        "module host.other; public fn fetch()->Int { 43 }",
        "module host.surface; public mount host.native.{fetch as selected};",
        "module host.client; mount host.surface.{selected}; public fn probe()->Int { selected() }",
    ]
    .map(|s| Parser::new(s).parse_module().unwrap());
    for order in [[0, 1, 2, 3], [3, 2, 1, 0]] {
        let files = order.map(|i| &sources[i]);
        let mut cg = VbcCodegen::new();
        cg.collect_unit_declarations(&files).unwrap();
        cg.resolve_pending_imports();
        cg.compile_unit_items(&files, ItemFailurePolicy::Strict)
            .unwrap();
        let exports = cg.export_functions();
        assert_eq!(
            exports["host.surface.selected"]
                .foreign_declaration
                .as_deref(),
            Some("host.native.fetch#0")
        );
        let module = cg.finalize_module().unwrap();
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        let probe = wire.find_function_by_name("host.client.probe").unwrap();
        let function = wire.get_function(probe).unwrap();
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let ops = verum_vbc::bytecode::decode_instructions(&wire.bytecode[start..end]).unwrap();
        assert!(ops.iter().any(|op| matches!(op, verum_vbc::instruction::Instruction::FfiExtended { sub_op, .. } if *sub_op == verum_vbc::instruction::SystemSubOpcode::CallFfiC as u8)));
        assert_eq!(
            wire.get_string(wire.ffi_symbols[0].name),
            Some("real_symbol")
        );
    }
}
