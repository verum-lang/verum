//! Task #20 user-side drift-pin.
//!
//! After `load_stdlib_from_embedded` runs, the
//! re-exporting modules' ExportTables must include every
//! `(local_name, source_module)` leaf captured in
//! `metadata.module_reexports`.  This test triggers the load path
//! once via `CompilationPipeline::compile_string` (a minimal user
//! source), then probes the session's ModuleRegistry for
//! `core.base.exports.get("temp_dir")` etc.

use std::path::PathBuf;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};
use verum_common::Text;

#[test]
fn core_base_reexports_visible_in_export_table() {
    // Minimal user source — triggers stdlib load via the pipeline.
    let source = "fn main() {}\n";
    let options = CompilerOptions {
        input: PathBuf::from("task20_probe.vr"),
        output: PathBuf::from("task20_probe"),
        ..Default::default()
    };
    let mut session = Session::new(options);
    {
        let mut pipeline = CompilationPipeline::new(&mut session);
        // Drive the pipeline far enough that load_stdlib_modules runs.
        // `compile_string` returns Err if user-side typecheck fails, but
        // the stdlib load happens early and is what we're probing.
        let _ = pipeline.compile_string(source);
    }

    let registry = session.module_registry();
    let reg = registry.read();
    let info = match reg.get_by_path("core.base") {
        verum_common::Maybe::Some(s) => s,
        verum_common::Maybe::None => {
            panic!("core.base must be registered after stdlib load")
        }
    };

    let names: Vec<String> = info
        .exports
        .all_exports()
        .map(|(name, _)| name.as_str().to_string())
        .collect();
    eprintln!("core.base.exports has {} entries", names.len());

    for expected in &[
        "temp_dir",
        "args_count",
        "set_var",
        "home_dir",
        "var_opt",
        "Maybe",
        "Result",
        "replace",
        "swap",
    ] {
        assert!(
            info.exports.get(&Text::from(*expected)).is_some(),
            "core.base.exports should contain `{expected}` after \
             load_stdlib_from_embedded applies module_reexports — \
             names were {:?}",
            names
        );
    }
}
