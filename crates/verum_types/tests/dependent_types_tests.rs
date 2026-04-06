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
//! Tests for dependent type checking: type narrowing, dependent functions,
//! type-level computation, length-indexed vectors, and proof-carrying code.

use indexmap::IndexMap;
use verum_common::{List, Map, Text};
use verum_types::context::{TypeContext, TypeEnv};
use verum_types::dependent_match::{ConstructorRefinement, DependentPatternChecker, Motive};
use verum_types::ty::{EqConst, EqTerm, InductiveConstructor, Type, TypeVar};
use verum_types::unify::Unifier;

// =============================================================================
// DEPENDENT FUNCTION TYPES (Pi types with value-dependent returns)
// =============================================================================

#[test]
fn test_pi_type_dependent_return_narrowing() {
    // (n: Nat) -> Vec<T, n>: return type depends on input value
    let pi = Type::pi(
        Text::from("n"),
        Type::Generic {
            name: Text::from("Nat"),
            args: List::new(),
        },
        Type::Generic {
            name: Text::from("Vec"),
            args: List::from_iter([
                Type::Generic {
                    name: Text::from("T"),
                    args: List::new(),
                },
                Type::Generic {
                    name: Text::from("n"),
                    args: List::new(),
                },
            ]),
        },
    );

    assert!(pi.is_dependent());

    // Verify the structure
    if let Type::Pi {
        param_name,
        param_type,
        return_type,
    } = &pi
    {
        assert_eq!(param_name.as_str(), "n");
        // The return type should reference 'n'
        if let Type::Generic { name, args } = return_type.as_ref() {
            assert_eq!(name.as_str(), "Vec");
            assert_eq!(args.len(), 2);
        } else {
            panic!("Expected Generic return type");
        }
    } else {
        panic!("Expected Pi type");
    }
}

#[test]
fn test_pi_type_head_function_requires_nonzero() {
    // head: (n: Nat) -> Vec<T, Succ(n)> -> T
    // The Succ(n) index ensures the vector is non-empty
    let succ_n = Type::Generic {
        name: Text::from("Succ"),
        args: List::from_iter([Type::Generic {
            name: Text::from("n"),
            args: List::new(),
        }]),
    };

    let vec_succ_n = Type::Generic {
        name: Text::from("Vec"),
        args: List::from_iter([
            Type::Generic {
                name: Text::from("T"),
                args: List::new(),
            },
            succ_n,
        ]),
    };

    let head_type = Type::pi(
        Text::from("n"),
        Type::Generic {
            name: Text::from("Nat"),
            args: List::new(),
        },
        Type::Function {
            params: List::from_iter([vec_succ_n]),
            return_type: Box::new(Type::Generic {
                name: Text::from("T"),
                args: List::new(),
            }),
            contexts: None,
            type_params: List::new(),
            properties: None,
        },
    );

    assert!(head_type.is_dependent());
}

#[test]
fn test_pi_type_zip_preserves_length() {
    // zip: (n: Nat) -> Vec<A, n> -> Vec<B, n> -> Vec<(A, B), n>
    // The same length index 'n' ensures both vectors have equal length
    let n = Type::Generic {
        name: Text::from("n"),
        args: List::new(),
    };

    let vec_a_n = Type::Generic {
        name: Text::from("Vec"),
        args: List::from_iter([
            Type::Generic {
                name: Text::from("A"),
                args: List::new(),
            },
            n.clone(),
        ]),
    };

    let vec_b_n = Type::Generic {
        name: Text::from("Vec"),
        args: List::from_iter([
            Type::Generic {
                name: Text::from("B"),
                args: List::new(),
            },
            n.clone(),
        ]),
    };

    let result_type = Type::Generic {
        name: Text::from("Vec"),
        args: List::from_iter([
            Type::Tuple(List::from_iter([
                Type::Generic {
                    name: Text::from("A"),
                    args: List::new(),
                },
                Type::Generic {
                    name: Text::from("B"),
                    args: List::new(),
                },
            ])),
            n,
        ]),
    };

    let zip_type = Type::pi(
        Text::from("n"),
        Type::Generic {
            name: Text::from("Nat"),
            args: List::new(),
        },
        Type::Function {
            params: List::from_iter([vec_a_n, vec_b_n]),
            return_type: Box::new(result_type),
            contexts: None,
            type_params: List::new(),
            properties: None,
        },
    );

    assert!(zip_type.is_dependent());
}

