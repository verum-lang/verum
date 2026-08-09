// `verum vbc-version <archive>` — inspect a .vbc archive header.
//

// Reads the first 96 bytes of the archive (the VbcHeader) and prints
// magic / version / section offsets / hashes. Verifies magic and
// version-compatibility against the consumer (this binary's
// VERSION_MAJOR / VERSION_MINOR), printing a clear OK / NOT-COMPATIBLE
// banner.
//

// Tracked under #175 (VBC bytecode versioning + migration path).

use crate::error::{CliError, Result};
use colored::Colorize;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use verum_vbc::format::{HEADER_SIZE, MAGIC, VERSION_MAJOR, VERSION_MINOR, VbcFlags, VbcHeader};

/// Decode just the fixed-size header (96 bytes) from the front of an
/// archive. Avoids full module deserialisation so this works on
/// archives whose body the current consumer can't decode (e.g. older
/// minor versions with unknown opcodes — the whole point of a header
/// inspector is to learn THAT before failing on the body).
fn decode_header_only(data: &[u8]) -> Result<VbcHeader> {
    if data.len() < HEADER_SIZE {
        return Err(CliError::Custom(format!(
            "file is {} bytes; need at least {} bytes for VBC header",
            data.len(),
            HEADER_SIZE
        )));
    }
    let mut o = 0usize;
    macro_rules! r_u16 {
        () => {{
            let b: [u8; 2] = data[o..o + 2].try_into().unwrap();
            o += 2;
            u16::from_le_bytes(b)
        }};
    }
    macro_rules! r_u32 {
        () => {{
            let b: [u8; 4] = data[o..o + 4].try_into().unwrap();
            o += 4;
            u32::from_le_bytes(b)
        }};
    }
    macro_rules! r_u64 {
        () => {{
            let b: [u8; 8] = data[o..o + 8].try_into().unwrap();
            o += 8;
            u64::from_le_bytes(b)
        }};
    }

    let magic: [u8; 4] = data[o..o + 4].try_into().unwrap();
    o += 4;
    let version_major = r_u16!();
    let version_minor = r_u16!();
    let flags_bits = r_u32!();
    let module_name_offset = r_u32!();
    let type_table_offset = r_u32!();
    let type_table_count = r_u32!();
    let function_table_offset = r_u32!();
    let function_table_count = r_u32!();
    let constant_pool_offset = r_u32!();
    let constant_pool_count = r_u32!();
    let string_table_offset = r_u32!();
    let string_table_size = r_u32!();
    let bytecode_offset = r_u32!();
    let bytecode_size = r_u32!();
    let specialization_table_offset = r_u32!();
    let specialization_table_count = r_u32!();
    let source_map_offset = r_u32!();
    let source_map_size = r_u32!();
    let content_hash = r_u64!();
    let dependency_hash = r_u64!();
    let extensions_offset = r_u32!();
    let extensions_size = r_u32!();
    debug_assert_eq!(o, HEADER_SIZE);
    Ok(VbcHeader {
        magic,
        version_major,
        version_minor,
        flags: VbcFlags::from_bits_truncate(flags_bits),
        module_name_offset,
        type_table_offset,
        type_table_count,
        function_table_offset,
        function_table_count,
        constant_pool_offset,
        constant_pool_count,
        string_table_offset,
        string_table_size,
        bytecode_offset,
        bytecode_size,
        specialization_table_offset,
        specialization_table_count,
        source_map_offset,
        source_map_size,
        content_hash,
        dependency_hash,
        extensions_offset,
        extensions_size,
    })
}

