use super::*;
use crate::types::{CbgrTier, Mutability, TypeParamId};

fn reference(inner: TypeRef, mutability: Mutability, tier: CbgrTier) -> TypeRef {
    TypeRef::Reference {
        inner: Box::new(inner),
        mutability,
        tier,
    }
}

fn compiler() -> (VbcCodegen, FunctionInfo) {
    let module =
        verum_fast_parser::Parser::new("module operand; fn identity<T>(value: T) -> T { value }")
            .parse_module()
            .unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("operand"));
    compiler.collect_unit_declarations(&[&module]).unwrap();
    let function = compiler
        .ctx
        .lookup_qualified_function("operand.identity")
        .unwrap()
        .clone();
    (compiler, function)
}

#[test]
fn shadowed_names_do_not_remove_or_solve_the_enclosing_declaration_slots() {
    let (mut compiler, function) = compiler();
    compiler.ctx.current_function = Some("operand.identity".into());
    compiler.ctx.current_generic_param_ids.insert("T".into(), 7);
    let enclosing = TypeRef::Generic(function.type_param_ids[0]);
    let shadow = TypeRef::Generic(TypeParamId(7));
    assert!(compiler.declared_type_in_scope(&enclosing));
    assert!(compiler.declared_type_in_scope(&shadow));
    assert!(!compiler.declared_type_in_scope(&TypeRef::Generic(TypeParamId(9))));
    let mut bindings = TypeSubstitution::empty();
    bindings.bind(TypeParamId(0), enclosing.clone());
    assert!(!compiler.bind_source_declaration(&TypeRef::Generic(TypeParamId(0)), &shadow, &mut bindings));
    assert_eq!(bindings.get(TypeParamId(0)), Some(&enclosing));
}

#[test]
fn outer_reference_adaptation_keeps_exact_pointee_witness() {
    let (mut compiler, function) = compiler();
    compiler
        .ctx
        .current_generic_param_ids
        .insert("Caller".into(), 7);
    let formal = reference(
        TypeRef::Generic(TypeParamId(0)),
        Mutability::Immutable,
        CbgrTier::Tier2,
    );
    for tier in [CbgrTier::Tier0, CbgrTier::Tier1, CbgrTier::Tier2] {
        for access in [Mutability::Immutable, Mutability::Mutable] {
            let actual = reference(TypeRef::Generic(TypeParamId(7)), access, tier);
            let mut bindings = TypeSubstitution::empty();
            assert!(compiler
                .bind_declared_operand(&function, 0, &formal, &actual, &mut bindings)
                .is_some());
            assert_eq!(
                bindings.get(TypeParamId(0)),
                Some(&TypeRef::Generic(TypeParamId(7)))
            );
            assert!(
                !compiler.bind_source_declaration(&formal, &actual, &mut TypeSubstitution::empty())
                    || (tier == CbgrTier::Tier2 && access == Mutability::Immutable)
            );
        }
    }
}

#[test]
fn operand_matching_cannot_create_mutability_or_capture_an_unbound_caller() {
    let (compiler, function) = compiler();
    let formal = reference(
        TypeRef::Generic(TypeParamId(0)),
        Mutability::Mutable,
        CbgrTier::Tier2,
    );
    for actual in [
        reference(
            TypeRef::Concrete(TypeId::INT),
            Mutability::Immutable,
            CbgrTier::Tier0,
        ),
        reference(
            TypeRef::Generic(TypeParamId(7)),
            Mutability::Mutable,
            CbgrTier::Tier2,
        ),
    ] {
        let mut bindings = TypeSubstitution::empty();
        assert!(compiler
            .bind_declared_operand(&function, 0, &formal, &actual, &mut bindings)
            .is_none());
        assert!(bindings.get(TypeParamId(0)).is_none());
    }
}

