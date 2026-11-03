#![cfg(feature = "codegen")]
//! Execute the actual source SQLite error adapter through serialized VBC.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};
#[test]
fn actual_sqlite_error_mapping() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core");
    let path = root.join("database/sqlite/native/l7_api/database.vr");
    let mut source = std::fs::read_to_string(&path).unwrap();
    source.push_str(
        r#"
fn audit_io_mapping() -> Int {
    let actual = db_error_from_conn(ConnectionError.IoFailure("disk-full".to_text()));
    match actual {
        SqliteApiDbError.DbIoError(message) => if message == "disk-full" { 47 } else { 0 },
        _ => 0,
    }
}

"#,
    );
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut c =
        VbcCodegen::with_config(CodegenConfig::new(path.to_str().unwrap()).with_validation());
    let m = c
        .compile_module_with_mounts(&ast, path.to_str().unwrap(), root.to_str().unwrap())
        .unwrap();
    let m = Arc::new(
        verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&m).unwrap(),
        )
        .unwrap(),
    );
    for name in ["audit_io_mapping"] {
        let id = m
            .find_function_by_name(&format!(
                "core.database.sqlite.native.l7_api.database.{name}"
            ))
            .or_else(|| m.find_function_by_name(name))
            .unwrap();
        let value = Interpreter::new(m.clone()).execute_function(id).unwrap();
        assert_eq!(value.as_i64(), 47, "{name}");
    }
}
