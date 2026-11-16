//! Numeric classification does not grant atomic representation or target support.
use super::*;

#[test]
fn primitive_layout_and_numeric_bounds_use_the_actual_source_implementations() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for (ty, bound, accepted) in [
        ("Int", "Sized", true), ("UInt64", "Sized", true),
        ("Int128", "Sized", true), ("UInt128", "Sized", true),
        ("Float", "Sized", true), ("Float32", "Sized", true),
        ("Bool", "Sized", true), ("Char", "Sized", true), ("()", "Sized", true),
        ("Int128", "Integer", true), ("UInt128", "Integer", true),
        ("Int128", "Atomic", false), ("UInt128", "Atomic", false),
        ("Float", "Integer", false), ("Bool", "Integer", false),
        ("Float", "Atomic", false), ("UInt64", "Atomic", true),
        ("Bool", "Atomic", true), ("Pretend", "Sized", false),
    ] {
        let source = format!(r#"
            module capability_client;
            mount core.base.protocols.{{Sized, Integer, Atomic}};
            type Pretend is {{ value: Int }};
            fn require<T: {bound}>() {{}}
            fn probe() {{ require<{ty}>(); }}
        "#);
        let module = Parser::new(&source).parse_module().unwrap();
        let result = check_source_module(&mut phase.configured_checker(), "capability_client", &module, &registry);
        if accepted {
            result.unwrap_or_else(|errors| panic!("{ty}: {bound}: {errors:#?}"));
        } else {
            let errors = result.expect_err("numeric or layout assumptions invented an implementation");
            assert!(errors.iter().any(|error| error.code() == Some("E405") && error.message().contains(bound)), "{ty}: {bound}: {errors:#?}");
        }
    }
}

#[test]
fn atomic_calls_enforce_numeric_and_representation_bounds() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for (ty, accepted) in [("UInt64", true), ("Int128", false), ("UInt128", false), ("Float", false), ("Bool", false)] {
        let source = format!(r#"
            module atomic_client;
            mount core.base.protocols.{{Integer, Atomic}};
            mount core.intrinsics.atomic.{{atomic_add, MemoryOrder}};
            fn add(value: &mut {ty}, amount: {ty})->{ty} {{
                atomic_add(value, amount, MemoryOrder.Relaxed)
            }}
        "#);
        let module = Parser::new(&source).parse_module().unwrap();
        let result = check_source_module(&mut phase.configured_checker(), "atomic_client", &module, &registry);
        if accepted {
            result.unwrap_or_else(|errors| panic!("{ty}: {errors:#?}"));
        } else {
            let errors = result.expect_err("an atomic caller omitted a declared bound");
            assert!(errors.iter().any(|error| error.code() == Some("E405")), "{ty}: {errors:#?}");
        }
    }
}
