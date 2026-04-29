//! `verum foreign-import` subcommand — parse a Coq / Lean4 / Mizar
//! / Isabelle source file and emit a Verum `.vr` skeleton with one
//! `@axiom` declaration per imported theorem, attributed back to the
//! source via `@framework(<system>, "<source>:<line>")`.

use crate::error::{CliError, Result};
use std::path::PathBuf;
use verum_verification::foreign_import::{
    importer_for, ForeignSystem, ForeignTheorem,
};

fn parse_system(s: &str) -> Result<ForeignSystem> {
    ForeignSystem::from_name(s).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "--from must be one of coq / lean4 / mizar / isabelle (or aliases rocq / mathlib / hol), got '{}'",
            s
        ))
    })
}

fn validate_format(s: &str) -> Result<()> {
    if s != "skeleton" && s != "json" && s != "summary" {
        return Err(CliError::InvalidArgument(format!(
            "--format must be 'skeleton', 'json', or 'summary', got '{}'",
            s
        )));
    }
    Ok(())
}

/// Parse a foreign source file and emit the requested output.
pub fn run_import(
    from: &str,
    file: &PathBuf,
    out: Option<&PathBuf>,
    format: &str,
) -> Result<()> {
    let system = parse_system(from)?;
    validate_format(format)?;

    let importer = importer_for(system);
    let theorems = importer.parse_file(file).map_err(|e| {
        CliError::VerificationFailed(format!(
            "import {} from {}: {}",
            system.name(),
            file.display(),
            e
        ))
    })?;

    let rendered = match format {
        "skeleton" => render_skeleton(&theorems, system, file),
        "json" => render_json(&theorems, system, file),
        "summary" => render_summary(&theorems, system, file),
        _ => unreachable!(),
    };

    match out {
        Some(path) => {
            std::fs::write(path, &rendered).map_err(|e| {
                CliError::VerificationFailed(format!(
                    "write {}: {}",
                    path.display(),
                    e
                ))
            })?;
        }
        None => print!("{}", rendered),
    }
    Ok(())
}

fn render_skeleton(
    theorems: &[ForeignTheorem],
    system: ForeignSystem,
    file: &PathBuf,
) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "// Auto-imported from {} source: {}\n",
        system.name(),
        file.display()
    ));
    s.push_str(&format!(
        "// {} declaration(s) extracted.  Proof bodies admitted as\n",
        theorems.len()
    ));
    s.push_str("// `@axiom`; replace with a Verum proof to discharge\n");
    s.push_str("// each citation.\n\n");
    for t in theorems {
        s.push_str(t.to_verum_skeleton().as_str());
    }
    s
}

