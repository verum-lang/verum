use std::io::{Read, Write};
fn main() -> std::io::Result<()> {
    use verum_proof::proof_checker::Foundation;
    let args: Vec<_> = std::env::args().skip(1).collect();
    let foundation = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [verum_proof::process::WORKER_ARG] => return verum_proof::process::worker(),
        [verum_proof::process::EXTENSIONAL_WORKER_ARG] => {
            return verum_proof::process::worker_with_foundation(Foundation::FunctionExtensionality);
        }
        [] => Foundation::Intensional,
        ["--foundation", "function-extensionality"] => Foundation::FunctionExtensionality,
        _ => {
            eprintln!("usage: verum-proof [--foundation function-extensionality] < judgment.json");
            std::process::exit(2);
        }
    };
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(verum_proof::admission::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let report =
        verum_proof::process::replay_with_foundation(&std::env::current_exe()?, &bytes, foundation);
    serde_json::to_writer(std::io::stdout().lock(), &report)?;
    std::io::stdout().write_all(b"\n")?;
    if !report.accepted {
        std::process::exit(1);
    }
    Ok(())
}
