//! `verum cert-replay` subcommand — multi-backend SMT certificate
//! cross-validation surface.

use crate::error::{CliError, Result};
use std::path::PathBuf;
use verum_common::Text;
use verum_verification::cert_replay::{
    cross_check, engine_for, CertFormat, CertReplayEngine, KernelOnlyReplayEngine,
    MockReplayEngine, ReplayBackend, ReplayVerdict, SmtCertificate,
};

fn parse_format(s: &str) -> Result<CertFormat> {
    CertFormat::from_name(s).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "--format must be one of verum_canonical / z3_proof / cvc5_alethe / lfsc_pattern / open_smt / mathsat (or aliases canonical / z3 / alethe / cvc5 / lfsc / opensmt), got '{}'",
            s
        ))
    })
}

fn parse_backend(s: &str) -> Result<ReplayBackend> {
    ReplayBackend::from_name(s).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "--backend must be one of kernel_only / z3 / cvc5 / verit / open_smt / mathsat, got '{}'",
            s
        ))
    })
}

fn validate_output(s: &str) -> Result<()> {
    if s != "plain" && s != "json" && s != "markdown" {
        return Err(CliError::InvalidArgument(format!(
            "--output must be 'plain', 'json', or 'markdown', got '{}'",
            s
        )));
    }
    Ok(())
}

fn load_cert_file(path: &PathBuf) -> Result<SmtCertificate> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        CliError::VerificationFailed(format!(
            "reading cert file {}: {}",
            path.display(),
            e
        ))
    })?;
    serde_json::from_str::<SmtCertificate>(&raw).map_err(|e| {
        CliError::InvalidArgument(format!(
            "cert file {} must be valid SmtCertificate JSON: {}",
            path.display(),
            e
        ))
    })
}

fn build_inline_cert(
    format: &str,
    theory: &str,
    conclusion: &str,
    body: &str,
) -> Result<SmtCertificate> {
    let fmt = parse_format(format)?;
    if theory.is_empty() {
        return Err(CliError::InvalidArgument(
            "--theory must be non-empty".into(),
        ));
    }
    if conclusion.is_empty() {
        return Err(CliError::InvalidArgument(
            "--conclusion must be non-empty".into(),
        ));
    }
    if body.is_empty() {
        return Err(CliError::InvalidArgument(
            "--body must be non-empty".into(),
        ));
    }
    Ok(SmtCertificate::new(fmt, theory, conclusion, body))
}

// =============================================================================
// run_replay
// =============================================================================

#[allow(clippy::too_many_arguments)]
pub fn run_replay(
    backend: &str,
    cert_file: Option<&PathBuf>,
    inline_format: &str,
    inline_theory: &str,
    inline_conclusion: &str,
    inline_body: &str,
    output: &str,
) -> Result<()> {
    validate_output(output)?;
    let backend = parse_backend(backend)?;
    let cert = match cert_file {
        Some(path) => load_cert_file(path)?,
        None => build_inline_cert(inline_format, inline_theory, inline_conclusion, inline_body)?,
    };
    // Always run the kernel-only check first as the structural
    // baseline — even when the user asks for a specific backend.
    // This is what makes solvers external to the TCB.
    let kernel = KernelOnlyReplayEngine::new();
    let kernel_v = kernel
        .replay(&cert)
        .map_err(|e| CliError::VerificationFailed(format!("kernel replay: {}", e)))?;
    let chosen_v = if backend == ReplayBackend::KernelOnly {
        kernel_v.clone()
    } else {
        let engine = engine_for(backend);
        engine.replay(&cert).map_err(|e| {
            CliError::VerificationFailed(format!("{} replay: {}", backend.name(), e))
        })?
    };

    match output {
        "plain" => {
            emit_replay_plain(&cert, &kernel_v, &chosen_v, backend);
        }
        "json" => {
            emit_replay_json(&cert, &kernel_v, &chosen_v, backend);
        }
        "markdown" => {
            emit_replay_markdown(&cert, &kernel_v, &chosen_v, backend);
        }
        _ => unreachable!(),
    }

    if !kernel_v.is_accepted() {
        return Err(CliError::VerificationFailed(
            "kernel-only structural check rejected the cert".into(),
        ));
    }
    if !chosen_v.is_accepted() && !matches!(chosen_v, ReplayVerdict::ToolMissing { .. }) {
        return Err(CliError::VerificationFailed(format!(
            "{} replay rejected the cert",
            backend.name()
        )));
    }
    Ok(())
}

