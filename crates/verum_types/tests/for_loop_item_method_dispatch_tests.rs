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
//! Regression tests for the for-loop item type-inference defect (#52).
//!
//! Root cause: after resolving the iterator element type (`elem_ty`) via
//! `resolve_into_iterator_protocol` or the duck-typing fallback chain, the
//! current unifier substitution was never applied to `elem_ty` before it was
//! bound to the loop variable.  When the item type is a TypeVar already
//! unified elsewhere (e.g., through a `.map()` closure return or an impl
//! substitution), the loop variable stayed as an opaque TypeVar, and any
//! inherent method call on it (like `Ordering.reverse()`) failed with
//! "method not found on type variable".
//!
//! Fix: `let elem_ty = self.unifier.apply(&elem_ty);` added after the
//! elem_ty resolution match in the `ForIn` arm of `infer.rs`.
//!
//! All tests are self-contained (no stdlib dependency) so they run against a
//! fresh `TypeChecker::new()`.  The duck-typing iterator path is used: a
//! custom `XxxIter` type with `has_next(&self) -> Bool` + `next(&mut self)
//! -> Item` methods exercises the same resolution code path.

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn typecheck_ok(code: &str, label: &str) {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(td) = &item.kind {
            let _ = checker.register_type_declaration(&td);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Impl(impl_block) = &item.kind {
            let _ = checker.register_impl_block(impl_block);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(f) = &item.kind {
            let _ = checker.register_function_signature(&f);
        }
    }
    let errs: Vec<String> = module
        .items
        .iter()
        .filter_map(|item| checker.check_item(item).err().map(|e| format!("{:?}", e)))
        .collect();
    assert!(errs.is_empty(), "{}: {:?}", label, errs);
}

// ─── 1. For-loop item must pick up inherent methods from variant type ─────────
//
// Duck-typing path: DirIter has has_next() + next() → elem_ty = Dir.
// Before the fix, elem_ty could remain a TypeVar in certain resolution paths
// and `d.flip()` would fail with "method not found on type variable".
#[test]
fn for_loop_item_inherent_method_via_duck_typing_iter() {
    typecheck_ok(
        r#"
type Dir is Up | Down | Left | Right;

implement Dir {
    public fn flip(self) -> Dir {
        match self {
            Up    => Down,
            Down  => Up,
            Left  => Right,
            Right => Left,
        }
    }
    public fn is_vertical(self) -> Bool {
        match self { Up => true, Down => true, _ => false }
    }
}

type DirIter is { pos: Int };

implement DirIter {
    public fn has_next(&self) -> Bool { self.pos < 4 }
    public fn next(&mut self) -> Dir {
        let d = match self.pos {
            0 => Up,
            1 => Down,
            2 => Left,
            _ => Right,
        };
        self.pos = self.pos + 1;
        d
    }
}

fn flip_all(iter: DirIter) {
    for d in iter {
        let _ = d.flip();
    }
}
"#,
        "for_loop_item_inherent_method_via_duck_typing_iter",
    );
}

// ─── 2. Bool-returning method on loop variable ────────────────────────────────
#[test]
fn for_loop_bool_method_on_item() {
    typecheck_ok(
        r#"
type Signal is On | Off;

implement Signal {
    public fn is_on(self) -> Bool {
        match self { On => true, Off => false }
    }
}

type SignalIter is { pos: Int };

implement SignalIter {
    public fn has_next(&self) -> Bool { self.pos < 2 }
    public fn next(&mut self) -> Signal {
        self.pos = self.pos + 1;
        if self.pos == 1 { On } else { Off }
    }
}

fn count_on(iter: SignalIter) -> Int {
    let mut n: Int = 0;
    for s in iter {
        if s.is_on() { n = n + 1; }
    }
    n
}
"#,
        "for_loop_bool_method_on_item",
    );
}

// ─── 3. Chained method calls on the loop item ─────────────────────────────────
#[test]
fn for_loop_chained_methods_on_item() {
    typecheck_ok(
        r#"
type Coin is Heads | Tails;

implement Coin {
    public fn flip(self) -> Coin {
        match self { Heads => Tails, Tails => Heads }
    }
    public fn is_heads(self) -> Bool {
        match self { Heads => true, Tails => false }
    }
}

type CoinIter is { pos: Int };

implement CoinIter {
    public fn has_next(&self) -> Bool { self.pos < 3 }
    public fn next(&mut self) -> Coin {
        self.pos = self.pos + 1;
        if self.pos % 2 == 0 { Heads } else { Tails }
    }
}

fn double_flip_heads(iter: CoinIter) -> Int {
    let mut n: Int = 0;
    for c in iter {
        if c.flip().flip().is_heads() { n = n + 1; }
    }
    n
}
"#,
        "for_loop_chained_methods_on_item",
    );
}

// ─── 4. Item used in match after method call ──────────────────────────────────
#[test]
fn for_loop_match_result_of_item_method() {
    typecheck_ok(
        r#"
type Grade is A | B | C | F;

implement Grade {
    public fn bump(self) -> Grade {
        match self {
            F => C,
            C => B,
            B => A,
            A => A,
        }
    }
}

type GradeIter is { pos: Int };

implement GradeIter {
    public fn has_next(&self) -> Bool { self.pos < 4 }
    public fn next(&mut self) -> Grade {
        self.pos = self.pos + 1;
        match self.pos {
            1 => F,
            2 => C,
            3 => B,
            _ => A,
        }
    }
}

fn count_as(iter: GradeIter) -> Int {
    let mut n: Int = 0;
    for g in iter {
        let bumped = g.bump();
        match bumped {
            A => { n = n + 1; }
            _ => {}
        }
    }
    n
}
"#,
        "for_loop_match_result_of_item_method",
    );
}
