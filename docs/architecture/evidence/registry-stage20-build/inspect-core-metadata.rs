//! Read-only audit using the same CoreMetadata/bincode decoder as the CLI.
use verum_types::core_metadata::CoreMetadata;

fn main() {
    let path = std::env::args().nth(1).expect("metadata path");
    let bytes = std::fs::read(path).expect("read metadata");
    let metadata: CoreMetadata = bincode::deserialize(&bytes).expect("decode complete metadata");
    println!("types={} functions={} protocols={}", metadata.types.len(), metadata.functions.len(), metadata.protocols.len());
    for key in ["IoResult", "core.io.protocols.IoResult", "Response", "core.net.http.Response", "Result"] {
        let descriptor = metadata.types.iter().find(|(name, _)| name.as_str() == key)
            .map(|(_, descriptor)| descriptor).expect("selected descriptor");
        println!("{key}: {descriptor:?}");
    }
}
