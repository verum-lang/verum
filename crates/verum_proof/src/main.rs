use std::io::{Read, Write};
use verum_proof::{admission::ReplayOutcome, limits::ReplayLimits, proof_checker::Foundation};

const USAGE: &str = "usage: verum-proof [--foundation function-extensionality] [--max-bytes N] [--max-nodes N] [--max-depth N] [--wall-ms N] [--cpu-seconds N] [--memory-bytes N] [--stack-bytes N] [--output-bytes N] < judgment.json";

fn options(args: &[String]) -> Result<(Foundation, ReplayLimits), String> {
    let mut foundation = Foundation::Intensional;
    let mut limits = ReplayLimits::default();
    let mut seen = std::collections::BTreeSet::new();
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if !seen.insert(flag) {
            return Err(format!("duplicate option: {flag}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if flag == "--foundation" {
            foundation = match value.as_str() {
                "intensional" => Foundation::Intensional,
                "function-extensionality" => Foundation::FunctionExtensionality,
                _ => return Err("unsupported foundation".into()),
            };
            continue;
        }
        let number = value
            .parse::<u64>()
            .map_err(|_| format!("invalid nonnegative integer for {flag}"))?;
        let size = || {
            usize::try_from(number)
                .map_err(|_| format!("value for {flag} exceeds addressable size"))
        };
        match flag.as_str() {
            "--max-bytes" => limits.admission.max_bytes = size()?,
            "--max-nodes" => limits.admission.max_nodes = size()?,
            "--max-depth" => limits.admission.max_depth = size()?,
            "--wall-ms" => limits.process.wall_time_ms = number,
            "--cpu-seconds" => limits.process.cpu_seconds = number,
            "--memory-bytes" => limits.process.memory_bytes = number,
            "--stack-bytes" => limits.process.stack_bytes = number,
            "--output-bytes" => limits.process.output_bytes = number,
            _ => return Err(format!("unknown option: {flag}")),
        }
    }
    if !limits.is_valid() {
        return Err("resource limits cannot be represented safely".into());
    }
    Ok((foundation, limits))
}

fn main() -> std::io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("{USAGE}");
        return Ok(());
    }
    if let Some(first) = args.first() {
        let foundation = match first.as_str() {
            verum_proof::process::WORKER_ARG => Some(Foundation::Intensional),
            verum_proof::process::EXTENSIONAL_WORKER_ARG => {
                Some(Foundation::FunctionExtensionality)
            }
            _ => None,
        };
        if let Some(foundation) = foundation {
            let limits = match args.as_slice() {
                [_] => ReplayLimits::default(),
                [_, flag, policy] if flag == verum_proof::process::POLICY_ARG => {
                    serde_json::from_str::<ReplayLimits>(policy).map_err(std::io::Error::other)?
                }
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "invalid worker policy arguments",
                    ));
                }
            };
            return verum_proof::process::worker_with_limits(foundation, limits);
        }
    }
    let (foundation, limits) = match options(&args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(limits.admission.read_limit().unwrap())
        .read_to_end(&mut bytes)?;
    let report = verum_proof::process::replay_with_limits(
        &std::env::current_exe()?,
        &bytes,
        foundation,
        limits,
    );
    serde_json::to_writer(std::io::stdout().lock(), &report)?;
    std::io::stdout().write_all(b"\n")?;
    let status = match report.outcome {
        ReplayOutcome::Accepted => 0,
        ReplayOutcome::Rejected => 1,
        ReplayOutcome::Unsupported => 2,
        ReplayOutcome::ResourceExhausted => 3,
        ReplayOutcome::Inconclusive => 4,
    };
    if status != 0 {
        std::process::exit(status);
    }
    Ok(())
}
