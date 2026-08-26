//! T0882 — a mount naming a module of THIS unit is not resolved by
//! guessing.
//!
//! A two-file project whose own `resolve` takes two references bound
//! `core.encoding.json_pointer.resolve` instead — the standard library's
//! own two-reference `resolve` — and died inside it on
//!
//! ```text
//! method 'Map.keys' not found on receiver of runtime kind `<nil>`
//! ```
//!
//! an error naming nothing the author wrote. A DIFFERENT arity was fine,
//! which is what kept this looking niche until a real program hit it.
//!
//! The cause was ordinary and the remedy is about ordering, not about
//! precedence. Files are collected one at a time, so a mount in the entry
//! file runs BEFORE the sibling's declarations exist; every qualified
//! probe misses for that reason, and the ladder's bare-name last resort
//! then takes the slot whichever library function won the first-wins
//! race. The unit now records its own module paths before walking any of
//! them, so "not registered YET" is told apart from "not in this unit at
//! all": the first waits for the deferred pass, the second keeps the
//! fallback and its warning.
//!
//! What this test pins is the DERIVED FACT — that the unit knows its own
//! module paths — plus the end-to-end call. The rendering of such a call
//! inside a format literal is a separate, still-open defect in the type
//! checker (it types a project call as the stdlib namesake's return type,
//! so `f"{n}"` prints `None` while `n + 0` is 7); this file deliberately
//! reads the value arithmetically rather than through interpolation, so
//! it measures the binding and not the residual.

use std::path::{Path, PathBuf};
use std::process::Command;

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create project dir");
    }
    std::fs::write(path, body).expect("write project file");
}

/// A project whose own function shares a name AND an arity with a
/// standard-library function.
fn scaffold() -> PathBuf {
    let root = std::env::temp_dir().join("verum-mounted-sibling-namesake");
    let _ = std::fs::remove_dir_all(&root);

    write(
        &root.join("verum.toml"),
        "[cog]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2026\"\n",
    );
    write(&root.join("mod.vr"), "module demo;\n");
    write(
        &root.join("src").join("lib.vr"),
        // Same NAME and same ARITY as `core.encoding.json_pointer.resolve`,
        // which also takes two references.
        "module demo.lib;\n\
         public type Holder is { v: Int };\n\
         public type Key is (Text);\n\
         public fn resolve(h: &List<Holder>, k: &Key) -> Int {\n\
         \x20   let mut n = 0;\n\
         \x20   for x in h.iter() { n = n + x.v; }\n\
         \x20   n\n\
         }\n",
    );
    write(
        &root.join("src").join("main.vr"),
        "module demo.main;\n\
         mount demo.lib.{Holder, Key, resolve};\n\
         fn main() {\n\
         \x20   let mut hs: List<Holder> = List.new();\n\
         \x20   hs.push(Holder { v: 7 });\n\
         \x20   let k = Key(\"x\");\n\
         \x20   let n = resolve(&hs, &k);\n\
         \x20   print(f\"answer={n + 0}\");\n\
         }\n",
    );
    root
}

/// The `verum` binary for this target directory.
///
/// Searched by walking UP from the test binary rather than by a fixed
/// number of `pop`s: the first version of this helper assumed
/// `<target>/<profile>/deps/<test>` and silently found nothing, so the
/// test reported `ok` in 0.00s without running the program at all — a
/// pass that measured the absence of its own instrument.
fn verum_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    loop {
        let candidate = dir.join("verum");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

#[test]
fn a_mounted_sibling_wins_its_name_against_a_stdlib_namesake() {
    // Absent, this panics rather than returning: a skip that prints
    // `ok` is indistinguishable from a check that ran.
    let verum = verum_binary().expect(
        "no `verum` binary in any ancestor of this test binary — build one \
         with `cargo build -p verum_cli --release` before running this gate",
    );
    let root = scaffold();

    let out = Command::new(&verum)
        .arg("run")
        .current_dir(&root)
        .output()
        .expect("run the project");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stdout.contains("answer=7"),
        "the project's own `resolve` must answer, not a stdlib namesake\n\
         --- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    assert!(
        !stderr.contains("Map.keys"),
        "binding a stdlib namesake surfaced as a panic inside ITS body\n\
         --- stderr ---\n{stderr}"
    );
}
