//! SQLite WAL operations cross the pager boundary through the writer that owns both indexes.
use super::*;

#[test]
fn sqlite_wal_reader_uses_the_private_writer_and_wire_header_contracts() {
    let session = crate::Session::new(crate::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for owner in [
        "core.database.sqlite.native.l1_pager.wal_reader",
        "core.database.sqlite.native.l1_pager.wal_writer",
        "core.database.sqlite.native.l1_pager.wal_state",
        "core.database.sqlite.native.l1_pager.wal_index",
        "core.database.sqlite.native.l1_pager.journal.wal",
        "core.database.sqlite.native.l1_pager.recovery",
        "core.database.sqlite.native.wal_frame_layout.constants",
        "core.database.sqlite.native.wal_frame_layout.file_header",
        "core.database.sqlite.native.wal_frame_layout.frame_header",
    ] {
        let module = registry.read().get_by_path(owner).unwrap().ast.clone();
        check_source_types(
            &mut phase.configured_checker(),
            owner,
            &module,
            &registry,
            |error| error.to_diagnostic_with_span(Some(|span| session.convert_span(span))),
        )
        .unwrap_or_else(|errors| panic!("{owner}: {errors:#?}"));
    }

    let fixture = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../core-tests/database/sqlite/native/l1_pager/wal_recovery_test.vr"),
    )
    .expect("SQLite WAL fixture");
    let module = Parser::new(&fixture)
        .parse_module()
        .expect("SQLite WAL fixture");
    let owner = "core.database.sqlite.native.l1_pager.tests.wal_recovery";
    check_source_module(&mut phase.configured_checker(), owner, &module, &registry)
        .unwrap_or_else(|errors| panic!("{owner}: {errors:#?}"));

    for owner in ["core.database.sqlite.native.l1_pager.probe", "application"] {
        let module = |body: &str| {
            let source = format!(
                "module {owner}; mount core.database.sqlite.native.l1_pager.wal_writer.WalWriter; {body}"
            );
            Parser::new(&source)
                .parse_module()
                .expect("WAL client grammar")
        };
        let observer = module("fn last_frame(writer: &WalWriter) -> Int { writer.mx_frame() }");
        check_source_module(&mut phase.configured_checker(), owner, &observer, &registry)
            .unwrap_or_else(|errors| panic!("{owner}: {errors:#?}"));

        for (body, field) in [
            (
                "fn read(writer: &WalWriter) { let state = &writer.state; }",
                "state",
            ),
            (
                "fn borrow(writer: &mut WalWriter) { let index = &mut writer.index; }",
                "index",
            ),
            (
                "fn borrow(writer: &mut WalWriter) { let backend = &mut writer.backend; }",
                "backend",
            ),
            (
                "fn steal(writer: &mut WalWriter) { let seed = &mut writer.header_seed; }",
                "header_seed",
            ),
            (
                "fn steal(writer: &mut WalWriter) { let sum = &mut writer.running; }",
                "running",
            ),
            (
                "fn seek(writer: &mut WalWriter) { writer.write_cursor = 0; }",
                "write_cursor",
            ),
            (
                "fn forge(writer: WalWriter) -> WalWriter { WalWriter { write_cursor: 0, ..writer } }",
                "backend",
            ),
        ] {
            let errors = check_source_module(
                &mut phase.configured_checker(),
                owner,
                &module(body),
                &registry,
            )
            .expect_err("WAL client accessed a private writer field");
            assert!(
                errors.iter().any(|error| error.code() == Some("E202")
                    && error.message().contains(field)
                    && error
                        .message()
                        .contains("core.database.sqlite.native.l1_pager.wal_writer")),
                "{owner}: {body}: {errors:#?}"
            );
        }
    }
}

#[test]
fn sqlite_pager_keeps_live_cache_and_journal_state_with_the_owner() {
    let session = crate::Session::new(crate::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for owner in [
        "core.database.sqlite.native.l0_vfs.locking",
        "core.database.sqlite.native.l0_vfs.posix_vfs",
        "core.database.sqlite.native.l1_pager.pager",
        "core.database.sqlite.native.l1_pager.page_cache",
        "core.database.sqlite.native.l1_pager.actor",
        "core.database.sqlite.native.l1_pager.wal_pager",
        "core.database.sqlite.native.l1_pager.vfs_injection",
    ] {
        let module = registry.read().get_by_path(owner).unwrap().ast.clone();
        check_source_module(&mut phase.configured_checker(), owner, &module, &registry)
            .unwrap_or_else(|errors| panic!("{owner}: {errors:#?}"));
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (file, module_name) in [
        (
            "core-tests/database/sqlite/native/l1_pager/page_cache_test.vr",
            "page_cache",
        ),
        (
            "core-tests/database/sqlite/native/l1_pager/posix_page_test.vr",
            "posix_page",
        ),
        (
            "vcs/specs/L2-standard/database/sqlite/l1_pager/page_cache_lru.vr",
            "cache_lru",
        ),
    ] {
        let fixture = std::fs::read_to_string(workspace.join(file)).expect("pager fixture");
        let module = Parser::new(&fixture)
            .parse_module()
            .expect("pager fixture grammar");
        let owner = format!("core.database.sqlite.native.l1_pager.tests.{module_name}");
        check_source_module(&mut phase.configured_checker(), &owner, &module, &registry)
            .unwrap_or_else(|errors| panic!("{owner}: {errors:#?}"));
    }

    for (nominal, field) in [
        ("page_cache.CachedFrame", "dirty"),
        ("page_cache.CachedFrame", "pin_count"),
        ("page_cache.PageCache", "frames"),
        ("pager.Pager", "dirty_count"),
        ("pager.PageFrame", "data"),
        ("wal_reader.WalFrameRead", "payload"),
    ] {
        let leaf = nominal.rsplit('.').next().unwrap();
        let source = format!(
            "module application; mount core.database.sqlite.native.l1_pager.{nominal};
             fn mutate(owner: &mut {leaf}) {{ let leak = &mut owner.{field}; }}"
        );
        let module = Parser::new(&source)
            .parse_module()
            .expect("pager visibility probe");
        let errors = check_source_module(
            &mut phase.configured_checker(),
            "application",
            &module,
            &registry,
        )
        .expect_err("external code borrowed private pager state");
        assert!(
            errors
                .iter()
                .any(|d| d.code() == Some("E202") && d.message().contains(field)),
            "{leaf}.{field}: {errors:#?}"
        );
    }
}
