use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let archive_path = PathBuf::from(
        std::env::args().nth(1).unwrap_or_else(|| {
            "target/release/build/verum_compiler-5dee93e2b79950a1/out/stdlib_runtime.vbca".to_string()
        }),
    );
    let file = std::fs::File::open(&archive_path)?;
    let archive = verum_vbc::archive::read_archive(file)?;
    println!("Archive entries containing 'set':");
    for entry in &archive.index {
        if entry.name.contains("set") {
            println!("  {}", entry.name);
        }
    }
    for cand in ["core.collections.set", "set", "collections.set", "core.collections", "core.collections.set.set"] {
        if let Some(idx) = archive.get_entry_index(cand) {
            let module = archive.load_module_by_index(idx)?;
            println!("{} ({} fns)", cand, module.functions.len());
            for f in &module.functions {
                let n = module.strings.get(f.name).unwrap_or("");
                if n == "Set.insert" || n == "Set.contains" {
                    println!("\n=== {} id={} len={} ===", n, f.id.0, f.bytecode_length);
                    let body = &module.bytecode[f.bytecode_offset as usize..(f.bytecode_offset + f.bytecode_length) as usize];
                    if let Ok(insns) = verum_vbc::bytecode::decode_instructions(body) {
                        let mut pc = 0usize;
                        for (i, instr) in insns.iter().enumerate() {
                            let s = verum_vbc::bytecode::instruction_size(instr);
                            let extra = match instr {
                                verum_vbc::Instruction::Call { func_id, .. } => {
                                    let local = module.functions.iter().find(|x| x.id.0 == *func_id).and_then(|f| module.strings.get(f.name));
                                    let ext = module.external_function_names.iter().find(|(fid, _)| fid.0 == *func_id).and_then(|(_, sid)| module.strings.get(*sid));
                                    format!("  // func={} local={:?} ext={:?}", func_id, local, ext)
                                }
                                verum_vbc::Instruction::CallM { method_id, .. } => {
                                    format!("  // method='{}'", module.strings.get(verum_vbc::types::StringId(*method_id)).unwrap_or("?"))
                                }
                                _ => String::new(),
                            };
                            println!("  [{:3}] pc={:3} {:?}{}", i, pc, instr, extra);
                            pc += s;
                        }
                    }
                }
            }
            break;
        }
    }
    Ok(())
}
