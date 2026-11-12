//! Automatic stdlib archive identity shared by the build script and its controls.
//! The caller supplies core files in relative-path order; producer files use
//! sorted checkout-relative paths. Dependency reporting remains independent of baking.

use std::path::Path;

mod producer_inputs;

#[cfg(test)]
pub(crate) fn compute_archive_key<'a>(
    project_root: &Path,
    schema: &str,
    sorted_core_files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    report_dependency: impl FnMut(&Path),
) -> blake3::Hash {
    let producer = producer_hasher(project_root, schema, report_dependency);
    finish_archive_key(producer, sorted_core_files)
}

pub(crate) fn producer_hasher(
    project_root: &Path,
    schema: &str,
    mut report_dependency: impl FnMut(&Path),
) -> blake3::Hasher {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"verum-stdlib-producer");
    hash_part(&mut hasher, schema.as_bytes());
    let inputs = producer_inputs::collect(
        project_root,
        Path::new("crates/verum_stdlib_precompiler/Cargo.toml"),
    )
    .unwrap_or_else(|error| panic!("cannot identify stdlib producer inputs: {error}"));
    for relative in inputs.directories.iter().chain(inputs.files.iter()) {
        // Every reported path exists. Directories detect additions/removals
        // without a nonexistent file keeping every Cargo invocation dirty.
        report_dependency(&project_root.join(relative));
    }
    for relative in inputs.files {
        let name = relative
            .components()
            .map(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .expect("UTF-8 producer input path")
            })
            .collect::<verum_common::List<_>>()
            .join("/");
        hash_part(&mut hasher, name.as_bytes());
        let bytes = std::fs::read(project_root.join(&relative))
            .unwrap_or_else(|error| panic!("cannot read producer input {name}: {error}"));
        hash_part(&mut hasher, &bytes);
    }
    hasher
}

fn hash_part(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

pub(crate) fn finish_archive_key<'a>(
    mut hasher: blake3::Hasher,
    sorted_core_files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> blake3::Hash {
    hasher.update(b"core:");
    for (rel_path, bytes) in sorted_core_files {
        // Path-prefix the content to avoid two files swapping
        // names but keeping bytes producing the same digest.
        hasher.update(rel_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(bytes);
        hasher.update(b"\0");
    }
    hasher.finalize()
}
