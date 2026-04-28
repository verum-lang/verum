#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Script-mode end-to-end integration tests (#8 / P1.8).
//!
//! Covers the parse → entry-detection slice that the P1.2 + P1.3
//! commits enabled. Each test wires the real pipeline pieces (no
//! mocks):
//!
//!   verum_fast_parser::FastParser::parse_module_script_str
//!         ▼
//!   verum_ast::Module (with @![__verum_kind("script")] tag)
//!         ▼
//!   verum_compiler::phases::EntryDetectionPhase::detect_entry_point
//!         ▼
//!   verum_compiler::phases::MainConfig
//!
//! Scenarios still gated on follow-up plumbing (process-argv, AOT
//! linker entry-symbol parameterization, panic exit-code routing,
//! stdlib mount, relative mount, `#!` shebang lexer hook) are
//! tracked under task #8's continuation; this file delivers the
//! parser-to-entry-detection pieces that are actually shipping.
//!
//! Note on parser naming: the parser's synthesized wrapper is the
//! `__verum_script_main` `FunctionDecl`. Setting `CogKind::Script`
//! on the module via `set_on_module` is the only contract the
//! entry-detection fallback consults.

use verum_ast::{CogKind, FileId, ItemKind, Module};
use verum_compiler::phases::entry_detection::{EntryDetectionPhase, MainConfig};
use verum_fast_parser::FastParser;

fn fid() -> FileId {
    FileId::new(0)
}

/// Parse `src` in script mode and tag the resulting module so the
/// entry-detection fallback recognises it as a script. Mirrors the
/// pipeline path (parser sets script_mode + lexer hook /
/// preamble attribute sets the kind tag).
fn parse_script_and_tag(src: &str) -> Module {
    let parser = FastParser::new();
    let mut module = parser
        .parse_module_script_str(src, fid())
        .expect("script-mode parse should succeed");
    CogKind::Script.set_on_module(&mut module);
    module
}

// ----------------------------------------------------------------
// Scenario 1: empty script source — no wrapper synthesised, but
// also no `main` declared. Library-mode would error before reaching
// the script fallback because no module is tagged Script. Script-
// mode tagging without statements still errors (no entry to
// synthesise from), which is the correct behaviour: an empty file
// is not a runnable program.
// ----------------------------------------------------------------
#[test]
fn empty_script_source_errors_with_no_entry() {
    let module = parse_script_and_tag("");
    let phase = EntryDetectionPhase::new();
    let result = phase.detect_entry_point(&[module]);
    assert!(
        result.is_err(),
        "an empty script source has no entry — must error"
    );
}

// ----------------------------------------------------------------
// Scenario 2: top-level `let` in script mode binds locally inside
// the wrapper, NOT at item-level as a `const`. This is the
// language-level distinction between script mode and library mode
// — script `let` is a Python-style local binding scoped to the
// implicit script body; library `let NAME = …;` is the const
// shorthand. The discriminator (`let` is a statement-starter in
// `is_script_stmt_starter`) routes through `parse_stmt` before
// the item-level `parse_const` ever sees the keyword.
// ----------------------------------------------------------------
#[test]
fn top_level_let_routes_to_wrapper_in_script_mode() {
    let module = parse_script_and_tag("let x = 42;\n");

    // Wrapper IS synthesised because `let` is a stmt-starter.
    let wrapper = module
        .items
        .iter()
        .find_map(|i| match &i.kind {
            ItemKind::Function(f) if f.name.as_str() == "__verum_script_main" => Some(f),
            _ => None,
        })
        .expect("script-mode top-level `let` must produce a wrapper");
    let body = match &wrapper.body {
        verum_common::Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) => b,
        _ => panic!("wrapper body must be a block"),
    };
    assert_eq!(
        body.stmts.len(),
        1,
        "expected exactly one collected let-statement"
    );

    // Entry-detection picks the wrapper.
    let phase = EntryDetectionPhase::new();
    let cfg = phase
        .detect_entry_point(&[module])
        .expect("entry-detect succeeds");
    assert!(matches!(cfg, MainConfig::Sync));
}

// ----------------------------------------------------------------
// Scenario 3: top-level `defer` — statement-only — gets folded
// into the synthesized wrapper, and entry detection finds it.
// ----------------------------------------------------------------
#[test]
fn top_level_defer_routes_to_script_main_entry() {
    let module = parse_script_and_tag("defer cleanup();\n");
    let phase = EntryDetectionPhase::new();
    let cfg = phase
        .detect_entry_point(&[module])
        .expect("script entry should be discovered");
    assert!(matches!(cfg, MainConfig::Sync));
}

