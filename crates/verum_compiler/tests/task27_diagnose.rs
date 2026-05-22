//! Task #27 diagnose — print metadata internals.

use verum_types::core_metadata::CoreMetadata;

fn load_metadata() -> CoreMetadata {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/stdlib_runtime.core_metadata"));
    bincode::deserialize::<CoreMetadata>(bytes).expect("deserialise embedded metadata")
}

#[test]
fn dump_internals() {
    let m = load_metadata();
    let mut iter_fns: Vec<&str> = m.functions.iter()
        .filter(|(_, fd)| fd.module_path.as_str().starts_with("core.base"))
        .map(|(name, _)| name.as_str())
        .collect();
    iter_fns.sort();
    eprintln!("Functions in core.base.*: {} total", iter_fns.len());
    for n in iter_fns.iter().take(40) {
        eprintln!("  {}", n);
    }

    let mut base_modules: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (_, fd) in m.functions.iter() {
        let mp = fd.module_path.as_str();
        if mp.starts_with("core.base") {
            base_modules.insert(mp.to_string());
        }
    }
    eprintln!("Distinct core.base.* module_paths:");
    for mp in &base_modules {
        eprintln!("  {}", mp);
    }

    eprintln!("---");
    let core_prelude = m.module_reexports
        .get(&verum_common::Text::from("core.prelude"));
    eprintln!("core.prelude reexports: {:?}", core_prelude.map(|l| l.len()));
    let core_base = m.module_reexports
        .get(&verum_common::Text::from("core.base"));
    eprintln!("core.base reexports: {:?}", core_base.map(|l| l.len()));
}