// =============================================================================
// TYPE-LEVEL COMPUTATION
// =============================================================================

#[test]
fn test_motive_substitution_nat_to_zero() {
    // Motive: (n: Nat) -> Vec<T, n>
    // Substituting n -> Zero should give Vec<T, Zero>
    let motive = Motive::simple(
        Text::from("n"),
        Type::Generic {
            name: Text::from("Nat"),
            args: List::new(),
        },
        Type::Generic {
            name: Text::from("Vec"),
            args: List::from_iter([
                Type::Generic {
                    name: Text::from("T"),
                    args: List::new(),
                },
                Type::Generic {
                    name: Text::from("n"),
                    args: List::new(),
                },
            ]),
        },
    );

    let zero_term = EqTerm::Const(EqConst::Named(Text::from("Zero")));
    let result = motive.apply(&zero_term);

    // After substitution, n should become Zero
    if let Type::Generic { name, args } = &result {
        assert_eq!(name.as_str(), "Vec");
        assert_eq!(args.len(), 2);
        // Second arg should be Zero (converted from term)
        if let Type::Generic { name: inner_name, .. } = &args[1] {
            assert_eq!(inner_name.as_str(), "Zero");
        } else {
            panic!("Expected Zero in second type arg");
        }
    } else {
        panic!("Expected Generic type after substitution");
    }
}

#[test]
fn test_motive_substitution_nat_to_succ() {
    // Motive: (n: Nat) -> Vec<T, n>
    // Substituting n -> Succ(m) should give Vec<T, Succ(m)>
    let motive = Motive::simple(
        Text::from("n"),
        Type::Generic {
            name: Text::from("Nat"),
            args: List::new(),
        },
        Type::Generic {
            name: Text::from("Vec"),
            args: List::from_iter([
                Type::Generic {
                    name: Text::from("T"),
                    args: List::new(),
                },
                Type::Generic {
                    name: Text::from("n"),
                    args: List::new(),
                },
            ]),
        },
    );

    // succ(m) as a term
    let succ_m = EqTerm::App {
        func: Box::new(EqTerm::Const(EqConst::Named(Text::from("Succ")))),
        args: List::from_iter([EqTerm::Var(Text::from("m"))]),
    };

    let result = motive.apply(&succ_m);

    // After substitution, n should become Succ applied to m
    if let Type::Generic { name, args } = &result {
        assert_eq!(name.as_str(), "Vec");
        assert_eq!(args.len(), 2);
        // Second arg should be Succ(m) - converted to a type-level application
        if let Type::Generic {
            name: inner_name,
            args: inner_args,
        } = &args[1]
        {
            assert_eq!(inner_name.as_str(), "Succ");
            assert_eq!(inner_args.len(), 1);
        } else {
            panic!("Expected Succ in second type arg, got {:?}", &args[1]);
        }
    } else {
        panic!("Expected Generic type after substitution");
    }
}

#[test]
fn test_motive_type_substitution() {
    // Test apply_type: substitute a type for the parameter
    let motive = Motive::simple(
        Text::from("n"),
        Type::Generic {
            name: Text::from("Nat"),
            args: List::new(),
        },
        Type::Generic {
            name: Text::from("Vec"),
            args: List::from_iter([
                Type::Generic {
                    name: Text::from("T"),
                    args: List::new(),
                },
                Type::Generic {
                    name: Text::from("n"),
                    args: List::new(),
                },
            ]),
        },
    );

    let replacement = Type::Meta {
        name: Text::from("5"),
        ty: Box::new(Type::Generic {
            name: Text::from("Nat"),
            args: List::new(),
        }),
        refinement: None,
    };

    let result = motive.apply_type(&replacement);

    if let Type::Generic { name, args } = &result {
        assert_eq!(name.as_str(), "Vec");
        // The 'n' in Vec<T, n> should now be the Meta type for 5
        if let Type::Meta { name: meta_name, .. } = &args[1] {
            assert_eq!(meta_name.as_str(), "5");
        } else {
            panic!("Expected Meta type, got {:?}", &args[1]);
        }
    }
}