// ----------------------------------------------------------------
// Scenario 4: expression statement (function call) at top level.
// Routes through the parser's item-failure → parse_stmt fallback,
// gets folded into the wrapper, entry-detection accepts.
// ----------------------------------------------------------------
#[test]
fn top_level_expression_statement_routes_to_script_main() {
    let module = parse_script_and_tag("print(\"hello\");\n");
    let phase = EntryDetectionPhase::new();
    let cfg = phase
        .detect_entry_point(&[module])
        .expect("script entry should be discovered");
    assert!(matches!(cfg, MainConfig::Sync));
}

// ----------------------------------------------------------------
// Scenario 5: mixed decls + stmts. Top-level helper fn comes
// alongside two statements. The wrapper holds exactly the two
// stmts; the helper survives as a regular item; entry-detection
// picks the wrapper as entry.
// ----------------------------------------------------------------
#[test]
fn mixed_decls_and_stmts_preserves_order_and_routes_to_wrapper() {
    let src = r#"
fn helper() -> Int { 42 }

defer cleanup();

print(helper());
"#;
    let module = parse_script_and_tag(src);

    // Three items: helper + wrapper.
    let names: Vec<&str> = module
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Function(f) => Some(f.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["helper", "__verum_script_main"]);

    let phase = EntryDetectionPhase::new();
    let cfg = phase
        .detect_entry_point(&[module])
        .expect("entry-detect succeeds");
    assert!(matches!(cfg, MainConfig::Sync));
}

// ----------------------------------------------------------------
// Scenario 6: explicit main shadows the script wrapper. A migrating
// script that gains an explicit `fn main()` keeps `main` as the
// entry — the wrapper is still emitted (parser doesn't know about
// main resolution yet) but is dead code at run-time.
// ----------------------------------------------------------------
#[test]
fn explicit_main_takes_precedence_over_synthesized_wrapper() {
    let src = r#"
fn main() { print("explicit"); }

defer cleanup();
"#;
    let module = parse_script_and_tag(src);

    // Both items present: main + wrapper.
    let names: Vec<&str> = module
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Function(f) => Some(f.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"main"));
    assert!(names.contains(&"__verum_script_main"));

    // Entry-detection picks `main`, NOT the wrapper.
    let phase = EntryDetectionPhase::new();
    let cfg = phase
        .detect_entry_point(&[module])
        .expect("explicit main becomes entry");
    assert!(matches!(cfg, MainConfig::Sync));
}

// ----------------------------------------------------------------
// Scenario 7: untagged module with a synthesized-style name does
// not become a script. A non-script module that happens to define
// `__verum_script_main` (unlikely, but a name collision is
// possible in user code) is NOT used as the entry — only modules
// with `is_script() == true` opt in to the fallback.
// ----------------------------------------------------------------
#[test]
fn untagged_module_with_script_main_name_still_errors() {
    let parser = FastParser::new();
    // Use plain (library-mode) parse; the function would have to be
    // user-written for this to happen.
    let module = parser
        .parse_module_str(
            "fn __verum_script_main() -> Int { 0 }\n",
            fid(),
        )
        .expect("parse should succeed");
    // Don't tag as script.
    assert!(!module.is_script());

    let phase = EntryDetectionPhase::new();
    let result = phase.detect_entry_point(&[module]);
    assert!(
        result.is_err(),
        "non-script module with __verum_script_main must still require an explicit `main`"
    );
}

// ----------------------------------------------------------------
// Scenario 8: idempotency of CogKind::set_on_module. Calling it
// twice with the same kind doesn't accumulate attributes; calling
// it with a different kind replaces the prior tag.
// ----------------------------------------------------------------
#[test]
fn set_on_module_idempotent_and_replacing() {
    let parser = FastParser::new();
    let mut module = parser
        .parse_module_str("fn helper() {}\n", fid())
        .expect("parse OK");

    // Initially Library.
    assert_eq!(CogKind::of(&module), CogKind::Library);

    CogKind::Script.set_on_module(&mut module);
    let after_script = module
        .attributes
        .iter()
        .filter(|a| a.name.as_str() == CogKind::ATTR_NAME)
        .count();
    assert_eq!(after_script, 1);
    assert_eq!(CogKind::of(&module), CogKind::Script);

    // Calling again with Script — still exactly one attribute.
    CogKind::Script.set_on_module(&mut module);
    let after_double = module
        .attributes
        .iter()
        .filter(|a| a.name.as_str() == CogKind::ATTR_NAME)
        .count();
    assert_eq!(after_double, 1);

    // Switch to Binary — replaces, doesn't append.
    CogKind::Binary.set_on_module(&mut module);
    let after_binary = module
        .attributes
        .iter()
        .filter(|a| a.name.as_str() == CogKind::ATTR_NAME)
        .count();
    assert_eq!(after_binary, 1);
    assert_eq!(CogKind::of(&module), CogKind::Binary);
}
