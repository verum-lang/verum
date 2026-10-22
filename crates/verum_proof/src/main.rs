use std::io::{Read, Write};
fn main() -> std::io::Result<()> {
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new(verum_proof::process::WORKER_ARG))
    {
        return verum_proof::process::worker();
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(verum_proof::admission::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let report = verum_proof::process::replay(&std::env::current_exe()?, &bytes);
    serde_json::to_writer(std::io::stdout().lock(), &report)?;
    std::io::stdout().write_all(b"\n")?;
    if !report.accepted {
        std::process::exit(1);
    }
    Ok(())
}
