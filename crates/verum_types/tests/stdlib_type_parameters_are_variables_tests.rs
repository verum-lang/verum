//! A type's parameters are VARIABLES, and there is one of each per type.
//!
//! `__type_params_<Name>` records a generic type's parameters in
//! declaration order. It is read by two different consumers for two
//! different things: the ordered NAMES drive by-name substitution, and
//! each stored VALUE, when it is a type variable, supplies the `T<id>`
//! key that `substitute_type_params` actually matches a payload on.
//!
//! The metadata registrar wrote the names correctly and stored the
//! literal `Type::Int` for every value, on the reading that only the
//! names are ever consulted. What that cost (T0891): a sibling module's
//! `Result<Int, EA>` came back from
//! `try_build_variant_from_constructors` as
//!
//! ```text
//! Variant({Ok: Int, Err: Var(18340)})
//! ```
//!
//! — the first parameter substituted by name, the second left standing,
//! with `EA`'s full variant body sitting unused in the type arguments.
//! A complete match over `EA`'s constructors was then refused as
//! non-exhaustive, and the workaround an author reaches for is the
//! catch-all `Err(_) => …` that exhaustiveness exists to make
//! unnecessary.
//!
//! A placeholder is a fact stated wrongly. The consumer had no way to
//! tell it from a real answer, which is why these tests assert the
//! SHAPE of what is stored and not merely that something is.
//!
//! The second test covers the other half. Recording a parameter requires
//! having one variable to record, and the registrar minted a fresh set
//! per CASE — so `Result`'s `T` in `Ok(T)` and `Result`'s `T` were two
//! different variables that happened to share a spelling. Beyond
//! blocking the fix, that is wrong on its own terms:
//! `type Pair<T> is Both(T, T) | One(T)` could not force its three
//! payloads to agree.

use verum_common::{List, Maybe, Text};
use verum_types::TypeChecker;
use verum_types::core_metadata::{
    CoreMetadata, GenericParam, TypeDescriptor, TypeDescriptorKind, VariantCase, VariantPayload,
};
use verum_types::ty::Type;

fn param(name: &str) -> GenericParam {
    GenericParam {
        name: name.into(),
        bounds: List::new(),
        default: Maybe::None,
        type_bounds: List::new(),
        pid: Maybe::None,
    }
}

fn tuple_case(name: &str, payload: &[&str]) -> VariantCase {
    VariantCase {
        name: name.into(),
        payload: Maybe::Some(VariantPayload::Tuple(
            payload.iter().map(|t| Text::from(*t)).collect(),
        )),
    }
}

/// A variant descriptor as the precompiled archive delivers one.
fn variant_descriptor(name: &str, params: &[&str], cases: List<VariantCase>) -> TypeDescriptor {
    TypeDescriptor {
        name: name.into(),
        module_path: "test.module".into(),
        origin_module_path: Maybe::None,
        generic_params: params.iter().map(|p| param(p)).collect(),
        kind: TypeDescriptorKind::Variant { cases },
        size: Maybe::None,
        alignment: Maybe::None,
        methods: List::new(),
        implements: List::new(),
        decl_span: Maybe::None,
        is_transparent_wrapper: false,
    }
}

fn checker_with(descriptor: TypeDescriptor) -> TypeChecker {
    let mut metadata = CoreMetadata::default();
    let name = descriptor.name.clone();
    metadata.types.insert(name.clone(), descriptor);
    metadata.type_declaration_order.push(name);
    TypeChecker::new_with_core_eager(std::sync::Arc::new(metadata))
}

/// The recorded parameters of `Outcome<T, E>`, in declaration order.
fn recorded_params(checker: &TypeChecker, type_name: &str) -> Vec<(String, Type)> {
    match checker.lookup_type_for_testing(&format!("__type_params_{}", type_name)) {
        Some(Type::Record(fields)) => fields
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.clone()))
            .collect(),
        other => panic!("__type_params_{} is {:?}, expected a record", type_name, other),
    }
}

#[test]
fn every_recorded_parameter_is_a_type_variable() {
    let checker = checker_with(variant_descriptor(
        "Outcome",
        &["T", "E"],
        List::from_iter([tuple_case("Fine", &["T"]), tuple_case("Bad", &["E"])]),
    ));

    let params = recorded_params(&checker, "Outcome");
    assert_eq!(
        params.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        vec!["T", "E"],
        "the parameter names, in declaration order"
    );

    for (name, ty) in params.iter() {
        assert!(
            matches!(ty, Type::Var(_)),
            "parameter `{}` is recorded as {:?}; a parameter is a VARIABLE, and a \
             placeholder here silently disables the `T<id>` substitution key \
             (T0891 — the second type argument stopped being substituted at all)",
            name,
            ty
        );
    }

    let ids: Vec<usize> = params
        .iter()
        .map(|(_, ty)| match ty {
            Type::Var(tv) => tv.id(),
            _ => unreachable!(),
        })
        .collect();
    assert_ne!(
        ids[0], ids[1],
        "two distinct parameters must be two distinct variables"
    );
}

#[test]
fn one_variable_answers_for_a_parameter_in_every_case() {
    // `Pair<T> is Both(T, T) | One(T)` — the parameter appears three
    // times across two cases, and all three are the same `T`.
    let checker = checker_with(variant_descriptor(
        "Pair",
        &["T"],
        List::from_iter([tuple_case("Both", &["T", "T"]), tuple_case("One", &["T"])]),
    ));

    let recorded = match recorded_params(&checker, "Pair").pop() {
        Some((_, Type::Var(tv))) => tv.id(),
        other => panic!("Pair's parameter is recorded as {:?}", other),
    };

    let payloads = checker.constructor_payloads_for_testing("Pair");
    assert_eq!(payloads.len(), 2, "two constructors: {:?}", payloads);

    let mut seen: Vec<usize> = Vec::new();
    for (case, args) in payloads.iter() {
        for (i, arg) in args.iter().enumerate() {
            match arg {
                Type::Var(tv) => seen.push(tv.id()),
                other => panic!("{}'s payload {} is {:?}, expected a variable", case, i, other),
            }
        }
    }
    assert_eq!(seen.len(), 3, "three payload occurrences of `T`");

    for id in seen.iter() {
        assert_eq!(
            *id, recorded,
            "every occurrence of `T` must be the SAME variable the type records \
             for its parameter; per-case variables mean `Pair<Int>` cannot force \
             its own payloads to agree, and leave the parameter with no \
             representative to record"
        );
    }
}
