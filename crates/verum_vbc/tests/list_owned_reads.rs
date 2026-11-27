#![cfg(feature = "codegen")]

#[path = "fixtures/list_storage.rs"]
mod list_storage;

use std::sync::Arc;
use verum_vbc::interpreter::Interpreter;

const SOURCE: &str = r#"
type OwnedRow is { bytes: List<Int> };

fn saved_row() -> Maybe<OwnedRow> {
    let mut rows: List<OwnedRow> = List.new();
    let bytes: List<Int> = [42, 43];
    rows.PUSH(OwnedRow { bytes: bytes });
    let mut saved = rows.GET(0);
    match &mut saved {
        Maybe.Some(row) => { row.bytes[0] = 77; },
        Maybe.None => assert(false),
    }
    match rows.GET(0) {
        Maybe.Some(row) => assert_eq(row.bytes[0], 42),
        Maybe.None => assert(false),
    }
    assert(rows.GET(-1) is Maybe.None);
    assert(rows.GET(1) is Maybe.None);
    saved
}

fn read_probe() -> Int {
    match saved_row() {
        Maybe.Some(row) => row.bytes[0],
        Maybe.None => -1,
    }
}
"#;

#[test]
fn owned_reads_survive_mutation_and_destruction_in_either_order() {
    for (get, push, force_source) in [
        ("get", "push", false),
        ("checked_get", "checked_push", true),
    ] {
        let source = SOURCE.replace("GET", get).replace("PUSH", push);
        let module = list_storage::compile(&source, force_source);
        let entry = module.find_function_by_name("read_probe").unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("{get}: {error}"));
        assert!(result.is_int(), "{get}: {result:?}");
        assert_eq!(result.as_i64(), 77, "{get}");
    }
}

#[test]
fn reading_a_slot_cannot_duplicate_an_affine_resource() {
    let source = r#"
        type affine Ticket is { serial: Int };
        fn affine_probe() -> Int {
            let mut tickets: List<Ticket> = List.new();
            tickets.PUSH(Ticket { serial: 4 });
            match tickets.GET(0) {
                Maybe.Some(ticket) => ticket.serial,
                Maybe.None => -1,
            }
        }
    "#;
    for (get, push, force_source) in [
        ("get", "push", false),
        ("checked_get", "checked_push", true),
    ] {
        let source = source.replace("GET", get).replace("PUSH", push);
        let module = list_storage::compile(&source, force_source);
        let entry = module.find_function_by_name("affine_probe").unwrap();
        let error = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("effective ownership is Affine") && message.contains("Ticket"),
            "{get}: {error}"
        );
    }
}
