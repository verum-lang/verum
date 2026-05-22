use verum_types::core_metadata::CoreMetadata;

fn load_metadata() -> CoreMetadata {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/stdlib_runtime.core_metadata"));
    bincode::deserialize::<CoreMetadata>(bytes).expect("deserialise embedded metadata")
}

#[test]
fn semver_in_types() {
    let m = load_metadata();
    let key = verum_common::Text::from("SemVer");
    if let Some(td) = m.types.get(&key) {
        eprintln!("SemVer in metadata.types: methods = {:?}",
            td.methods.iter().map(|t| t.as_str().to_string()).collect::<Vec<_>>());
    } else {
        eprintln!("SemVer NOT in metadata.types");
    }
    let cmp_key = verum_common::Text::from("SemVer.cmp");
    if let Some(fd) = m.functions.get(&cmp_key) {
        eprintln!("SemVer.cmp: params={:?} return={}",
            fd.params.iter().map(|p| p.ty.as_str().to_string()).collect::<Vec<_>>(),
            fd.return_type.as_str());
    } else {
        eprintln!("SemVer.cmp NOT in metadata.functions");
    }
}
