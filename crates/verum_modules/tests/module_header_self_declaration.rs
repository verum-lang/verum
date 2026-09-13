//! `module X;` at the head of a file — self-header or child promise (A175).
//!
//! `validate_module_headers_against_filesystem` compares module
//! declarations against the files next to them, so every verdict it
//! gives is relative to `file_path.parent()`. Two ways that went wrong,
//! both measured on one three-file cog (`verum.toml` naming it
//! `minicog`, `src/mod.vr`, `src/thing.vr`, `src/main.vr`) which RAN and
//! printed the right answer while emitting four warnings:
//!
//!   * three of them named the "file" as `minicog` — not a path at all.
//!     Three of the five call sites iterate a `(path, source)` map whose
//!     key can be a logical module name; `parent_dir` is then empty and
//!     every candidate is looked up in the process CWD, so `thing` and
//!     `main` were reported missing while `src/thing.vr` and
//!     `src/main.vr` sat on disk.
//!
//!   * the fourth was the cog root's own header. `core/mod.vr` writes
//!     `module core;` in silence because its directory happens to be
//!     named `core`; a cog whose root is `src/mod.vr` gets no such luck.
//!
//! The discriminator for the second is VISIBILITY, and it is measured:
//! across all of `core/` there is exactly ONE simple-name module
//! declaration without visibility — `core/mod.vr:75` — while all 2307
//! others are `public module <name>;` and every one is a child promise.

use std::fs;

use tempfile::TempDir;
use verum_ast::FileId;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_modules::loader::validate_module_headers_against_filesystem;

fn parse(source: &str) -> verum_ast::Module {
    let lexer = Lexer::new(source, FileId::dummy());
    VerumParser::new()
        .parse_module(lexer, FileId::dummy())
        .expect("test source must parse")
}

/// A validator that compares an AST against the filesystem can say
/// nothing about a file that is not on the filesystem.
#[test]
fn a_path_that_is_not_a_file_yields_no_verdict() {
    let ast = parse("module minicog;\n\npublic module thing;\npublic module main;\n");

    let warnings =
        validate_module_headers_against_filesystem(std::path::Path::new("minicog"), &ast);

    assert!(
        warnings.is_empty(),
        "a logical module name is not a path; every candidate would be \
         resolved against the process CWD, so the only honest answer is \
         none — got {} warning(s)",
        warnings.len()
    );
}

/// The cog root's own header, in a directory that is not named after the
/// cog. This is the shape every user cog has.
#[test]
fn a_cog_root_naming_its_own_cog_is_silent() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let root = src.join("mod.vr");
    fs::write(&root, "module minicog;\n\npublic module thing;\n").unwrap();
    fs::write(src.join("thing.vr"), "module minicog.thing;\n").unwrap();

    let ast = parse("module minicog;\n\npublic module thing;\n");
    let warnings = validate_module_headers_against_filesystem(&root, &ast);

    assert!(
        warnings.is_empty(),
        "`module minicog;` in src/mod.vr is the cog root's own header, the \
         same role `module core;` plays in core/mod.vr — got: {:?}",
        warnings.iter().map(|w| w.message()).collect::<Vec<_>>()
    );
}

/// `<stem>.vr` declaring `module <stem>;` is that file's short-form
/// header, exactly as `<dir>/mod.vr` declaring `module <dir>;` is.
///
/// This one was silent BEFORE the fix as well, but for a reason that is
/// an accident rather than a rule: the first candidate the check builds
/// is `<parent>/thing.vr`, which is the file itself, so `any_exists`
/// happened to be true. The exemption states the intent instead of
/// resting on that coincidence — and the coincidence does not survive a
/// file loaded from a different directory than the one it names.
#[test]
fn a_file_naming_itself_is_silent() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("thing.vr");
    fs::write(&file, "module thing;\n").unwrap();

    let ast = parse("module thing;\n");
    let warnings = validate_module_headers_against_filesystem(&file, &ast);

    assert!(
        warnings.is_empty(),
        "no `thing/thing.vr` is expected to exist — got: {:?}",
        warnings.iter().map(|w| w.message()).collect::<Vec<_>>()
    );
}

/// The other polarity, and the reason the exemptions are narrow: a
/// PUBLIC child promise with no source behind it is still a real
/// dangling declaration, and must still be reported.
#[test]
fn a_public_child_promise_with_no_source_is_still_reported() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let root = src.join("mod.vr");
    fs::write(&root, "module minicog;\n\npublic module missing;\n").unwrap();

    let ast = parse("module minicog;\n\npublic module missing;\n");
    let warnings = validate_module_headers_against_filesystem(&root, &ast);

    assert_eq!(
        warnings.len(),
        1,
        "the cog header is exempt, the dangling public child is not — got: {:?}",
        warnings.iter().map(|w| w.message()).collect::<Vec<_>>()
    );
    assert!(
        warnings[0].message().contains("missing"),
        "the surviving warning must name the dangling child: {}",
        warnings[0].message()
    );
}

/// And a child promise whose source DOES exist stays silent, which is
/// the property the whole check exists to express.
#[test]
fn a_public_child_promise_with_a_source_is_silent() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let root = src.join("mod.vr");
    fs::write(&root, "module minicog;\n\npublic module thing;\n").unwrap();
    fs::write(src.join("thing.vr"), "module minicog.thing;\n").unwrap();

    let ast = parse("module minicog;\n\npublic module thing;\n");
    let warnings = validate_module_headers_against_filesystem(&root, &ast);

    assert!(
        warnings.is_empty(),
        "src/thing.vr is right there — got: {:?}",
        warnings.iter().map(|w| w.message()).collect::<Vec<_>>()
    );
}
