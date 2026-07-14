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
//! CONST-GENERIC-IMPL-METHODS-1 (task #10) — regression tests.
//!
//! Root cause: `impl_pattern_slot_matches` (the ONE per-instantiation
//! impl-gating authority, `infer/env.rs`) had no `Type::Meta` arm.  A
//! const-generic impl `implement<const SIZE: Int> Stack<SIZE>` registers
//! its self-type arg pattern slot as the const's BASE type (`Int` — the
//! `GenericParamKind::Const` registration arms bind
//! `define_type(SIZE, base_ty)`), while a const-generic RECEIVER
//! instantiation (`Stack<256>`, or the alias `type Tiny is Stack<1024>;`
//! expanded) carries `Type::Meta { ty: Int, value: Some(256) }` (see
//! `eval_const_arg`).  The primitive pattern arm compared
//! `Meta == Int` → `false`, so `inherent_method_pattern_allows`
//! rejected the receiver and method lookup died with
//! `E400: no method named `capacity` found for type `StackAllocator``.
//!
//! The fix makes const args ride the same wildcard/pin semantics as
//! type args inside the single slot matcher (no sibling table):
//!  * primitive pattern slot accepts a `Meta` receiver of the same base
//!    type (generic const impl matches every instantiation);
//!  * `Meta{value: Some}` pattern slots pin the value (an impl written
//!    against a concrete instantiation only matches that value) —
//!    mirroring unify's value-carrying-Meta precedence;
//!  * `Meta{value: None}` pattern slots are wildcards like `Var`.

use verum_ast::span::Span;
use verum_ast::ty::Path;
use verum_ast::Ident;
use verum_common::ConstValue;
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;
use verum_types::Type;

fn named(name: &str) -> Type {
    Type::Named {
        path: Path::single(Ident::new(name, Span::dummy())),
        args: verum_common::List::new(),
    }
}

fn meta_int(v: i64) -> Type {
    Type::meta_value(ConstValue::Int(v as i128), Type::Int)
}

fn meta_decl(name: &str) -> Type {
    Type::meta(name.into(), Type::Int, None)
}

// ─── Pure slot-matcher unit tests ────────────────────────────────────────────

/// The dominant registered shape today: `implement<const SIZE: Int>
/// Stack<SIZE>` registers the pattern slot as the const's base type
/// (`Int`).  Every concrete instantiation (`Meta{value: Some(_), ty:
/// Int}`) must match it — this is the exact arm whose absence produced
/// the E400 class.
#[test]
fn primitive_pattern_slot_accepts_const_instantiation_of_same_base() {
    assert!(TypeChecker::impl_pattern_slot_matches(
        &meta_int(256),
        &Type::Int
    ));
    assert!(TypeChecker::impl_pattern_slot_matches(
        &meta_int(1024),
        &Type::Int
    ));
}

/// A const instantiation of a DIFFERENT base type must not match a
/// primitive slot (`Stack<true>` receiver vs `Int`-kinded const param).
#[test]
fn primitive_pattern_slot_rejects_const_instantiation_of_other_base() {
    let bool_const = Type::meta_value(ConstValue::Bool(true), Type::Bool);
    assert!(!TypeChecker::impl_pattern_slot_matches(
        &bool_const,
        &Type::Int
    ));
}

/// Plain primitive receiver vs primitive slot — pre-existing behavior
/// must be preserved (direct `Stack<256>.new()` receivers whose args
/// collapsed to `Int` upstream).
#[test]
fn primitive_pattern_slot_still_accepts_equal_primitive() {
    assert!(TypeChecker::impl_pattern_slot_matches(&Type::Int, &Type::Int));
    assert!(!TypeChecker::impl_pattern_slot_matches(
        &Type::Float,
        &Type::Int
    ));
}

/// Value-pinned pattern slot (`implement Stack<1024>`): equal values
/// match, differing values reject — per-instantiation gating, the same
/// discipline task #35 introduced for type args (`Register<T, ReadOnly>`
/// vs `Register<T, WriteOnly>`).
#[test]
fn value_pinned_pattern_slot_pins_receiver_value() {
    let pat = meta_int(1024);
    assert!(TypeChecker::impl_pattern_slot_matches(&meta_int(1024), &pat));
    assert!(!TypeChecker::impl_pattern_slot_matches(&meta_int(256), &pat));
}

/// Value-pinned pattern slot vs a receiver whose const collapsed to the
/// base type upstream: value information is lost, stay permissive on
/// base-type agreement (mirrors unify's Meta-vs-non-Meta rule).
#[test]
fn value_pinned_pattern_slot_permissive_on_base_type_receiver() {
    let pat = meta_int(1024);
    assert!(TypeChecker::impl_pattern_slot_matches(&Type::Int, &pat));
    assert!(!TypeChecker::impl_pattern_slot_matches(&Type::Text, &pat));
}

/// Declaration-site const param slot (`Meta{value: None}`) is a
/// wildcard, exactly like an impl-level `Var` type param.
#[test]
fn declaration_site_meta_pattern_slot_is_wildcard() {
    let pat = meta_decl("SIZE");
    assert!(TypeChecker::impl_pattern_slot_matches(&meta_int(256), &pat));
    assert!(TypeChecker::impl_pattern_slot_matches(&Type::Int, &pat));
    assert!(TypeChecker::impl_pattern_slot_matches(&named("Foo"), &pat));
}

/// An unresolved receiver const (declaration-site Meta on the RECEIVER
/// side, e.g. inside another generic impl body) stays permissive
/// against a value-pinned slot — inference must not be pinned
/// prematurely.
#[test]
fn unresolved_receiver_meta_is_permissive_against_pinned_slot() {
    let pat = meta_int(1024);
    assert!(TypeChecker::impl_pattern_slot_matches(
        &meta_decl("SIZE"),
        &pat
    ));
}