#[test]
fn test_capture_avoiding_substitution_in_pi() {
    // Test that substitution doesn't go under binders that shadow the variable
    // Motive: (n: Nat) -> Pi(n: Int, Bool)
    // The inner Pi binds 'n', so substituting n in the outer scope
    // should NOT affect the inner 'n'
    let motive = Motive::simple(
        Text::from("n"),
        Type::Generic {
            name: Text::from("Nat"),
            args: List::new(),
        },
        Type::Pi {
            param_name: Text::from("n"), // shadows outer n
            param_type: Box::new(Type::Int),
            return_type: Box::new(Type::Bool),
        },
    );

    let zero_term = EqTerm::Const(EqConst::Nat(0));
    let result = motive.apply(&zero_term);

    // The Pi type should be unchanged because 'n' is shadowed
    assert!(matches!(result, Type::Pi { .. }));
    if let Type::Pi {
        param_name,
        param_type,
        return_type,
    } = &result
    {
        assert_eq!(param_name.as_str(), "n");
        assert_eq!(**param_type, Type::Int);
        assert_eq!(**return_type, Type::Bool);
    }
}

#[test]
fn test_capture_avoiding_substitution_in_sigma() {
    // Similar test for Sigma types
    let motive = Motive::simple(
        Text::from("x"),
        Type::Int,
        Type::Sigma {
            fst_name: Text::from("x"), // shadows outer x
            fst_type: Box::new(Type::Bool),
            snd_type: Box::new(Type::Text),
        },
    );

    let term = EqTerm::Const(EqConst::Int(42));
    let result = motive.apply(&term);

    // Sigma should be unchanged since x is shadowed
    if let Type::Sigma {
        fst_name,
        fst_type,
        snd_type,
    } = &result
    {
        assert_eq!(fst_name.as_str(), "x");
        assert_eq!(**fst_type, Type::Bool);
        assert_eq!(**snd_type, Type::Text);
    } else {
        panic!("Expected Sigma type unchanged");
    }
}

// =============================================================================
// LENGTH-INDEXED VECTORS
// =============================================================================

#[test]
fn test_constructor_refinement_nil_sets_zero() {
    // Matching Nil on Vec<T, n> should refine n to Zero
    let nil_ctor = InductiveConstructor {
        name: Text::from("Nil"),
        type_params: List::new(),
        args: List::new(), // nullary constructor
        return_type: Box::new(Type::Generic {
            name: Text::from("Vec"),
            args: List::from_iter([
                Type::Generic {
                    name: Text::from("T"),
                    args: List::new(),
                },
                Type::Generic {
                    name: Text::from("Zero"),
                    args: List::new(),
                },
            ]),
        }),
    };

    let scrutinee_ty = Type::Generic {
        name: Text::from("Vec"),
        args: List::from_iter([
            Type::Generic {
                name: Text::from("T"),
                args: List::new(),
            },
            Type::Generic {
                name: Text::from("n"),
                args: List::new(),
            },
        ]),
    };

    let mut refinement = ConstructorRefinement::empty(nil_ctor);

    // The index substitution from matching Nil: n -> Zero
    refinement.index_subst.insert(
        Text::from("n"),
        Type::Generic {
            name: Text::from("Zero"),
            args: List::new(),
        },
    );

    // Refine the scrutinee type: Vec<T, n> -> Vec<T, Zero>
    let refined = refinement.refine_type(&scrutinee_ty);

    if let Type::Generic { name, args } = &refined {
        assert_eq!(name.as_str(), "Vec");
        if let Type::Generic {
            name: idx_name, ..
        } = &args[1]
        {
            assert_eq!(idx_name.as_str(), "Zero");
        } else {
            panic!("Expected Zero index after refinement");
        }
    }
}

