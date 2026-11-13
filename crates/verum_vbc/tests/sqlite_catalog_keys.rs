//! Exercise the production catalog's qualified identity on the VM collection carrier.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_ast::ItemKind;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module_validated, interpreter::Interpreter,
    serialize::serialize_module,
};

#[test]
fn catalog_identity_survives_vbc_round_trip() {
    let mut source = Parser::new(include_str!(
        "../../../core/database/sqlite/native/l6_session/schema_cache.vr"
    ))
    .parse_module()
    .unwrap();
    source.items.retain(|item| match &item.kind {
        ItemKind::Type(decl) => decl.name.as_str() == "CatalogKey",
        ItemKind::Function(decl) => matches!(
            decl.name.as_str(),
            "qualify" | "normalized_schema" | "display_name"
        ),
        _ => false,
    });
    assert_eq!(source.items.len(), 4);
    let control = Parser::new(
        r#"
fn catalog_key_probe() -> Int {
    let mut entries: Map<CatalogKey, Int> = Map.new();
    entries.insert(qualify(&"a.b", &"c"), 11);
    entries.insert(qualify(&"a", &"b.c"), 22);
    entries.insert(qualify(&"", &"a.b.c"), 33);
    entries.insert(qualify(&"схема", &"имя.таблицы"), 44);
    assert_eq(entries.len(), 4);
    assert_eq(entries.get(qualify(&"main", &"a.b.c")).unwrap(), 33);
    assert_eq(entries.remove(qualify(&"a", &"b.c")).unwrap(), 22);
    assert(entries.get(qualify(&"a", &"b.c")).is_none());
    assert_eq(entries.get(qualify(&"a.b", &"c")).unwrap(), 11);
    assert_eq(entries.get(qualify(&"схема", &"имя.таблицы")).unwrap(), 44);
    assert_eq(display_name(&"", &"a.b.c"), "a.b.c");
    entries.len()
}
"#,
    )
    .parse_module()
    .unwrap();
    source.items.extend(control.items);
    let module = VbcCodegen::new().compile_module(&source).unwrap();
    let wire = deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap();
    for (path, module) in [("source", module), ("wire", wire)] {
        let id = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("catalog_key_probe"))
            .unwrap()
            .id;
        let value = Interpreter::new(Arc::new(module))
            .execute_function(id)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(value.is_int() && value.as_i64() == 3, "{path}: {value:?}");
    }
}
