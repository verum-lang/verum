//! T1583: optional parameter spelling stays absent across archive body import.
#![cfg(feature = "codegen")]

use verum_common::Map;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::{FunctionId, VbcModule};
use verum_vbc::types::StringId;

fn source_module() -> VbcModule {
    let source = r#"
        type Cell is { value: Int };
        implement Cell {
            fn owned(self) -> Int { self.value }
            fn borrowed(&self) -> Int { self.value }
            fn mutable(&mut self) -> Int { self.value }
        }
        fn ordinary(value: Int) -> Int { value }
    "#;
    let ast = Parser::new(source)
        .parse_module()
        .expect("grammar-valid source");
    VbcCodegen::new()
        .compile_module(&ast)
        .expect("source descriptors")
}

fn roundtrip(source: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(source).expect("serialize"),
    )
    .expect("deserialize")
}

fn import(source: &VbcModule, repeat: bool) -> VbcModule {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..20 {
        codegen
            .ctx_mut()
            .intern_string_raw(&format!("unrelated_{i}"));
    }
    codegen.import_archive_module_types(source);
    let remap: Map<_, _> = source
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id.0, FunctionId(7000 + i as u32)))
        .collect();
    let remap = remap.into(); // Existing archive API accepts its legacy map type.
    assert!(codegen.merge_archive_function_bodies(source, &remap) > 0);
    if repeat {
        assert_eq!(codegen.merge_archive_function_bodies(source, &remap), 0);
    }
    codegen
        .finalize_module_from_state()
        .expect("finalize imported bodies")
}

fn spelling(module: &VbcModule, name: &str) -> StringId {
    let function = module
        .get_function(module.find_function_by_name(name).expect(name))
        .unwrap();
    assert_eq!(function.params.len(), 1, "{name}");
    function.params[0].type_name
}

#[test]
fn source_self_markers_survive_wire_body_merge_finalize_and_repeated_import() {
    let source = source_module();
    assert!(
        source
            .get_string(StringId::EMPTY)
            .is_some_and(|text| !text.is_empty())
    );
    for name in ["Cell.owned", "Cell.borrowed", "Cell.mutable"] {
        assert_eq!(spelling(&source, name), StringId::EMPTY);
    }
    let source = roundtrip(&source);
    for repeat in [false, true] {
        let imported = import(&source, repeat);
        for module in [imported.clone(), roundtrip(&imported)] {
            for name in ["Cell.owned", "Cell.borrowed", "Cell.mutable"] {
                assert_eq!(
                    spelling(&module, name),
                    StringId::EMPTY,
                    "{name}: absent Self spelling"
                );
            }
        }
    }
}

#[test]
fn regular_parameter_spelling_is_reinterned_in_the_destination_table() {
    let source = roundtrip(&source_module());
    assert_ne!(spelling(&source, "ordinary"), StringId::EMPTY);
    for repeat in [false, true] {
        let imported = import(&source, repeat);
        for module in [imported.clone(), roundtrip(&imported)] {
            let id = spelling(&module, "ordinary");
            assert_ne!(id, StringId::EMPTY);
            assert_eq!(module.get_string(id), Some("Int"));
        }
    }
}

#[test]
fn missing_optional_spelling_does_not_become_source_string_zero() {
    let mut source = source_module();
    let id = source.find_function_by_name("ordinary").unwrap();
    source
        .functions
        .iter_mut()
        .find(|f| f.id == id)
        .unwrap()
        .params[0]
        .type_name = StringId::EMPTY;
    let source = roundtrip(&source);
    assert!(
        source
            .get_string(StringId::EMPTY)
            .is_some_and(|text| !text.is_empty())
    );
    let imported = import(&source, true);
    assert_eq!(spelling(&roundtrip(&imported), "ordinary"), StringId::EMPTY);
}
