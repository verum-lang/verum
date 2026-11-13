//! Universal associated contracts must hold for every permitted family argument.
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> Vec<String> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.set_generic_associated_types_enabled(true);
    checker.register_primitives();
    checker.set_current_module_path("families");
    let mut errors: Vec<String> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| format!("{:?}", error.to_diagnostic()))
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{:?}", error.to_diagnostic()));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|error| format!("{:?}", error.to_diagnostic())),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|error| format!("{error:?}")),
    );
    errors
}
fn source(
    item: &str,
    family_parameter: &str,
    implementation_parameter: &str,
    condition: &str,
) -> String {
    format!(
        r#"module families;
        type Row is protocol {{}};
        type Stream is protocol {{type Item;}};
        type Connection is protocol {{type Rows<{family_parameter}>: Stream<Item=R>;}};
        type Cursor<T> is {{value:T}};
        implement<T{condition}> Stream for Cursor<T> {{type Item={item};}}
        type Db is {{}};
        implement Connection for Db {{type Rows<{implementation_parameter}> =Cursor<R>;}}
    "#
    )
}
fn accept(source: &str) {
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:#?}");
}
fn reject(source: &str) {
    let errors = errors(source);
    assert!(
        !errors.is_empty(),
        "invalid associated contract was accepted"
    );
    assert!(
        errors.iter().any(|error| error.contains("E405")
            || error.contains("associated")
            || error.contains("implementation")),
        "{errors:#?}"
    );
}

#[test]
fn unconstrained_family_and_matching_item_pass() {
    accept(&source("T", "R", "R", ""));
}
#[test]
fn wrong_associated_item_is_rejected() {
    reject(&source("Text", "R", "R", ""));
}
#[test]
fn conditional_impl_cannot_prove_an_unconstrained_family() {
    reject(&source("T", "R", "R", ": Row"));
}
#[test]
fn declared_parameter_assumption_proves_conditional_impl() {
    accept(&source("T", "R: Row", "R: Row", ": Row"));
}
#[test]
fn implementation_cannot_narrow_a_family_parameter() {
    reject(&source("T", "R", "R: Row", ": Row"));
}
#[test]
fn missing_stream_implementation_is_rejected() {
    reject(
        r#"module families;
    type Stream is protocol {type Item;};type Connection is protocol {type Rows<R>:Stream<Item=R>;};
    type Cursor<T> is {value:T};type Db is {};implement Connection for Db {type Rows<R> =Cursor<R>;}
"#,
    );
}
#[test]
fn alpha_renaming_keeps_owner_and_family_variables_distinct() {
    accept(
        r#"module families;
    type Stream<A> is protocol {type Item;};
    type Connection<A> is protocol {type Rows<R>:Stream<A,Item=R>;fn identity<R>(&self,value:R)->R {value}};
    type Cursor<A,R> is {owner:A,value:R};
    implement<A,R> Stream<A> for Cursor<A,R> {type Item=R;}
    type Db<A> is {value:A};
    implement<T> Connection<T> for Db<T> {type Rows<U> =Cursor<T,U>;}
"#,
    );
}
#[test]
fn family_shadowing_cannot_capture_the_owner() {
    accept(
        r#"module families;
    type Stream<A> is protocol {type Item;};
    type Connection<A> is protocol {type Rows<A>:Stream<A,Item=A>;};
    type Cursor<R> is {value:R};implement<R> Stream<R> for Cursor<R> {type Item=R;}
    type Db<A> is {value:A};implement<T> Connection<T> for Db<T> {type Rows<U> =Cursor<U>;}
"#,
    );
}
#[test]
fn self_projection_uses_its_own_implementation() {
    accept(
        r#"module families;
    type Stream is protocol {type Item;};type Connection is protocol {type Item;type Rows:Stream<Item=Self.Item>;};
    type Cursor<T> is {value:T};implement<T> Stream for Cursor<T> {type Item=T;}
    type Db<T> is {value:T};implement<T> Connection for Db<T> {type Item=T;type Rows=Cursor<T>;}
"#,
    );
}
#[test]
fn missing_required_associated_type_is_rejected() {
    reject(
        r#"module families;
    type Stream is protocol {type Item;};type Db is {};implement Stream for Db {}
"#,
    );
}
#[test]
fn parameterized_super_protocol_obligation_is_not_erased() {
    reject(
        r#"module families;
    type Row is protocol {}; type Required<T: Row> is protocol {};type Stream<T> is protocol extends Required<T> {type Item;};
    type Connection is protocol {type Rows<R>:Stream<R,Item=R>;};
    type Cursor<T> is {value:T};implement<T> Stream<T> for Cursor<T> {type Item=T;}
    type Db is {};implement Connection for Db {type Rows<R> =Cursor<R>;}
"#,
    );
}
#[test]
fn parameterized_super_protocol_obligation_is_proved() {
    accept(
        r#"module families;
    type Row is protocol {}; type Required<T: Row> is protocol {};type Stream<T: Row> is protocol extends Required<T> {type Item;};
    type Connection is protocol {type Rows<R: Row>:Stream<R,Item=R>;};
    type Cursor<T> is {value:T};implement<T: Row> Required<T> for Cursor<T> {}
    implement<T: Row> Stream<T> for Cursor<T> {type Item=T;}
    type Db is {};implement Connection for Db {type Rows<R: Row> =Cursor<R>;}
"#,
    );
}

