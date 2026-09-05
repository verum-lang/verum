#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! A64 — a `T.default()` in one method leaves a binding that a LATER `*self`
//! in the same `implement` block resolves against.
//!
//! Reduced to four lines outside the stdlib, 2026-09-06:
//!
//! ```text
//!   type M<T> is None | Some(T);
//!   implement<T> M<T> {
//!       public fn d(self) -> T where T: Default { T.default() }
//!       public fn take(&mut self) -> M<T> { let old = *self; *self = None; old }
//!   }
//! ```
//!
//! gives `error<E404>: Ambiguous type for 'old': the inferred type
//! 'None(Unit) or Some(&&_)' is not fully determined` — twice, and both in
//! `take`, which never mentions `Default`. Two things in that string are the
//! finding: the payload is `&&_`, a DOUBLE reference where `*self` on a
//! `&mut M<T>` should give `M<T>`, and it is unresolved.
//!
//! THREE DISCRIMINATORS, each one variable:
//!
//! * ORDER decides. `T.default()` before `take` fails; `take` before
//!   `T.default()` is clean.
//! * THE PROTOCOL decides. `T.zero()` and `T.one()` in the identical position
//!   are both clean. Only `Default` does it.
//! * THE MATCH IS IRRELEVANT. A bare `T.default()` with no `match` and no
//!   `self` breaks it just as well; a `match self` that never calls `default`
//!   does not.
//!
//! WHY THIS MATTERS BEYOND THE ROW: `core/base/maybe.vr`'s
//! `unwrap_or_default` still carries `None => 0` — a literal that is wrong
//! for every `T` but `Int` — and the comment there says it "stays a literal".
//! It is not a cosmetic workaround: removing it takes that file from 0 errors
//! to 2, and both are in `take`/`replace`, which call neither. The literal is
//! what PINS the variable.
//!
//! THESE TESTS ARE HONEST-RED-ADJACENT: they assert the CURRENT behaviour so
//! that a fix flips them loudly, and they name what the right answer is.
//! `default_before_take_is_ambiguous_today` is the one to delete when the
//! leak is fixed.

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(td) = &item.kind {
            let _ = checker.register_type_declaration(td);
        }
    }
    let mut errs: Vec<String> = module
        .items
        .iter()
        .filter_map(|item| checker.check_item(item).err().map(|e| format!("{:?}", e)))
        .collect();
    errs.extend(checker.diagnostics().iter().map(|d| format!("{:?}", d)));
    errs.retain(|e| e.contains("E404") || e.contains("Ambiguous"));
    errs
}

const DECL: &str = "type M<T> is None | Some(T);\n\n";
const TAKE: &str =
    "    public fn take(&mut self) -> M<T> {\n        let old = *self;\n        *self = None;\n        old\n    }\n";

fn impl_block(first: &str, second: &str) -> String {
    format!("{DECL}implement<T> M<T> {{\n{first}\n{second}}}\n")
}

fn call_method(call: &str, bound: &str) -> String {
    format!("    public fn d(self) -> T\n    where T: {bound} {{\n        {call}\n    }}\n")
}

/// ORDER. The same two methods, swapped.
#[test]
fn order_decides_whether_the_binding_leaks() {
    let before = impl_block(&call_method("T.default()", "Default"), TAKE);
    let after = impl_block(TAKE, &call_method("T.default()", "Default"));
    assert!(
        !errors(&before).is_empty(),
        "documented behaviour: `T.default()` BEFORE `take` leaves `*self` ambiguous. \
         If this now passes, the leak is fixed — delete this test and the \
         `None => 0` literal in core/base/maybe.vr"
    );
    assert!(
        errors(&after).is_empty(),
        "`take` before `T.default()` must stay clean: {:?}",
        errors(&after)
    );
}

/// THE PROTOCOL. `Zero` and `One` in the identical position do not leak.
#[test]
fn only_default_leaks_not_zero_or_one() {
    for (call, bound) in [("T.zero()", "Zero"), ("T.one()", "One")] {
        let src = impl_block(&call_method(call, bound), TAKE);
        assert!(
            errors(&src).is_empty(),
            "`{call}` before `take` must be clean — only `Default` leaks: {:?}",
            errors(&src)
        );
    }
}

/// THE MATCH IS IRRELEVANT. A `match self` that never calls `default` is
/// clean, so the scrutinee is not what does it.
#[test]
fn a_match_on_self_without_default_does_not_leak() {
    let first = "    public fn d(self) -> Maybe<T> {\n        match self {\n            \
                 Some(v) => Maybe.Some(v),\n            None => Maybe.None,\n        }\n    }\n";
    let src = impl_block(first, TAKE);
    assert!(
        errors(&src).is_empty(),
        "a `match self` with no `default` call must be clean: {:?}",
        errors(&src)
    );
}

/// `take` on its own is clean — the deref is not the defect by itself.
#[test]
fn a_deref_of_a_mut_variant_receiver_is_fine_alone() {
    let src = format!("{DECL}implement<T> M<T> {{\n{TAKE}}}\n");
    assert!(
        errors(&src).is_empty(),
        "`let old = *self;` alone must be clean: {:?}",
        errors(&src)
    );
}

