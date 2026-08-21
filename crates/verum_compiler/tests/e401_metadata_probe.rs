//! Throwaway probe (E401 investigation): what does the SHIPPED archive
//! hold for the witness modules, at each layer — entry list, per-entry
//! type descriptors, and the converted CoreMetadata?

use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_vbc::archive::read_archive_from_file;

#[test]
#[ignore]
fn dump_witness_modules() {
    let path = std::env::var("VBCA").expect("set VBCA=/path/to/runtime.vbca");
    let archive = read_archive_from_file(&path).expect("archive readable");

    for (idx, entry) in archive.index.iter().enumerate() {
        if !entry.name.contains("s_definable") && !entry.name.contains("rich_s") {
            continue;
        }
        println!("== entry[{idx}] `{}`", entry.name);
        match archive.load_module_by_index(idx) {
            Ok(module) => {
                for ty in &module.types {
                    let tn = module.strings.get(ty.name).unwrap_or_default();
                    let origin = ty
                        .origin_module
                        .and_then(|sid| module.strings.get(sid))
                        .unwrap_or_default();
                    println!("   type `{}` kind={:?} origin=`{}`", tn, ty.kind, origin);
                }
            }
            Err(e) => println!("   DECODE ERROR: {e:?}"),
        }
    }

    let meta = archive_to_core_metadata(&archive);
    let hits: Vec<&str> = meta
        .types
        .keys()
        .map(|k| k.as_str())
        .filter(|k| k.contains("Formally") || k.contains("RichS"))
        .collect();
    println!("== meta.types hits: {hits:?}");
}
