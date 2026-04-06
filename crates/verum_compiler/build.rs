//! Build script for verum_compiler
//!
//! Embeds the Verum standard library (core/*.vr) into the compiler binary
//! as a zstd-compressed archive. This enables single-binary distribution
//! without external stdlib dependencies.
//!
//! Archive format:
//!   [file_count: u32]
//!   [index: (path_len: u16, path: utf8, content_offset: u32, content_len: u32) × file_count]
//!   [data: concatenated .vr source texts]
//!
//! At runtime, the archive is decompressed once into memory (~2ms for 4.7MB)
//! and provides instant access to all stdlib source files via path lookup.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    // Locate core/ directory (two levels up from crates/verum_compiler/)
    let project_root = Path::new(&manifest_dir).parent().unwrap().parent().unwrap();
    let core_dir = project_root.join("core");

    if !core_dir.exists() {
        // No core/ directory — skip embedding (for CI or minimal builds)
        let archive_path = Path::new(&out_dir).join("stdlib_archive.bin");
        fs::write(&archive_path, &[] as &[u8]).unwrap();
        println!("cargo:rustc-env=STDLIB_ARCHIVE_PATH={}", archive_path.display());
        println!("cargo:warning=core/ directory not found — embedded stdlib disabled");
        return;
    }

    // Collect all .vr files
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect_vr_files(&core_dir, &core_dir, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));

    // Build uncompressed archive
    let archive = build_archive(&files);

    // Compress with zstd (level 19 for maximum compression, decompression is still fast)
    let compressed = zstd::encode_all(archive.as_slice(), 19).unwrap();

    // Write to OUT_DIR
    let archive_path = Path::new(&out_dir).join("stdlib_archive.zst");
    fs::write(&archive_path, &compressed).unwrap();

    println!("cargo:rustc-env=STDLIB_ARCHIVE_PATH={}", archive_path.display());
    println!(
        "cargo:warning=Embedded stdlib: {} files, {:.1}KB compressed (from {:.1}KB)",
        files.len(),
        compressed.len() as f64 / 1024.0,
        archive.len() as f64 / 1024.0,
    );

    // Rerun if any .vr file changes
    println!("cargo:rerun-if-changed={}", core_dir.display());
    for (path, _) in &files {
        println!("cargo:rerun-if-changed={}", core_dir.join(path).display());
    }
}

fn collect_vr_files(dir: &Path, root: &Path, files: &mut Vec<(String, Vec<u8>)>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_vr_files(&path, root, files);
            } else if path.extension().map_or(false, |e| e == "vr") {
                let relative = path.strip_prefix(root).unwrap().to_string_lossy().to_string();
                // Normalize to forward slashes for cross-platform consistency
                let relative = relative.replace('\\', "/");
                if let Ok(content) = fs::read(&path) {
                    files.push((relative, content));
                }
            }
        }
    }
}

fn build_archive(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut archive = Vec::new();
    let file_count: u32 = files.len().try_into()
        .expect("too many stdlib files to fit in u32");

    // Header: file count
    archive.extend_from_slice(&file_count.to_le_bytes());

    // Calculate data section offset using checked arithmetic to prevent overflow.
    // path.len() is cast safely via try_into, and the sum uses checked_add.
    let mut index_size = 0u32;
    for (path, _) in files {
        let path_len: u32 = path.len().try_into()
            .unwrap_or_else(|_| panic!("stdlib path too long: {}", path));
        // 2 (path_len field) + path bytes + 4 (offset) + 4 (content_len)
        let entry_size = 2u32.checked_add(path_len)
            .and_then(|s| s.checked_add(4 + 4))
            .expect("stdlib archive index entry too large");
        index_size = index_size.checked_add(entry_size)
            .expect("stdlib archive index too large");
    }
    let data_offset = 4u32.checked_add(index_size)
        .expect("stdlib archive header + index too large"); // after header + index

    // Build index and data
    let mut data_section = Vec::new();
    let mut index_section = Vec::new();

    for (path, content) in files {
        let path_bytes = path.as_bytes();
        let data_section_len: u32 = data_section.len().try_into()
            .expect("stdlib archive data section too large");
        let content_offset = data_offset.checked_add(data_section_len)
            .expect("stdlib archive content offset overflow");
        let content_len: u32 = content.len().try_into()
            .expect("stdlib file content too large");

        // Index entry
        index_section.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
        index_section.extend_from_slice(path_bytes);
        index_section.extend_from_slice(&content_offset.to_le_bytes());
        index_section.extend_from_slice(&content_len.to_le_bytes());

        // Data
        data_section.extend_from_slice(content);
    }

    archive.extend_from_slice(&index_section);
    archive.extend_from_slice(&data_section);
    archive
}