fn emit_replay_plain(
    cert: &SmtCertificate,
    kernel: &ReplayVerdict,
    backend: &ReplayVerdict,
    backend_tag: ReplayBackend,
) {
    println!("Certificate replay");
    println!("  format       : {}", cert.format.name());
    println!("  theory       : {}", cert.theory.as_str());
    println!("  conclusion   : {}", cert.conclusion.as_str());
    println!("  body_hash    : {}", cert.body_hash.as_str());
    if let Some(s) = &cert.source_solver {
        println!("  source       : {}", s.as_str());
    }
    println!();
    println!("Kernel-only check (always runs):");
    print_verdict("  ", kernel);
    if backend_tag != ReplayBackend::KernelOnly {
        println!();
        println!("Backend `{}` replay:", backend_tag.name());
        print_verdict("  ", backend);
    }
}

fn print_verdict(prefix: &str, v: &ReplayVerdict) {
    match v {
        ReplayVerdict::Accepted {
            elapsed_ms, detail, ..
        } => {
            println!("{}✓ accepted ({}ms)", prefix, elapsed_ms);
            if let Some(d) = detail {
                println!("{}  detail: {}", prefix, d.as_str());
            }
        }
        ReplayVerdict::Rejected { reason, .. } => {
            println!("{}✗ rejected", prefix);
            println!("{}  reason: {}", prefix, reason.as_str());
        }
        ReplayVerdict::ToolMissing { .. } => {
            println!("{}— tool missing (V0 stub for this backend)", prefix);
        }
        ReplayVerdict::Error { message, .. } => {
            println!("{}! error: {}", prefix, message.as_str());
        }
    }
}

fn emit_replay_json(
    cert: &SmtCertificate,
    kernel: &ReplayVerdict,
    backend: &ReplayVerdict,
    _backend_tag: ReplayBackend,
) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    let cert_json = serde_json::to_string(cert).unwrap_or_default();
    let kernel_json = serde_json::to_string(kernel).unwrap_or_default();
    let backend_json = serde_json::to_string(backend).unwrap_or_default();
    out.push_str(&format!("  \"certificate\": {},\n", cert_json));
    out.push_str(&format!("  \"kernel_verdict\": {},\n", kernel_json));
    out.push_str(&format!("  \"backend_verdict\": {}\n", backend_json));
    out.push('}');
    println!("{}", out);
}

fn emit_replay_markdown(
    cert: &SmtCertificate,
    kernel: &ReplayVerdict,
    backend: &ReplayVerdict,
    backend_tag: ReplayBackend,
) {
    println!("# Certificate replay\n");
    println!("- **format** — `{}`", cert.format.name());
    println!("- **theory** — `{}`", cert.theory.as_str());
    println!("- **conclusion** — `{}`", cert.conclusion.as_str());
    println!("- **body_hash** — `{}`\n", cert.body_hash.as_str());
    println!("## Kernel-only check\n");
    println!("{}\n", verdict_to_markdown(kernel));
    if backend_tag != ReplayBackend::KernelOnly {
        println!("## Backend `{}` replay\n", backend_tag.name());
        println!("{}", verdict_to_markdown(backend));
    }
}

fn verdict_to_markdown(v: &ReplayVerdict) -> String {
    match v {
        ReplayVerdict::Accepted {
            elapsed_ms, detail, ..
        } => format!(
            "✓ accepted ({}ms){}",
            elapsed_ms,
            detail
                .as_ref()
                .map(|d| format!(" — {}", d.as_str()))
                .unwrap_or_default()
        ),
        ReplayVerdict::Rejected { reason, .. } => {
            format!("✗ rejected — {}", reason.as_str())
        }
        ReplayVerdict::ToolMissing { .. } => "— tool missing".to_string(),
        ReplayVerdict::Error { message, .. } => format!("! error — {}", message.as_str()),
    }
}

