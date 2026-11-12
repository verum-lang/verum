//! Content identity shared by executable compilation caches.
//!
//! Executables must remain immutable while running. A version, file length or
//! timestamp cannot distinguish incompatible compilers. Missing or changing
//! executable bytes therefore cannot authorize reuse of a compiled artifact.

use std::{fs::File, io, path::Path, sync::OnceLock};

/// Digest the current executable once per process, retaining failures as well
/// as successes. Consumers choose whether an unavailable cache is fatal; they
/// must not replace a missing identity with a weaker key.
pub fn executable() -> io::Result<&'static blake3::Hash> {
    static IDENTITY: OnceLock<io::Result<blake3::Hash>> = OnceLock::new();
    IDENTITY
        .get_or_init(|| file(&std::env::current_exe()?))
        .as_ref()
        .map_err(|error| io::Error::new(error.kind(), error.to_string()))
}

/// Hash a nonempty regular tool file, refusing observed changes during reading.
/// Copying identical bytes preserves the identity regardless of timestamps.
pub fn file(path: &Path) -> io::Result<blake3::Hash> {
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "compiler is not a nonempty regular file",
        ));
    }
    let mut hash = blake3::Hasher::new();
    hash.update_reader(&mut file)?;
    let after = file.metadata()?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "compiler changed while computing its identity",
        ));
    }
    Ok(hash.finalize())
}
