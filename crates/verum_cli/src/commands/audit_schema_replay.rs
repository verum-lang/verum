//! Isolated replay inputs. Successful elaboration is not a soundness certificate.
use std::{
    io,
    path::Path,
    process::{Command, Output},
};

const ROOT: &str = "session KernelSoundness = HOL +\n  options [document = false, timeout = 1800]\n  theories\n    KernelSoundness\n";

/// Replay exactly one generated theory in a private, explicitly named session.
/// Both inputs must remain unchanged; missing files never mean zero assumptions.
pub fn isabelle(executable: &Path, theory: &[u8]) -> io::Result<Output> {
    let directory = tempfile::Builder::new()
        .prefix("verum-isabelle-")
        .tempdir()?;
    let theory_path = directory.path().join("KernelSoundness.thy");
    let root_path = directory.path().join("ROOT");
    std::fs::write(&theory_path, theory)?;
    std::fs::write(&root_path, ROOT)?;
    let output = Command::new(executable)
        .args(["build", "-d", ".", "-v", "KernelSoundness"])
        .current_dir(directory.path())
        .output()?;
    if std::fs::read(&theory_path)? != theory || std::fs::read(&root_path)? != ROOT.as_bytes() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Isabelle replay inputs changed during execution",
        ));
    }
    Ok(output)
}
