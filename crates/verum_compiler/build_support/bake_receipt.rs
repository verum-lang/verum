//! Readable versioned identity of a completed stdlib bytecode bake.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub(crate) struct Inputs<'a> {
    pub producer: &'a str,
    pub target: &'a str,
    pub source: SourceDigests,
}

pub(crate) struct SourceDigests {
    pub fingerprint: blake3::Hash,
    pub sdk: blake3::Hash,
}

/// Read the exact sorted source set, excluding generated test programs.
pub(crate) fn source_files(root: &Path) -> io::Result<Vec<(String, Vec<u8>)>> {
    fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, Vec<u8>)>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_name() == "target" {
                continue;
            }
            if entry.file_type()?.is_dir() {
                collect(root, &path, files)?;
            } else if path.extension().is_some_and(|ext| ext == "vr") {
                let relative = path
                    .strip_prefix(root)
                    .expect("source root")
                    .to_str()
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "non-UTF8 stdlib source path")
                    })?
                    .replace('\\', "/");
                files.push((relative, fs::read(path)?));
            } else if entry.file_type()?.is_symlink() && path.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "cannot snapshot a symlinked source directory: {}",
                        path.display()
                    ),
                ));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

pub(crate) fn source_digests<'a>(
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> SourceDigests {
    let mut framed = blake3::Hasher::new();
    let mut sdk = blake3::Hasher::new();
    for (path, bytes) in files {
        for part in [path.as_bytes(), bytes] {
            framed.update(&(part.len() as u64).to_le_bytes());
            framed.update(part);
            sdk.update(part);
            sdk.update(b"\0");
        }
    }
    SourceDigests {
        fingerprint: framed.finalize(),
        sdk: sdk.finalize(),
    }
}

pub(crate) fn path(archive: &Path) -> PathBuf {
    archive.with_added_extension("receipt")
}

impl Inputs<'_> {
    pub(crate) fn encode(&self, archive: blake3::Hash) -> String {
        format!(
            "verum-stdlib-bake-v1\nproducer={}\ntarget={}\nsource={}\nsdk={}\narchive={}\n",
            self.producer, self.target, self.source.fingerprint, self.source.sdk, archive
        )
    }

    pub(crate) fn verify(&self, archive: &Path, digest: blake3::Hash) -> io::Result<()> {
        if fs::read(path(archive))? != self.encode(digest).as_bytes() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "stdlib bake receipt does not match producer, target, source or archive: {}",
                    archive.display()
                ),
            ));
        }
        Ok(())
    }

    pub(crate) fn verify_file(&self, archive: &Path) -> io::Result<()> {
        let digest = blake3::Hasher::new()
            .update_reader(fs::File::open(archive)?)?
            .finalize();
        self.verify(archive, digest)
    }
}