pub fn execute(archive: &Path, raw: bool) -> Result<()> {
    let mut buf = Vec::with_capacity(HEADER_SIZE * 2);
    let mut f = File::open(archive)
        .map_err(|e| CliError::Custom(format!("cannot open {}: {}", archive.display(), e)))?;
    let _ = f
        .by_ref()
        .take(HEADER_SIZE as u64 * 16)
        .read_to_end(&mut buf)
        .map_err(|e| CliError::Custom(format!("read error on {}: {}", archive.display(), e)))?;
    let header = decode_header_only(&buf)?;

    if raw {
        // Stable, machine-parseable single-line key=value form for
        // scripting. Order matches the on-wire layout.
        println!(
            "magic={} major={} minor={} flags={:#010x} \
             module_name_offset={} type_table=({},{}) function_table=({},{}) \
             constant_pool=({},{}) string_table=({},{}) bytecode=({},{}) \
             specialization_table=({},{}) source_map=({},{}) extensions=({},{}) \
             content_hash={:#018x} dependency_hash={:#018x} compatible={}",
            String::from_utf8_lossy(&header.magic),
            header.version_major,
            header.version_minor,
            header.flags.bits(),
            header.module_name_offset,
            header.type_table_offset,
            header.type_table_count,
            header.function_table_offset,
            header.function_table_count,
            header.constant_pool_offset,
            header.constant_pool_count,
            header.string_table_offset,
            header.string_table_size,
            header.bytecode_offset,
            header.bytecode_size,
            header.specialization_table_offset,
            header.specialization_table_count,
            header.source_map_offset,
            header.source_map_size,
            header.extensions_offset,
            header.extensions_size,
            header.content_hash,
            header.dependency_hash,
            header.is_magic_valid() && header.is_version_compatible(),
        );
        return Ok(());
    }

    println!("{} {}", "VBC archive:".bold(), archive.display());
    println!();

    let magic_str = String::from_utf8_lossy(&header.magic);
    let magic_label = if header.is_magic_valid() {
        format!("{} ({})", magic_str, "ok".green())
    } else {
        format!(
            "{} ({} — expected {})",
            magic_str,
            "wrong".red(),
            String::from_utf8_lossy(&MAGIC)
        )
    };
    println!("  Magic:                {}", magic_label);

    let ver_str = format!("{}.{}", header.version_major, header.version_minor);
    let ver_label = if header.is_version_compatible() {
        format!("{} ({})", ver_str, "compatible".green())
    } else {
        format!(
            "{} ({} — consumer supports {}.0-{}.{})",
            ver_str,
            "incompatible".red(),
            VERSION_MAJOR,
            VERSION_MAJOR,
            VERSION_MINOR,
        )
    };
    println!("  Version:              {}", ver_label);
    println!("  Flags:                {:#010x}", header.flags.bits());
    println!();
    println!("  Module name offset:   {}", header.module_name_offset);
    println!(
        "  Type table:           offset={} count={}",
        header.type_table_offset, header.type_table_count
    );
    println!(
        "  Function table:       offset={} count={}",
        header.function_table_offset, header.function_table_count
    );
    println!(
        "  Constant pool:        offset={} count={}",
        header.constant_pool_offset, header.constant_pool_count
    );
    println!(
        "  String table:         offset={} size={}",
        header.string_table_offset, header.string_table_size
    );
    println!(
        "  Bytecode:             offset={} size={}",
        header.bytecode_offset, header.bytecode_size
    );
    println!(
        "  Specialization table: offset={} count={}",
        header.specialization_table_offset, header.specialization_table_count
    );
    if header.source_map_offset > 0 {
        println!(
            "  Source map:           offset={} size={}",
            header.source_map_offset, header.source_map_size
        );
    } else {
        println!("  Source map:           {}", "absent".dimmed());
    }
    if header.extensions_offset > 0 {
        println!(
            "  Extensions:           offset={} size={}",
            header.extensions_offset, header.extensions_size
        );
    } else {
        println!("  Extensions:           {}", "absent".dimmed());
    }
    println!();
    println!("  Content hash:         {:#018x}", header.content_hash);
    println!("  Dependency hash:      {:#018x}", header.dependency_hash);
    println!();
    if !header.is_magic_valid() {
        println!("{}", "FAIL: magic mismatch".red().bold());
        return Err(CliError::Custom("VBC archive magic mismatch".into()));
    }
    if !header.is_version_compatible() {
        println!(
            "{}",
            format!(
                "FAIL: archive is v{}.{}, consumer supports v{}.0-{}.{}",
                header.version_major,
                header.version_minor,
                VERSION_MAJOR,
                VERSION_MAJOR,
                VERSION_MINOR
            )
            .red()
            .bold()
        );
        return Err(CliError::Custom("VBC archive version not supported".into()));
    }
    println!("{}", "OK".green().bold());
    Ok(())
}

