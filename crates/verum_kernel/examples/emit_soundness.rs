//! Emit the kernel-soundness export file for one of the four
//! foundations (Lean / Coq / Isabelle / Agda) to stdout.
//!
//! Usage:
//!     cargo run -p verum_kernel --example emit_soundness -- lean
//!     cargo run -p verum_kernel --example emit_soundness -- coq
//!     cargo run -p verum_kernel --example emit_soundness -- isabelle
//!     cargo run -p verum_kernel --example emit_soundness -- agda
//!
//! Used by the manual external-prover-replay harness when verum_cli
//! itself is unbuildable (parallel-agent breakage). Mirrors the
//! production audit gate's exporter output exactly — same
//! SoundnessExporter, same backend impls.

use verum_kernel::{
    AgdaBackend, CoqBackend, IsabelleBackend, LeanBackend, SoundnessExporter,
};

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let exporter = SoundnessExporter::new();
    match arg.as_str() {
        "lean" => print!("{}", exporter.emit::<LeanBackend>(&LeanBackend::default())),
        "coq" => print!("{}", exporter.emit::<CoqBackend>(&CoqBackend::default())),
        "isabelle" => print!(
            "{}",
            exporter.emit::<IsabelleBackend>(&IsabelleBackend::default())
        ),
        "agda" => print!("{}", exporter.emit::<AgdaBackend>(&AgdaBackend::default())),
        _ => {
            eprintln!("Usage: emit_soundness {{lean | coq | isabelle | agda}}");
            std::process::exit(2);
        }
    }
}
