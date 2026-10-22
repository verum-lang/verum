use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn collect(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("identity source directory") {
        let path = entry.expect("identity source entry").path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}
fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.parent().unwrap().parent().unwrap();
    let mut files = vec![root.join("grammar/verum.ebnf")];
    for name in [
        "verum_source_proof",
        "verum_fast_parser",
        "verum_ast",
        "verum_lexer",
        "verum_common",
        "verum_diagnostics",
    ] {
        let crate_root = root.join("crates").join(name);
        collect(&crate_root.join("src"), &mut files);
        files.push(crate_root.join("Cargo.toml"));
    }
    files.push(manifest.join("build.rs"));
    files.sort();
    let mut hash = Sha256::new();
    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
        let relative = file.strip_prefix(root).unwrap().to_string_lossy();
        let bytes = fs::read(&file).expect("identity source readable");
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    println!(
        "cargo:rustc-env=VERUM_SOURCE_ELABORATOR={:x}",
        hash.finalize()
    );
}
