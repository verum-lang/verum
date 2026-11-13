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
