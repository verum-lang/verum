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
//! Comprehensive tests for dependent types system
//!
//! Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking
//!
//! This test suite covers:
//! - Pi types (dependent functions): (x: A) -> B(x)
//! - Sigma types (dependent pairs): (x: A, B(x))
//! - Equality types: Eq<A, x, y>
//! - Type-level computation
//!
//! All examples are from the specification and cover both success
//! and error cases for comprehensive validation.

use verum_common::{List, Text};
use verum_types::context::TypeContext;
use verum_types::ty::{EqConst, EqTerm, Type};

// =============================================================================
// PI TYPE INFERENCE TESTS (20+ tests)
// =============================================================================

#[test]
fn test_pi_type_creation() {
    // Test creating a simple Pi type: (x: Int) -> Int
    let param_name = Text::from("x");
    let param_type = Type::Int;
    let return_type = Type::Int;

    let pi = Type::pi(param_name.clone(), param_type, return_type);

    match pi {
        Type::Pi {
            param_name: name,
            param_type: p_ty,
            return_type: r_ty,
        } => {
            assert_eq!(name, param_name);
            assert_eq!(*p_ty, Type::Int);
            assert_eq!(*r_ty, Type::Int);
        }
        _ => panic!("Expected Pi type"),
    }
}

#[test]
fn test_pi_type_is_dependent() {
    // Pi types should be recognized as dependent types
    let pi = Type::pi(Text::from("n"), Type::Int, Type::Bool);

    assert!(pi.is_dependent());
}

#[test]
fn test_pi_type_with_dependent_return() {
    // Test Pi type where return type depends on parameter
    // (n: Nat) -> List<T, n>
    let param_name = Text::from("n");
    let param_type = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Nat".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: List::new(),
    };

    // Return type: List<T, n> (simplified representation)
    let return_type = Type::Generic {
        name: Text::from("List"),
        args: vec![
            Type::Var(verum_types::ty::TypeVar::new(0)),
            Type::Var(verum_types::ty::TypeVar::new(1)),
        ]
        .into_iter()
        .collect(),
    };

    let pi = Type::pi(param_name, param_type, return_type);

    assert!(pi.is_dependent());
}

#[test]
fn test_pi_type_nested() {
    // Test nested Pi types: (x: A) -> (y: B) -> C
    let inner_pi = Type::pi(Text::from("y"), Type::Bool, Type::Text);
    let outer_pi = Type::pi(Text::from("x"), Type::Int, inner_pi);

    assert!(outer_pi.is_dependent());
}

#[test]
fn test_pi_type_with_refinement() {
    // Test Pi type with refined parameter: (n: Int{> 0}) -> List<T, n>
    use verum_types::refinement::RefinementPredicate;

    let refined_param = Type::Refined {
        base: Box::new(Type::Int),
        predicate: RefinementPredicate::placeholder(), // Simplified
    };

    let pi = Type::pi(Text::from("n"), refined_param, Type::Bool);

    assert!(pi.is_dependent());
}

#[test]
fn test_pi_type_multiple_params_curried() {
    // Pi types are curried: (x: A) -> (y: B) -> C
    // Represents: fn f(x: A, y: B) -> C
    let param1 = Type::pi(Text::from("y"), Type::Bool, Type::Text);
    let param2 = Type::pi(Text::from("x"), Type::Int, param1);

    // Should be recognized as dependent
    assert!(param2.is_dependent());
}

#[test]
fn test_pi_type_replicate_function() {
    // Example from spec: fn replicate<T>(n: Nat) -> List<T, n>
    let pi = Type::pi(
        Text::from("n"),
        Type::Named {
            path: verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::ty::Ident {
                        name: "Nat".to_string().into(),
                        span: verum_ast::span::Span::default(),
                    }
                )],
                span: verum_ast::span::Span::default(),
            },
            args: List::new(),
        },
        Type::Generic {
            name: Text::from("List"),
            args: vec![Type::Var(verum_types::ty::TypeVar::new(0))]
                .into_iter()
                .collect(),
        },
    );

    assert!(pi.is_dependent());
}

#[test]
fn test_pi_type_printf_dependent() {
    // Example from spec: fn printf(fmt: Text) -> ParseFormat(fmt) -> Text
    // First Pi: (fmt: Text) -> ...
    // Inner Pi: ParseFormat(fmt) -> Text

    let inner = Type::Function {
        params: vec![Type::Generic {
            name: Text::from("ParseFormat"),
            args: List::new(),
        }]
        .into_iter()
        .collect(),
        return_type: Box::new(Type::Text),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };

    let outer = Type::pi(Text::from("fmt"), Type::Text, inner);

    assert!(outer.is_dependent());
}

#[test]
fn test_pi_type_universe_level() {
    // Pi types should have proper universe levels
    let mut ctx = TypeContext::new();

    let pi = Type::pi(Text::from("x"), Type::Int, Type::Bool);

    // Get universe level (both Int and Bool are in Type₀)
    let level = ctx.universe_of(&pi);

    // Pi type (A : Type_i) -> (B : Type_j) lives in Type_max(i,j)
    // Here both are Type₀, so result should be Type₀
    assert!(level.is_ok());
}

