// Dump Map.contains_key bytecode to see how `entry.key == key` compiles.
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let archive_path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        for cand in [
            "target/release/build/verum_compiler-5dee93e2b79950a1/out/stdlib_runtime.vbca",
            "target/precompiled-stdlib/runtime.vbca",
        ] {
            if std::path::Path::new(cand).exists() {
                return PathBuf::from(cand);
            }
        }
        PathBuf::from("target/precompiled-stdlib/runtime.vbca")
    });
    let file = std::fs::File::open(&archive_path)?;
    let archive = verum_vbc::archive::read_archive(file)?;
    let candidates = ["core.collections.map", "collections.map", "core.collections", "map"];
    let mut idx = None;
    let mut name = String::new();
    for c in candidates {
        if let Some(i) = archive.get_entry_index(c) {
            idx = Some(i);
            name = c.to_string();
            break;
        }
    }
    let idx = idx.ok_or("map module not found")?;
    let module = archive.load_module_by_index(idx)?;
    println!("Loaded {} ({} types, {} functions)", name, module.types.len(), module.functions.len());

    for f in &module.functions {
        let fname = module.strings.get(f.name).unwrap_or("");
        if fname == "Map.contains_key" {
            println!("\n=== Map.contains_key (id={}, off={}, len={}) ===",
                f.id.0, f.bytecode_offset, f.bytecode_length);
            let body = &module.bytecode[f.bytecode_offset as usize..(f.bytecode_offset + f.bytecode_length) as usize];
            if let Ok(insns) = verum_vbc::bytecode::decode_instructions(body) {
                let mut pc = 0usize;
                for (i, instr) in insns.iter().enumerate() {
                    let size = verum_vbc::bytecode::instruction_size(instr);
                    let extra = match instr {
                        verum_vbc::Instruction::CmpG { protocol_id, .. } => {
                            format!("  // protocol_id={}", protocol_id)
                        }
                        verum_vbc::Instruction::CallM { method_id, .. } => {
                            format!("  // method_id={} = '{}'", method_id,
                                module.strings.get(verum_vbc::types::StringId(*method_id)).unwrap_or("?"))
                        }
                        _ => String::new(),
                    };
                    println!("  [{:3}] pc={:3} {:?}{}", i, pc, instr, extra);
                    pc += size;
                }
            }
        }
    }
    Ok(())
}