#[test]
fn test_constructor_refinement_cons_sets_succ() {
    // Matching Cons on Vec<T, n> should refine n to Succ(m)
    let cons_ctor = InductiveConstructor {
        name: Text::from("Cons"),
        type_params: List::new(),
        args: List::from_iter([
            Box::new(Type::Generic {
                name: Text::from("T"),
                args: List::new(),
            }),
            Box::new(Type::Generic {
                name: Text::from("Vec"),
                args: List::from_iter([
                    Type::Generic {
                        name: Text::from("T"),
                        args: List::new(),
                    },
                    Type::Generic {
                        name: Text::from("m"),
                        args: List::new(),
                    },
                ]),
            }),
        ]),
        return_type: Box::new(Type::Generic {
            name: Text::from("Vec"),
            args: List::from_iter([
                Type::Generic {
                    name: Text::from("T"),
                    args: List::new(),
                },
                Type::Generic {
                    name: Text::from("Succ"),
                    args: List::from_iter([Type::Generic {
                        name: Text::from("m"),
                        args: List::new(),
                    }]),
                },
            ]),
        }),
    };

    let scrutinee_ty = Type::Generic {
        name: Text::from("Vec"),
        args: List::from_iter([
            Type::Generic {
                name: Text::from("T"),
                args: List::new(),
            },
            Type::Generic {
                name: Text::from("n"),
                args: List::new(),
            },
        ]),
    };

    let mut refinement = ConstructorRefinement::empty(cons_ctor);

    // n -> Succ(m)
    refinement.index_subst.insert(
        Text::from("n"),
        Type::Generic {
            name: Text::from("Succ"),
            args: List::from_iter([Type::Generic {
                name: Text::from("m"),
                args: List::new(),
            }]),
        },
    );

    let refined = refinement.refine_type(&scrutinee_ty);

    if let Type::Generic { name, args } = &refined {
        assert_eq!(name.as_str(), "Vec");
        if let Type::Generic {
            name: idx_name,
            args: idx_args,
        } = &args[1]
        {
            assert_eq!(idx_name.as_str(), "Succ");
            assert_eq!(idx_args.len(), 1);
        } else {
            panic!("Expected Succ index after refinement");
        }
    }
}

#[test]
fn test_refinement_in_function_type() {
    // Test that refinement propagates through function types
    let mut refinement = ConstructorRefinement::empty(InductiveConstructor {
        name: Text::from("Cons"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    });

    refinement
        .index_subst
        .insert(Text::from("n"), Type::Int);

    // Function type: (Vec<T, n>) -> n
    let fn_type = Type::Function {
        params: List::from_iter([Type::Generic {
            name: Text::from("Vec"),
            args: List::from_iter([
                Type::Generic {
                    name: Text::from("T"),
                    args: List::new(),
                },
                Type::Generic {
                    name: Text::from("n"),
                    args: List::new(),
                },
            ]),
        }]),
        return_type: Box::new(Type::Generic {
            name: Text::from("n"),
            args: List::new(),
        }),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };

    let refined = refinement.refine_type(&fn_type);

    if let Type::Function {
        params,
        return_type,
        ..
    } = &refined
    {
        // Return type 'n' should be refined to Int
        assert_eq!(**return_type, Type::Int);
        // Parameter should also have 'n' refined
        if let Type::Generic { args, .. } = &params[0] {
            assert_eq!(args[1], Type::Int);
        }
    } else {
        panic!("Expected Function type");
    }
}

// =============================================================================
// DISJOINT CONSTRUCTOR DETECTION (absurd pattern detection)
// =============================================================================

#[test]
fn test_absurd_constraint_zero_vs_succ() {
    // A constraint Zero = Succ(m) should be detected as absurd
    let ctor = InductiveConstructor {
        name: Text::from("Nil"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);

    // Add constraint: Zero = Succ(m) (impossible)
    refinement.constraints.push((
        Type::Generic {
            name: Text::from("Zero"),
            args: List::new(),
        },
        Type::Generic {
            name: Text::from("Succ"),
            args: List::from_iter([Type::Generic {
                name: Text::from("m"),
                args: List::new(),
            }]),
        },
    ));

    assert!(refinement.is_absurd());
}

#[test]
fn test_absurd_constraint_none_vs_some() {
    // A constraint None = Some(x) should be detected as absurd
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);

    refinement.constraints.push((
        Type::Generic {
            name: Text::from("None"),
            args: List::new(),
        },
        Type::Generic {
            name: Text::from("Some"),
            args: List::from_iter([Type::Int]),
        },
    ));

    assert!(refinement.is_absurd());
}

#[test]
fn test_absurd_constraint_nil_vs_cons() {
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);

    refinement.constraints.push((
        Type::Generic {
            name: Text::from("Nil"),
            args: List::new(),
        },
        Type::Generic {
            name: Text::from("Cons"),
            args: List::from_iter([Type::Int]),
        },
    ));

    assert!(refinement.is_absurd());
}

