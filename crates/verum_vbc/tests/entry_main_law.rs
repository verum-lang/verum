//! The ONE entry-point rule (`VbcModule::entry_main`), both polarities.
//!
//! Before the rule existed, a `module X;` header qualified `fn main`
//! into `X.main`; the interpreter said "No main function found" while
//! the AOT link kept the runtime shim's weak `verum_main` and the
//! binary silently exited 1 — a tier-identity violation on the very
//! first program a newcomer writes with a module header.

use verum_vbc::module::{EntryMain, FunctionDescriptor, ParamDescriptor, VbcModule};
use verum_vbc::types::{TypeId, TypeRef};

fn module_with_fns(names: &[&str]) -> VbcModule {
    let mut m = VbcModule::new("probe".to_string());
    for name in names {
        let sid = m.intern_string(name);
        m.functions.push(FunctionDescriptor::new(sid));
    }
    m
}

/// Same, but every listed function is given `arity` parameters — the
/// shape that tells a program's entry point apart from a foreign-ABI
/// shim that happens to be called `main`.
fn module_with_arities(entries: &[(&str, usize)]) -> VbcModule {
    let mut m = VbcModule::new("probe".to_string());
    for (name, arity) in entries {
        let sid = m.intern_string(name);
        let mut desc = FunctionDescriptor::new(sid);
        for i in 0..*arity {
            let pname = m.intern_string(&format!("p{i}"));
            desc.params.push(ParamDescriptor {
                name: pname,
                type_ref: TypeRef::Concrete(TypeId::I64),
                is_mut: false,
                default: None,
                type_name: pname,
            });
        }
        m.functions.push(desc);
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

// ---------------------------------------------------------------------
// A `main` that takes arguments is not this program's entry
// ---------------------------------------------------------------------
//
// The runtime calls the entry with no arguments, so a `main` that
// REQUIRES parameters could not be invoked as one even if it were
// chosen. The standard library ships exactly such a function: the
// platform's C-ABI process entry, `darwin_entry.main(argc, argv)`,
// which dyld calls and which forwards to the user's program.

#[test]
fn a_parameterised_main_is_not_a_candidate() {
    // The shape that made every `module X;` program ambiguous:
    // the platform shim against the program's own entry.
    let m = module_with_arities(&[("darwin_entry.main", 2), ("app.main", 0)]);
    assert_eq!(m.entry_main(), EntryMain::Unique { index: 1 });
}

#[test]
fn a_parameterised_bare_main_is_not_the_entry_either() {
    let m = module_with_arities(&[("main", 2), ("app.main", 0)]);
    assert_eq!(
        m.entry_main(),
        EntryMain::Unique { index: 1 },
        "a bare `main` taking arguments cannot be called as the entry, \
         so it must not outrank one that can"
    );
}

/// The negative pole. Without it, a rule that rejected EVERY `main`
/// would satisfy both assertions above by finding nothing at all.
#[test]
fn a_zero_argument_main_is_still_the_entry() {
    let m = module_with_arities(&[("helper", 3), ("app.main", 0)]);
    assert_eq!(m.entry_main(), EntryMain::Unique { index: 1 });
}

/// And ambiguity between two REAL entries is still surfaced — the
/// arity rule narrows the candidate set, it does not resolve ties.
#[test]
fn two_zero_argument_qualified_mains_are_still_ambiguous() {
    let m = module_with_arities(&[("a.main", 0), ("b.main", 0), ("shim.main", 2)]);
    match m.entry_main() {
        EntryMain::Ambiguous { candidates } => {
            assert_eq!(candidates, vec!["a.main".to_string(), "b.main".to_string()]);
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}
