//! The ONE entry-point rule (`VbcModule::entry_main`), both polarities.
//!
//! Before the rule existed, a `module X;` header qualified `fn main`
//! into `X.main`; the interpreter said "No main function found" while
//! the AOT link kept the runtime shim's weak `verum_main` and the
//! binary silently exited 1 — a tier-identity violation on the very
//! first program a newcomer writes with a module header.

use verum_vbc::module::{EntryMain, FunctionDescriptor, VbcModule};

fn module_with_fns(names: &[&str]) -> VbcModule {
    let mut m = VbcModule::new("probe".to_string());
    for name in names {
        let sid = m.intern_string(name);
        m.functions.push(FunctionDescriptor::new(sid));
    }
    m
}

#[test]
fn bare_main_wins_even_next_to_qualified_mains() {
    let m = module_with_fns(&["helper", "pp3.main", "main"]);
    assert_eq!(m.entry_main(), EntryMain::Unique { index: 2 });
}

#[test]
fn a_unique_qualified_main_is_the_entry() {
    let m = module_with_fns(&["helper", "pp3.main", "pp3.helper"]);
    assert_eq!(m.entry_main(), EntryMain::Unique { index: 1 });
}

#[test]
fn no_main_at_all_is_honest_none() {
    let m = module_with_fns(&["helper", "lib.compute"]);
    assert_eq!(m.entry_main(), EntryMain::None);
}

/// Ambiguity is SURFACED, never resolved by registration order.
#[test]
fn two_qualified_mains_are_ambiguous() {
    let m = module_with_fns(&["a.main", "b.main"]);
    match m.entry_main() {
        EntryMain::Ambiguous { candidates } => {
            assert_eq!(candidates, vec!["a.main".to_string(), "b.main".to_string()]);
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

/// `main` as a SUFFIX of a longer identifier is not an entry
/// (`domain` ends with "main"; `x.remain` does not even end with
/// ".main") — the rule matches the path segment, not the substring.
#[test]
fn suffix_lookalikes_are_not_entries() {
    let m = module_with_fns(&["domain", "x.remain", "chain"]);
    assert_eq!(m.entry_main(), EntryMain::None);
}
