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
use verum_vbc::format::{
    VbcFlags, VbcHeader, HEADER_SIZE, MAGIC, VERSION_MAJOR, VERSION_MINOR,
};

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
    let mut f = File::open(archive).map_err(|e| {
        CliError::Custom(format!("cannot open {}: {}", archive.display(), e))
    })?;
    let _ = f
        .by_ref()
        .take(HEADER_SIZE as u64 * 16)
        .read_to_end(&mut buf)
        .map_err(|e| {
            CliError::Custom(format!("read error on {}: {}", archive.display(), e))
        })?;
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
    println!(
        "  Dependency hash:      {:#018x}",
        header.dependency_hash
    );
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
