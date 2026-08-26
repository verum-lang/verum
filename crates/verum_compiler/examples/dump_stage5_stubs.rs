// Which names did the bake leave as unresolved stage-5 stubs?
//
// A stage-5 id is minted when a dotted or ambiguous call resolves
// nowhere at compile time; the archive load is supposed to chase it by
// name. When the chase misses, the runtime panics naming only the id —
// this prints the id → name mapping the archive carries, so the failing
// id can be read as a name.
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let archive_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/precompiled-stdlib/runtime.vbca"));
    let wanted_id: Option<u32> = std::env::args().nth(2).and_then(|s| s.parse().ok());

    let file = std::fs::File::open(&archive_path)?;
    let archive = verum_vbc::archive::read_archive(file)?;

    let mut total = 0usize;
    for entry in archive.index.iter() {
        let Ok(module) = archive.load_module(&entry.name) else {
            continue;
        };
        for f in module.functions.iter() {
            if !verum_vbc::stub_ranges::is_stub_id(f.id.0) {
                continue;
            }
            total += 1;
            let name = module.strings.get(f.name).unwrap_or("<?>");
            match wanted_id {
                Some(id) if f.id.0 != id => continue,
                _ => println!(
                    "  {:<12} {:<3} {} :: {name}",
                    f.id.0,
                    verum_vbc::stub_ranges::stage_of(f.id.0).unwrap_or(0),
                    entry.name
                ),
            }
        }
    }
    println!("{total} stub-id descriptors in the archive");
    Ok(())
}
