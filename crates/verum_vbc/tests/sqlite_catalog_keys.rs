//! Exercise the production catalog's qualified identity on the VM collection carrier.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_ast::{
    ItemKind, Module,
    decl::{ImplItemKind, ImplKind},
};
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
    assert_in_vm_and_archive(&source, "catalog_key_probe", 3);
}

fn assert_in_vm_and_archive(source: &Module, entry: &str, expected: i64) {
    let module = VbcCodegen::new().compile_module(source).unwrap();
    let wire = deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap();
    for (path, module) in [("source", module), ("wire", wire)] {
        let id = module.find_function_by_name(entry).expect(entry);
        let value = Interpreter::new(Arc::new(module))
            .execute_function(id)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(
            value.is_int() && value.as_i64() == expected,
            "{path}: {value:?}"
        );
    }
}

#[test]
fn catalog_registration_lookup_and_invalidation_survive_vbc_round_trip() {
    let mut catalog = Parser::new(include_str!(
        "../../../core/database/sqlite/native/l6_session/schema_cache.vr"
    ))
    .parse_module()
    .unwrap();
    // The SQL parser enters one layer above this test, through table_meta_from_create.
    catalog.items.retain(|item| match &item.kind {
        ItemKind::Type(_) => true,
        ItemKind::Impl(implementation) => matches!(implementation.kind, ImplKind::Inherent(_)),
        ItemKind::Function(function) => function.name.as_str() != "table_meta_from_create",
        _ => false,
    });
    for (declaration, name) in [
        (include_str!("../../../core/base/result.vr"), "Result"),
        (include_str!("../../../core/base/maybe.vr"), "Maybe"),
        (
            include_str!("../../../core/database/sqlite/native/l2_record/affinity.vr"),
            "Affinity",
        ),
    ] {
        let mut dependency = Parser::new(declaration).parse_module().unwrap();
        dependency.items.retain_mut(|item| match &mut item.kind {
            ItemKind::Type(ty) => ty.name.as_str() == name,
            ItemKind::Impl(implementation) if name == "Result" && matches!(implementation.kind, ImplKind::Inherent(_)) => {
                implementation.items.retain(|item| matches!(&item.kind, ImplItemKind::Function(f) if matches!(f.name.as_str(), "unwrap" | "is_err")));
                !implementation.items.is_empty()
            }
            _ => false,
        });
        catalog.items.extend(dependency.items);
    }
    let control = Parser::new(
        r#"
fn table(schema: Text, name: Text, root: Int) -> TableMeta {
    TableMeta { schema, name, root_page: root, kind: TkTable, columns: [], primary_key_cols: [],
                strict: false, without_rowid: false, created_sql: "" }
}
fn index_meta(schema: Text, name: Text, root: Int) -> IndexMeta {
    IndexMeta { schema, name, root_page: root, table_name: "items", columns: [], unique: false }
}
fn catalog_api_probe() -> Int {
    let mut cache = schema_cache_new();
    cache.register_table(table("a.b", "c", 2)).unwrap();
    cache.register_table(table("a", "b.c", 3)).unwrap();
    cache.register_table(table("", "a.b.c", 4)).unwrap();
    cache.register_table(table("схема", "имя.таблицы", 5)).unwrap();
    assert(cache.register_table(table("main", "a.b.c", 6)).is_err());
    assert_eq(cache.table_count(), 4);
    assert_eq(cache.lookup_table(&"a", &"b.c").unwrap().root_page, 3);
    assert_eq(cache.lookup_table(&"a.b", &"c").unwrap().root_page, 2);
    cache.replace_table(table("a", "b.c", 7));
    assert_eq(cache.lookup_table(&"a", &"b.c").unwrap().root_page, 7);
    let before = cache.cookie();
    cache.drop_table(&"a.b", &"c").unwrap();
    assert(cache.drop_table(&"a.b", &"c").is_err());
    assert_eq(cache.cookie(), before + 1);
    assert(cache.has_table(&"main", &"a.b.c"));
    assert(cache.has_table(&"схема", &"имя.таблицы"));
    cache.register_index(index_meta("a.b", "c", 10)).unwrap();
    cache.register_index(index_meta("a", "b.c", 11)).unwrap();
    cache.register_index(index_meta("main", "a.b.c", 12)).unwrap();
    assert(cache.register_index(index_meta("", "a.b.c", 13)).is_err());
    assert_eq(cache.index_count(), 3);
    cache.drop_index(&"a", &"b.c").unwrap();
    assert(cache.drop_index(&"a", &"b.c").is_err());
    assert_eq(cache.lookup_index(&"a.b", &"c").unwrap().root_page, 10);
    assert_eq(cache.lookup_index(&"", &"a.b.c").unwrap().root_page, 12);
    cache.cookie() - before
}
"#,
    )
    .parse_module()
    .unwrap();
    catalog.items.extend(control.items);
    assert_in_vm_and_archive(&catalog, "catalog_api_probe", 2);
}
