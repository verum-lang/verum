//! T0892 — a context declared in one module is usable from another.
//!
//! It was none of mountable, providable, or callable. Two gaps, and the
//! first hid the second:
//!
//!   * the project loader's export table was a COPY of the canonical
//!     export walk with the visibility check removed, and the copy had
//!     drifted — it knew Function, Type, Protocol, Const and Static and
//!     swallowed everything else in a `_ => Ok(())`. An explicit
//!     `context` declaration was in that everything-else, so
//!     `mount demo.lib.{Clock}` reported `cannot find 'Clock'`;
//!
//!   * with the mount resolving, `provide Clock = backend;` still
//!     reported `undefined context: Clock`, because the import
//!     registered the context with the RESOLVER and never with the
//!     context CHECKER — and `provide` consults the checker.
//!
//! Together they meant dependency injection stopped at the module
//! boundary, which is the one place it is needed: a context declared
//! where it is consumed is a context nobody needed to inject.
//!
//! THE NAMES MATTER. An earlier version of this measurement used
//! `Clock`, which `core/context/standard.vr` also declares — so it
//! passed against the STANDARD LIBRARY's context while the program's own
//! was still invisible, and reported a fix that had not happened. Every
//! name here is one the standard library does not use.
//!
//! Not a vcs spec: those are single-file, and the only way to have two
//! modules in one file is an inline `module { … }` block, which drops a
//! context entirely (T0893). The two-file project is the shape this
//! defect is about anyway.

use std::path::{Path, PathBuf};
use std::process::Command;

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create project dir");
    }
    std::fs::write(path, body).expect("write project file");
}

fn scaffold() -> PathBuf {
    let root = std::env::temp_dir().join("verum-mounted-context");
    let _ = std::fs::remove_dir_all(&root);

    write(
        &root.join("verum.toml"),
        "[cog]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2026\"\n",
    );
    write(&root.join("mod.vr"), "module demo;\n");
    write(
        &root.join("src").join("lib.vr"),
        // `Alpha` is one method with no parameters; `Beta` has two and a
        // non-primitive return, so the shape under test is not the
        // narrowest one that could work.
        "module demo.lib;\n\
         public context Alpha {\n\
         \x20   fn a() -> Int;\n\
         }\n\
         public context Beta {\n\
         \x20   fn b() -> Text;\n\
         \x20   fn c() -> Int;\n\
         }\n\
         public type AlphaImpl is { x: Int };\n\
         implement AlphaImpl {\n\
         \x20   public fn a(&self) -> Int { self.x }\n\
         }\n\
         public type BetaImpl is { s: Text, n: Int };\n\
         implement BetaImpl {\n\
         \x20   public fn b(&self) -> Text { self.s.clone() }\n\
         \x20   public fn c(&self) -> Int { self.n }\n\
         }\n",
    );
    write(
        &root.join("src").join("main.vr"),
        "module demo.main;\n\
         mount demo.lib.{Alpha, AlphaImpl, Beta, BetaImpl};\n\
         fn one() -> Int using [Alpha] { Alpha.a() }\n\
         fn two() -> Text using [Beta] { Beta.b() }\n\
         fn both() -> Int using [Alpha, Beta] { Alpha.a() + Beta.c() }\n\
         fn main() {\n\
         \x20   provide Alpha = AlphaImpl { x: 4 };\n\
         \x20   provide Beta = BetaImpl { s: \"beta\", n: 5 };\n\
         \x20   print(f\"alpha={one()} beta={two()} both={both()}\");\n\
         }\n",
    );
    root
}

/// The `verum` binary for this target directory, found by walking UP —
/// absent, this panics rather than skipping, because a skip that prints
/// `ok` is not a measurement.
fn verum_binary() -> PathBuf {
    let exe = std::env::current_exe().expect("current exe");
    let mut dir = exe.parent();
    while let Some(d) = dir {
        let candidate = d.join("verum");
        if candidate.is_file() {
            return candidate;
        }
        dir = d.parent();
    }
    panic!(
        "no `verum` binary in any ancestor of this test binary — build one \
         with `cargo build -p verum_cli --release` before running this gate"
    )
}

#[test]
fn a_context_from_another_module_is_mountable_providable_and_callable() {
    let verum = verum_binary();
    let root = scaffold();

    let out = Command::new(&verum)
        .arg("run")
        .current_dir(&root)
        .output()
        .expect("run the project");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // `both=9` is the load-bearing one: a single function depending on
    // TWO mounted contexts, which fails if only one per module survives
    // the boundary.
    assert!(
        stdout.contains("alpha=4 beta=beta both=9"),
        "a mounted context must be usable: expected `alpha=4 beta=beta both=9`\n\
         --- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    assert!(
        !stderr.contains("undefined context"),
        "the context must reach the context CHECKER, not only the resolver\n\
         --- stderr ---\n{stderr}"
    );
    assert!(
        !stderr.contains("E401"),
        "the context must be in the module's export table\n--- stderr ---\n{stderr}"
    );
}