#[test]
fn test_non_absurd_same_constructor() {
    // Same constructor should NOT be absurd
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);

    refinement.constraints.push((
        Type::Generic {
            name: Text::from("Succ"),
            args: List::from_iter([Type::Generic {
                name: Text::from("n"),
                args: List::new(),
            }]),
        },
        Type::Generic {
            name: Text::from("Succ"),
            args: List::from_iter([Type::Generic {
                name: Text::from("m"),
                args: List::new(),
            }]),
        },
    ));

    assert!(!refinement.is_absurd());
}

#[test]
fn test_absurd_meta_different_values() {
    // Two different concrete Meta values for the same type are contradictory
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);

    refinement.constraints.push((
        Type::Meta {
            name: Text::from("0"),
            ty: Box::new(Type::Generic {
                name: Text::from("Nat"),
                args: List::new(),
            }),
            refinement: None,
        },
        Type::Meta {
            name: Text::from("1"),
            ty: Box::new(Type::Generic {
                name: Text::from("Nat"),
                args: List::new(),
            }),
            refinement: None,
        },
    ));

    assert!(refinement.is_absurd());
}

#[test]
fn test_non_absurd_meta_same_value() {
    // Same concrete Meta value should NOT be absurd
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);

    refinement.constraints.push((
        Type::Meta {
            name: Text::from("0"),
            ty: Box::new(Type::Generic {
                name: Text::from("Nat"),
                args: List::new(),
            }),
            refinement: None,
        },
        Type::Meta {
            name: Text::from("0"),
            ty: Box::new(Type::Generic {
                name: Text::from("Nat"),
                args: List::new(),
            }),
            refinement: None,
        },
    ));

    assert!(!refinement.is_absurd());
}

// =============================================================================
// PROOF-CARRYING CODE (Equality types and the J eliminator)
// =============================================================================

#[test]
fn test_eq_term_substitution() {
    // Test that substitution works correctly in EqTerm
    let motive = Motive::simple(
        Text::from("x"),
        Type::Int,
        Type::Eq {
            ty: Box::new(Type::Int),
            lhs: Box::new(EqTerm::Var(Text::from("x"))),
            rhs: Box::new(EqTerm::Const(EqConst::Int(0))),
        },
    );

    let five = EqTerm::Const(EqConst::Int(5));
    let result = motive.apply(&five);

    // After substituting x -> 5, should get Eq<Int, 5, 0>
    if let Type::Eq { lhs, rhs, .. } = &result {
        assert!(matches!(**lhs, EqTerm::Const(EqConst::Int(5))));
        assert!(matches!(**rhs, EqTerm::Const(EqConst::Int(0))));
    } else {
        panic!("Expected Eq type after substitution");
    }
}

#[test]
fn test_eq_term_refl_to_type() {
    // Converting Refl(x) to a type should produce Eq<_, x, x>
    let motive = Motive::simple(Text::from("p"), Type::Unit, Type::Unit);

    let refl_term = EqTerm::Refl(Box::new(EqTerm::Var(Text::from("a"))));
    let ty = motive.apply(&refl_term);

    // Refl(a) converted to a type should give an Eq type
    // The term_to_type conversion handles this
    // (In the current implementation, this goes through the motive which
    // doesn't contain 'p' in its result type, so result is just Unit)
    assert_eq!(ty, Type::Unit);
}