/// SHARPENED 2026-09-06: the deref is not required, and neither is the match.
/// `T.default()` in one method plus `let r = self;` in the NEXT is enough —
/// eleven lines — and the message names `self`'s own type with an unresolved
/// payload:
///
/// ```text
///   error<E404>: Ambiguous type for `r`:
///     the inferred type `&mut None(Unit) or Some(_)` is not fully determined
/// ```
///
/// Four controls, each one variable: `let x = 1;` in place of `let r = self;`
/// is clean; `T.zero()` in place of `T.default()` is clean; the second method
/// alone is clean; and an explicit annotation (`let old: M<T> = *self;`) does
/// NOT rescue it — which says the ambiguity is settled independently of what
/// the author wrote down.
#[test]
fn a_let_bound_self_after_a_default_call_is_ambiguous_today() {
    let leaking = format!(
        "{DECL}implement<T> M<T> {{\n\
         {}\n    public fn take(&mut self) -> Int {{\n        let r = self;\n        1\n    }}\n}}\n",
        call_method("T.default()", "Default")
    );
    assert!(
        !errors(&leaking).is_empty(),
        "documented behaviour: `let r = self;` after a sibling `T.default()` is \
         ambiguous. If this now passes, the leak is fixed — delete this test and \
         the `None => 0` literal in core/base/maybe.vr"
    );

    // The controls, each differing in ONE thing.
    let let_int = format!(
        "{DECL}implement<T> M<T> {{\n\
         {}\n    public fn take(&mut self) -> Int {{\n        let x = 1;\n        x\n    }}\n}}\n",
        call_method("T.default()", "Default")
    );
    let with_zero = format!(
        "{DECL}implement<T> M<T> {{\n\
         {}\n    public fn take(&mut self) -> Int {{\n        let r = self;\n        1\n    }}\n}}\n",
        call_method("T.zero()", "Zero")
    );
    let alone = format!(
        "{DECL}implement<T> M<T> {{\n    public fn take(&mut self) -> Int {{\n        \
         let r = self;\n        1\n    }}\n}}\n"
    );
    for (what, src) in [("let x = 1", &let_int), ("T.zero()", &with_zero), ("alone", &alone)] {
        assert!(
            errors(src).is_empty(),
            "control `{what}` must be clean — it is what makes the repro specific: {:?}",
            errors(src)
        );
    }
}

/// FULLY CHARACTERISED 2026-09-06. It is neither `self` nor the variant type:
/// it is `T` ITSELF. After a method in the block calls `T.default()`, any
/// LATER `let` in that block whose type MENTIONS the impl's parameter `T` is
/// reported ambiguous.
///
/// The two controls that settle it are the last two here: a CONCRETE payload
/// (`M<Int>`) is clean, and a DIFFERENT generic type that merely mentions `T`
/// (`List<T>`) fails. So the impl's rigid `T` is consumed by the `Default`
/// discharge and not restored for the next method — a statement about the
/// impl block's type-parameter scope, not about deref, variants, or
/// `unwrap_or_default`.
#[test]
fn any_later_let_mentioning_the_impl_parameter_is_ambiguous_today() {
    let block = |second: &str| {
        format!(
            "{DECL}implement<T> M<T> {{\n{}\n{second}}}\n",
            call_method("T.default()", "Default")
        )
    };

    // Mentions `T` — all ambiguous today.
    for second in [
        "    public fn f(&mut self) -> Int {\n        let r = self;\n        1\n    }\n",
        "    public fn f(&mut self) -> Int {\n        let r: M<T> = None;\n        1\n    }\n",
        "    public fn f(&mut self, m: M<T>) -> Int {\n        let r = m;\n        1\n    }\n",
        // `let r: List<T> = List.new();` belongs to the CLI, not here: this
        // harness has no stdlib, so `List` does not resolve and the case is
        // clean for a reason that is not the language. Measured through
        // `verum check`, it gives `Ambiguous type for r: List<_>` — the same
        // defect, and the control that shows the trigger is `T` itself rather
        // than the variant type.
    ] {
        assert!(
            !errors(&block(second)).is_empty(),
            "documented behaviour: a later `let` mentioning `T` is ambiguous. \
             If this now passes the leak is fixed — see the header.\n{second}"
        );
    }

    // Does NOT mention `T`, or is not a `let` — clean, and this is what makes
    // the characterisation specific rather than a blanket "the block is broken".
    for (what, second) in [
        ("concrete payload",
         "    public fn f(&mut self) -> Int {\n        let r: M<Int> = None;\n        1\n    }\n"),
        ("no let at all",
         "    public fn f(&mut self) -> M<T> {\n        None\n    }\n"),
    ] {
        assert!(
            errors(&block(second)).is_empty(),
            "control `{what}` must be clean: {:?}",
            errors(&block(second))
        );
    }
}
