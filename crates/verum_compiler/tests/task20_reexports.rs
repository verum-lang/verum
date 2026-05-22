//! Task #20 — drift-pin for the `module_reexports` metadata field.
//!
//! Reads the embedded `runtime.core_metadata` produced by the
//! precompile and asserts that `metadata.module_reexports` carries
//! the `public mount X.{...}` chains we depend on at user-compile
//! time (free-fn / type re-exports through `core.base`).

use verum_types::core_metadata::CoreMetadata;

fn load_metadata() -> CoreMetadata {
    let bytes = include_bytes!(concat!(
        env!("OUT_DIR"),
        "/stdlib_runtime.core_metadata"
    ));
    bincode::deserialize::<CoreMetadata>(bytes).expect("deserialise embedded metadata")
}

#[test]
fn module_reexports_populated() {
    let m = load_metadata();
    eprintln!(
        "module_reexports: {} re-exporting modules",
        m.module_reexports.len()
    );
    assert!(
        m.module_reexports.len() >= 10,
        "expected at least 10 re-exporting modules, got {}",
        m.module_reexports.len()
    );
}

#[test]
fn core_base_reexports_env_functions() {
    let m = load_metadata();
    let core_base = m
        .module_reexports
        .get(&verum_common::Text::from("core.base"))
        .expect("core.base should appear as a re-exporting module");

    let mut names: Vec<&str> = core_base.iter().map(|(n, _)| n.as_str()).collect();
    names.sort();
    names.dedup();
    eprintln!("core.base re-exports {} leaves: {:?}", names.len(), names);

    for expected in &[
        "temp_dir",
        "args_count",
        "set_var",
        "home_dir",
        "var_opt",
        "exit",
        "Maybe",
        "Some",
        "None",
        "Result",
        "Ok",
        "Err",
        "replace",
        "swap",
    ] {
        assert!(
            names.contains(expected),
            "core.base should re-export `{expected}` — leaves were {:?}",
            names
        );
    }
}

#[test]
fn core_base_memory_reexport_source_resolves() {
    let m = load_metadata();
    let core_base = m
        .module_reexports
        .get(&verum_common::Text::from("core.base"))
        .expect("core.base should appear as a re-exporting module");

    let replace = core_base
        .iter()
        .find(|(n, _)| n.as_str() == "replace")
        .expect("`replace` must be a re-export leaf in core.base");

    assert_eq!(
        replace.1.as_str(),
        "core.base.memory",
        "`replace` should resolve to its declaring module core.base.memory"
    );
}
