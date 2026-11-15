//! Numeric extrema need a scalar declaration, not a marker protocol.
use verum_ast::Span;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, TypeError};

#[derive(Default)]
struct Problems {
    errors: List<Text>,
    bound_spans: List<Span>,
}

impl Problems {
    fn record(&mut self, error: &TypeError) {
        if let TypeError::OtherWithCodeSpanned { code, span, .. } = error {
            if code == "E428" {
                self.bound_spans.push(*span);
            }
        }
        self.errors.push(format!("{error:?}").into());
    }
}

fn check(source: &str) -> Problems {
    check_in_owner(source, "numeric_properties")
}

fn check_in_owner(source: &str, owner: &str) -> Problems {
    let ast = Parser::new(source)
        .parse_module()
        .expect("canonical source grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path(owner);
    let mut problems = Problems::default();
    for error in checker.prepare_module_declarations(&ast) {
        problems.record(&error);
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            problems.record(&error);
        }
    }
    for error in checker.take_deferred_errors() {
        problems.record(&error);
    }
    for error in checker.diagnostic_sources() {
        problems.record(error);
    }
    problems
}

fn accepted(source: &str) {
    let problems = check(source);
    assert!(
        problems.errors.is_empty(),
        "{source}: {:?}",
        problems.errors
    );
}

fn refused_bound(source: &str, property: &str) {
    refused_bound_in_owner(source, property, "numeric_properties");
}

fn refused_bound_in_owner(source: &str, property: &str, owner: &str) {
    let problems = check_in_owner(source, owner);
    assert!(
        problems.bound_spans.iter().any(|span| {
            source
                .get(span.start as usize..span.end as usize)
                .is_some_and(|query| query.ends_with(property))
        }),
        "expected a located numeric-bound refusal: {source}: {:?}",
        problems.errors
    );
}

#[test]
fn scalar_extrema_keep_the_declared_result_type() {
    for name in [
        "Int", "Int8", "Int16", "Int32", "Int64", "Int128", "ISize", "Byte", "UInt8", "UInt16",
        "UInt32", "UInt64", "UInt128", "USize", "Float", "Float32", "Float64", "i8", "u64", "f32",
    ] {
        for property in ["min", "max"] {
            accepted(&format!("fn probe()->{name} {{{name}.{property}}}"));
        }
    }
}

#[test]
fn transparent_aliases_keep_their_numeric_target() {
    for property in ["min", "max"] {
        for source in [
            format!(
                "type Wide is UInt128; type Alias is Wide; fn probe()->Alias {{Alias.{property}}}"
            ),
            format!("type Scalar<T> is T; fn probe()->Scalar<Int16> {{Scalar<Int16>.{property}}}"),
            format!("type Real is Float32; fn probe()->Real {{Real.{property}}}"),
        ] {
            accepted(&source);
        }
    }
    let problems = check("type Wide is UInt128; fn probe()->Bool {Wide.max}");
    assert!(
        problems
            .errors
            .iter()
            .any(|error| error.contains("Mismatch")),
        "{:?}",
        problems.errors
    );
}

#[test]
fn generic_extrema_stay_deferred_until_the_type_is_known() {
    for property in ["min", "max"] {
        accepted(&format!(
            "fn bound<T>()->T {{T.{property}}}
            fn narrow()->UInt8 {{bound<UInt8>()}}
            fn wide()->Int128 {{bound<Int128>()}}"
        ));
        accepted(&format!(
            "type Identity<T> is T;
            fn bound<T>()->T {{Identity<T>.{property}}}"
        ));
        accepted(&format!(
            "type Numeric is protocol {{}};
            implement Numeric for UInt8 {{}}
            fn bound<T: Numeric>()->T {{T.{property}}}
            fn probe()->UInt8 {{bound<UInt8>()}}"
        ));
    }
}

#[test]
fn a_numeric_marker_cannot_supply_a_scalar_carrier() {
    for (declaration, name) in [
        ("type Token is { tag: Int };", "Token"),
        ("type Token is ();", "Token"),
        ("", "Bool"),
        ("", "Char"),
    ] {
        for property in ["min", "max"] {
            for owner in ["numeric_properties", ""] {
                refused_bound_in_owner(
                    &format!(
                        "type Numeric is protocol {{}}; {declaration}
                        implement Numeric for {name} {{}}
                        fn probe()->{name} {{{name}.{property}}}"
                    ),
                    property,
                    owner,
                );
            }
        }
    }
}

#[test]
fn aliases_of_nonnumeric_declarations_are_refused() {
    for target in ["Bool", "Char", "Token", "(Int, Int)"] {
        for property in ["min", "max"] {
            refused_bound(
                &format!(
                    "type Token is {{ value: Int }};
                type Alias is {target}; fn probe()->Alias {{Alias.{property}}}"
                ),
                property,
            );
        }
    }
}

#[test]
fn a_nominal_numeric_spelling_does_not_borrow_primitive_bounds() {
    for name in ["Int16", "UInt32", "Float32"] {
        for property in ["min", "max"] {
            refused_bound(
                &format!(
                    "type {name} is {{ value: Bool }};
                type Alias is {name}; fn probe()->Alias {{Alias.{property}}}"
                ),
                property,
            );
        }
    }
}

#[test]
fn nonnumeric_layout_properties_still_typecheck() {
    accepted(
        "fn boolean()->Int {Bool.bits} fn character()->Int {Char.bits}
        type Token is {value: Int}; fn record()->Int {Token.size}",
    );
}
