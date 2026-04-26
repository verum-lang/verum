//! End-to-end smoke tests for `verum export --to <lean|coq|dedukti|metamath>`
//! focusing on the VVA §8.5 framework-lineage → target-library
//! mapping (lineage imports emitted in the header, unmapped lineages
//! annotated as comments, unknown lineages never breaking the export).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn verum_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../..");
    p.push("target");
    p.push("release");
    p.push("verum");
    fs::canonicalize(p).expect("target/release/verum missing; run `cargo build --release --bin verum`")
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let mut dir = std::env::temp_dir();
        dir.push(format!("verum-export-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        Self { dir }
    }

    fn write_manifest(&self) -> &Self {
        fs::write(
            self.dir.join("verum.toml"),
            "[package]\nname = \"export-lineage-test\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        self
    }

    fn write_source(&self, src: &str) -> &Self {
        fs::write(self.dir.join("src/lib.vr"), src).unwrap();
        self
    }

    fn run_export(&self, target: &str) -> String {
        let output = Command::new(verum_bin())
            .args(["export", "--to", target])
            .current_dir(&self.dir)
            .output()
            .expect("verum export invocation failed");
        assert!(
            output.status.success(),
            "verum export --to {target} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn read_export(&self, target: &str, file: &str) -> String {
        let path = self.dir.join("certificates").join(target).join(file);
        fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("could not read {}: {e}", path.display())
        })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

// -----------------------------------------------------------------------------
// Mapped lineages → known imports
// -----------------------------------------------------------------------------

#[test]
fn lean_export_emits_mathlib_imports_for_mapped_lineages() {
    let s = Scratch::new("lean_mapped");
    s.write_manifest().write_source(
        "@framework(lurie_htt, \"Lurie 2009. HTT §6.2.2.7\")\n\
         public axiom yoneda_ff<C: SmallCategory>() ensures Bool;\n\
         \n\
         @framework(schreiber_dcct, \"Schreiber 2013. DCCT §4.2\")\n\
         public axiom cohesion_shape<X: Cohesive>() ensures Bool;\n\
         \n\
         fn main() { print(\"ok\"); }\n",
    );
    s.run_export("lean");
    let lean = s.read_export("lean", "export.lean");

    // Known lineages produce `import Mathlib.*` stanzas.
    assert!(
        lean.contains("import Mathlib.CategoryTheory.Category.Basic"),
        "lurie_htt import missing:\n{lean}"
    );
    assert!(
        lean.contains("import Mathlib.CategoryTheory.Sites.Sheaf"),
        "schreiber_dcct import missing:\n{lean}"
    );
    // Imports come before the first axiom body.
    let first_axiom = lean.find("axiom ").unwrap();
    let last_import = lean.rfind("import Mathlib.").unwrap();
    assert!(
        last_import < first_axiom,
        "imports must precede declarations:\n{lean}"
    );
    // No "no mapping yet" noise because both are mapped.
    assert!(
        !lean.contains("has no Lean-library mapping yet"),
        "spurious unmapped annotation:\n{lean}"
    );
}

#[test]
fn coq_export_emits_require_for_mapped_lineages() {
    let s = Scratch::new("coq_mapped");
    s.write_manifest().write_source(
        "@framework(lurie_htt, \"Lurie 2009. HTT §6.2.2.7\")\n\
         public axiom yoneda_ff<C: SmallCategory>() ensures Bool;\n\
         \n\
         fn main() { print(\"ok\"); }\n",
    );
    s.run_export("coq");
    let coq = s.read_export("coq", "export.v");

    assert!(
        coq.contains("Require Import Category.Theory.Category."),
        "lurie_htt Coq stanza missing:\n{coq}"
    );
}

// -----------------------------------------------------------------------------
// Unmapped lineages → annotated as comments
// -----------------------------------------------------------------------------

#[test]
fn lean_export_annotates_unmapped_lineages() {
    let s = Scratch::new("lean_unmapped");
    s.write_manifest().write_source(
        "@framework(user_specific_package, \"Internal 2024. §1\")\n\
         public axiom internal_fact<T: Type>() ensures Bool;\n\
         \n\
         fn main() { print(\"ok\"); }\n",
    );
    s.run_export("lean");
    let lean = s.read_export("lean", "export.lean");

    assert!(
        lean.contains("framework lineage `user_specific_package` has no Lean-library mapping yet"),
        "unmapped annotation missing for user package:\n{lean}"
    );
    // Axiom body is still emitted — unknown lineage does not break export.
    assert!(
        lean.contains("axiom internal_fact"),
        "axiom body missing despite unmapped lineage:\n{lean}"
    );
}

// -----------------------------------------------------------------------------
// Mixed lineages → both stanzas and annotations appear
// -----------------------------------------------------------------------------

#[test]
fn lean_export_mixes_mapped_and_unmapped_lineages() {
    let s = Scratch::new("lean_mixed");
    s.write_manifest().write_source(
        "@framework(lurie_htt, \"Lurie 2009. HTT §6.2.2.7\")\n\
         public axiom mapped_one<C: SmallCategory>() ensures Bool;\n\
         \n\
         @framework(novel_extension, \"Lab Note 2026\")\n\
         public axiom novel_fact<A: Type>() ensures Bool;\n\
         \n\
         fn main() { print(\"ok\"); }\n",
    );
    s.run_export("lean");
    let lean = s.read_export("lean", "export.lean");

    // Mapped import present.
    assert!(lean.contains("import Mathlib.CategoryTheory.Category.Basic"));
    // Unmapped annotation present.
    assert!(lean.contains("framework lineage `novel_extension` has no Lean-library mapping yet"));
    // Both axioms present.
    assert!(lean.contains("axiom mapped_one"));
    assert!(lean.contains("axiom novel_fact"));
}

// -----------------------------------------------------------------------------
// Dedukti / Metamath never crash on unmapped lineage (current MVP:
// no mapping table; plain axiom scaffolds)
// -----------------------------------------------------------------------------

#[test]
fn dedukti_export_preserves_citation_in_comment() {
    let s = Scratch::new("dedukti_citation");
    s.write_manifest().write_source(
        "@framework(lurie_htt, \"Lurie 2009. HTT §6.2.2.7\")\n\
         public axiom yoneda_ff<C: SmallCategory>() ensures Bool;\n\
         \n\
         fn main() { print(\"ok\"); }\n",
    );
    s.run_export("dedukti");
    let dk = s.read_export("dedukti", "export.dk");

    // Citation present in a comment (so external reviewers see the
    // provenance even without the separate audit report).
    assert!(
        dk.contains("Lurie 2009. HTT §6.2.2.7"),
        "citation missing from Dedukti output:\n{dk}"
    );
    // Axiom statement preserved.
    assert!(dk.contains("yoneda_ff : Prop."));
}

// -----------------------------------------------------------------------------
// Determinism — same input produces byte-identical output.
// -----------------------------------------------------------------------------

#[test]
fn export_output_is_deterministic() {
    let s = Scratch::new("determinism");
    s.write_manifest().write_source(
        "@framework(lurie_htt, \"Lurie 2009. HTT §6.2.2.7\")\n\
         public axiom a<C: SmallCategory>() ensures Bool;\n\
         \n\
         @framework(schreiber_dcct, \"Schreiber 2013. DCCT §4.2\")\n\
         public axiom b<X: Cohesive>() ensures Bool;\n\
         \n\
         fn main() { print(\"ok\"); }\n",
    );
    s.run_export("lean");
    let first = s.read_export("lean", "export.lean");
    s.run_export("lean");
    let second = s.read_export("lean", "export.lean");

    assert_eq!(
        first, second,
        "export output should be byte-identical across runs"
    );
}
