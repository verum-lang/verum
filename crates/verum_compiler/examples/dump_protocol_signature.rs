// What does the baked metadata record as a protocol method's return type?
//
// `?` on `e.fmt_debug(f)` fails with "`Result` does not implement `Try`",
// and the diagnostic names a BARE `Result`. The declaration says
// `-> Result<(), FormatError>`, so this prints what actually survived
// into the metadata the type checker reads.
//
// Second argument (optional): a path to a `runtime.core_metadata` FILE.
// Without it the EMBEDDED metadata is read, which is whatever the
// running binary was built with — so a before/after comparison made
// that way silently compares one bake with itself. Point it at a saved
// copy of the pre-change bake and at the fresh one to measure a change
// with a single instrument.
fn main() {
    let protocol = std::env::args().nth(1).unwrap_or_else(|| "Debug".to_string());
    let meta = match std::env::args().nth(2) {
        Some(path) => {
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("cannot read {path}: {e}");
                    return;
                }
            };
            match bincode::deserialize::<verum_types::core_metadata::CoreMetadata>(&bytes) {
                Ok(m) => std::sync::Arc::new(m),
                Err(e) => {
                    eprintln!("cannot decode {path}: {e}");
                    return;
                }
            }
        }
        None => match verum_compiler::embedded_stdlib_metadata::get_runtime_metadata() {
            Some(m) => m,
            None => {
                eprintln!("no embedded runtime metadata");
                return;
            }
        },
    };
    let mut found = false;
    for (name, desc) in meta.protocols.iter() {
        if name.as_str() != protocol {
            continue;
        }
        found = true;
        println!(
            "protocol {name}: {} required, {} default",
            desc.required_methods.len(),
            desc.default_methods.len()
        );
        for m in desc.required_methods.iter().chain(desc.default_methods.iter()) {
            println!(
                "  {:<16} params=[{}] -> {:?}",
                m.name.as_str(),
                m.params
                    .iter()
                    .map(|p| p.ty.as_str().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                m.return_type.as_str()
            );
        }
    }
    if !found {
        println!("protocol {protocol} not in metadata");
    }
    // Also: what does the metadata record for a FUNCTION of that name?
    for (name, f) in meta.functions.iter() {
        if name.as_str().ends_with(protocol.as_str()) || name.as_str() == protocol.as_str() {
            println!("  fn {name} -> {:?}", f.return_type.as_str());
        }
    }
}
