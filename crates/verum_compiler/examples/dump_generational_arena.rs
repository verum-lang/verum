// Task #8 diagnostic — dump GenerationalArena descriptor + new/with_config
// bytecode from the stdlib archive.
//
// Usage: cargo run --release -p verum_compiler --example dump_generational_arena
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let archive_path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        // Resolve the freshest archive across release / debug / out dirs.
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
    println!("Loading archive: {}", archive_path.display());
    let file = std::fs::File::open(&archive_path)?;
    let archive = verum_vbc::archive::read_archive(file)?;
    println!("Archive: {} modules", archive.module_count());

    // Try several candidate entry names for arena.vr
    let candidates = ["core.mem.arena", "mem.arena", "core.mem", "arena"];
    let mut found_idx = None;
    let mut found_name = String::new();
    for cand in candidates {
        if let Some(idx) = archive.get_entry_index(cand) {
            found_idx = Some(idx);
            found_name = cand.to_string();
            break;
        }
    }
    if found_idx.is_none() {
        // Print all entries containing "arena"
        println!("\nNo direct hit. Entries containing 'arena':");
        for entry in &archive.index {
            if entry.name.contains("arena") || entry.name.contains("mem") {
                println!("  {}", entry.name);
            }
        }
        return Err("arena module not found".into());
    }
    let entry_idx = found_idx.unwrap();
    let module = archive.load_module_by_index(entry_idx)?;
    println!("Loaded module '{}' with {} types, {} functions", found_name, module.types.len(), module.functions.len());

    // Find GenerationalArena TypeDescriptor
    println!("\n=== TypeDescriptor for GenerationalArena ===");
    for td in &module.types {
        let tname = module.strings.get(td.name).unwrap_or("<?>");
        if tname == "GenerationalArena" || tname.contains("GenerationalArena") {
            println!("  TypeId({}) name='{}' kind={:?}", td.id.0, tname, td.kind);
            println!("  fields ({} declared):", td.fields.len());
            for (idx, f) in td.fields.iter().enumerate() {
                let fname = module.strings.get(f.name).unwrap_or("<?>");
                println!("    [{}] name='{}' type_ref={:?}", idx, fname, f.type_ref);
            }
        }
    }

    // Find ArenaConfig TypeDescriptor for comparison
    println!("\n=== TypeDescriptor for ArenaConfig ===");
    for td in &module.types {
        let tname = module.strings.get(td.name).unwrap_or("<?>");
        if tname == "ArenaConfig" {
            println!("  TypeId({}) name='{}' kind={:?}", td.id.0, tname, td.kind);
            println!("  fields ({} declared):", td.fields.len());
            for (idx, f) in td.fields.iter().enumerate() {
                let fname = module.strings.get(f.name).unwrap_or("<?>");
                println!("    [{}] name='{}' type_ref={:?}", idx, fname, f.type_ref);
            }
        }
    }

    // Find GenerationalArena.new function
    println!("\n=== function 'GenerationalArena.new' ===");
    for f in &module.functions {
        let name = module.strings.get(f.name).unwrap_or("");
        if name == "GenerationalArena.new" || name == "GenerationalArena.with_config" {
            println!(
                "\n--- {} (id={}, offset={}, len={}, params={}) ---",
                name, f.id.0, f.bytecode_offset, f.bytecode_length, f.params.len()
            );
            let body = &module.bytecode[f.bytecode_offset as usize..(f.bytecode_offset + f.bytecode_length) as usize];
            match verum_vbc::bytecode::decode_instructions(body) {
                Ok(insns) => {
                    let mut byte_offset = 0usize;
                    for (idx, instr) in insns.iter().enumerate() {
                        let size = verum_vbc::bytecode::instruction_size(instr);
                        // Annotate New / SetF / Call / CallM
                        let extra = match instr {
                            verum_vbc::Instruction::New { type_id, field_count, .. } => {
                                let name = module.types.iter().find(|t| t.id.0 == *type_id)
                                    .map(|t| module.strings.get(t.name).unwrap_or("<?>").to_string())
                                    .unwrap_or_else(|| "<unknown TypeId>".to_string());
                                format!("  // type='{}' field_count={}", name, field_count)
                            }
                            verum_vbc::Instruction::SetF { field_idx, .. } => {
                                format!("  // field_idx={}", field_idx)
                            }
                            verum_vbc::Instruction::Call { func_id, .. } => {
                                let local = module.functions.iter().find(|x| x.id.0 == *func_id).and_then(|f| module.strings.get(f.name));
                                let ext = module.external_function_names.iter().find(|(fid, _)| fid.0 == *func_id).and_then(|(_, sid)| module.strings.get(*sid));
                                format!("  // func_id={} local={:?} extern={:?}", func_id, local, ext)
                            }
                            verum_vbc::Instruction::CallM { method_id, .. } => {
                                format!("  // method_id={} = '{}'",
                                    method_id,
                                    module.strings.get(verum_vbc::types::StringId(*method_id)).unwrap_or("<?>"))
                            }
                            _ => String::new(),
                        };
                        println!("  [{:3}] pc={:3} size={} {:?}{}", idx, byte_offset, size, instr, extra);
                        byte_offset += size;
                    }
                }
                Err(e) => println!("  decode failed: {}", e),
            }
        }
    }

    // Show all GenerationalArena methods
    println!("\n=== All functions with 'GenerationalArena' in name ===");
    for f in &module.functions {
        let name = module.strings.get(f.name).unwrap_or("");
        if name.contains("GenerationalArena") {
            println!("  id={} name='{}'", f.id.0, name);
        }
    }
    Ok(())
}
