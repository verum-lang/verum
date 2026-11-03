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

pub(crate) fn file_digest(path: &Path) -> io::Result<blake3::Hash> {
    Ok(blake3::Hasher::new()
        .update_reader(fs::File::open(path)?)?
        .finalize())
}

struct Sidecars {
    metadata: blake3::Hash,
    symbols: blake3::Hash,
}

impl Sidecars {
    fn read(archive: &Path) -> io::Result<Self> {
        Ok(Self {
            metadata: file_digest(&archive.with_extension("core_metadata"))?,
            symbols: file_digest(&archive.with_extension("symbol_graph"))?,
        })
    }
}

impl Inputs<'_> {
    fn encode(&self, archive: blake3::Hash, sidecars: &Sidecars) -> String {
        format!(
            "verum-stdlib-bake-v2\nproducer={}\ntarget={}\nsource={}\nsdk={}\narchive={}\nmetadata={}\nsymbols={}\n",
            self.producer,
            self.target,
            self.source.fingerprint,
            self.source.sdk,
            archive,
            sidecars.metadata,
            sidecars.symbols,
        )
    }

    pub(crate) fn record(&self, archive: &Path, digest: blake3::Hash) -> io::Result<()> {
        fs::write(path(archive), self.encode(digest, &Sidecars::read(archive)?))
    }

    fn attested_sidecars(&self, archive: &Path, digest: blake3::Hash) -> io::Result<Sidecars> {
        let receipt = fs::read_to_string(path(archive))?;
        let mut lines = receipt.lines().rev();
        let symbols = lines
            .next()
            .and_then(|line| line.strip_prefix("symbols="))
            .and_then(|hex| blake3::Hash::from_hex(hex).ok());
        let metadata = lines
            .next()
            .and_then(|line| line.strip_prefix("metadata="))
            .and_then(|hex| blake3::Hash::from_hex(hex).ok());
        if let (Some(symbols), Some(metadata)) = (symbols, metadata) {
            let recorded = Sidecars { metadata, symbols };
            if self.encode(digest, &recorded) == receipt {
                return Ok(recorded);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "stdlib bake receipt does not match producer, target, source or archive: {}",
                archive.display()
            ),
        ))
    }

    /// Sidecar repair can start from a valid archive even if an index is missing.
    pub(crate) fn verify_archive(&self, archive: &Path, digest: blake3::Hash) -> io::Result<()> {
        self.attested_sidecars(archive, digest).map(|_| ())
    }

    /// Check the bytes in a candidate or copied bundle against one receipt.
    pub(crate) fn verify_copy(&self, archive: &Path, copy: &Path) -> io::Result<()> {
        let recorded = self.attested_sidecars(archive, file_digest(copy)?)?;
        let actual = Sidecars::read(copy)?;
        for (extension, expected, found) in [
            ("core_metadata", recorded.metadata, actual.metadata),
            ("symbol_graph", recorded.symbols, actual.symbols),
        ] {
            if expected != found {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "stdlib bake receipt expected {expected} for {}, found {found}",
                        copy.with_extension(extension).display()
                    ),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn verify_file(&self, archive: &Path) -> io::Result<()> {
        self.verify_copy(archive, archive)
    }
}
