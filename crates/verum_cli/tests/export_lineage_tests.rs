//! Source-faithful foreign certificate admission (T1773).
//!
//! The old E2E cases required successful export of untyped `Prop` declarations
//! with `sorry`/`Admitted`. Those outputs did not preserve the source statement.
//! Their lineage-rendering coverage remains in commands/export.rs unit tests;
//! this boundary now requires explicit unsupported failure and no file mutation.
use std::{fs, path::PathBuf, process::Command};
fn verum_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}
struct Scratch {
    dir: PathBuf,
}
impl Scratch {
    fn new(name: &str, source: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("verum-export-{name}-{}", std::process::id()));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("verum.toml"),
            "[package]\nname = \"export-proof-test\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(dir.join("src/lib.vr"), source).unwrap();
        Self { dir }
    }
    fn denied(&self, target: &str) -> String {
        let output = Command::new(verum_bin())
            .args(["export", "--to", target])
            .current_dir(&self.dir)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "a lossy renderer must not issue a certificate"
        );
        let diagnostic = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            diagnostic.contains("exact source statement"),
            "{diagnostic}"
        );
        assert!(
            !self.dir.join("certificates").exists(),
            "failure created certificate artifacts"
        );
        diagnostic
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
#[test]
fn mapped_lineage_cannot_authorize_an_erased_lean_statement() {
    let s = Scratch::new(
        "lean-mapped",
        "@framework(lurie_htt, \"Lurie 2009\") public axiom a<C:SmallCategory>() ensures Bool;",
    );
    s.denied("lean");
}
#[test]
fn mapped_lineage_cannot_authorize_an_admitted_coq_proof() {
    let s = Scratch::new(
        "coq-mapped",
        "@framework(lurie_htt, \"Lurie 2009\") public axiom a<C:SmallCategory>() ensures Bool;",
    );
    s.denied("coq");
}
#[test]
fn unmapped_lineage_cannot_authorize_a_certificate() {
    let s = Scratch::new(
        "unmapped",
        "@framework(unregistered, \"citation\") public axiom a() ensures Bool;",
    );
    s.denied("lean");
}
#[test]
fn mixing_lineages_does_not_change_proof_admission() {
    let s = Scratch::new(
        "mixed",
        "@framework(lurie_htt, \"Lurie 2009\") public axiom a() ensures Bool; @framework(unregistered, \"citation\") public axiom b() ensures Bool;",
    );
    s.denied("lean");
}
#[test]
fn all_foreign_proof_targets_fail_before_creating_files() {
    let s = Scratch::new("targets", "theorem id(a:Type,x:a):a proof=x;");
    for target in ["lean", "coq", "dedukti", "agda", "metamath"] {
        s.denied(target);
    }
}
#[test]
fn repeated_failure_preserves_user_selected_existing_output() {
    let s = Scratch::new("output", "theorem id(a:Type,x:a):a proof=x;");
    let output = s.dir.join("previous.lean");
    fs::write(&output, "pre-existing reviewed result").unwrap();
    for _ in 0..2 {
        let result = Command::new(verum_bin())
            .args(["export", "--to", "lean", "--output"])
            .arg(&output)
            .current_dir(&s.dir)
            .output()
            .unwrap();
        assert!(!result.status.success());
        let diagnostic = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            diagnostic.contains("exact source statement"),
            "{diagnostic}"
        );
        assert_eq!(
            fs::read_to_string(&output).unwrap(),
            "pre-existing reviewed result"
        );
        assert!(!s.dir.join("previous.lean.provenance.json").exists());
    }
}
