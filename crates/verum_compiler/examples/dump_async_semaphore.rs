// Dump AsyncSemaphore.new's bytecode from the stdlib archive for diagnostic.
//
// Usage: cargo run --release -p verum_compiler --example dump_async_semaphore

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let archive_path = PathBuf::from("target/precompiled-stdlib/runtime.vbca");
    println!("Loading archive: {}", archive_path.display());
    let file = std::fs::File::open(&archive_path)?;
    let archive = verum_vbc::archive::read_archive(file)?;
    println!("Archive: {} modules", archive.module_count());

    // Find core.async.semaphore module — print all entries with 'semaphore' in name
    println!("\n=== Archive entries containing 'semaphore' ===");
    for entry in &archive.index {
        if entry.name.contains("semaphore") || entry.name.contains("async") {
            println!("  {}", entry.name);
        }
    }
    // Try several candidate names
    let candidates = [
        "core.async.semaphore",
        "async.semaphore",
        "core.async",
    ];
    let mut found_name = String::new();
    let mut entry_idx = 0;
    for cand in candidates {
        if let Some(idx) = archive.get_entry_index(cand) {
            found_name = cand.to_string();
            entry_idx = idx;
            break;
        }
    }
    if found_name.is_empty() {
        return Err("no async semaphore module found".into());
    }
    let mod_name = found_name.as_str();
    let module = archive.load_module_by_index(entry_idx)?;
    println!("Loaded module '{}' with {} functions", mod_name, module.functions.len());

    // Print external_function_names
    println!("\n=== external_function_names ({} entries) ===", module.external_function_names.len());
    for (fid, sid) in &module.external_function_names {
        let name = module.strings.get(*sid).unwrap_or("<?>");
        println!("  archive_id={} -> name={:?}", fid.0, name);
    }

    // Find AsyncSemaphore.new
    let target = "AsyncSemaphore.new";
    let mut found = None;
    for f in &module.functions {
        let name = module.strings.get(f.name).unwrap_or("");
        if name == target {
            found = Some(f);
            break;
        }
    }
    let func = found.ok_or_else(|| format!("function {} not found", target))?;
    println!(
        "\n=== function '{}' descriptor.id={} bytecode_offset={} bytecode_length={} ===",
        target, func.id.0, func.bytecode_offset, func.bytecode_length
    );

    // Decode the bytecode
    let body_bytes = &module.bytecode[func.bytecode_offset as usize..(func.bytecode_offset + func.bytecode_length) as usize];
    let instructions = verum_vbc::bytecode::decode_instructions(body_bytes)
        .map_err(|e| format!("decode failed: {}", e))?;
    println!("Decoded {} instructions", instructions.len());

    // Print each instruction with its byte offset
    let mut byte_offset = 0;
    for (idx, instr) in instructions.iter().enumerate() {
        let size = verum_vbc::bytecode::instruction_size(instr);
        println!("  [{:3}] pc={:3} size={} {:?}", idx, byte_offset, size, instr);
        byte_offset += size;
    }

    // Show all functions that involve "Semaphore" or "Mutex" in name
    println!("\n=== Functions with 'Semaphore' / 'Mutex' / 'Shared' / 'Deque' in name ===");
    for f in &module.functions {
        let name = module.strings.get(f.name).unwrap_or("");
        if name.contains("Semaphore") || name.contains("Mutex") || name.contains("Shared") || name.contains("Deque") {
            println!("  id={} name='{}'", f.id.0, name);
        }
    }

    // Dump available_permits and its closure
    println!("\n=== available_permits body ===");
    for f in &module.functions {
        let name = module.strings.get(f.name).unwrap_or("");
        if name == "AsyncSemaphore.available_permits" || name.contains("available_permits$closure") {
            println!("\n--- {} (id={}, len={}) ---", name, f.id.0, f.bytecode_length);
            let body = &module.bytecode[f.bytecode_offset as usize..(f.bytecode_offset + f.bytecode_length) as usize];
            if let Ok(insns) = verum_vbc::bytecode::decode_instructions(body) {
                let mut byte_offset = 0;
                for (idx, instr) in insns.iter().enumerate() {
                    let size = verum_vbc::bytecode::instruction_size(instr);
                    // For CallM, lookup method_id in strings
                    let extra = if let verum_vbc::Instruction::CallM { method_id, .. } = instr {
                        format!("  // method_id={} = '{}'", method_id, module.strings.get(verum_vbc::types::StringId(*method_id)).unwrap_or("?"))
                    } else if let verum_vbc::Instruction::Call { func_id, .. } = instr {
                        // Resolve func_id name
                        let local = module.functions.iter().find(|f| f.id.0 == *func_id).and_then(|f| module.strings.get(f.name));
                        let ext = module.external_function_names.iter().find(|(fid, _)| fid.0 == *func_id).and_then(|(_, sid)| module.strings.get(*sid));
                        format!("  // func_id={} name={:?}{}", func_id, local, if let Some(n) = ext { format!(" (extern={})", n) } else { String::new() })
                    } else {
                        String::new()
                    };
                    println!("  [{:3}] pc={:3} size={} {:?}{}", idx, byte_offset, size, instr, extra);
                    byte_offset += size;
                }
            }
        }
    }

    Ok(())
}