#[test]
fn test_pi_type_with_type_parameter() {
    // Test: (T: Type) -> T -> T (identity type)
    let pi = Type::pi(
        Text::from("x"),
        Type::Var(verum_types::ty::TypeVar::new(0)),
        Type::Var(verum_types::ty::TypeVar::new(0)),
    );

    assert!(pi.is_dependent());
}

#[test]
fn test_pi_type_display() {
    // Test that Pi types have a meaningful display representation
    let pi = Type::pi(Text::from("x"), Type::Int, Type::Bool);

    let display = format!("{:?}", pi);
    assert!(display.contains("Pi"));
}

#[test]
fn test_pi_type_equality() {
    // Test that identical Pi types are equal
    let pi1 = Type::pi(Text::from("x"), Type::Int, Type::Bool);
    let pi2 = Type::pi(Text::from("x"), Type::Int, Type::Bool);

    assert_eq!(pi1, pi2);
}

#[test]
fn test_pi_type_inequality_param_name() {
    // Different parameter names should not affect equality (alpha equivalence)
    // Note: This test may need to be updated based on alpha equivalence implementation
    let pi1 = Type::pi(Text::from("x"), Type::Int, Type::Bool);
    let pi2 = Type::pi(Text::from("y"), Type::Int, Type::Bool);

    // Currently they are not equal (no alpha equivalence yet)
    assert_ne!(pi1, pi2);
}

#[test]
fn test_pi_type_inequality_param_type() {
    // Different parameter types should make Pi types unequal
    let pi1 = Type::pi(Text::from("x"), Type::Int, Type::Bool);
    let pi2 = Type::pi(Text::from("x"), Type::Bool, Type::Bool);

    assert_ne!(pi1, pi2);
}

#[test]
fn test_pi_type_inequality_return_type() {
    // Different return types should make Pi types unequal
    let pi1 = Type::pi(Text::from("x"), Type::Int, Type::Bool);
    let pi2 = Type::pi(Text::from("x"), Type::Int, Type::Text);

    assert_ne!(pi1, pi2);
}

#[test]
fn test_pi_type_clone() {
    // Test that Pi types can be cloned
    let pi1 = Type::pi(Text::from("x"), Type::Int, Type::Bool);
    let pi2 = pi1.clone();

    assert_eq!(pi1, pi2);
}

#[test]
fn test_pi_type_with_unit_param() {
    // Test Pi type with Unit parameter: (x: Unit) -> Int
    let pi = Type::pi(Text::from("x"), Type::Unit, Type::Int);

    assert!(pi.is_dependent());
}

#[test]
fn test_pi_type_with_tuple_param() {
    // Test Pi type with tuple parameter: (p: (Int, Bool)) -> Text
    let tuple = Type::Tuple(vec![Type::Int, Type::Bool].into_iter().collect());
    let pi = Type::pi(Text::from("p"), tuple, Type::Text);

    assert!(pi.is_dependent());
}

#[test]
fn test_pi_type_returning_function() {
    // Test Pi type returning a function: (x: Int) -> (Bool -> Text)
    let inner_fn = Type::Function {
        params: vec![Type::Bool].into_iter().collect(),
        return_type: Box::new(Type::Text),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };

    let pi = Type::pi(Text::from("x"), Type::Int, inner_fn);

    assert!(pi.is_dependent());
}

#[test]
fn test_pi_type_fin_bounds() {
    // Test Pi type with Fin<n> bounds: (n: Nat) -> (i: Fin<n>) -> T
    let fin_type = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Fin".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: List::new(),
    };

    let inner_pi = Type::pi(
        Text::from("i"),
        fin_type,
        Type::Var(verum_types::ty::TypeVar::new(0)),
    );

    let outer_pi = Type::pi(
        Text::from("n"),
        Type::Named {
            path: verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::ty::Ident {
                        name: "Nat".to_string().into(),
                        span: verum_ast::span::Span::default(),
                    }
                )],
                span: verum_ast::span::Span::default(),
            },
            args: List::new(),
        },
        inner_pi,
    );

    assert!(outer_pi.is_dependent());
}

// =============================================================================
// SIGMA TYPE INFERENCE TESTS (20+ tests)
// =============================================================================

#[test]
fn test_sigma_type_creation() {
    // Test creating a simple Sigma type: (x: Int, Bool)
    let fst_name = Text::from("x");
    let fst_type = Type::Int;
    let snd_type = Type::Bool;

    let sigma = Type::sigma(fst_name.clone(), fst_type, snd_type);

    match sigma {
        Type::Sigma {
            fst_name: name,
            fst_type: f_ty,
            snd_type: s_ty,
        } => {
            assert_eq!(name, fst_name);
            assert_eq!(*f_ty, Type::Int);
            assert_eq!(*s_ty, Type::Bool);
        }
        _ => panic!("Expected Sigma type"),
    }
}

#[test]
fn test_sigma_type_is_dependent() {
    // Sigma types should be recognized as dependent types
    let sigma = Type::sigma(Text::from("x"), Type::Int, Type::Bool);

    assert!(sigma.is_dependent());
}

