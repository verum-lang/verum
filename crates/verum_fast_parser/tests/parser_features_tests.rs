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
//! Tests for parser features: calc proofs, context parameters, type-level functions,
//! constrained type aliases, generator fn*, and format tag validation.

use verum_ast::span::FileId;
use verum_ast::ty::GenericParamKind;
use verum_ast::{Expr, ExprKind, ItemKind, Module};
use verum_common::List;
use verum_fast_parser::{ParseError, VerumParser};
use verum_lexer::Lexer;

// =============================================================================
// HELPERS
// =============================================================================

fn parse_module(source: &str) -> Result<Module, List<ParseError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id)
}

fn assert_parses(source: &str) {
    parse_module(source).unwrap_or_else(|e| {
        let msgs: Vec<String> = e.iter().map(|err| format!("{:?}", err)).collect();
        panic!("Failed to parse:\n{}\nErrors: {}", source, msgs.join("\n"))
    });
}

fn assert_fails(source: &str) {
    if parse_module(source).is_ok() {
        panic!("Expected parse failure but succeeded:\n{}", source);
    }
}

// =============================================================================
// 1. CALC PROOFS
// Grammar: calc_chain = 'calc' , '{' , calc_step , { calc_step } , '}' ;
// =============================================================================

#[test]
fn test_calc_chain_basic_equality() {
    // Basic calc chain with == relation
    assert_parses(r#"
theorem commutative(): x + y == y + x {
    proof {
        calc {
            x + y
            == { by simp } y + x
        }
    }
}
"#);
}

#[test]
fn test_calc_chain_multiple_steps() {
    // Calc chain with multiple steps
    assert_parses(r#"
theorem transitivity(): a == d {
    proof {
        calc {
            a
            == { by simp } b
            == { by simp } c
            == { by simp } d
        }
    }
}
"#);
}

#[test]
fn test_calc_chain_comparison_relations() {
    // Calc chain with < and <= relations
    assert_parses(r#"
theorem ordering(): a < d {
    proof {
        calc {
            a
            < { by simp } b
            <= { by simp } c
            < { by simp } d
        }
    }
}
"#);
}

#[test]
fn test_calc_chain_alt_syntax() {
    // Alternative syntax: relation target_expr by justification
    assert_parses(r#"
theorem equiv(): x == z {
    proof {
        calc {
            x
            == y by simp
            == z by simp
        }
    }
}
"#);
}

#[test]
fn test_calc_chain_implies() {
    // Calc chain with implies (=>) relation
    assert_parses(r#"
theorem implication(): p {
    proof {
        calc {
            p
            => { by assumption } q
            => { by simp } r
        }
    }
}
"#);
}

#[test]
fn test_calc_chain_complex_expressions() {
    // Calc chain with complex expressions
    assert_parses(r#"
theorem algebra(): f(x) + g(y) == result {
    proof {
        calc {
            f(x) + g(y)
            == { by simp } h(x, y)
            == { by ring } result
        }
    }
}
"#);
}

// =============================================================================
// 2. CONTEXT PARAMETERS
// Grammar: context_param = 'using' , identifier ;
// =============================================================================

#[test]
fn test_context_param_basic() {
    // Basic context parameter in generic list
    let source = r#"
fn identity<T, using C>(value: T) -> T using C {
    value
}
"#;
    let module = parse_module(source).unwrap();
    assert!(!module.items.is_empty());

    if let ItemKind::Function(f) = &module.items[0].kind {
        let generics = &f.generics;
        assert_eq!(generics.len(), 2);
        assert!(matches!(&generics[1].kind, GenericParamKind::Context { name } if name.as_str() == "C"));
    } else {
        panic!("Expected function item");
    }
}

#[test]
fn test_context_param_multiple() {
    // Multiple context parameters
    assert_parses(r#"
fn combine<T, U, using C1, using C2>(a: T, b: U) -> T using [C1, C2] {
    a
}
"#);
}

#[test]
fn test_context_param_higher_order() {
    // Context parameter used in higher-order function type
    assert_parses(r#"
fn map<T, U, using C>(iter: List<T>, f: fn(T) -> U using C) -> List<U> using C {
    iter
}
"#);
}

#[test]
fn test_context_param_in_type_def() {
    // Context parameter used in a type definition
    assert_parses(r#"
type Transformer<T, U, using C> is {
    transform: fn(T) -> U using C,
};
"#);
}

// =============================================================================
// 3. TYPE-LEVEL FUNCTIONS
// Grammar: type_function_def = 'type' , identifier , '<' , type_function_params , '>' , '=' , type_expr , ';' ;
// =============================================================================

#[test]
fn test_type_level_function_apply() {
    // type Apply<F<_>, A> = F<A>;
    assert_parses("type Apply<F<_>, A> = F<A>;");
}

#[test]
fn test_type_level_function_map() {
    // type Map<F<_>, List<A>> maps F over List<A>
    assert_parses("type MapType<F<_>, A> = List<F<A>>;");
}

#[test]
fn test_type_level_function_identity() {
    // Identity type function
    assert_parses("type Identity<T> = T;");
}

#[test]
fn test_type_level_function_pair() {
    // Pair type function
    assert_parses("type Pair<A, B> = (A, B);");
}

#[test]
fn test_type_level_function_with_bounds() {
    // Type function with bounds on parameters
    assert_parses("type Sortable<T: Ord> = List<T>;");
}

#[test]
fn test_type_level_function_nested() {
    // Nested type application
    assert_parses("type Nested<F<_>, G<_>, A> = F<G<A>>;");
}

// =============================================================================
// 4. CONSTRAINED TYPE ALIASES
// Grammar: constrained_type_alias = 'type' , identifier , '<' , constrained_params , '>' ,
//                                   '=' , type_expr , [ type_alias_where ] , ';' ;
// =============================================================================

#[test]
fn test_constrained_type_alias_basic() {
    // type NumList<T: Num> = List<T>;
    let source = "type NumList<T: Num> = List<T>;";
    let module = parse_module(source).unwrap();
    assert!(!module.items.is_empty());
}

#[test]
fn test_constrained_type_alias_multiple_bounds() {
    // Multiple bounds on type parameter
    assert_parses("type PrintableList<T: Display + Debug> = List<T>;");
}

#[test]
fn test_constrained_type_alias_multiple_params() {
    // Multiple constrained parameters
    assert_parses("type SortedMap<K: Ord, V: Clone> = Map<K, V>;");
}

#[test]
fn test_constrained_type_alias_with_where() {
    // Constrained alias with where clause
    assert_parses("type IterMap<I: Iterator, F> = Map<I, F> where type I.Item: Display;");
}

#[test]
fn test_constrained_type_alias_default() {
    // Constrained alias with default type parameter
    assert_parses("type Container<T: Clone = Int> = List<T>;");
}

// =============================================================================
// 5. GENERATOR FUNCTION fn*
// Grammar: fn_keyword = 'fn' , [ '*' ] for generator functions
// =============================================================================

#[test]
fn test_generator_function_basic() {
    // Basic generator function
    let source = r#"
fn* count() {
    yield 1;
    yield 2;
    yield 3;
}
"#;
    let module = parse_module(source).unwrap();
    assert!(!module.items.is_empty());

    if let ItemKind::Function(f) = &module.items[0].kind {
        assert!(f.is_generator, "Function should be marked as generator");
    } else {
        panic!("Expected function item");
    }
}

#[test]
fn test_generator_function_with_params() {
    // Generator with parameters
    assert_parses(r#"
fn* range(start: Int, end: Int) {
    let mut i = start;
    while i < end {
        yield i;
        i = i + 1;
    }
}
"#);
}

#[test]
fn test_generator_function_with_return_type() {
    // Generator with explicit return type
    assert_parses(r#"
fn* fibonacci() -> Int {
    let mut a = 0;
    let mut b = 1;
    loop {
        yield a;
        let temp = a + b;
        a = b;
        b = temp;
    }
}
"#);
}

#[test]
fn test_async_generator_function() {
    // Async generator function
    assert_parses(r#"
async fn* fetch_pages(urls: List<Text>) {
    for url in urls {
        yield url;
    }
}
"#);
}

#[test]
fn test_generator_function_with_generics() {
    // Generic generator function
    assert_parses(r#"
fn* repeat<T: Clone>(value: T, count: Int) {
    let mut i = 0;
    while i < count {
        yield value.clone();
        i = i + 1;
    }
}
"#);
}

// =============================================================================
// 6. FORMAT TAG VALIDATION (json#"...")
// =============================================================================

#[test]
fn test_json_valid_empty_object() {
    assert_parses(r#"fn f() { let x = json#"{}"; }"#);
}

#[test]
fn test_json_valid_empty_array() {
    assert_parses(r#"fn f() { let x = json#"[]"; }"#);
}

#[test]
fn test_json_valid_string() {
    assert_parses(r#"fn f() { let x = json#"\"hello\""; }"#);
}

#[test]
fn test_json_valid_number() {
    assert_parses(r#"fn f() { let x = json#"42"; }"#);
}

#[test]
fn test_json_valid_negative_number() {
    assert_parses(r#"fn f() { let x = json#"-3.14"; }"#);
}

#[test]
fn test_json_valid_boolean() {
    assert_parses(r#"fn f() { let x = json#"true"; }"#);
}

#[test]
fn test_json_valid_null() {
    assert_parses(r#"fn f() { let x = json#"null"; }"#);
}

#[test]
fn test_json_valid_nested_object() {
    assert_parses(r#"fn f() { let x = json#"{\"a\": {\"b\": [1, 2]}}"; }"#);
}

#[test]
fn test_json_valid_array_of_objects() {
    assert_parses(r#"fn f() { let x = json#"[{\"id\": 1}, {\"id\": 2}]"; }"#);
}

#[test]
fn test_json_valid_triple_quote() {
    assert_parses(r#"fn f() { let x = json#"""{"key": "value"}"""; }"#);
}

#[test]
fn test_json_invalid_unbalanced_brace() {
    // Missing closing brace
    assert_fails(r#"fn f() { let x = json#"{\"key\": 1"; }"#);
}

#[test]
fn test_json_invalid_unbalanced_bracket() {
    // Missing closing bracket
    assert_fails(r#"fn f() { let x = json#"[1, 2, 3"; }"#);
}

#[test]
fn test_json_invalid_extra_closing_brace() {
    // Extra closing brace
    assert_fails(r#"fn f() { let x = json#"}"; }"#);
}

#[test]
fn test_json_invalid_extra_closing_bracket() {
    // Extra closing bracket
    assert_fails(r#"fn f() { let x = json#"]"; }"#);
}

#[test]
fn test_json_invalid_starts_with_letter() {
    // JSON starting with invalid character
    assert_fails(r#"fn f() { let x = json#"hello"; }"#);
}

#[test]
fn test_sql_not_validated() {
    // SQL tagged literals should not be validated at parse time
    assert_parses(r#"fn f() { let x = sql#"SELECT * FROM users WHERE id = 1"; }"#);
}

#[test]
fn test_regex_not_validated() {
    // Regex tagged literals should not be validated at parse time
    assert_parses(r#"fn f() { let x = rx#"[a-z]+\d+"; }"#);
}

// =============================================================================
// 7. EDGE CASES AND COMBINATIONS
// =============================================================================

#[test]
fn test_generator_with_context_param() {
    // Generator function with context parameter
    assert_parses(r#"
fn* items<T, using C>(source: Source<T>) -> T using C {
    yield source.next();
}
"#);
}

#[test]
fn test_constrained_type_alias_with_hkt() {
    // Constrained type alias with HKT parameter
    assert_parses("type Mapped<F<_>, T: Clone> = F<T>;");
}

#[test]
fn test_calc_chain_in_lemma() {
    // Calc chain inside a lemma
    assert_parses(r#"
lemma simple_lemma(): x + 0 == x {
    proof {
        calc {
            x + 0
            == { by simp } x
        }
    }
}
"#);
}
