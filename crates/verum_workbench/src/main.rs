use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use verum_workbench::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let mut config = Config {
        workspace: std::env::current_dir()?,
        verum: PathBuf::from("verum"),
        proof: PathBuf::from("verum-source-replay"),
        codex: PathBuf::from("codex"),
        port: 0,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if matches!(arg.as_str(), "--help" | "-h") {
            println!(
                "Verum Studio\nverum-studio [--workspace DIR] [--verum EXE] [--proof EXE] [--codex EXE] [--port PORT]\nOpen the printed local URL. Codex uses its own Studio ChatGPT sign-in. No API key is requested."
            );
            return Ok(());
        }
        let value = args.next().context("Option requires a value")?;
        match arg.as_str() {
            "--workspace" => config.workspace = value.into(),
            "--verum" => config.verum = value.into(),
            "--proof" => config.proof = value.into(),
            "--codex" => config.codex = value.into(),
            "--port" => config.port = value.parse()?,
            _ => bail!("Unknown option {arg}"),
        }
    }
    verum_workbench::server::serve(config).await
}