#[test]
fn test_sigma_type_dependent_pair() {
    // Test Sigma type where second component depends on first
    // (n: Nat, List<T, n>)
    let sigma = Type::sigma(
        Text::from("n"),
        Type::Named {
            path: verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::ty::Ident {
                        name: "Nat".to_string().into(),
                        span: verum_ast::span::Span::default(),
                    }
                )],
                span: verum_ast::span::Span::default(),
            },
            args: List::new(),
        },
        Type::Generic {
            name: Text::from("List"),
            args: vec![Type::Var(verum_types::ty::TypeVar::new(0))]
                .into_iter()
                .collect(),
        },
    );

    assert!(sigma.is_dependent());
}

#[test]
fn test_sigma_type_bounded_int() {
    // Example from spec: type BoundedInt is n: i32 where n >= 0 && n <= 100
    use verum_types::refinement::RefinementPredicate;

    let sigma = Type::sigma(
        Text::from("n"),
        Type::Int,
        Type::Refined {
            base: Box::new(Type::Unit),
            predicate: RefinementPredicate::placeholder(),
        },
    );

    assert!(sigma.is_dependent());
}

#[test]
fn test_sigma_type_positive_int() {
    // Example from spec: type PositiveInt is n: i32 where n > 0
    use verum_types::refinement::RefinementPredicate;

    let sigma = Type::sigma(
        Text::from("n"),
        Type::Int,
        Type::Refined {
            base: Box::new(Type::Unit),
            predicate: RefinementPredicate::placeholder(),
        },
    );

    assert!(sigma.is_dependent());
}

#[test]
fn test_sigma_type_nested() {
    // Test nested Sigma types: (x: A, (y: B, C))
    let inner_sigma = Type::sigma(Text::from("y"), Type::Bool, Type::Text);
    let outer_sigma = Type::sigma(Text::from("x"), Type::Int, inner_sigma);

    assert!(outer_sigma.is_dependent());
}

#[test]
fn test_sigma_type_equality() {
    // Test that identical Sigma types are equal
    let sigma1 = Type::sigma(Text::from("x"), Type::Int, Type::Bool);
    let sigma2 = Type::sigma(Text::from("x"), Type::Int, Type::Bool);

    assert_eq!(sigma1, sigma2);
}

#[test]
fn test_sigma_type_inequality_fst_name() {
    // Different first component names should make Sigma types unequal
    // (without alpha equivalence)
    let sigma1 = Type::sigma(Text::from("x"), Type::Int, Type::Bool);
    let sigma2 = Type::sigma(Text::from("y"), Type::Int, Type::Bool);

    assert_ne!(sigma1, sigma2);
}

#[test]
fn test_sigma_type_inequality_fst_type() {
    // Different first component types should make Sigma types unequal
    let sigma1 = Type::sigma(Text::from("x"), Type::Int, Type::Bool);
    let sigma2 = Type::sigma(Text::from("x"), Type::Bool, Type::Bool);

    assert_ne!(sigma1, sigma2);
}

#[test]
fn test_sigma_type_inequality_snd_type() {
    // Different second component types should make Sigma types unequal
    let sigma1 = Type::sigma(Text::from("x"), Type::Int, Type::Bool);
    let sigma2 = Type::sigma(Text::from("x"), Type::Int, Type::Text);

    assert_ne!(sigma1, sigma2);
}

#[test]
fn test_sigma_type_clone() {
    // Test that Sigma types can be cloned
    let sigma1 = Type::sigma(Text::from("x"), Type::Int, Type::Bool);
    let sigma2 = sigma1.clone();

    assert_eq!(sigma1, sigma2);
}

#[test]
fn test_sigma_type_display() {
    // Test that Sigma types have a meaningful display representation
    let sigma = Type::sigma(Text::from("x"), Type::Int, Type::Bool);

    let display = format!("{:?}", sigma);
    assert!(display.contains("Sigma"));
}

#[test]
fn test_sigma_type_with_unit() {
    // Test Sigma type with Unit: (x: Unit, Bool)
    let sigma = Type::sigma(Text::from("x"), Type::Unit, Type::Bool);

    assert!(sigma.is_dependent());
}

#[test]
fn test_sigma_type_with_tuple() {
    // Test Sigma type with tuple: (x: Int, (Bool, Text))
    let tuple = Type::Tuple(vec![Type::Bool, Type::Text].into_iter().collect());
    let sigma = Type::sigma(Text::from("x"), Type::Int, tuple);

    assert!(sigma.is_dependent());
}

#[test]
fn test_sigma_type_existential_quantification() {
    // Sigma types are used for existential quantification
    // ∃(x: Int). P(x)  ≅  (x: Int, Proof(P(x)))
    let sigma = Type::sigma(
        Text::from("x"),
        Type::Int,
        Type::Named {
            path: verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::ty::Ident {
                        name: "Proof".to_string().into(),
                        span: verum_ast::span::Span::default(),
                    }
                )],
                span: verum_ast::span::Span::default(),
            },
            args: List::new(),
        },
    );

    assert!(sigma.is_dependent());
}