/// Dump the bytecode of every function whose name contains `needle`.
///
/// Why this exists: three separate defects this campaign differ ONLY
/// between the baked archive and the same source compiled locally —
/// `Mutex.try_lock` never reaching the atomic-CAS opcode, a generic
/// tuple impl misdispatching, and a re-export leaf missing from a
/// module surface. No user-code reproduction can reach any of them,
/// because the thing under suspicion is what the BAKE produced. Until
/// now the only archive inspection available was `vbc-version`, which
/// parses the header and stops, so the baked body was unreadable and
/// every question about it cost a 46-minute rebuild to guess at.
///
/// The output is deliberately raw — module, function, byte length and
/// the opcode stream — because the question it answers is "is the
/// instruction there at all", not "what does this program mean".
pub fn dump_function(archive: &Path, needle: &str) -> Result<()> {
    let arch = verum_vbc::archive::read_archive_from_file(archive)
        .map_err(|e| CliError::Custom(format!("cannot read archive: {e}")))?;

    let mut found = 0usize;
    // Slice the module disassembly to one function. `disassemble_module`
    // marks each body with `; fn <name>(`, so a section runs to the next
    // such marker. Computed lazily and once per module — a stdlib module
    // disassembles to megabytes and most modules match nothing.
    fn section_for<'a>(disasm: &'a str, name: &str) -> Option<&'a str> {
        let marker = format!("; fn {}(", name);
        let start = disasm.find(&marker)?;
        let rest = &disasm[start..];
        let end = rest
            .get(1..)
            .and_then(|tail| tail.find("\n; fn "))
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        Some(&rest[..end])
    }

    for (entry, data) in arch.index.iter().zip(arch.module_data.iter()) {
        let module = match verum_vbc::deserialize::deserialize_module(data) {
            Ok(m) => m,
            Err(e) => {
                println!(
                    "{} {}: {}",
                    "skip".yellow(),
                    entry.name,
                    format!("undecodable: {e:?}").dimmed()
                );
                continue;
            }
        };
        let mut disasm: Option<String> = None;
        for f in &module.functions {
            let name = module.strings.get(f.name).unwrap_or("<unnamed>");
            if !name.contains(needle) {
                continue;
            }
            found += 1;
            if disasm.is_none() {
                disasm = Some(verum_vbc::disassemble::disassemble_module(&module));
            }
            let start = f.bytecode_offset as usize;
            let end = start.saturating_add(f.bytecode_length as usize);
            println!(
                "\n{} {}  {} {}  {} {} bytes @ {}",
                "module".dimmed(),
                entry.name.cyan(),
                "fn".dimmed(),
                name.green().bold(),
                "body".dimmed(),
                f.bytecode_length,
                start
            );
            if f.bytecode_length == 0 {
                // An empty body is the signature of a forward
                // declaration or of a body the codegen dropped — the
                // exact thing `[lenient] SKIP` leaves behind, and the
                // reason a call can "succeed" while doing nothing.
                println!("  {}", "EMPTY BODY".red().bold());
                continue;
            }
            match module.bytecode.get(start..end) {
                Some(code) => {
                    for (i, chunk) in code.chunks(16).enumerate() {
                        let hex: Vec<String> =
                            chunk.iter().map(|b| format!("{b:02x}")).collect();
                        println!("  {:04x}  {}", i * 16, hex.join(" "));
                    }
                }
                None => println!(
                    "  {} offset {}..{} outside a {}-byte section",
                    "OUT OF RANGE".red().bold(),
                    start,
                    end,
                    module.bytecode.len()
                ),
            }
            // Hex answers "is the byte there"; the decoded form answers
            // "which instruction is it", and hand-decoding a stream whose
            // operand widths vary per opcode is how one mistakes an
            // operand byte for an opcode.
            match disasm.as_deref().and_then(|d| section_for(d, name)) {
                Some(sec) => {
                    println!("  {}", "— decoded —".dimmed());
                    for line in sec.lines() {
                        println!("  {line}");
                    }
                }
                None => println!("  {}", "(no decoded section)".dimmed()),
            }
            // Say plainly what an empty decode means here, because it
            // means NOTHING about this function. `deserialize_module`
            // restores raw `bytecode` and does not rebuild the decoded
            // instruction list, so the disassembler prints "(no decoded
            // instructions)" for EVERY archive-loaded function —
            // verified against three, including one measured to work.
            // Without this line the next reader takes it for evidence.
            if disasm
                .as_deref()
                .and_then(|d| section_for(d, name))
                .is_some_and(|s| s.contains("(no decoded instructions)"))
            {
                println!(
                    "  {}",
                    "note: archive modules carry raw bytecode only — an empty decode is \
                     universal here and says nothing about this function; read the hex"
                        .dimmed()
                );
            }
        }
    }

    if found == 0 {
        println!("{} no function name contains {:?}", "none".yellow(), needle);
    } else {
        println!("\n{} {} function(s)", "total".dimmed(), found);
    }
    Ok(())
}

/// Resolve a string id against every module's string table.
///
/// `CallM`'s third operand is a method id — a StringId — and reading a
/// baked body leaves you holding that number with no way to turn it
/// into a name. Without this the question "which method does this call
/// actually name" costs a rebuild to answer, which is the same tax
/// `--dump-fn` was written to remove.
pub fn resolve_string(archive: &Path, id: u32) -> Result<()> {
    let arch = verum_vbc::archive::read_archive_from_file(archive)
        .map_err(|e| CliError::Custom(format!("cannot read archive: {e}")))?;

    let mut hits = 0usize;
    for (entry, data) in arch.index.iter().zip(arch.module_data.iter()) {
        let Ok(module) = verum_vbc::deserialize::deserialize_module(data) else {
            continue;
        };
        if let Some(s) = module.strings.get(verum_vbc::types::StringId(id)) {
            hits += 1;
            println!("{:<28} {}", entry.name.cyan(), s.green().bold());
        }
    }
    if hits == 0 {
        println!("{} id {} is in no module's string table", "none".yellow(), id);
    } else {
        // String ids are PER MODULE, so the same number names different
        // things in different modules. Say so rather than letting a
        // single-line answer read as global truth.
        println!(
            "\n{} {} module(s) — ids are per-module, so pick the row whose module owns the body you read",
            "total".dimmed(),
            hits
        );
    }
    Ok(())
}
