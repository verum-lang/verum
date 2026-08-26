// Does the ARCHIVE carry a protocol's methods as variants?
//
// The metadata writer builds a protocol's method list from the type
// descriptor's `variants`, each expected to hold a
// `TypeRef::Function` payload. Every protocol in the baked metadata
// reports zero methods, so this prints what the archive actually holds
// for the type: how many variants, and what shape each payload is.
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let archive_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/precompiled-stdlib/runtime.vbca"));
    let wanted = std::env::args().nth(2).unwrap_or_else(|| "Debug".to_string());

    let file = std::fs::File::open(&archive_path)?;
    let archive = verum_vbc::archive::read_archive(file)?;

    for entry in archive.index.iter() {
        let Ok(module) = archive.load_module(&entry.name) else {
            continue;
        };
        for f in module.functions.iter() {
            let fname = module.strings.get(f.name).unwrap_or("<?>");
            if fname == wanted {
                println!(
                    "{}::fn {fname}  return_type={:?}  return_type_name={:?}",
                    entry.name,
                    f.return_type,
                    f.return_type_name.and_then(|sid| module.strings.get(sid)).unwrap_or("")
                );
            }
        }
        for ty in module.types.iter() {
            let name = module.strings.get(ty.name).unwrap_or("<?>");
            if name != wanted {
                continue;
            }
            println!(
                "{}::{name}  variants={} type_params={}",
                entry.name,
                ty.variants.len(),
                ty.type_params.len()
            );
            for v in ty.variants.iter() {
                let vname = module.strings.get(v.name).unwrap_or("<?>");
                let shape = match &v.payload {
                    Some(verum_vbc::types::TypeRef::Function { params, .. }) => {
                        format!("Function/{}", params.len())
                    }
                    Some(other) => format!("{other:?}"),
                    None => "none".to_string(),
                };
                println!("    {vname:<20} payload={shape}");
            }
        }
    }
    Ok(())
}