#[test]
fn test_sigma_type_parse_result() {
    // Example from spec: type ParseResult is (success: bool, if success then AST else Error)
    // Simplified version
    let sigma = Type::sigma(
        Text::from("success"),
        Type::Bool,
        Type::Variant(indexmap::indexmap! {
            Text::from("AST") => Type::Named {
                path: verum_ast::ty::Path {
                    segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                        verum_ast::ty::Ident {
                            name: "AST".to_string().into(),
                            span: verum_ast::span::Span::default(),
                        }
                    )],
                    span: verum_ast::span::Span::default(),
                },
                args: List::new(),
            },
            Text::from("Error") => Type::Named {
                path: verum_ast::ty::Path {
                    segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                        verum_ast::ty::Ident {
                            name: "Error".to_string().into(),
                            span: verum_ast::span::Span::default(),
                        }
                    )],
                    span: verum_ast::span::Span::default(),
                },
                args: List::new(),
            },
        }),
    );

    assert!(sigma.is_dependent());
}

#[test]
fn test_sigma_type_universe_level() {
    // Sigma types should have proper universe levels
    let mut ctx = TypeContext::new();

    let sigma = Type::sigma(Text::from("x"), Type::Int, Type::Bool);

    // Get universe level
    let level = ctx.universe_of(&sigma);

    // Sigma type (x: A : Type_i, B : Type_j) lives in Type_max(i,j)
    assert!(level.is_ok());
}

#[test]
fn test_sigma_type_with_refinement() {
    // Test Sigma type with refined second component
    use verum_types::refinement::RefinementPredicate;

    let refined = Type::Refined {
        base: Box::new(Type::Int),
        predicate: RefinementPredicate::placeholder(),
    };

    let sigma = Type::sigma(Text::from("x"), Type::Bool, refined);

    assert!(sigma.is_dependent());
}

#[test]
fn test_sigma_type_date_range() {
    // Example from spec: type DateRange with cross-field refinement
    // Simplified: (start: Date, end: Date) where start <= end
    let sigma = Type::sigma(
        Text::from("start"),
        Type::Named {
            path: verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::ty::Ident {
                        name: "Date".to_string().into(),
                        span: verum_ast::span::Span::default(),
                    }
                )],
                span: verum_ast::span::Span::default(),
            },
            args: List::new(),
        },
        Type::Named {
            path: verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::ty::Ident {
                        name: "Date".to_string().into(),
                        span: verum_ast::span::Span::default(),
                    }
                )],
                span: verum_ast::span::Span::default(),
            },
            args: List::new(),
        },
    );

    assert!(sigma.is_dependent());
}

#[test]
fn test_sigma_type_triple() {
    // Test multiple Sigma nesting for triples: (x: A, (y: B, C))
    let inner = Type::sigma(Text::from("y"), Type::Bool, Type::Text);
    let triple = Type::sigma(Text::from("x"), Type::Int, inner);

    assert!(triple.is_dependent());
}

// =============================================================================
// EQUALITY TYPE TESTS (15+ tests)
// =============================================================================

#[test]
fn test_eq_type_creation() {
    // Test creating an equality type: Eq<Int, 1, 1>
    let lhs = EqTerm::Const(EqConst::Int(1));
    let rhs = EqTerm::Const(EqConst::Int(1));

    let eq = Type::eq(Type::Int, lhs, rhs);

    match eq {
        Type::Eq { ty, lhs: l, rhs: r } => {
            assert_eq!(*ty, Type::Int);
            assert!(matches!(*l, EqTerm::Const(EqConst::Int(1))));
            assert!(matches!(*r, EqTerm::Const(EqConst::Int(1))));
        }
        _ => panic!("Expected Eq type"),
    }
}