fn render_json(
    theorems: &[ForeignTheorem],
    system: ForeignSystem,
    file: &PathBuf,
) -> String {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!(
        "  \"system\": \"{}\",\n",
        system.name()
    ));
    out.push_str(&format!(
        "  \"source\": \"{}\",\n",
        json_escape(&file.display().to_string())
    ));
    out.push_str(&format!("  \"count\": {},\n", theorems.len()));
    out.push_str("  \"theorems\": [\n");
    for (i, t) in theorems.iter().enumerate() {
        out.push_str(&format!(
            "    {{ \"name\": \"{}\", \"kind\": \"{}\", \"line\": {}, \"statement\": \"{}\", \"framework_citation\": \"{}\" }}{}\n",
            json_escape(t.name.as_str()),
            json_escape(match t.kind {
                verum_verification::foreign_import::ForeignTheoremKind::Theorem => "theorem",
                verum_verification::foreign_import::ForeignTheoremKind::Lemma => "lemma",
                verum_verification::foreign_import::ForeignTheoremKind::Corollary => "corollary",
                verum_verification::foreign_import::ForeignTheoremKind::Axiom => "axiom",
                verum_verification::foreign_import::ForeignTheoremKind::Definition => "def",
            }),
            t.source_line,
            json_escape(t.statement.as_str()),
            json_escape(t.framework_citation.as_str()),
            if i + 1 < theorems.len() { "," } else { "" }
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

fn render_summary(
    theorems: &[ForeignTheorem],
    system: ForeignSystem,
    file: &PathBuf,
) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Imported {} declaration(s) from {} source `{}`:\n\n",
        theorems.len(),
        system.name(),
        file.display()
    ));
    use verum_verification::foreign_import::ForeignTheoremKind;
    let mut by_kind: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for t in theorems {
        let k = match t.kind {
            ForeignTheoremKind::Theorem => "theorem",
            ForeignTheoremKind::Lemma => "lemma",
            ForeignTheoremKind::Corollary => "corollary",
            ForeignTheoremKind::Axiom => "axiom",
            ForeignTheoremKind::Definition => "def",
        };
        *by_kind.entry(k).or_insert(0) += 1;
    }
    s.push_str("  By kind:\n");
    for (k, n) in &by_kind {
        s.push_str(&format!("    {:<10} {:>4}\n", k, n));
    }
    s.push_str("\n  Names:\n");
    for t in theorems {
        s.push_str(&format!(
            "    {}:{}  {}\n",
            t.source_file.as_str(),
            t.source_line,
            t.name.as_str()
        ));
    }
    s
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp(content: &str, ext: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(&format!(".{}", ext))
            .tempfile()
            .unwrap();
        f.as_file_mut().write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    // ----- parse_system / validate_format -----

    #[test]
    fn parse_system_round_trips_canonical_names() {
        for name in ["coq", "lean4", "mizar", "isabelle"] {
            assert!(parse_system(name).is_ok());
        }
    }

    #[test]
    fn parse_system_accepts_aliases() {
        assert!(parse_system("rocq").is_ok());
        assert!(parse_system("mathlib").is_ok());
        assert!(parse_system("hol").is_ok());
    }

    #[test]
    fn parse_system_rejects_unknown() {
        assert!(matches!(
            parse_system("garbage"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn validate_format_accepts_three_forms() {
        assert!(validate_format("skeleton").is_ok());
        assert!(validate_format("json").is_ok());
        assert!(validate_format("summary").is_ok());
    }

    #[test]
    fn validate_format_rejects_unknown() {
        assert!(matches!(
            validate_format("yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    // ----- run_import skeleton output -----

    #[test]
    fn run_import_coq_emits_skeleton() {
        let src = "Theorem foo : True.\nProof. trivial. Qed.\n";
        let f = write_temp(src, "v");
        let r = run_import("coq", &f.path().to_path_buf(), None, "skeleton");
        assert!(r.is_ok());
    }

    #[test]
    fn run_import_lean_emits_skeleton() {
        let src = "theorem foo : True := trivial\n";
        let f = write_temp(src, "lean");
        let r = run_import("lean4", &f.path().to_path_buf(), None, "skeleton");
        assert!(r.is_ok());
    }

    #[test]
    fn run_import_writes_to_out_path() {
        let src = "Theorem foo : True.\nProof. trivial. Qed.\n";
        let in_file = write_temp(src, "v");
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("imported.vr");
        let r = run_import(
            "coq",
            &in_file.path().to_path_buf(),
            Some(&out_path),
            "skeleton",
        );
        assert!(r.is_ok());
        let body = std::fs::read_to_string(&out_path).unwrap();
        assert!(body.contains("@framework(coq"));
        assert!(body.contains("public theorem foo"));
        assert!(body.contains("proof by axiom"));
    }

    #[test]
    fn run_import_json_format_works() {
        let src = "theorem foo : True := trivial\n";
        let in_file = write_temp(src, "lean");
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("imported.json");
        let r = run_import(
            "lean4",
            &in_file.path().to_path_buf(),
            Some(&out_path),
            "json",
        );
        assert!(r.is_ok());
        let body = std::fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["system"], "lean4");
        assert_eq!(parsed["count"], 1);
    }

    #[test]
    fn run_import_summary_format_works() {
        let src = "Theorem foo : True.\nLemma bar : True.\n";
        let in_file = write_temp(src, "v");
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("imported.txt");
        let r = run_import(
            "coq",
            &in_file.path().to_path_buf(),
            Some(&out_path),
            "summary",
        );
        assert!(r.is_ok());
        let body = std::fs::read_to_string(&out_path).unwrap();
        assert!(body.contains("Imported 2 declaration"));
        assert!(body.contains("theorem"));
        assert!(body.contains("lemma"));
    }

    #[test]
    fn run_import_rejects_unknown_system() {
        let src = "anything\n";
        let f = write_temp(src, "txt");
        let r = run_import("garbage", &f.path().to_path_buf(), None, "skeleton");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_import_rejects_unknown_format() {
        let src = "anything\n";
        let f = write_temp(src, "v");
        let r = run_import("coq", &f.path().to_path_buf(), None, "yaml");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_import_missing_file_errors() {
        let bad_path = PathBuf::from("/nonexistent/path/to/file.v");
        let r = run_import("coq", &bad_path, None, "skeleton");
        assert!(r.is_err());
    }

    // ----- json_escape -----

    #[test]
    fn json_escape_handles_control_chars() {
        assert_eq!(json_escape("a\nb"), "a\\nb");
        assert_eq!(json_escape("a\"b"), "a\\\"b");
    }
}
