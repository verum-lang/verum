//! Admit the actual SQLite API owners through the complete source declaration graph.
use super::*;

const SQLITE_LIST_OWNERS: [&str; 33] = [
    "core.database.sqlite.native.autoincrement_register_api.register_",
    "core.database.sqlite.native.busy_retry_policy.schedule",
    "core.database.sqlite.native.case_when_api.shape",
    "core.database.sqlite.native.create_table_default_cols.implicit",
    "core.database.sqlite.native.create_table_default_cols.infer",
    "core.database.sqlite.native.cross_db_ref_api.resolver",
    "core.database.sqlite.native.cte_api.clause",
    "core.database.sqlite.native.cte_api.definition",
    "core.database.sqlite.native.drop_modules_keepset_api.keepset",
    "core.database.sqlite.native.eqp_node_emitter.node",
    "core.database.sqlite.native.integrity_walker_api.walker",
    "core.database.sqlite.native.journal_header_api.constants",
    "core.database.sqlite.native.json1_fn_taxonomy.entry",
    "core.database.sqlite.native.json_path_api.path",
    "core.database.sqlite.native.keyword_conflict_table.entry",
    "core.database.sqlite.native.math_fn_package.registry",
    "core.database.sqlite.native.pcache_dirty_list.list",
    "core.database.sqlite.native.prefix_like_opt.pattern",
    "core.database.sqlite.native.prefix_like_opt.range",
    "core.database.sqlite.native.recursive_cte_queue.queue",
    "core.database.sqlite.native.returning_clause_shape.output",
    "core.database.sqlite.native.row_value_api.shape",
    "core.database.sqlite.native.session_attach_api.session",
    "core.database.sqlite.native.stat1_schema_api.row",
    "core.database.sqlite.native.stmt_param_mapping.mapping",
    "core.database.sqlite.native.tokenizer_interface_api.token",
    "core.database.sqlite.native.update_from_api.spec",
    "core.database.sqlite.native.upsert_api.target",
    "core.database.sqlite.native.vdbe_subprogram_api.frame",
    "core.database.sqlite.native.view_variants_api.spec",
    "core.database.sqlite.native.where_clause_term_api.clause",
    "core.database.sqlite.native.window_partition_api.runner",
    "core.database.sqlite.native.xfilter_argv_api.frame",
];

#[test]
fn sqlite_api_list_constructors_use_canonical_source_declarations() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let mut failures: List<Text> = List::new();
    for owner in SQLITE_LIST_OWNERS {
        let module = registry
            .read()
            .get_by_path(owner)
            .map(|entry| entry.ast.clone());
        let Some(module) = module else {
            failures.push(format!("{owner}: source owner missing from the release graph").into());
            continue;
        };
        if let Err(errors) = check_source_types(
            &mut phase.configured_checker(),
            owner,
            &module,
            &registry,
            |error| error.to_diagnostic_with_span(Some(|span| session.convert_span(span))),
        ) {
            failures.push(format!("{owner}: {errors:#?}").into());
        }
    }
    assert!(
        failures.is_empty(),
        "{} / {} SQLite API source owners refused:\n{}",
        failures.len(),
        SQLITE_LIST_OWNERS.len(),
        failures.join("\n\n")
    );
}

#[test]
fn sqlite_list_components_keep_descriptor_fields_inside_their_boundary() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let mut failures = List::new();
    for owner in [
        "core.database.sqlite.native.cross_db_ref_api.reference",
        "core.database.sqlite.native.cte_api.definition",
        "core.database.sqlite.native.integrity_walker_api.finding",
    ] {
        let module = registry.read().get_by_path(owner).unwrap().ast.clone();
        if let Err(errors) = check_source_types(
            &mut phase.configured_checker(),
            owner,
            &module,
            &registry,
            |error| error.to_diagnostic_with_span(Some(|span| session.convert_span(span))),
        ) {
            failures.push((owner, errors));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");

    for (component, mounts, operations, constructor) in [
        (
            "core.database.sqlite.native.cross_db_ref_api",
            "mount core.database.sqlite.native.cross_db_ref_api.reference.{CrossDbTableRef, qualified};",
            [
                ("fn read(r:&CrossDbTableRef)->Bool { r.schema == r.table }", true),
                ("fn write(r:&mut CrossDbTableRef) { r.schema = \"main\"; r.table = \"items\"; }", true),
                ("fn construct()->CrossDbTableRef { CrossDbTableRef { schema: \"main\", table: \"items\" } }", true),
            ],
            "fn construct()->CrossDbTableRef { qualified(\"main\", \"items\") }",
        ),
        (
            "core.database.sqlite.native.cte_api",
            "mount core.database.sqlite.native.cte_api.definition.{CteDefinition, new}; mount core.database.sqlite.native.cte_api.materialization.default_hint; mount core.collections.list.List;",
            [
                ("fn read(c:&CteDefinition)->Text { c.name }", true),
                ("fn write(c:&mut CteDefinition) { c.name = \"renamed\"; }", true),
                ("fn construct()->CteDefinition { CteDefinition { name: \"items\", columns: List.new(), body_sql: \"SELECT 1\", hint: default_hint(), recursive: false } }", false),
            ],
            "fn construct()->CteDefinition { new(\"items\", \"SELECT 1\") }",
        ),
        (
            "core.database.sqlite.native.integrity_walker_api",
            "mount core.database.sqlite.native.integrity_walker_api.finding.{Finding, FindingKind, FkOk, ok_sentinel};",
            [
                ("fn read(f:&Finding)->FindingKind { f.kind }", true),
                ("fn write(f:&mut Finding) { f.kind = FkOk; }", true),
                ("fn construct()->Finding { Finding { kind: FkOk, detail: \"ok\" } }", false),
            ],
            "fn construct()->Finding { ok_sentinel() }",
        ),
    ] {
        for (owner, inside) in [
            (format!("{component}.probe"), true),
            (format!("{component}_extra"), false),
            ("core.database.sqlite.native.client".to_owned(), false),
        ] {
            for (body, component_access) in operations {
                let source = format!("module {owner}; mount base.{{Bool, Text}}; {mounts} {body}");
                let module = Parser::new(&source).parse_module().unwrap();
                let result = check_source_module(&mut phase.configured_checker(), &owner, &module, &registry);
                if inside && component_access {
                    result.unwrap_or_else(|errors| panic!("{owner}: {body}: {errors:#?}"));
                } else {
                    let errors = result.expect_err("caller gained access to descriptor storage");
                    assert!(errors.iter().any(|error| error.code() == Some("E202")),
                        "{owner}: {body}: {errors:#?}");
                }
            }
            let source = format!("module {owner}; {mounts} {constructor}");
            let module = Parser::new(&source).parse_module().unwrap();
            check_source_module(&mut phase.configured_checker(), &owner, &module, &registry)
                .unwrap_or_else(|errors| panic!("public constructor refused: {owner}: {errors:#?}"));
        }
    }
}