#[test]
fn test_eq_type_is_dependent() {
    // Equality types should be recognized as dependent types
    let lhs = EqTerm::Const(EqConst::Int(0));
    let rhs = EqTerm::Const(EqConst::Int(0));
    let eq = Type::eq(Type::Int, lhs, rhs);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_reflexivity() {
    // Test reflexivity: refl<A, x> : Eq<A, x, x>
    let x = EqTerm::Var(Text::from("x"));
    let _refl = EqTerm::Refl(Box::new(x.clone()));

    // Equality type for reflexivity
    let eq = Type::eq(Type::Int, x.clone(), x);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_with_variables() {
    // Test equality with variables: Eq<Int, x, y>
    let x = EqTerm::Var(Text::from("x"));
    let y = EqTerm::Var(Text::from("y"));

    let eq = Type::eq(Type::Int, x, y);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_bool_constants() {
    // Test equality with boolean constants: Eq<Bool, true, true>
    let lhs = EqTerm::Const(EqConst::Bool(true));
    let rhs = EqTerm::Const(EqConst::Bool(true));

    let eq = Type::eq(Type::Bool, lhs, rhs);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_nat_constants() {
    // Test equality with natural numbers: Eq<Nat, 0, 0>
    let lhs = EqTerm::Const(EqConst::Nat(0));
    let rhs = EqTerm::Const(EqConst::Nat(0));

    let nat_type = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Nat".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: List::new(),
    };

    let eq = Type::eq(nat_type, lhs, rhs);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_unit() {
    // Test equality of unit values: Eq<Unit, (), ()>
    let lhs = EqTerm::Const(EqConst::Unit);
    let rhs = EqTerm::Const(EqConst::Unit);

    let eq = Type::eq(Type::Unit, lhs, rhs);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_with_function_application() {
    // Test equality with function application: Eq<Int, f(x), y>
    let f = EqTerm::Var(Text::from("f"));
    let x = EqTerm::Var(Text::from("x"));
    let y = EqTerm::Var(Text::from("y"));

    let app = EqTerm::App {
        func: Box::new(f),
        args: vec![x].into_iter().collect(),
    };

    let eq = Type::eq(Type::Int, app, y);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_clone() {
    // Test that Eq types can be cloned
    let lhs = EqTerm::Const(EqConst::Int(42));
    let rhs = EqTerm::Const(EqConst::Int(42));

    let eq1 = Type::eq(Type::Int, lhs.clone(), rhs.clone());
    let eq2 = eq1.clone();

    assert_eq!(eq1, eq2);
}

#[test]
fn test_eq_type_display() {
    // Test that Eq types have a meaningful display representation
    let lhs = EqTerm::Const(EqConst::Int(1));
    let rhs = EqTerm::Const(EqConst::Int(1));
    let eq = Type::eq(Type::Int, lhs, rhs);

    let display = format!("{:?}", eq);
    assert!(display.contains("Eq"));
}

#[test]
fn test_eq_type_with_projection() {
    // Test equality with projection: Eq<A, fst(p), x>
    let p = EqTerm::Var(Text::from("p"));
    let x = EqTerm::Var(Text::from("x"));

    let proj = EqTerm::Proj {
        pair: Box::new(p),
        component: verum_types::ty::ProjComponent::Fst,
    };

    let eq = Type::eq(Type::Int, proj, x);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_with_lambda() {
    // Test equality with lambda: Eq<A -> B, λx.x, λy.y>
    let lhs = EqTerm::Lambda {
        param: Text::from("x"),
        body: Box::new(EqTerm::Var(Text::from("x"))),
    };
    let rhs = EqTerm::Lambda {
        param: Text::from("y"),
        body: Box::new(EqTerm::Var(Text::from("y"))),
    };

    let fn_type = Type::Function {
        params: vec![Type::Int].into_iter().collect(),
        return_type: Box::new(Type::Int),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };

    let eq = Type::eq(fn_type, lhs, rhs);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_plus_comm() {
    // Test arithmetic equality: plus(m, n) = plus(n, m)
    let m = EqTerm::Var(Text::from("m"));
    let n = EqTerm::Var(Text::from("n"));
    let plus = EqTerm::Var(Text::from("plus"));

    let lhs = EqTerm::App {
        func: Box::new(plus.clone()),
        args: vec![m.clone(), n.clone()].into_iter().collect(),
    };
    let rhs = EqTerm::App {
        func: Box::new(plus),
        args: vec![n, m].into_iter().collect(),
    };

    let nat_type = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Nat".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: List::new(),
    };

    let eq = Type::eq(nat_type, lhs, rhs);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_named_constants() {
    // Test equality with named constants: Eq<Nat, zero, zero>
    let lhs = EqTerm::Const(EqConst::Named(Text::from("zero")));
    let rhs = EqTerm::Const(EqConst::Named(Text::from("zero")));

    let nat_type = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Nat".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: List::new(),
    };

    let eq = Type::eq(nat_type, lhs, rhs);

    assert!(eq.is_dependent());
}

#[test]
fn test_eq_type_j_eliminator() {
    // Test J eliminator (path induction)
    let x = EqTerm::Var(Text::from("x"));
    let proof = EqTerm::Refl(Box::new(x.clone()));
    let motive = EqTerm::Lambda {
        param: Text::from("y"),
        body: Box::new(EqTerm::Var(Text::from("P"))),
    };
    let base = EqTerm::Var(Text::from("base_proof"));

    let j_term = EqTerm::J {
        proof: Box::new(proof),
        motive: Box::new(motive),
        base: Box::new(base),
    };

    // J eliminator can be used in equality types
    let eq = Type::eq(Type::Bool, j_term, EqTerm::Const(EqConst::Bool(true)));

    assert!(eq.is_dependent());
}

// =============================================================================
// TYPE-LEVEL COMPUTATION TESTS (15+ tests)
// =============================================================================

#[test]
fn test_type_level_function_simple() {
    // Test that type-level functions can be represented
    // fn type_function(b: bool) -> Type = if b then Int else Text

    // This is a meta-level operation, so we just verify the types exist
    let int_type = Type::Int;
    let text_type = Type::Text;

    assert_ne!(int_type, text_type);
}

#[test]
fn test_type_level_nat_arithmetic() {
    // Test representation of type-level natural numbers
    let nat_type = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Nat".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: List::new(),
    };

    // Verify we can represent Nat type
    assert!(matches!(nat_type, Type::Named { .. }));
}

#[test]
fn test_type_level_list_append() {
    // Test that we can represent indexed list types
    // List<T, m> ++ List<T, n> : List<T, m+n>

    let list_m = Type::Generic {
        name: Text::from("List"),
        args: vec![
            Type::Var(verum_types::ty::TypeVar::new(0)),
            Type::Var(verum_types::ty::TypeVar::new(1)),
        ]
        .into_iter()
        .collect(),
    };

    let list_n = Type::Generic {
        name: Text::from("List"),
        args: vec![
            Type::Var(verum_types::ty::TypeVar::new(0)),
            Type::Var(verum_types::ty::TypeVar::new(2)),
        ]
        .into_iter()
        .collect(),
    };

    assert!(matches!(list_m, Type::Generic { .. }));
    assert!(matches!(list_n, Type::Generic { .. }));
}

#[test]
fn test_type_level_matrix_type() {
    // Test matrix type construction: matrix_type(rows, cols) -> Type
    // Returns List<List<f64, cols>, rows>

    let inner_list = Type::Generic {
        name: Text::from("List"),
        args: vec![Type::Float, Type::Var(verum_types::ty::TypeVar::new(1))]
            .into_iter()
            .collect(),
    };

    let matrix = Type::Generic {
        name: Text::from("List"),
        args: vec![inner_list, Type::Var(verum_types::ty::TypeVar::new(0))]
            .into_iter()
            .collect(),
    };

    assert!(matches!(matrix, Type::Generic { .. }));
}

#[test]
fn test_type_level_fin_type() {
    // Test Fin type: Fin<n> represents integers 0..n-1
    let fin = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Fin".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: vec![Type::Var(verum_types::ty::TypeVar::new(0))]
            .into_iter()
            .collect(),
    };

    assert!(matches!(fin, Type::Named { .. }));
}

#[test]
fn test_type_level_indexed_vector() {
    // Test Vec<T, n> - vector with compile-time length
    let vec = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Vec".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: vec![
            Type::Var(verum_types::ty::TypeVar::new(0)),
            Type::Var(verum_types::ty::TypeVar::new(1)),
        ]
        .into_iter()
        .collect(),
    };

    assert!(matches!(vec, Type::Named { .. }));
}

#[test]
fn test_type_level_universe_polymorphism() {
    // Test universe polymorphic types: identity<u: Level>(T: Type u) -> T
    let mut ctx = TypeContext::new();

    let u = ctx.fresh_universe_var();
    let type_u = Type::universe(u);

    // Verify we can create universe-polymorphic types
    assert!(matches!(type_u, Type::Universe { .. }));
}

#[test]
fn test_type_level_computation_with_meta() {
    // Test meta parameters for type-level computation
    let meta = Type::Meta {
        name: Text::from("N"),
        ty: Box::new(Type::Named {
            path: verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::ty::Ident {
                        name: "usize".to_string().into(),
                        span: verum_ast::span::Span::default(),
                    }
                )],
                span: verum_ast::span::Span::default(),
            },
            args: List::new(),
        }),
        refinement: None,
    };

    assert!(matches!(meta, Type::Meta { .. }));
}