#[test]
fn nested_reference_qualifiers_and_existing_bindings_remain_exact() {
    let (compiler, function) = compiler();
    let inner = reference(
        TypeRef::Generic(TypeParamId(0)),
        Mutability::Immutable,
        CbgrTier::Tier2,
    );
    let formal = reference(inner, Mutability::Immutable, CbgrTier::Tier2);
    let actual = reference(
        reference(
            TypeRef::Concrete(TypeId::INT),
            Mutability::Immutable,
            CbgrTier::Tier0,
        ),
        Mutability::Mutable,
        CbgrTier::Tier0,
    );
    assert!(compiler
        .bind_declared_operand(
            &function,
            0,
            &formal,
            &actual,
            &mut TypeSubstitution::empty()
        )
        .is_none());

    let formal = reference(
        TypeRef::Generic(TypeParamId(0)),
        Mutability::Immutable,
        CbgrTier::Tier2,
    );
    let actual = reference(
        TypeRef::Concrete(TypeId::INT),
        Mutability::Mutable,
        CbgrTier::Tier0,
    );
    let mut bindings = TypeSubstitution::empty();
    bindings.bind(TypeParamId(0), TypeRef::Concrete(TypeId::BOOL));
    assert!(compiler
        .bind_declared_operand(&function, 0, &formal, &actual, &mut bindings)
        .is_none());
    assert_eq!(
        bindings.get(TypeParamId(0)),
        Some(&TypeRef::Concrete(TypeId::BOOL))
    );
}

fn member(base: TypeRef) -> TypeRef {
    TypeRef::AssociatedProjection {
        base: Box::new(base), qualifier: None, assoc: "Item".into(), args: Default::default(),
    }
}

#[test]
fn associated_operand_comparison_uses_the_bound_callers_scope() {
    let (mut compiler, _) = compiler();
    for (name, id) in [("First", 0), ("Second", 7)] {
        compiler.ctx.current_generic_param_ids.insert(name.into(), id);
    }
    let formal = member(TypeRef::Generic(TypeParamId(0)));
    let actual = member(TypeRef::Generic(TypeParamId(7)));
    let mut bindings = TypeSubstitution::empty();
    bindings.bind(TypeParamId(0), TypeRef::Generic(TypeParamId(7)));
    assert!(compiler.bind_source_declaration(&formal, &actual, &mut bindings));
    // Equal numeric IDs are not equal declarations across the call boundary.
    assert!(!compiler.bind_source_declaration(&formal, &formal, &mut bindings));
    assert_eq!(bindings.get(TypeParamId(0)), Some(&TypeRef::Generic(TypeParamId(7))));

    let mut missing = TypeSubstitution::empty();
    assert!(!compiler.bind_source_declaration(&formal, &formal, &mut missing));
    assert!(missing.is_empty(), "an Item equality cannot infer its receiver");
}

#[test]
fn bound_operand_normalization_preserves_array_compatibility_and_rigid_callers() {
    let (mut compiler, _) = compiler();
    for (name, id) in [("First", 0), ("Second", 7)] {
        compiler.ctx.current_generic_param_ids.insert(name.into(), id);
    }
    let formal = TypeRef::instantiated(TypeId::ARRAY, [TypeRef::Generic(TypeParamId(0))]);
    let mut bindings = TypeSubstitution::empty();
    bindings.bind(TypeParamId(0), TypeRef::Generic(TypeParamId(7)));
    for (id, accepted) in [(7, true), (0, false)] {
        let actual = TypeRef::Array { element: Box::new(TypeRef::Generic(TypeParamId(id))), length: 3 };
        assert_eq!(compiler.bind_source_declaration(&formal, &actual, &mut bindings), accepted);
    }
    assert_eq!(bindings.get(TypeParamId(0)), Some(&TypeRef::Generic(TypeParamId(7))));
}

#[test]
fn callable_allowance_keeps_the_selected_producer_witness_and_direction() {
    let (compiler, function) = compiler();
    let callable = |is_async, argument| TypeRef::Function {
        is_async, params: [argument].into_iter().collect(),
        return_type: Box::new(TypeRef::Concrete(TypeId::INT)), contexts: Default::default(),
    };
    for expected_async in [false, true] {
        for actual_async in [false, true] {
            let formal = callable(expected_async, TypeRef::Generic(TypeParamId(0)));
            let actual = callable(actual_async, TypeRef::Concrete(TypeId::INT));
            let mut bindings = TypeSubstitution::empty();
            let accepted = compiler.bind_declared_operand(&function, 0, &formal, &actual, &mut bindings);
            assert_eq!(accepted.is_some(), expected_async || !actual_async);
            if accepted.is_some() {
                assert_eq!(bindings.get(TypeParamId(0)), Some(&TypeRef::Concrete(TypeId::INT)));
            } else {
                assert!(bindings.is_empty(), "rejection cannot publish a partial witness");
            }
        }
    }
}
