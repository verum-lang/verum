//! T0881 — a project's own modules resolve through the RUN path, not
//! only through `check`.
//!
//! A two-file program reported 0 errors under `verum check` and
//! "E402: module `demo.helper` not found" under `verum run`. One source,
//! two commands, two verdicts — and the run path is the one every script
//! and every example in the documentation uses.
//!
//! Three separate causes had to be removed, each hidden by the one in
//! front of it:
//!
//!   1. the module root came from the DIRECTORY NAME rather than the cog
//!      name, so a project in a directory called `projC` registered
//!      `projC.helper` for a file declaring `module demo.helper;` —
//!      invisible for `core/`, where the two coincide;
//!   2. a `src/` directory became a module SEGMENT (`demo.src.helper`),
//!      likewise invisible for `core/`, which has no `src`;
//!   3. the entry-point search accepted a METHOD named `main`
//!      (`EnvTaskId.main` from the standard library), so a program with
//!      exactly one entry point was refused as ambiguous.
//!
//! This test builds real projects in a temp directory and checks the
//! module paths the loader derives, which is the fact all three causes
//! are about. Running them end to end belongs to the CLI suites; what is
//! pinned here is the derivation, because that is what silently produced
//! a name nobody spells.

use std::path::{Path, PathBuf};

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create project dir");
    }
    std::fs::write(path, body).expect("write project file");
}

/// Build a project whose directory name deliberately DIFFERS from its
/// cog name, and return its root.
fn scaffold(dir_name: &str, cog_name: &str, use_src: bool) -> PathBuf {
    let root = std::env::temp_dir()
        .join("verum-multi-file-project-test")
        .join(dir_name);
    let _ = std::fs::remove_dir_all(&root);

    write(
        &root.join("verum.toml"),
        &format!("[cog]\nname = \"{cog_name}\"\nversion = \"0.1.0\"\nedition = \"2026\"\n"),
    );
    // The loader treats a directory as a project only when a `mod.vr`
    // sits beside the manifest.
    write(&root.join("mod.vr"), "module demo;\n");

    let src = if use_src { root.join("src") } else { root.clone() };
    write(
        &src.join("helper.vr"),
        "module demo.helper;\npublic fn answer() -> Int { 42 }\n",
    );
    write(
        &src.join("main.vr"),
        "module demo.main;\nmount demo.helper.{answer};\nfn main() { print(f\"answer={answer()}\"); }\n",
    );
    root
}

/// The module paths the loader derives for a project, as the pipeline
/// would register them.
fn derived_module_paths(root: &Path, use_src: bool) -> Vec<String> {
    use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

    let entry = if use_src {
        root.join("src").join("main.vr")
    } else {
        root.join("main.vr")
    };
    let options = CompilerOptions {
        input: entry,
        output: std::env::temp_dir().join("verum-multi-file-project-out"),
        ..Default::default()
    };
    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);
    pipeline
        .load_project_modules_for_testing()
        .expect("project load should not error");
    let mut paths = pipeline.loaded_project_module_paths_for_testing();
    paths.sort();
    paths
}

#[test]
fn the_module_root_is_the_cog_name_not_the_directory_name() {
    // Directory `checkout-dir-name` vs cog `demo`: the declarations in
    // the files say `demo.*`, and that is what must be registered.
    let root = scaffold("checkout-dir-name", "demo", false);
    let paths = derived_module_paths(&root, false);
    assert!(
        paths.iter().any(|p| p == "demo.helper"),
        "expected `demo.helper` among the registered paths, got {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.starts_with("checkout-dir-name")),
        "the checkout directory name must not appear in a module path: {paths:?}"
    );
}

#[test]
fn a_hyphenated_cog_name_becomes_an_identifier() {
    // Manifests carry hyphens; module paths are dotted identifiers.
    let root = scaffold("hyphen-dir", "verum-registry", false);
    let paths = derived_module_paths(&root, false);
    assert!(
        paths.iter().any(|p| p == "verum_registry.helper"),
        "expected `verum_registry.helper`, got {paths:?}"
    );
}

#[test]
fn src_is_a_source_root_not_a_module_segment() {
    let root = scaffold("src-layout", "demo", true);
    let paths = derived_module_paths(&root, true);
    assert!(
        paths.iter().any(|p| p == "demo.helper"),
        "expected `demo.helper`, got {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.contains(".src.")),
        "`src` is the source root and must not become a module segment: {paths:?}"
    );
}

/// The negative pole. Without it, a derivation that answered
/// `demo.helper` for everything would pass every assertion above.
#[test]
fn a_nested_directory_is_still_a_module_segment() {
    let root = scaffold("nested-layout", "demo", true);
    write(
        &root.join("src").join("protocol").join("authority.vr"),
        "module demo.protocol.authority;\npublic fn who() -> Int { 1 }\n",
    );
    let paths = derived_module_paths(&root, true);
    assert!(
        paths.iter().any(|p| p == "demo.protocol.authority"),
        "a real subdirectory MUST contribute its segment: {paths:?}"
    );
}