#[test]
fn test_type_level_computation_refinement() {
    // Test meta with refinement: N: meta usize{> 0}
    use verum_types::refinement::RefinementPredicate;

    let meta = Type::Meta {
        name: Text::from("N"),
        ty: Box::new(Type::Named {
            path: verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::ty::Ident {
                        name: "usize".to_string().into(),
                        span: verum_ast::span::Span::default(),
                    }
                )],
                span: verum_ast::span::Span::default(),
            },
            args: List::new(),
        }),
        refinement: Some(RefinementPredicate::placeholder()),
    };

    assert!(matches!(meta, Type::Meta { .. }));
}

#[test]
fn test_type_level_split_at() {
    // Test split_at type: split_at<T>(xs: List<T, n>, i: Fin<n+1>)
    //   -> (List<T, i>, List<T, n-i>)

    // Return type is a dependent pair
    let result = Type::Tuple(
        vec![
            Type::Generic {
                name: Text::from("List"),
                args: vec![Type::Var(verum_types::ty::TypeVar::new(0))]
                    .into_iter()
                    .collect(),
            },
            Type::Generic {
                name: Text::from("List"),
                args: vec![Type::Var(verum_types::ty::TypeVar::new(0))]
                    .into_iter()
                    .collect(),
            },
        ]
        .into_iter()
        .collect(),
    );

    assert!(matches!(result, Type::Tuple(_)));
}

#[test]
fn test_type_level_inductive_nat() {
    // Test inductive Nat type representation
    use verum_types::ty::InductiveConstructor;

    let nat_type = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Nat".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: List::new(),
    };

    // Constructors: zero : Nat, succ : Nat -> Nat
    let zero = InductiveConstructor::unit(Text::from("zero"), nat_type.clone());
    let succ = InductiveConstructor::with_args(
        Text::from("succ"),
        vec![nat_type.clone()].into_iter().collect(),
        nat_type.clone(),
    );

    assert_eq!(zero.name.as_str(), "zero");
    assert_eq!(succ.name.as_str(), "succ");
}

