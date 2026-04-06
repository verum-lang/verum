#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Unit tests for pipeline.rs
//
// Migrated from src/pipeline.rs to comply with CLAUDE.md test organization.

use std::io::Write;
use tempfile::NamedTempFile;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

#[test]
fn test_pipeline_load_source() {
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "fn main() -> Int {{ 42 }}").unwrap();
    let opts = CompilerOptions::new(temp_file.path().to_path_buf(), "output".into());
    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);
    let file_id = pipeline.phase_load_source().unwrap();
    assert!(session.get_source(file_id).is_some());
}
