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
//! Parser tests for universe polymorphism syntax.
//!
//! Tests the `universe u` generic parameter form (verum-ext.md §2.1) and the
//! existing `u: Level` alternative form.  Also tests `Type(max(u, v))` level
//! expressions inside type annotations.
//!
//! Grammar rules exercised:
//!   universe_param = 'universe' , identifier ;
//!   level_param    = identifier , ':' , 'Level' ;
//!   universe_type  = 'Type' , [ '(' , universe_level_expr , ')' ] ;
//!   universe_level_expr = integer_lit | identifier
//!                       | 'max' '(' universe_level_expr ',' universe_level_expr ')' ;

use verum_ast::span::FileId;
use verum_ast::ty::{GenericParamKind, UniverseLevelExpr};
use verum_ast::{ItemKind, TypeKind};
use verum_common::List;
use verum_lexer::Lexer;
use verum_fast_parser::{ParseError, VerumParser};

/// Parse helper.
fn parse_module(input: &str) -> Result<verum_ast::Module, List<ParseError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id)
}

// ============================================================================
// `universe u` generic parameter form
// ============================================================================

#[test]
fn test_universe_param_in_function() {
    // @universe_poly fn id<universe u, A: Type(u)>(x: A) -> A { x }
    let input = r#"
@universe_poly
fn id<universe u, A: Type(u)>(x: A) -> A { x }
"#;
    let result = parse_module(input);
    assert!(
        result.is_ok(),
        "Should parse function with `universe u` param: {:?}",
        result.err()
    );

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            let generics = &func.generics;
            assert!(generics.len() >= 1, "Should have at least 1 generic param");
            // First param should be a Level (universe) param named 'u'
            match &generics[0].kind {
                GenericParamKind::Level { name } => {
                    assert_eq!(name.name.as_str(), "u", "Universe param name should be 'u'");
                }
                other => panic!(
                    "Expected Level param at index 0, got {:?}",
                    other
                ),
            }
        }
        other => panic!("Expected Function item, got {:?}", other),
    }
}

#[test]
fn test_universe_param_in_type_decl() {
    // @universe_poly type Pair<universe u, universe v, A: Type(u), B: Type(v)> is { fst: A, snd: B };
    let input = r#"
@universe_poly
type Pair<universe u, universe v, A: Type(u), B: Type(v)> is {
    fst: A,
    snd: B,
};
"#;
    let result = parse_module(input);
    assert!(
        result.is_ok(),
        "Should parse type decl with two `universe` params: {:?}",
        result.err()
    );

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Type(type_decl) => {
            let generics = &type_decl.generics;
            assert!(generics.len() >= 2, "Should have at least 2 generic params");
            match &generics[0].kind {
                GenericParamKind::Level { name } => {
                    assert_eq!(name.name.as_str(), "u");
                }
                other => panic!("Expected Level param 'u' at index 0, got {:?}", other),
            }
            match &generics[1].kind {
                GenericParamKind::Level { name } => {
                    assert_eq!(name.name.as_str(), "v");
                }
                other => panic!("Expected Level param 'v' at index 1, got {:?}", other),
            }
        }
        other => panic!("Expected Type item, got {:?}", other),
    }
}

// ============================================================================
// `u: Level` generic parameter form (existing, should still work)
// ============================================================================

#[test]
fn test_level_bound_param_in_function() {
    // fn id<u: Level, A: Type(u)>(x: A) -> A { x }
    let input = r#"
fn id<u: Level, A: Type(u)>(x: A) -> A { x }
"#;
    let result = parse_module(input);
    assert!(
        result.is_ok(),
        "Should parse function with `u: Level` param: {:?}",
        result.err()
    );

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            let generics = &func.generics;
            assert!(generics.len() >= 1);
            match &generics[0].kind {
                GenericParamKind::Level { name } => {
                    assert_eq!(name.name.as_str(), "u");
                }
                other => panic!("Expected Level param at index 0, got {:?}", other),
            }
        }
        other => panic!("Expected Function item, got {:?}", other),
    }
}

// ============================================================================
// `Type(u)` and `Type(max(u, v))` universe level expressions
// ============================================================================

#[test]
fn test_universe_type_with_variable() {
    // fn id<u: Level, A: Type(u)>(x: A) -> A { x }
    // The return type `A` here refers to the type param; testing Type(u) via
    // a bound is the natural place for it.
    let input = r#"
fn check<u: Level>(t: Type(u)) -> Bool { true }
"#;
    let result = parse_module(input);
    assert!(
        result.is_ok(),
        "Should parse function using Type(u) as a parameter type: {:?}",
        result.err()
    );
}

#[test]
fn test_universe_type_with_max() {
    // fn f<universe u, universe v, A: Type(u), B: Type(v)>(a: A, b: B) -> Type(max(u, v)) { ... }
    let input = r#"
@universe_poly
fn pair_type<universe u, universe v>(a: Type(u), b: Type(v)) -> Type(max(u, v)) { a }
"#;
    let result = parse_module(input);
    assert!(
        result.is_ok(),
        "Should parse function with Type(max(u, v)) return type: {:?}",
        result.err()
    );
}

#[test]
fn test_universe_type_bare() {
    // Bare `Type` should still parse (equivalent to Type(0))
    let input = r#"
fn id_type(t: Type) -> Type { t }
"#;
    let result = parse_module(input);
    assert!(
        result.is_ok(),
        "Should parse bare `Type` as universe type: {:?}",
        result.err()
    );
}

#[test]
fn test_universe_type_concrete_level() {
    // Type(0), Type(1) are concrete levels
    let input = r#"
fn small_type(t: Type(0)) -> Type(1) { t }
"#;
    let result = parse_module(input);
    assert!(
        result.is_ok(),
        "Should parse Type(0) and Type(1): {:?}",
        result.err()
    );
}

// ============================================================================
// @universe_poly attribute validation
// ============================================================================

#[test]
fn test_universe_poly_attr_on_fn_no_warning() {
    // @universe_poly is a known attribute on functions — should produce no validation warnings
    let input = r#"
@universe_poly
fn id<universe u, A: Type(u)>(x: A) -> A { x }
"#;
    // Just check it parses: the attribute validator only warns for unknown attrs
    let result = parse_module(input);
    assert!(
        result.is_ok(),
        "@universe_poly fn should parse without error: {:?}",
        result.err()
    );
}

#[test]
fn test_universe_poly_attr_on_type_no_warning() {
    let input = r#"
@universe_poly
type Box<universe u, A: Type(u)> is { value: A };
"#;
    let result = parse_module(input);
    assert!(
        result.is_ok(),
        "@universe_poly type should parse without error: {:?}",
        result.err()
    );
}
