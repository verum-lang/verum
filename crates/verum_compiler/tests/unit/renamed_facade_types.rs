use super::*;

#[test]
fn public_facades_preserve_nominal_identity_of_renamed_implementation_types() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let routes = [
        ("core.mesh.k8s", "Listener", "core.mesh.k8s.gateway", "K8sListener"),
        ("core.mesh.xds", "Listener", "core.mesh.xds.resources", "XdsListener"),
        ("core.mesh.xds", "Subscription", "core.mesh.xds.client", "XdsSubscription"),
        ("core.mesh.xds", "Node", "core.mesh.xds.types", "XdsNode"),
        ("core.protobuf", "Cursor", "core.protobuf.wire", "ProtobufCursor"),
        ("core.database.sqlite.native.l1_pager", "CheckpointMode", "core.database.sqlite.native.l1_pager.actor", "PagerCheckpointMode"),
        ("core.database.sqlite.native.l1_pager", "TextEncoding", "core.database.sqlite.native.l1_pager.db_header", "DbHeaderTextEncoding"),
        ("core.database.sqlite.native.l1_pager", "SavepointStack", "core.database.sqlite.native.l1_pager.savepoint", "PagerSavepointStack"),
        ("core.database.sqlite.native.l2_record", "SqliteApiValue", "core.database.sqlite.native.l2_record.type_coercion", "CoercionSqliteApiValue"),
        ("core.database.sqlite.native.l3_btree", "IntegrityReport", "core.database.sqlite.native.l3_btree.integrity", "BtreeIntegrityReport"),
    ];
    let owner = "application.facades";
    let mut source = format!("module {owner};\n");
    for (i, (facade, alias, declaring, original)) in routes.iter().enumerate() {
        source.push_str(&format!("mount {facade}.{alias} as Public{i};\nmount {declaring}.{original};\n"));
    }
    for (i, (_, _, _, original)) in routes.iter().enumerate() {
        source.push_str(&format!("fn identity{i}(value: Public{i}) -> {original} {{ value }}\n"));
    }
    let check = |source: &str| {
        let ast = Parser::new(source).parse_module().expect("parse external client");
        check_source_module(&mut phase.configured_checker(), owner, &ast, &registry)
    };
    check(&source).unwrap_or_else(|errors| panic!("release facade types: {errors:#?}"));
    source.push_str("fn mixed(value: Public0) -> Public1 { value }");
    let errors = check(&source).expect_err("independently declared listeners must remain distinct");
    assert!(errors.iter().any(|error| error.code() == Some("E400")), "{errors:#?}");
}
