// Does `core.io.file` carry BODIES, or only declarations?
//
// `File.open(path)` fails at run time with "stage-5 qualified
// cross-module fn stub never resolved", while the archive's strings do
// contain `File.open`. A name in the string table is not a body, so
// this prints the body length of every function the module declares.
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let archive_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/precompiled-stdlib/runtime.vbca"));
    let file = std::fs::File::open(&archive_path)?;
    let archive = verum_vbc::archive::read_archive(file)?;

    let module_name = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "core.io.file".to_string());
    let idx = archive
        .get_entry_index(&module_name)
        .ok_or_else(|| format!("{module_name} not in archive"))?;
    let module = archive.load_module_by_index(idx)?;

    println!(
        "{module_name}: {} types, {} functions",
        module.types.len(),
        module.functions.len()
    );

    let mut empty = 0usize;
    let mut nonempty = 0usize;
    for f in module.functions.iter() {
        let name = module.strings.get(f.name).unwrap_or("<?>");
        let len = f.bytecode_length;
        if len == 0 {
            empty += 1;
        } else {
            nonempty += 1;
        }
        if name.starts_with("File.") || name.starts_with("OpenOptions.") {
            println!("  {name:<40} body={len}");
        }
    }
    println!("bodies: {nonempty} present, {empty} empty");
    Ok(())
}