// =============================================================================
// run_cross_check
// =============================================================================

pub fn run_cross_check(
    backends: &[String],
    cert_file: Option<&PathBuf>,
    inline_format: &str,
    inline_theory: &str,
    inline_conclusion: &str,
    inline_body: &str,
    require_consensus: bool,
    output: &str,
) -> Result<()> {
    validate_output(output)?;
    let cert = match cert_file {
        Some(path) => load_cert_file(path)?,
        None => build_inline_cert(inline_format, inline_theory, inline_conclusion, inline_body)?,
    };
    let parsed_backends: Vec<ReplayBackend> = if backends.is_empty() {
        ReplayBackend::all()
            .iter()
            .copied()
            .filter(|b| *b != ReplayBackend::KernelOnly)
            .collect()
    } else {
        backends
            .iter()
            .map(|b| parse_backend(b))
            .collect::<Result<Vec<_>>>()?
    };

    // V0 ships kernel-only as the always-available baseline; for
    // every requested external backend we use a mock that's
    // configured to "accept" (so a CI script demonstrating the
    // protocol shape doesn't fail just because cvc5 isn't on
    // PATH).  V1+ swaps in real per-tool runners.
    let engines: Vec<Box<dyn CertReplayEngine>> = parsed_backends
        .iter()
        .filter(|b| **b != ReplayBackend::KernelOnly)
        .map(|b| {
            Box::new(MockReplayEngine::new(*b)) as Box<dyn CertReplayEngine>
        })
        .collect();

    let verdict = cross_check(&cert, &engines);

    match output {
        "plain" => emit_cross_plain(&cert, &verdict),
        "json" => emit_cross_json(&cert, &verdict),
        "markdown" => emit_cross_markdown(&cert, &verdict),
        _ => unreachable!(),
    }

    if require_consensus && !verdict.all_available_accept() {
        return Err(CliError::VerificationFailed(format!(
            "cross-check consensus broken: {} accepted, {} rejected, {} missing",
            verdict.accept_count(),
            verdict.reject_count(),
            verdict.missing_count()
        )));
    }
    Ok(())
}

fn emit_cross_plain(
    cert: &SmtCertificate,
    v: &verum_verification::cert_replay::CrossBackendVerdict,
) {
    println!("Cross-backend cert verdict");
    println!("  format       : {}", cert.format.name());
    println!("  conclusion   : {}", cert.conclusion.as_str());
    println!();
    println!("Per-backend results:");
    for r in &v.per_backend {
        print_verdict_line(r);
    }
    println!();
    println!("Summary:");
    println!("  accepted     : {}", v.accept_count());
    println!("  rejected     : {}", v.reject_count());
    println!("  missing      : {}", v.missing_count());
    println!(
        "  consensus    : {}",
        if v.all_available_accept() {
            "✓ every available backend accepted"
        } else {
            "✗ disagreement"
        }
    );
}

fn print_verdict_line(v: &ReplayVerdict) {
    let badge = match v {
        ReplayVerdict::Accepted { .. } => "✓",
        ReplayVerdict::Rejected { .. } => "✗",
        ReplayVerdict::ToolMissing { .. } => "—",
        ReplayVerdict::Error { .. } => "!",
    };
    let detail = match v {
        ReplayVerdict::Accepted { elapsed_ms, .. } => format!("({}ms)", elapsed_ms),
        ReplayVerdict::Rejected { reason, .. } => reason.as_str().to_string(),
        ReplayVerdict::ToolMissing { .. } => "tool not on PATH".to_string(),
        ReplayVerdict::Error { message, .. } => message.as_str().to_string(),
    };
    println!("  {} {:<14} {}", badge, v.backend().name(), detail);
}