#[test]
fn a_future_specialization_cannot_change_the_promised_item() {
    let source = format!(
        "{}\n@specialize implement Stream for Cursor<Int> {{type Item=Text;}}",
        source("T", "R", "R", "")
    );
    let errors = errors(&source);
    assert!(
        errors.iter().any(|error| {
            (error.contains("potential implementation") && error.contains("Item"))
                || (error.contains("E405")
                    && error.contains("Overlapping implementations")
                    && error.contains("families.Stream")
                    && error.contains("families.Cursor"))
        }),
        "{errors:#?}"
    );
}
#[test]
fn a_specialization_preserving_the_item_is_accepted() {
    let source = format!(
        "{}\n@specialize implement Stream for Cursor<Int> {{type Item=Int;}}",
        source("T", "R", "R", "")
    );
    accept(&source);
}

#[test]
fn inherited_parameter_assumption_proves_conditional_impl() {
    accept(
        r#"module families;
    type Row is protocol {}; type Special is protocol extends Row {};
    type Stream is protocol {type Item;};
    type Connection is protocol {type Rows<R: Special>:Stream<Item=R>;};
    type Cursor<T> is {value:T};implement<T: Row> Stream for Cursor<T> {type Item=T;}
    type Db is {};implement Connection for Db {type Rows<R: Special> =Cursor<R>;}
"#,
    );
}
#[test]
fn declared_associated_default_proves_matching_equality() {
    accept(
        r#"module families;
    type Stream<A> is protocol {type Item=A;};
    type Connection is protocol {type Rows<R>:Stream<R,Item=R>;};
    type Cursor<T> is {value:T};implement<T> Stream<T> for Cursor<T> {}
    type Db is {};implement Connection for Db {type Rows<R> =Cursor<R>;}
"#,
    );
}

