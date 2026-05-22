//! Task #27 — drift-pin for prelude glob expansion through
//! `module_reexports`.

use verum_types::core_metadata::CoreMetadata;

fn load_metadata() -> CoreMetadata {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/stdlib_runtime.core_metadata"));
    bincode::deserialize::<CoreMetadata>(bytes).expect("deserialise embedded metadata")
}

#[test]
fn core_prelude_reexports_iterator_free_fns() {
    let m = load_metadata();
    let prelude = m
        .module_reexports
        .get(&verum_common::Text::from("core.prelude"))
        .expect("core.prelude must be in module_reexports after #27 glob expansion");
    let names: Vec<&str> = prelude.iter().map(|(n, _)| n.as_str()).collect();
    eprintln!("core.prelude re-exports {} leaves", names.len());
    let missing: Vec<&str> = ["range", "repeat", "take", "swap", "replace", "args_count"]
        .iter()
        .filter(|e| !names.contains(*e))
        .copied()
        .collect();
    eprintln!("MISSING: {:?}", missing);
    // Print a sample of lowercase-starting names (free fns) to see
    // what IS there.
    let lowercase: Vec<&str> = names
        .iter()
        .filter(|n| n.chars().next().map(|c| c.is_lowercase()).unwrap_or(false))
        .copied()
        .collect();
    eprintln!("lowercase leaves ({}): {:?}", lowercase.len(), lowercase);

    for expected in &["range", "repeat", "take", "swap", "replace", "args_count"] {
        assert!(
            names.contains(expected),
            "core.prelude should re-export `{expected}` via glob",
        );
    }
}
