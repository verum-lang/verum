//! n-cell HIT surface parsing.
//!
//! Verifies that the parser correctly recognises the higher-cell
//! HIT syntax `Foo() = (lhs..rhs) .. (lhs'..rhs')` and computes
//! the variant's `path_dim` from the endpoint nesting depth.
//!
//! Note: when an n-cell variant is followed by another variant
//! (rather than `;`), the user must use a separate `type` decl
//! for the higher cell or trail with a `;`. The current parser
//! does not statically distinguish bit-or `|` from variant-list
//! `|` once an expression-position `..` appears, so the test
//! corpus uses the `;`-terminated form which is unambiguous.

use verum_ast::decl::{ItemKind, TypeDeclBody, Variant};
use verum_common::Maybe;
use verum_fast_parser::Parser;

fn parse_module(source: &str) -> verum_ast::Module {
    let mut parser = Parser::new(source);
    parser.parse_module().expect("parse")
}

fn first_type_variants(module: &verum_ast::Module, type_name: &str) -> Vec<Variant> {
    for item in module.items.iter() {
        if let ItemKind::Type(decl) = &item.kind {
            if decl.name.name.as_str() == type_name {
                match &decl.body {
                    TypeDeclBody::Variant(variants) | TypeDeclBody::Inductive(variants) => {
                        return variants.iter().cloned().collect();
                    }
                    other => panic!(
                        "type `{}` body is not Variant/Inductive: {:?}",
                        type_name, other
                    ),
                }
            }
        }
    }
    panic!(
        "type `{}` not found in module (items: {})",
        type_name,
        module.items.iter().count()
    );
}

#[test]
fn one_cell_loop_has_path_dim_one() {
    let source = r#"
public type S1 is
    Base
    | Loop() = Base..Base;
"#;
    let module = parse_module(source);
    let variants = first_type_variants(&module, "S1");
    let loop_variant = variants
        .iter()
        .find(|v| v.name.name.as_str() == "Loop")
        .expect("Loop variant");
    assert!(matches!(loop_variant.path_endpoints, Maybe::Some(_)));
    assert_eq!(loop_variant.path_dim, 1, "1-cell Loop must have path_dim=1");
}

#[test]
fn one_cell_seg_has_path_dim_one() {
    let source = r#"
public type Interval is
    Zero
    | One
    | Seg() = Zero..One;
"#;
    let module = parse_module(source);
    let variants = first_type_variants(&module, "Interval");
    let seg = variants
        .iter()
        .find(|v| v.name.name.as_str() == "Seg")
        .expect("Seg");
    assert_eq!(seg.path_dim, 1);
}

#[test]
fn point_variant_keeps_default_dim_one() {
    // Multi-variant to force variant body (not type alias).
    let source = r#"
public type S1Bool is
    Base
    | Other;
"#;
    let module = parse_module(source);
    let variants = first_type_variants(&module, "S1Bool");
    let base = variants
        .iter()
        .find(|v| v.name.name.as_str() == "Base")
        .expect("Base");
    assert!(matches!(base.path_endpoints, Maybe::None));
    assert_eq!(base.path_dim, 1, "point ctor's path_dim defaults to 1");
}

#[test]
fn one_cell_with_paren_endpoints_parses_and_keeps_dim_one() {
    // `(Base..Base)` syntax — paren-wrapped 1-cell endpoint.
    // The parser unwraps a single outer Paren before checking
    // for Range, so this also yields path_dim = 1.
    let source = r#"
public type S1Paren is
    Base
    | Loop() = (Base..Base);
"#;
    let module = parse_module(source);
    let variants = first_type_variants(&module, "S1Paren");
    let loop_variant = variants
        .iter()
        .find(|v| v.name.name.as_str() == "Loop")
        .expect("Loop");
    assert!(matches!(loop_variant.path_endpoints, Maybe::Some(_)));
    assert_eq!(loop_variant.path_dim, 1);
}

#[test]
fn two_cell_via_separate_decl_has_path_dim_two() {
    // To avoid the `|` ambiguity in expression position, declare
    // the 2-cell as the LAST (or sole) variant in its type so
    // the trailing `;` terminates the expression cleanly.
    let source = r#"
public type Surf2 is
    Surf() = (loop_a..loop_b)..(loop_b..loop_a);
"#;
    let module = parse_module(source);
    let variants = first_type_variants(&module, "Surf2");
    let surf = variants
        .iter()
        .find(|v| v.name.name.as_str() == "Surf")
        .expect("Surf");
    assert!(matches!(surf.path_endpoints, Maybe::Some(_)));
    assert_eq!(surf.path_dim, 2, "2-cell Surf must have path_dim=2");
}

#[test]
fn three_cell_via_sole_variant_has_path_dim_three() {
    let source = r#"
public type Cell3 is
    Cell3() = ((a..b)..(c..d))..((a..b)..(c..d));
"#;
    let module = parse_module(source);
    let variants = first_type_variants(&module, "Cell3");
    let c3 = variants
        .iter()
        .find(|v| v.name.name.as_str() == "Cell3")
        .expect("Cell3");
    assert_eq!(c3.path_dim, 3, "3-cell must have path_dim=3");
}
