// What does the baked metadata record as a protocol method's return type?
//
// `?` on `e.fmt_debug(f)` fails with "`Result` does not implement `Try`",
// and the diagnostic names a BARE `Result`. The declaration says
// `-> Result<(), FormatError>`, so this prints what actually survived
// into the metadata the type checker reads.
fn main() {
    let protocol = std::env::args().nth(1).unwrap_or_else(|| "Debug".to_string());
    let Some(meta) = verum_compiler::embedded_stdlib_metadata::get_runtime_metadata() else {
        eprintln!("no embedded runtime metadata");
        return;
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
}