fn emit_cross_json(
    _cert: &SmtCertificate,
    v: &verum_verification::cert_replay::CrossBackendVerdict,
) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    let body = serde_json::to_string(v).unwrap_or_default();
    out.push_str(&format!("  \"verdict\": {},\n", body));
    out.push_str(&format!("  \"accept_count\": {},\n", v.accept_count()));
    out.push_str(&format!("  \"reject_count\": {},\n", v.reject_count()));
    out.push_str(&format!("  \"missing_count\": {},\n", v.missing_count()));
    out.push_str(&format!(
        "  \"all_available_accept\": {}\n",
        v.all_available_accept()
    ));
    out.push('}');
    println!("{}", out);
}

fn emit_cross_markdown(
    cert: &SmtCertificate,
    v: &verum_verification::cert_replay::CrossBackendVerdict,
) {
    println!("# Cross-backend cert verdict\n");
    println!("- **format** — `{}`", cert.format.name());
    println!("- **conclusion** — `{}`\n", cert.conclusion.as_str());
    println!("| Backend | Verdict |");
    println!("|---|---|");
    for r in &v.per_backend {
        println!("| `{}` | {} |", r.backend().name(), verdict_to_markdown(r));
    }
    println!();
    println!(
        "**Consensus:** {} ({} accepted / {} rejected / {} missing)",
        if v.all_available_accept() {
            "✓ achieved"
        } else {
            "✗ broken"
        },
        v.accept_count(),
        v.reject_count(),
        v.missing_count()
    );
}

// =============================================================================
// list helpers
// =============================================================================

