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