#[test]
fn associated_sum_defaults_keep_their_declared_methods() {
    accept(r#"module families;
        type Receipt<T> is | Value(T);
        implement<T> Receipt<T> {
            fn take(self) -> T { match self { Value(value) => value } }
        }
        type Produce is protocol {
            type Output = Receipt<Int>;
            fn produce(self) -> Self.Output;
        };
        type Source is { tag:Int };
        implement Produce for Source {
            fn produce(self) -> Receipt<Int> { Receipt.Value(self.tag) }
        }
        fn obtain<P:Produce>(source:P) -> P.Output { source.produce() }
        fn probe(source:Source) -> Int { obtain(source).take() }
    "#);
}

#[test]
fn associated_sum_equalities_preserve_declared_nested_results() {
    let source = r#"module families;
        type Packet<T> is | Present(T) | Absent;
        type Stream is protocol { type Item; };
        type IntoStream is protocol {
            type Item;
            type Stream: Stream<Item = Self.Item>;
        };
        type Cursor<T> is { value:T };
        implement<T> Stream for Cursor<T> { type Item = (Packet<T>, Packet<T>); }
        implement<T> IntoStream for Cursor<T> {
            type Item = (Packet<T>, Packet<T>);
            type Stream = Cursor<T>;
        }
    "#;
    accept(source);
    reject(&source.replace("type Stream = Cursor<T>", "type Stream = Cursor<Bool>"));
}

#[test]
fn associated_sum_equalities_preserve_explicit_bound_results() {
    let source = r#"module families;
        type Packet<T> is | Present(T) | Absent;
        type Other<T> is | Present(T) | Absent;
        type Stored<T> is Packet<T>;
        type Stream is protocol { type Item; };
        type Source is protocol { type Stream<T>: Stream<Item = Packet<T>>; };
        type Cursor<T> is { value:T };
        implement<T> Stream for Cursor<T> { type Item = Packet<T>; }
        type Store is {};
        implement Source for Store { type Stream<T> = Cursor<T>; }
    "#;
    accept(source);
    accept(&source.replace("type Item = Packet<T>", "type Item = Stored<T>"));
    reject(&source.replace("Stream<T> = Cursor<T>", "Stream<T> = Cursor<Bool>"));
    let mismatches = errors(&source.replace("type Item = Packet<T>", "type Item = Other<T>"));
    assert!(mismatches.iter().any(|error| error.contains("E405")
        && error.contains("sets Item = families.Other")
        && error.contains("expected families.Packet")), "{mismatches:#?}");
}
#[test]
fn unknown_telescope_does_not_discharge_super_obligations() {
    use verum_types::protocol::ProtocolConstraints;
    let module = Parser::new(
        r#"module families;
      type Row is protocol {};type Stream is protocol extends Row {};
      type Db is {};implement Stream for Db {}
    "#,
    )
    .parse_module()
    .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("families");
    assert!(checker.prepare_module_declarations(&module).is_empty());
    let mut protocol = checker
        .protocol_checker
        .read()
        .get_protocol(&"families.Stream".into())
        .unwrap()
        .clone();
    protocol.constraints = ProtocolConstraints::Unknown;
    checker
        .protocol_checker
        .write()
        .register_protocol(protocol)
        .unwrap();
    let implementation = module
        .items
        .iter()
        .find(|item| matches!(item.kind, verum_ast::ItemKind::Impl(_)))
        .unwrap();
    let result = checker.check_item(implementation);
    assert!(
        result.is_err() || checker.diagnostics().iter().any(|d| d.is_error()),
        "unknown protocol telescope silently accepted inherited obligations"
    );
}

#[test]
fn failed_where_elaboration_cannot_be_an_empty_telescope() {
    let source = r#"module families;type Broken<T> is protocol where T: Missing {};type Db is {};implement Broken<Int> for Db {}"#;
    let diagnostics = errors(source);
    assert!(
        !diagnostics.is_empty(),
        "unresolved where-bound accepted: {source}"
    );
}

#[test]
fn actual_registered_specialization_must_preserve_universal_item() {
    use verum_common::List;
    use verum_types::{Type, advanced_protocols::SpecializationInfo, protocol::ProtocolBound};
    let source = source("T", "R", "R", "");
    let module = Parser::new(&source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.set_generic_associated_types_enabled(true);
    checker.register_primitives();
    checker.set_current_module_path("families");
    assert!(checker.prepare_module_declarations(&module).is_empty());
    let mut registry = checker.protocol_checker.write();
    let blanket = registry
        .all_implementations()
        .iter()
        .find(|implementation| implementation.protocol.to_string() == "families.Stream")
        .unwrap()
        .clone();
    let variable = *blanket.for_type.free_vars().iter().next().unwrap();
    let mut subst = verum_types::ty::Substitution::new();
    subst.insert(variable, Type::Int);
    let mut concrete = blanket.clone();
    assert!(matches!(concrete.parameters.as_slice(), [verum_types::context::GenericBinder::Type(owner)] if *owner == variable));
    // Specializing the only owned parameter produces a monomorphic declaration.
    concrete.parameters.clear();
    concrete.for_type = concrete.for_type.try_apply_subst(&subst).unwrap();
    concrete.associated_types.insert(
        "Item".into(),
        verum_types::protocol::AssociatedTypeAssignment::ordinary(Type::Text),
    );
    concrete.specialization = Some(SpecializationInfo {
        is_specialized: true,
        specializes: None,
        specificity_rank: 1,
        is_default: false,
        span: Default::default(),
    });
    concrete.span = verum_ast::Span::new(1, 2, verum_ast::FileId::new(1));
    registry
        .register_impl(concrete)
        .expect("real specialization registered");
    let mut bound = ProtocolBound::positive(blanket.protocol.clone(), List::new());
    bound
        .associated_types
        .insert("Item".into(), Type::Var(variable));
    let error = registry
        .check_universal_bounds(&blanket.for_type, &[bound], &[])
        .expect_err("conflicting specialization violates universal equality");
    assert!(
        error.contains("potential implementation") && error.contains("Item"),
        "{error}"
    );
}

#[test]
fn inherited_assumption_keeps_its_protocol_arguments() {
    reject(
        r#"module families;
    type Row<A> is protocol {};type Special<A> is protocol extends Row<A> {};
    type Stream is protocol {type Item;};type Connection is protocol {type Rows<R: Special<Text>>:Stream<Item=R>;};
    type Cursor<T> is {value:T};implement<T: Row<Int>> Stream for Cursor<T> {type Item=T;}
    type Db is {};implement Connection for Db {type Rows<R: Special<Text>> =Cursor<R>;}
"#,
    );
}
#[test]
fn declared_associated_default_cannot_change_the_promised_item() {
    reject(
        r#"module families;
    type Stream<A> is protocol {type Item=Text;};type Connection is protocol {type Rows<R>:Stream<R,Item=R>;};
    type Cursor<T> is {value:T};implement<T> Stream<T> for Cursor<T> {}
    type Db is {};implement Connection for Db {type Rows<R> =Cursor<R>;}
"#,
    );
}
#[test]
fn unconstrained_value_protocol_retains_ordinary_conformance() {
    accept(
        r#"module families;type Marker<N: meta Int> is protocol {};type Db is {};implement Marker<2> for Db {}"#,
    );
}

#[test]
fn refined_value_protocol_is_not_an_empty_constraint_scope() {
    let diagnostics = errors(
        r#"module families;
        type Positive<N: meta Int{>0}> is protocol {};type Db is {};
        implement Positive<-1> for Db {}
    "#,
    );
    assert!(diagnostics.iter().any(|error| error.contains("E405")
        && error.contains("refined protocol values") && error.contains("refinement witness")), "{diagnostics:#?}");
}