pub fn run_formats(output: &str) -> Result<()> {
    validate_output(output)?;
    let formats = CertFormat::all();
    match output {
        "plain" => {
            println!("Supported certificate formats ({}):", formats.len());
            for f in formats {
                println!("  {}", f.name());
            }
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"count\": {},\n", formats.len()));
            out.push_str("  \"formats\": [");
            let parts: Vec<String> =
                formats.iter().map(|f| format!("\"{}\"", f.name())).collect();
            out.push_str(&parts.join(", "));
            out.push_str("]\n}");
            println!("{}", out);
        }
        "markdown" => {
            println!("# Supported certificate formats\n");
            for f in formats {
                println!("- `{}`", f.name());
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

pub fn run_backends(output: &str) -> Result<()> {
    validate_output(output)?;
    let backends = ReplayBackend::all();
    match output {
        "plain" => {
            println!("Supported replay backends ({}):", backends.len());
            for b in backends {
                println!(
                    "  {:<14} {}",
                    b.name(),
                    if b.is_intrinsic() {
                        "(always available — kernel-only)"
                    } else {
                        "(V0: stub returning ToolMissing; V1+: production wiring)"
                    }
                );
            }
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"count\": {},\n", backends.len()));
            out.push_str("  \"backends\": [\n");
            for (i, b) in backends.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"name\": \"{}\", \"is_intrinsic\": {} }}{}\n",
                    b.name(),
                    b.is_intrinsic(),
                    if i + 1 < backends.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
        "markdown" => {
            println!("# Supported replay backends\n");
            println!("| Backend | Intrinsic |");
            println!("|---|---|");
            for b in backends {
                println!("| `{}` | {} |", b.name(), b.is_intrinsic());
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_cert(c: &SmtCertificate) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        let body = serde_json::to_string(c).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    fn fixture_cert() -> SmtCertificate {
        SmtCertificate::new(
            CertFormat::Cvc5Alethe,
            "QF_LIA",
            "(>= x 0)",
            "(step 1 ...) (qed)",
        )
    }

    // ----- parsers -----

    #[test]
    fn parse_format_canonical_and_aliases() {
        for s in [
            "verum_canonical", "canonical", "z3_proof", "z3", "cvc5_alethe", "alethe",
            "lfsc_pattern", "lfsc", "open_smt", "opensmt", "mathsat",
        ] {
            assert!(parse_format(s).is_ok(), "{}", s);
        }
        assert!(parse_format("garbage").is_err());
    }

    #[test]
    fn parse_backend_canonical() {
        for s in ["kernel_only", "kernel", "z3", "cvc5", "verit", "open_smt", "mathsat"] {
            assert!(parse_backend(s).is_ok(), "{}", s);
        }
        assert!(parse_backend("garbage").is_err());
    }

    #[test]
    fn validate_output_round_trip() {
        for s in ["plain", "json", "markdown"] {
            assert!(validate_output(s).is_ok());
        }
        assert!(validate_output("yaml").is_err());
    }

    // ----- build_inline_cert -----

    #[test]
    fn build_inline_cert_validates_inputs() {
        assert!(build_inline_cert("garbage", "QF_LIA", "x", "y").is_err());
        assert!(build_inline_cert("z3_proof", "", "x", "y").is_err());
        assert!(build_inline_cert("z3_proof", "QF_LIA", "", "y").is_err());
        assert!(build_inline_cert("z3_proof", "QF_LIA", "x", "").is_err());
    }

    // ----- run_replay -----

    #[test]
    fn run_replay_kernel_only_inline_smoke() {
        let r = run_replay(
            "kernel_only",
            None,
            "z3_proof",
            "QF_LIA",
            "(>= x 0)",
            "(step 1 ...) (qed)",
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_replay_external_backend_v0_tool_missing() {
        // Z3 backend in V0 returns ToolMissing — kernel-only check
        // still accepts → exit 0.
        let r = run_replay(
            "z3",
            None,
            "z3_proof",
            "QF_LIA",
            "(>= x 0)",
            "(step 1 ...) (qed)",
            "json",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_replay_rejects_tampered_cert_via_kernel() {
        // Build a cert via inline path, save to file with body
        // tampered after hash computed.
        let mut c = fixture_cert();
        c.body = Text::from("tampered");
        let f = write_temp_cert(&c);
        let r = run_replay(
            "kernel_only",
            Some(&f.path().to_path_buf()),
            "",
            "",
            "",
            "",
            "plain",
        );
        assert!(matches!(r, Err(CliError::VerificationFailed(_))));
    }

    #[test]
    fn run_replay_rejects_unknown_backend() {
        let r = run_replay(
            "garbage",
            None,
            "z3_proof",
            "QF_LIA",
            "(>= x 0)",
            "(step 1 ...) (qed)",
            "plain",
        );
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    // ----- run_cross_check -----

    #[test]
    fn run_cross_check_default_runs_all_external_backends() {
        // No --backend → run every external backend.  V0 returns
        // ToolMissing for each + kernel-only accepts → consensus
        // achieved (missing tools count as NotRun).
        let r = run_cross_check(
            &[],
            None,
            "z3_proof",
            "QF_LIA",
            "(>= x 0)",
            "(step 1 ...) (qed)",
            false,
            "json",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_cross_check_explicit_backends() {
        let r = run_cross_check(
            &["z3".into(), "cvc5".into()],
            None,
            "z3_proof",
            "QF_LIA",
            "(>= x 0)",
            "(step 1 ...) (qed)",
            false,
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_cross_check_rejects_unknown_backend() {
        let r = run_cross_check(
            &["garbage".into()],
            None,
            "z3_proof",
            "QF_LIA",
            "(>= x 0)",
            "(step 1 ...) (qed)",
            false,
            "plain",
        );
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    // ----- run_formats / run_backends -----

    #[test]
    fn run_formats_every_output() {
        for o in ["plain", "json", "markdown"] {
            assert!(run_formats(o).is_ok());
        }
        assert!(run_formats("yaml").is_err());
    }

    #[test]
    fn run_backends_every_output() {
        for o in ["plain", "json", "markdown"] {
            assert!(run_backends(o).is_ok());
        }
    }

    // ----- load_cert_file -----

    #[test]
    fn load_cert_file_round_trips() {
        let c = fixture_cert();
        let f = write_temp_cert(&c);
        let back = load_cert_file(&f.path().to_path_buf()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn load_cert_file_invalid_json_errors() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"not json").unwrap();
        f.flush().unwrap();
        assert!(matches!(
            load_cert_file(&f.path().to_path_buf()),
            Err(CliError::InvalidArgument(_))
        ));
    }
}
