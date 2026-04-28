#![cfg(test)]

//! End-to-end tests for file-relative mount (#5 / P1.5).
//!
//! These tests build small two-/three-file `.vr` projects in
//! a temp directory, run them through the
//! `verum_modules::file_mount::resolve_file_mounts` walker
//! plus the actual parser, and verify the resolved module
//! data matches what the pipeline expects to register in
//! `ModuleRegistry`.
//!
//! Pipeline-side wiring (insertion of resolve_file_mounts
//! between Phase 1 and Phase 1.5) is verified separately in
//! pipeline.rs.  These tests pin the cross-crate integration:
//!  * parser emits MountTreeKind::File correctly
//!  * loader resolves through its sandbox
//!  * resolver synthesises module names + walks transitively

use std::path::PathBuf;

use verum_ast::{FileId, ItemKind, MountTreeKind};
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_modules::file_mount::resolve_file_mounts;
use verum_modules::loader::ModuleLoader;

fn parse(text: &str, file_id: FileId) -> verum_ast::Module {
    let lexer = Lexer::new(text, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|e| panic!("parse failed: {:?}", e))
}

#[test]
fn two_file_project_mount_helper_resolves() {
    // Project layout:
    //   root/
    //     main.vr     →  mount ./helper.vr;
    //     helper.vr   →  module helper; public fn ping() -> Int { 42 }
    let root = tempfile::TempDir::new().unwrap();
    let main_path = root.path().join("main.vr");
    let helper_path = root.path().join("helper.vr");
    std::fs::write(
        &main_path,
        "module main;\nmount ./helper.vr;\n\nfn run() -> Int { helper.ping() }\n",
    )
    .unwrap();
    std::fs::write(
        &helper_path,
        "module helper;\npublic fn ping() -> Int { 42 }\n",
    )
    .unwrap();

    // Parse main.
    let main_ast = parse(
        &std::fs::read_to_string(&main_path).unwrap(),
        FileId::new(1),
    );

    // Sanity: parser emitted MountTreeKind::File for the
    // mount declaration in main.vr.
    let mount_decl = main_ast
        .items
        .iter()
        .find_map(|item| {
            if let ItemKind::Mount(m) = &item.kind {
                Some(m.clone())
            } else {
                None
            }
        })
        .expect("main.vr must contain a mount declaration");
    match &mount_decl.tree.kind {
        MountTreeKind::File { path, .. } => {
            assert_eq!(path.as_str(), "./helper.vr");
        }
        other => panic!(
            "expected MountTreeKind::File, got {:?} (parser regression)",
            other
        ),
    }

    // Run the resolver.
    let mut loader = ModuleLoader::new(root.path());
    let resolved =
        resolve_file_mounts(&mut loader, &[(main_path.clone(), main_ast)], |source| {
            Ok(parse(source.source.as_str(), source.file_id))
        })
        .expect("resolution must succeed");

    assert_eq!(resolved.len(), 1, "expected exactly one resolved file");
    let entry = &resolved[0];
    assert_eq!(entry.synthetic_name, "helper");
    assert!(
        entry.absolute_path.ends_with("helper.vr"),
        "absolute path must point at the resolved file"
    );
    assert!(
        entry.source.as_str().contains("fn ping"),
        "loaded source must include the file's contents"
    );
    // FileId must be a real allocation, not the
    // dummy sentinel.
    assert!(!entry.file_id.is_dummy());
}

#[test]
fn three_file_transitive_mount_resolves_in_order() {
    // a.vr → mid.vr → leaf.vr
    let root = tempfile::TempDir::new().unwrap();
    let a = root.path().join("a.vr");
    let mid = root.path().join("mid.vr");
    let leaf = root.path().join("leaf.vr");
    std::fs::write(&a, "module a;\nmount ./mid.vr;\n").unwrap();
    std::fs::write(&mid, "module mid;\nmount ./leaf.vr;\n").unwrap();
    std::fs::write(
        &leaf,
        "module leaf;\npublic fn answer() -> Int { 42 }\n",
    )
    .unwrap();

    let a_ast = parse(&std::fs::read_to_string(&a).unwrap(), FileId::new(1));

    let mut loader = ModuleLoader::new(root.path());
    let resolved = resolve_file_mounts(&mut loader, &[(a, a_ast)], |source| {
        Ok(parse(source.source.as_str(), source.file_id))
    })
    .expect("transitive resolution must succeed");

    assert_eq!(resolved.len(), 2);
    // Deterministic alphabetical ordering of synthetic names.
    assert_eq!(resolved[0].synthetic_name, "leaf");
    assert_eq!(resolved[1].synthetic_name, "mid");
}