#[test]
fn test_eq_term_lambda_substitution() {
    // Test that lambda terms have capture-avoiding substitution
    let motive = Motive::simple(
        Text::from("x"),
        Type::Int,
        Type::Eq {
            ty: Box::new(Type::Int),
            lhs: Box::new(EqTerm::Lambda {
                param: Text::from("y"),
                body: Box::new(EqTerm::Var(Text::from("x"))),
            }),
            rhs: Box::new(EqTerm::Var(Text::from("x"))),
        },
    );

    let replacement = EqTerm::Const(EqConst::Int(42));
    let result = motive.apply(&replacement);

    if let Type::Eq { lhs, rhs, .. } = &result {
        // Inside the lambda, x should be replaced with 42
        if let EqTerm::Lambda { body, .. } = lhs.as_ref() {
            assert!(matches!(**body, EqTerm::Const(EqConst::Int(42))));
        }
        // Direct reference should also be replaced
        assert!(matches!(**rhs, EqTerm::Const(EqConst::Int(42))));
    }
}

#[test]
fn test_eq_term_lambda_no_capture() {
    // Lambda binding the same variable should prevent substitution in body
    let motive = Motive::simple(
        Text::from("x"),
        Type::Int,
        Type::Eq {
            ty: Box::new(Type::Int),
            lhs: Box::new(EqTerm::Lambda {
                param: Text::from("x"), // shadows outer x
                body: Box::new(EqTerm::Var(Text::from("x"))),
            }),
            rhs: Box::new(EqTerm::Var(Text::from("x"))),
        },
    );

    let replacement = EqTerm::Const(EqConst::Int(99));
    let result = motive.apply(&replacement);

    if let Type::Eq { lhs, rhs, .. } = &result {
        // Inside the lambda with shadowing, x should NOT be replaced
        if let EqTerm::Lambda { body, .. } = lhs.as_ref() {
            assert!(matches!(**body, EqTerm::Var(_)));
        }
        // But the non-shadowed x should be replaced
        assert!(matches!(**rhs, EqTerm::Const(EqConst::Int(99))));
    }
}

#[test]
fn test_eq_term_j_substitution() {
    // J eliminator terms should also have substitution work
    let motive = Motive::simple(
        Text::from("x"),
        Type::Int,
        Type::Eq {
            ty: Box::new(Type::Int),
            lhs: Box::new(EqTerm::J {
                proof: Box::new(EqTerm::Var(Text::from("x"))),
                motive: Box::new(EqTerm::Var(Text::from("m"))),
                base: Box::new(EqTerm::Var(Text::from("x"))),
            }),
            rhs: Box::new(EqTerm::Var(Text::from("x"))),
        },
    );

    let replacement = EqTerm::Const(EqConst::Int(7));
    let result = motive.apply(&replacement);

    if let Type::Eq { lhs, rhs, .. } = &result {
        if let EqTerm::J { proof, motive: m, base } = lhs.as_ref() {
            assert!(matches!(**proof, EqTerm::Const(EqConst::Int(7))));
            // m should be unchanged (different variable)
            assert!(matches!(**m, EqTerm::Var(_)));
            assert!(matches!(**base, EqTerm::Const(EqConst::Int(7))));
        }
    }
}

#[test]
fn test_eq_term_proj_substitution() {
    // Projection terms should substitute correctly
    let motive = Motive::simple(
        Text::from("p"),
        Type::Unit,
        Type::Eq {
            ty: Box::new(Type::Int),
            lhs: Box::new(EqTerm::Proj {
                pair: Box::new(EqTerm::Var(Text::from("p"))),
                component: verum_types::ty::ProjComponent::Fst,
            }),
            rhs: Box::new(EqTerm::Var(Text::from("p"))),
        },
    );

    let replacement = EqTerm::Var(Text::from("my_pair"));
    let result = motive.apply(&replacement);

    if let Type::Eq { lhs, rhs, .. } = &result {
        if let EqTerm::Proj { pair, .. } = lhs.as_ref() {
            assert!(matches!(**pair, EqTerm::Var(_)));
            if let EqTerm::Var(name) = pair.as_ref() {
                assert_eq!(name.as_str(), "my_pair");
            }
        }
        if let EqTerm::Var(name) = rhs.as_ref() {
            assert_eq!(name.as_str(), "my_pair");
        }
    }
}

// =============================================================================
// REFINEMENT IN DEPENDENT TYPES (Pi, Sigma, Eq)
// =============================================================================

