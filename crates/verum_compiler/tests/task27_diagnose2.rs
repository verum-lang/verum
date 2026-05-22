use verum_types::core_metadata::CoreMetadata;

fn load_metadata() -> CoreMetadata {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/stdlib_runtime.core_metadata"));
    bincode::deserialize::<CoreMetadata>(bytes).expect("deserialise embedded metadata")
}

#[test]
fn dump_simple_fn_keys() {
    let m = load_metadata();
    let probes = ["range", "swap", "take", "replace", "repeat", "count_from", "args_count", "var", "set_var"];
    for n in &probes {
        let key = verum_common::Text::from(*n);
        match m.functions.get(&key) {
            Some(fd) => eprintln!("FN {} mod={}", n, fd.module_path.as_str()),
            None => eprintln!("FN {} NOT IN simple key", n),
        }
    }
    eprintln!("--- All keys ending in these names ---");
    for (name, fd) in m.functions.iter() {
        for probe in &probes {
            if name.as_str() == *probe || name.as_str().ends_with(&format!(".{}", probe)) {
                eprintln!("  key={} mod={}", name.as_str(), fd.module_path.as_str());
            }
        }
    }
    eprintln!("--- core.base functions COUNT={} ---", m.functions.iter().filter(|(_, fd)| fd.module_path.as_str() == "core.base").count());
    let core_base_keys: Vec<String> = m.functions.iter()
        .filter(|(_, fd)| fd.module_path.as_str() == "core.base")
        .map(|(n, _)| n.as_str().to_string())
        .filter(|n| !n.contains('.'))
        .collect();
    eprintln!("--- core.base SIMPLE (no-dot) keys [{}] ---", core_base_keys.len());
    for k in core_base_keys.iter().take(40) { eprintln!("  {}", k); }

    // Task #36 probe — show ALL "arg" keys
    eprintln!("=== all arg-related keys in metadata.functions ===");
    let mut arg_keys: Vec<String> = m.functions.iter()
        .filter(|(k, _)| {
            let s = k.as_str();
            s == "arg" || s.ends_with(".arg")
        })
        .map(|(k, fd)| format!("  '{}' (module_path={}, return={})", k.as_str(), fd.module_path.as_str(), fd.return_type.as_str()))
        .collect();
    arg_keys.sort();
    for k in arg_keys { eprintln!("{}", k); }
    eprintln!("---");

    for key_str in &["arg", "core.base.env.arg", "core.base.arg", "base.env.arg", "env.arg"] {
        let key = verum_common::Text::from(*key_str);
        if let Some(fd) = m.functions.get(&key) {
            eprintln!("--- key '{}' ---", key_str);
            eprintln!("  return_type: {}", fd.return_type.as_str());
            eprintln!("  module_path: {}", fd.module_path.as_str());
            for (i, p) in fd.params.iter().enumerate() {
                eprintln!("  param[{}]: name={} ty={}", i, p.name.as_str(), p.ty.as_str());
            }
        }
    }
    // Also probe var
    eprintln!("=== var probe ===");
    for key_str in &["var", "core.base.env.var", "core.base.var"] {
        let key = verum_common::Text::from(*key_str);
        if let Some(fd) = m.functions.get(&key) {
            eprintln!("--- key '{}' ---", key_str);
            eprintln!("  return_type: {}", fd.return_type.as_str());
            eprintln!("  module_path: {}", fd.module_path.as_str());
        }
    }

    // VERSION const probe
    for key in &["VERSION", "core.VERSION"] {
        let k = verum_common::Text::from(*key);
        if let Some(fd) = m.functions.get(&k) {
            eprintln!("=== const '{}' ===", key);
            eprintln!("  is_const: {}", fd.is_const);
            eprintln!("  return_type: {}", fd.return_type.as_str());
            eprintln!("  module_path: {}", fd.module_path.as_str());
        }
    }
    // Task #44 probe: Eq protocol super_protocols in metadata
    let eq_key = verum_common::Text::from("Eq");
    if let Some(pd) = m.protocols.get(&eq_key) {
        eprintln!("=== metadata.protocols[Eq] ===");
        eprintln!("  super_protocols: {:?}", pd.super_protocols.iter().map(|s| s.as_str().to_string()).collect::<Vec<_>>());
        eprintln!("  default_methods count: {}", pd.default_methods.len());
        for m in pd.default_methods.iter() { eprintln!("    default: {}", m.name.as_str()); }
    } else {
        eprintln!("=== NO metadata.protocols[Eq] ===");
    }
    let pe_key = verum_common::Text::from("PartialEq");
    if let Some(pd) = m.protocols.get(&pe_key) {
        eprintln!("=== metadata.protocols[PartialEq] ===");
        eprintln!("  super_protocols: {:?}", pd.super_protocols.iter().map(|s| s.as_str().to_string()).collect::<Vec<_>>());
        eprintln!("  default_methods count: {}", pd.default_methods.len());
        for m in pd.default_methods.iter() { eprintln!("    default: {}", m.name.as_str()); }
    }
    // Task #43 probe: does module_reexports[core.prelude] propagate base?
    let prelude_key = verum_common::Text::from("core.prelude");
    if let Some(leaves) = m.module_reexports.get(&prelude_key) {
        eprintln!("=== core.prelude leaves count: {} ===", leaves.len());
        for (local, source) in leaves.iter().filter(|(l, _)|
            ["set_var", "home_dir", "range", "count_from", "replace", "Maybe", "Some", "None"].contains(&l.as_str())
        ) {
            eprintln!("  ({}, {})", local.as_str(), source.as_str());
        }
    } else {
        eprintln!("=== NO core.prelude entry in module_reexports ===");
    }
    // Task #42 — does module_reexports[core.base] contain "arg"?
    eprintln!("=== core.base module_reexports for arg/args ===");
    let cb_key = verum_common::Text::from("core.base");
    if let Some(leaves) = m.module_reexports.get(&cb_key) {
        eprintln!("  total leaves: {}", leaves.len());
        for (local, source) in leaves.iter() {
            let l = local.as_str();
            if l == "arg" || l == "args" || l == "args_count" || l == "var" {
                eprintln!("  ({}, {})", l, source.as_str());
            }
        }
    } else {
        eprintln!("  NO ENTRY for core.base in module_reexports");
    }
}