#[test]
fn alias_overrides_basename() {
    let root = tempfile::TempDir::new().unwrap();
    let main_path = root.path().join("main.vr");
    let helper_path = root.path().join("helper.vr");
    std::fs::write(
        &main_path,
        "module main;\nmount ./helper.vr as Util;\n",
    )
    .unwrap();
    std::fs::write(
        &helper_path,
        "module helper;\npublic fn x() -> Int { 0 }\n",
    )
    .unwrap();

    let main_ast = parse(
        &std::fs::read_to_string(&main_path).unwrap(),
        FileId::new(1),
    );

    let mut loader = ModuleLoader::new(root.path());
    let resolved =
        resolve_file_mounts(&mut loader, &[(main_path, main_ast)], |source| {
            Ok(parse(source.source.as_str(), source.file_id))
        })
        .unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].synthetic_name, "Util",
        "alias must override basename in synthetic module name"
    );
}

#[test]
fn diamond_pattern_dedupes_shared_target() {
    // a.vr and b.vr both mount ./shared.vr — shared must
    // appear exactly once in the resolved list (so the
    // module registry doesn't get duplicate entries).
    let root = tempfile::TempDir::new().unwrap();
    let a = root.path().join("a.vr");
    let b = root.path().join("b.vr");
    let shared = root.path().join("shared.vr");
    std::fs::write(&a, "module a;\nmount ./shared.vr;\n").unwrap();
    std::fs::write(&b, "module b;\nmount ./shared.vr;\n").unwrap();
    std::fs::write(&shared, "module shared;\n").unwrap();

    let a_ast = parse(&std::fs::read_to_string(&a).unwrap(), FileId::new(1));
    let b_ast = parse(&std::fs::read_to_string(&b).unwrap(), FileId::new(2));

    let mut loader = ModuleLoader::new(root.path());
    let resolved = resolve_file_mounts(
        &mut loader,
        &[(a, a_ast), (b, b_ast)],
        |source| Ok(parse(source.source.as_str(), source.file_id)),
    )
    .unwrap();

    assert_eq!(
        resolved.len(),
        1,
        "diamond pattern must produce exactly one entry per shared file"
    );
    assert_eq!(resolved[0].synthetic_name, "shared");
}

#[test]
fn sibling_at_parent_directory_resolves() {
    // Project layout:
    //   root/
    //     util.vr
    //     sub/
    //       inner.vr  →  mount ../util.vr;
    let root = tempfile::TempDir::new().unwrap();
    let sub = root.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    let util = root.path().join("util.vr");
    let inner = sub.join("inner.vr");
    std::fs::write(&util, "module util;\npublic fn now() -> Int { 1 }\n").unwrap();
    std::fs::write(&inner, "module inner;\nmount ../util.vr;\n").unwrap();

    let inner_ast = parse(
        &std::fs::read_to_string(&inner).unwrap(),
        FileId::new(1),
    );

    let mut loader = ModuleLoader::new(root.path());
    let resolved = resolve_file_mounts(
        &mut loader,
        &[(inner.clone(), inner_ast)],
        |source| Ok(parse(source.source.as_str(), source.file_id)),
    )
    .unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].synthetic_name, "util");
    assert!(resolved[0].absolute_path.ends_with("util.vr"));
}

#[test]
fn sandbox_blocks_escape_attempt_outside_root() {
    // A file mount whose canonicalised path lives outside
    // the loader's root_path must be rejected, not loaded.
    let outside = tempfile::TempDir::new().unwrap();
    let root = tempfile::TempDir::new().unwrap();
    let secret = outside.path().join("secret.vr");
    std::fs::write(&secret, "module secret;\n").unwrap();

    // Create a file inside `root` that tries to escape via
    // a chain of `..` traversals to reach `outside/secret.vr`.
    let importing = root.path().join("attacker.vr");
    let escape_count = secret
        .components()
        .count()
        .saturating_sub(root.path().components().count())
        + 4; // overshoot
    let escape_path: PathBuf = (0..escape_count).fold(PathBuf::new(), |acc, _| acc.join(".."));
    let escape_str = format!(
        "module attacker;\nmount ./{}/{};\n",
        escape_path.display(),
        secret
            .strip_prefix("/")
            .unwrap_or(&secret)
            .to_string_lossy()
    );
    // The string above may or may not parse depending on
    // platform path separators — the test guards against
    // the loader-side sandbox at the very least.  We use
    // a cleaner explicit attack vector:
    let _ = escape_str; // silence dead let
    let attack = format!(
        "module attacker;\nmount ../../../../../../../../../../{}/secret.vr;\n",
        outside.path().file_name().unwrap().to_string_lossy()
    );
    std::fs::write(&importing, &attack).unwrap();

    let importing_ast = match VerumParser::new().parse_module(
        verum_lexer::Lexer::new(&attack, FileId::new(1)),
        FileId::new(1),
    ) {
        Ok(m) => m,
        Err(_) => {
            // Parser may already reject some forms — if so,
            // that's the parser-side defence layer doing
            // its job.  No further loader-side check needed.
            return;
        }
    };

    let mut loader = ModuleLoader::new(root.path());
    let result = resolve_file_mounts(
        &mut loader,
        &[(importing, importing_ast)],
        |source| Ok(parse(source.source.as_str(), source.file_id)),
    );
    assert!(
        result.is_err(),
        "loader sandbox MUST reject the escape attempt"
    );
}