#[test]
fn test_refinement_substitution_in_pi_type() {
    // ConstructorRefinement should substitute in Pi types correctly
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);
    refinement.index_subst.insert(
        Text::from("n"),
        Type::Generic {
            name: Text::from("Zero"),
            args: List::new(),
        },
    );

    let pi_type = Type::Pi {
        param_name: Text::from("x"),
        param_type: Box::new(Type::Generic {
            name: Text::from("n"),
            args: List::new(),
        }),
        return_type: Box::new(Type::Bool),
    };

    let refined = refinement.refine_type(&pi_type);

    if let Type::Pi { param_type, .. } = &refined {
        if let Type::Generic { name, .. } = param_type.as_ref() {
            assert_eq!(name.as_str(), "Zero");
        } else {
            panic!("Expected Zero after refinement in Pi param");
        }
    }
}

#[test]
fn test_refinement_substitution_in_sigma_type() {
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);
    refinement
        .index_subst
        .insert(Text::from("n"), Type::Int);

    let sigma_type = Type::Sigma {
        fst_name: Text::from("x"),
        fst_type: Box::new(Type::Generic {
            name: Text::from("n"),
            args: List::new(),
        }),
        snd_type: Box::new(Type::Bool),
    };

    let refined = refinement.refine_type(&sigma_type);

    if let Type::Sigma { fst_type, .. } = &refined {
        assert_eq!(**fst_type, Type::Int);
    }
}

#[test]
fn test_refinement_pi_shadowing() {
    // If Pi binds the same variable, refinement should NOT substitute under it
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);
    refinement
        .index_subst
        .insert(Text::from("n"), Type::Int);

    let pi_type = Type::Pi {
        param_name: Text::from("n"), // shadows the substitution variable
        param_type: Box::new(Type::Bool),
        return_type: Box::new(Type::Generic {
            name: Text::from("n"),
            args: List::new(),
        }),
    };

    let refined = refinement.refine_type(&pi_type);

    // Should be unchanged -- the Pi binder shadows 'n'
    assert_eq!(refined, pi_type);
}

#[test]
fn test_refinement_eq_type() {
    // Refinement in Eq types should substitute in the base type
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);
    refinement
        .index_subst
        .insert(Text::from("A"), Type::Int);

    let eq_type = Type::Eq {
        ty: Box::new(Type::Generic {
            name: Text::from("A"),
            args: List::new(),
        }),
        lhs: Box::new(EqTerm::Var(Text::from("x"))),
        rhs: Box::new(EqTerm::Var(Text::from("y"))),
    };

    let refined = refinement.refine_type(&eq_type);

    if let Type::Eq { ty, .. } = &refined {
        // The base type A should be refined to Int
        assert_eq!(**ty, Type::Int);
    }
}

// =============================================================================
// TERM-TO-TYPE CONVERSION
// =============================================================================

#[test]
fn test_term_to_type_var() {
    let motive = Motive::simple(Text::from("_"), Type::Unit, Type::Unit);
    let term = EqTerm::Var(Text::from("x"));
    let result = motive.apply(&term);
    // apply doesn't use the term directly on result_ty unless result_ty references param
    // So we test term_to_type indirectly through a substitution
    let m2 = Motive::simple(
        Text::from("v"),
        Type::Unit,
        Type::Generic {
            name: Text::from("v"),
            args: List::new(),
        },
    );
    let r2 = m2.apply(&EqTerm::Var(Text::from("hello")));
    if let Type::Generic { name, .. } = &r2 {
        assert_eq!(name.as_str(), "hello");
    }
}

#[test]
fn test_term_to_type_int_const() {
    let m = Motive::simple(
        Text::from("v"),
        Type::Unit,
        Type::Generic {
            name: Text::from("v"),
            args: List::new(),
        },
    );
    let r = m.apply(&EqTerm::Const(EqConst::Int(42)));
    // Int constant should become Meta type
    if let Type::Meta { name, ty, .. } = &r {
        assert_eq!(name.as_str(), "42");
        assert_eq!(**ty, Type::Int);
    } else {
        panic!("Expected Meta type for int const, got {:?}", r);
    }
}