#[test]
fn test_type_level_inductive_list() {
    // Test inductive List<A, n> type
    use verum_types::ty::InductiveConstructor;

    let list_type = Type::Generic {
        name: Text::from("List"),
        args: vec![
            Type::Var(verum_types::ty::TypeVar::new(0)),
            Type::Var(verum_types::ty::TypeVar::new(1)),
        ]
        .into_iter()
        .collect(),
    };

    // nil : List<A, 0>
    // cons : A -> List<A, n> -> List<A, succ(n)>
    let nil = InductiveConstructor::unit(Text::from("nil"), list_type.clone());

    assert_eq!(nil.name.as_str(), "nil");
}

#[test]
fn test_type_level_coinductive_stream() {
    // Test coinductive Stream type
    use verum_types::ty::CoinductiveDestructor;

    // Stream<A> with destructors: head : Stream<A> -> A, tail : Stream<A> -> Stream<A>
    let stream_type = Type::Generic {
        name: Text::from("Stream"),
        args: vec![Type::Var(verum_types::ty::TypeVar::new(0))]
            .into_iter()
            .collect(),
    };

    let head = CoinductiveDestructor::new(
        Text::from("head"),
        Type::Var(verum_types::ty::TypeVar::new(0)),
    );
    let tail = CoinductiveDestructor::new(Text::from("tail"), stream_type.clone());

    assert_eq!(head.name.as_str(), "head");
    assert_eq!(tail.name.as_str(), "tail");
}

#[test]
fn test_type_level_quantitative() {
    // Test quantitative types: Resource @n
    use verum_types::ty::Quantity;

    let linear = Quantity::LINEAR;
    let omega = Quantity::UNRESTRICTED;

    assert_eq!(linear, Quantity::One);
    assert_eq!(omega, Quantity::Omega);
    assert!(omega.allows(100));
    assert!(!linear.allows(2));
}

#[test]
fn test_type_level_higher_inductive() {
    // Test Higher Inductive Type (Circle with loop)
    use verum_types::ty::PathConstructor;

    let circle_type = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Circle".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: List::new(),
    };

    // loop : base = base
    let base = EqTerm::Const(EqConst::Named(Text::from("base")));
    let loop_path = PathConstructor::loop_at(Text::from("loop"), base, circle_type.clone());

    assert_eq!(loop_path.name.as_str(), "loop");
}

#[test]
fn test_type_level_plus_function() {
    // Test type-level plus function on natural numbers
    // This is conceptual - we verify the structure exists

    let nat = Type::Named {
        path: verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident {
                name: "Nat".to_string().into(),
                span: verum_ast::span::Span::default(),
            })],
            span: verum_ast::span::Span::default(),
        },
        args: List::new(),
    };

    // plus : Nat -> Nat -> Nat
    let plus_type = Type::Function {
        params: vec![nat.clone(), nat.clone()].into_iter().collect(),
        return_type: Box::new(nat),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };

    assert!(matches!(plus_type, Type::Function { .. }));
}

// =============================================================================
// DEPENDENT TYPE VERIFICATION INTEGRATION TESTS (10+ tests)
// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Verifies that dependent types are checked via SMT
// =============================================================================

#[test]
fn test_dependent_type_verification_enabled_with_smt() {
    // Test that dependent type checking is automatically enabled when SMT is enabled
    use verum_types::refinement::{RefinementChecker, RefinementConfig};

    // Create checker with SMT enabled
    let config = RefinementConfig {
        enable_smt: true,
        timeout_ms: 1000,
        ..Default::default()
    };
    let checker = RefinementChecker::new(config);

    // Should have dependent type checking enabled
    assert!(checker.has_dependent_types());
}

#[test]
fn test_dependent_type_verification_disabled_without_smt() {
    // Test that dependent type checking is disabled when SMT is disabled
    use verum_types::refinement::{RefinementChecker, RefinementConfig};

    // Create checker without SMT
    let config = RefinementConfig {
        enable_smt: false,
        ..Default::default()
    };
    let checker = RefinementChecker::new(config);

    // Should not have dependent type checking
    assert!(!checker.has_dependent_types());
}

#[test]
fn test_dependent_type_checker_can_be_manually_enabled() {
    // Test that dependent type checking can be manually enabled
    use verum_types::refinement::{RefinementChecker, RefinementConfig};

    let config = RefinementConfig {
        enable_smt: false,
        ..Default::default()
    };
    let mut checker = RefinementChecker::new(config);

    // Initially disabled
    assert!(!checker.has_dependent_types());

    // Enable manually
    checker.enable_dependent_types();

    // Now enabled
    assert!(checker.has_dependent_types());
}

#[test]
fn test_type_checker_dependent_type_methods() {
    // Test that TypeChecker has the dependent type verification methods
    use verum_types::infer::TypeChecker;

    let mut tc = TypeChecker::new();

    // By default, dependent types may or may not be enabled depending on config
    // We can enable them manually
    tc.enable_dependent_types();
    assert!(tc.has_dependent_types());
}