/// Type-arg slots are untouched by the const-generic arms: `Named`
/// pattern slots still pin by head name.
#[test]
fn named_pattern_slot_behavior_unchanged() {
    assert!(TypeChecker::impl_pattern_slot_matches(
        &named("ReadOnly"),
        &named("ReadOnly")
    ));
    assert!(!TypeChecker::impl_pattern_slot_matches(
        &named("WriteOnly"),
        &named("ReadOnly")
    ));
}

// ─── Gate-level tests (patterns registered from a real impl block) ──────────

/// Registers `implement<const SIZE: Int> Buf<SIZE>` and drives the gate
/// entry (`inherent_method_pattern_allows`) directly:
///  * `Meta{256}` receiver arg — must pass (the E400 class);
///  * NO receiver args (bare `Buf` — upstream legs drop the
///    instantiation, observed on alias static-call return types) — no
///    instantiation evidence, must stay permissive;
///  * a receiver arg of a foreign base type — must still reject, the
///    gate keeps its task-#35 gating power.
#[test]
fn gate_allows_const_instantiations_and_bare_receivers() {
    let code = r#"
type Buf<const SIZE: Int> is {
    top: Int,
};

implement<const SIZE: Int> Buf<SIZE> {
    public fn capacity(&self) -> Int {
        SIZE
    }
}
"#;
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(td) = &item.kind {
            let _ = checker.register_type_declaration(td);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Impl(impl_block) = &item.kind {
            checker
                .register_impl_block(impl_block)
                .expect("impl registration should succeed");
        }
    }

    let ty: verum_common::Text = "Buf".into();
    let m: verum_common::Text = "capacity".into();

    // Concrete const instantiation rides the type-arg path.
    assert!(
        checker.inherent_method_pattern_allows(&ty, &m, &[meta_int(256)]),
        "Meta{{256}} receiver must match the const-generic impl pattern"
    );
    // Receiver collapsed to the const's base type upstream.
    assert!(
        checker.inherent_method_pattern_allows(&ty, &m, &[Type::Int]),
        "Int-collapsed receiver must match the const-generic impl pattern"
    );
    // Bare receiver (instantiation dropped upstream) — permissive.
    assert!(
        checker.inherent_method_pattern_allows(&ty, &m, &[]),
        "arg-less receiver provides no instantiation evidence — permissive"
    );
    // A contradicting receiver arg still rejects.
    assert!(
        !checker.inherent_method_pattern_allows(&ty, &m, &[Type::Text]),
        "foreign-base receiver arg must still be rejected by the gate"
    );
}

// ─── Behavioral: full typecheck of the defect's repro shapes ────────────────

fn typecheck_ok(code: &str, label: &str) {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(td) = &item.kind {
            let _ = checker.register_type_declaration(td);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Impl(impl_block) = &item.kind {
            let _ = checker.register_impl_block(impl_block);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(f) = &item.kind {
            let _ = checker.register_function_signature(f);
        }
    }
    let errs: Vec<String> = module
        .items
        .iter()
        .filter_map(|item| checker.check_item(item).err().map(|e| format!("{:?}", e)))
        .collect();
    assert!(errs.is_empty(), "{}: {:?}", label, errs);
}

/// Direct instantiation: methods of a `implement<const SIZE: Int>` block
/// must resolve on `Buf<256>` receivers.
#[test]
fn const_generic_impl_methods_resolve_on_direct_instantiation() {
    typecheck_ok(
        r#"
type Buf<const SIZE: Int> is {
    top: Int,
};

implement<const SIZE: Int> Buf<SIZE> {
    public fn new() -> Self {
        Self { top: 0 }
    }
    public fn capacity(&self) -> Int {
        SIZE
    }
    public fn used(&self) -> Int {
        self.top
    }
}

fn main() {
    let mut a = Buf<256>.new();
    let c = a.capacity();
    let u = a.used();
}
"#,
        "direct const-generic instantiation",
    );
}

/// Alias expansion: `type Tiny is Buf<1024>;` — the alias preserves the
/// const arg as `Meta{value: Some(1024)}`, the exact shape the pattern
/// gate rejected pre-fix (E400 `no method named `capacity``).
#[test]
fn const_generic_impl_methods_resolve_through_alias() {
    typecheck_ok(
        r#"
type Buf<const SIZE: Int> is {
    top: Int,
};

implement<const SIZE: Int> Buf<SIZE> {
    public fn new() -> Self {
        Self { top: 0 }
    }
    public fn capacity(&self) -> Int {
        SIZE
    }
}

type Tiny is Buf<1024>;

fn main() {
    let t = Tiny.new();
    let c = t.capacity();
}
"#,
        "alias to const-generic instantiation",
    );
}

/// Multi-const-param impls (the ArenaAllocator / PoolAllocator shape).
#[test]
fn const_generic_impl_methods_resolve_with_two_const_params() {
    typecheck_ok(
        r#"
type Pool<const BLOCK_SIZE: Int, const BLOCK_COUNT: Int> is {
    used: Int,
};

implement<const BLOCK_SIZE: Int, const BLOCK_COUNT: Int> Pool<BLOCK_SIZE, BLOCK_COUNT> {
    public fn new() -> Self {
        Self { used: 0 }
    }
    public fn block_count(&self) -> Int {
        BLOCK_COUNT
    }
}

fn main() {
    let p = Pool<64, 32>.new();
    let n = p.block_count();
}
"#,
        "two const params",
    );
}