#[test]
fn test_term_to_type_nat_const() {
    let m = Motive::simple(
        Text::from("v"),
        Type::Unit,
        Type::Generic {
            name: Text::from("v"),
            args: List::new(),
        },
    );
    let r = m.apply(&EqTerm::Const(EqConst::Nat(3)));
    if let Type::Meta { name, ty, .. } = &r {
        assert_eq!(name.as_str(), "3");
        if let Type::Generic { name: inner, .. } = ty.as_ref() {
            assert_eq!(inner.as_str(), "Nat");
        }
    } else {
        panic!("Expected Meta type for nat const");
    }
}

#[test]
fn test_term_to_type_bool_const() {
    let m = Motive::simple(
        Text::from("v"),
        Type::Unit,
        Type::Generic {
            name: Text::from("v"),
            args: List::new(),
        },
    );
    let r = m.apply(&EqTerm::Const(EqConst::Bool(true)));
    if let Type::Meta { name, ty, .. } = &r {
        assert_eq!(name.as_str(), "true");
        assert_eq!(**ty, Type::Bool);
    }
}

#[test]
fn test_term_to_type_unit_const() {
    let m = Motive::simple(
        Text::from("v"),
        Type::Unit,
        Type::Generic {
            name: Text::from("v"),
            args: List::new(),
        },
    );
    let r = m.apply(&EqTerm::Const(EqConst::Unit));
    assert_eq!(r, Type::Unit);
}

// =============================================================================
// INDICES COMPATIBILITY
// =============================================================================

#[test]
fn test_indices_incompatible_zero_vs_succ() {
    // Vec<T, Zero> and Vec<T, Succ(n)> have incompatible indices
    let zero_idx = Type::Generic {
        name: Text::from("Zero"),
        args: List::new(),
    };
    let succ_idx = Type::Generic {
        name: Text::from("Succ"),
        args: List::from_iter([Type::Generic {
            name: Text::from("n"),
            args: List::new(),
        }]),
    };

    // Use the ConstructorRefinement to test
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };
    let mut refinement = ConstructorRefinement::empty(ctor);
    refinement.constraints.push((zero_idx, succ_idx));

    assert!(refinement.is_absurd());
}

#[test]
fn test_indices_compatible_succ_vs_succ() {
    // Succ(n) and Succ(m) are compatible (they unify when n = m)
    let succ_n = Type::Generic {
        name: Text::from("Succ"),
        args: List::from_iter([Type::Generic {
            name: Text::from("n"),
            args: List::new(),
        }]),
    };
    let succ_m = Type::Generic {
        name: Text::from("Succ"),
        args: List::from_iter([Type::Generic {
            name: Text::from("m"),
            args: List::new(),
        }]),
    };

    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };
    let mut refinement = ConstructorRefinement::empty(ctor);
    refinement.constraints.push((succ_n, succ_m));

    assert!(!refinement.is_absurd());
}

// =============================================================================
// ADDITIONAL DEPENDENT FAMILIES
// =============================================================================

#[test]
fn test_absurd_ok_vs_err() {
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);
    refinement.constraints.push((
        Type::Generic {
            name: Text::from("Ok"),
            args: List::from_iter([Type::Int]),
        },
        Type::Generic {
            name: Text::from("Err"),
            args: List::from_iter([Type::Text]),
        },
    ));

    assert!(refinement.is_absurd());
}

#[test]
fn test_absurd_leaf_vs_node() {
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);
    refinement.constraints.push((
        Type::Generic {
            name: Text::from("Leaf"),
            args: List::from_iter([Type::Int]),
        },
        Type::Generic {
            name: Text::from("Node"),
            args: List::new(),
        },
    ));

    assert!(refinement.is_absurd());
}

#[test]
fn test_non_absurd_unknown_constructors() {
    // Constructors not in known families should NOT be considered absurd
    let ctor = InductiveConstructor {
        name: Text::from("test"),
        type_params: List::new(),
        args: List::new(),
        return_type: Box::new(Type::Unit),
    };

    let mut refinement = ConstructorRefinement::empty(ctor);
    refinement.constraints.push((
        Type::Generic {
            name: Text::from("Foo"),
            args: List::new(),
        },
        Type::Generic {
            name: Text::from("Bar"),
            args: List::new(),
        },
    ));

    // Unknown constructors -- conservative, NOT absurd
    assert!(!refinement.is_absurd());
}