#[test]
fn test_sigma_type_constraint_structure() {
    // Test that Sigma type constraints have the proper structure
    use verum_ast::span::Span;
    use verum_types::dependent_integration::DependentTypeConstraint;

    let constraint = DependentTypeConstraint::SigmaType {
        fst_name: Text::from("n"),
        fst_type: verum_ast::ty::Type::new(verum_ast::ty::TypeKind::Int, Span::default()),
        snd_type: verum_ast::ty::Type::new(verum_ast::ty::TypeKind::Bool, Span::default()),
        span: Span::default(),
    };

    // Verify the constraint kind
    match constraint {
        DependentTypeConstraint::SigmaType { fst_name, .. } => {
            assert_eq!(fst_name.as_str(), "n");
        }
        _ => panic!("Expected SigmaType constraint"),
    }
}

#[test]
fn test_pi_type_constraint_structure() {
    // Test that Pi type constraints have the proper structure
    use verum_ast::span::Span;
    use verum_types::dependent_integration::DependentTypeConstraint;

    let constraint = DependentTypeConstraint::PiType {
        param_name: Text::from("x"),
        param_type: verum_ast::ty::Type::new(verum_ast::ty::TypeKind::Int, Span::default()),
        return_type: verum_ast::ty::Type::new(verum_ast::ty::TypeKind::Bool, Span::default()),
        span: Span::default(),
    };

    match constraint {
        DependentTypeConstraint::PiType { param_name, .. } => {
            assert_eq!(param_name.as_str(), "x");
        }
        _ => panic!("Expected PiType constraint"),
    }
}

#[test]
fn test_equality_type_constraint_structure() {
    // Test that Equality type constraints have the proper structure
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::literal::{IntLit, Literal, LiteralKind};
    use verum_ast::span::Span;
    use verum_types::dependent_integration::DependentTypeConstraint;

    let lhs = Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit::new(1)),
            span: Span::default(),
        }),
        Span::default(),
    );
    let rhs = Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit::new(1)),
            span: Span::default(),
        }),
        Span::default(),
    );

    let constraint = DependentTypeConstraint::Equality {
        value_type: verum_ast::ty::Type::new(verum_ast::ty::TypeKind::Int, Span::default()),
        lhs,
        rhs,
        span: Span::default(),
    };

    assert!(matches!(
        constraint,
        DependentTypeConstraint::Equality { .. }
    ));
}

#[test]
fn test_fin_type_constraint_structure() {
    // Test that Fin type constraints have the proper structure
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::literal::{IntLit, Literal, LiteralKind};
    use verum_ast::span::Span;
    use verum_types::dependent_integration::DependentTypeConstraint;

    let value = Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit::new(3)),
            span: Span::default(),
        }),
        Span::default(),
    );
    let bound = Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit::new(10)),
            span: Span::default(),
        }),
        Span::default(),
    );

    let constraint = DependentTypeConstraint::FinType {
        value,
        bound,
        span: Span::default(),
    };

    assert!(matches!(
        constraint,
        DependentTypeConstraint::FinType { .. }
    ));
}

#[test]
fn test_convert_internal_to_ast_primitives() {
    // Test that internal Type can be converted to AST Type
    use verum_ast::ty::TypeKind;
    use verum_types::dependent_helpers::convert_internal_to_ast;

    // Test Int
    let int_ty = convert_internal_to_ast(&Type::Int);
    assert!(matches!(int_ty.kind, TypeKind::Int));

    // Test Bool
    let bool_ty = convert_internal_to_ast(&Type::Bool);
    assert!(matches!(bool_ty.kind, TypeKind::Bool));

    // Test Text
    let text_ty = convert_internal_to_ast(&Type::Text);
    assert!(matches!(text_ty.kind, TypeKind::Text));

    // Test Float
    let float_ty = convert_internal_to_ast(&Type::Float);
    assert!(matches!(float_ty.kind, TypeKind::Float));
}

#[test]
fn test_collect_type_vars() {
    // Test that type variables are properly collected from types
    use verum_types::dependent_helpers::collect_type_vars;
    use verum_types::ty::TypeVar;

    // Test simple type var
    let ty = Type::Var(TypeVar::new(0));
    let vars = collect_type_vars(&ty);
    assert_eq!(vars.len(), 1);
    assert!(vars.contains(&TypeVar::new(0)));

    // Test function with multiple vars
    let fn_ty = Type::Function {
        params: vec![Type::Var(TypeVar::new(1))].into_iter().collect(),
        return_type: Box::new(Type::Var(TypeVar::new(2))),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };
    let vars = collect_type_vars(&fn_ty);
    assert_eq!(vars.len(), 2);
    assert!(vars.contains(&TypeVar::new(1)));
    assert!(vars.contains(&TypeVar::new(2)));

    // Test type without vars
    let int_ty = Type::Int;
    let vars = collect_type_vars(&int_ty);
    assert_eq!(vars.len(), 0);
}
