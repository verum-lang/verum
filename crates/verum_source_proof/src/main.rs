use std::io::{Read, Write};
use verum_proof::{
    admission::MAX_BYTES,
    process::{OUTPUT_BYTES, install_worker_limits},
};
use verum_source_proof::{SourceReport, replay};
fn main() -> std::io::Result<()> {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--worker"] {
        eprintln!("usage: verum-source-replay --worker < source-request.json");
        std::process::exit(2);
    }
    let report = if install_worker_limits().is_err() {
        SourceReport::failure(
            "unavailable",
            "process",
            "sandbox_unavailable",
            "The source worker could not install required process limits.",
        )
    } else {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        replay(&bytes)
    };
    let mut encoded = serde_json::to_vec(&report).map_err(std::io::Error::other)?;
    if encoded.len() as u64 > OUTPUT_BYTES {
        let mut failure = SourceReport::failure(
            "inconclusive",
            "process",
            "output_limit",
            "The source report exceeds its output budget.",
        );
        failure.source = report.source;
        encoded = serde_json::to_vec(&failure).map_err(std::io::Error::other)?;
    }
    std::io::stdout().lock().write_all(&encoded)
}
