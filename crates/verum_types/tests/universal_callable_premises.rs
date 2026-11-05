//! Universal associated contracts must hold for every permitted family argument.
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> Vec<String> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
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

fn source(actual: &str, required: &str, item: &str) -> String {
    format!(r#"module families;
        type Stream is protocol {{type Item;}};
        type Factory is protocol {{type Item;type Rows:Stream<Item=Self.Item>;}};
        type Rows<F> is {{f:F}};
        implement<B,F:{required}> Stream for Rows<F> {{type Item=B;}}
        type Db<F> is {{f:F}};
        implement<B,F{actual}> Factory for Db<F> {{type Item={item};type Rows=Rows<F>;}}
    "#)
}
fn accept(source:&str) {let errors=errors(source);assert!(errors.is_empty(),"{source}\n{errors:#?}");}
fn reject(source:&str) {let errors=errors(source);assert!(!errors.is_empty(),"unproved contract accepted: {source}");}
#[test]fn declared_generic_callable_proves_associated_result(){
    accept(&source(":fn(Int)->B","fn(Int)->B","B"));
}
#[test]fn missing_callable_premise_is_not_inferred_from_target(){
    reject(&source("","fn(Int)->B","B"));
}
#[test]fn wrong_parameter_contract_refuses(){
    reject(&source(":fn(Bool)->B","fn(Int)->B","B"));
}
#[test]fn arbitrary_result_variable_cannot_be_specialized(){
    reject(&source(":fn(Int)->B","fn(Int)->Int","B"));
}
#[test]fn wrong_associated_equality_is_not_authorized_by_a_callable(){
    reject(&source(":fn(Int)->B","fn(Int)->B","Bool"));
}
#[test]fn callable_value_coercion_does_not_prove_the_bound(){
    reject(&source(":fn(Int)->B","fn(&Int)->B","B"));
    reject(&source(":fn(&mut Int)->B","fn(&Int)->B","B"));
}
#[test]fn contexts_are_part_of_callable_evidence(){
    let prefix="context Service { fn read()->Int; } ";
    accept(&(prefix.to_owned()+&source(":fn(Int)->B using[Service]","fn(Int)->B using[Service]","B")));
    reject(&(prefix.to_owned()+&source(":fn(Int)->B using[Service]","fn(Int)->B","B")));
}
#[test]fn alpha_renaming_keeps_callers_and_implementations_separate(){
    let source=source(":fn(Int)->B","fn(Int)->B","B");
    accept(&source.replace("implement<B,F:fn(Int)->B> Stream", "implement<Output,F:fn(Int)->Output> Stream").replace("{type Item=B;}","{type Item=Output;}"));
}
#[test]fn callable_owner_retains_generic_identity_in_ordinary_bodies(){
    accept(r#"module families; fn identity<F:fn(Int)->Bool>(value:F)->F {value}"#);
}

#[test]fn higher_order_callable_premises_are_independent_of_declaration_order(){
    for bounds in ["B,G:fn(Int)->B,F:fn(Int)->G", "F:fn(Int)->G,G:fn(Int)->B,B"] {
        let source=format!(r#"module families;
            type Stream is protocol{{type Item;}};type Factory is protocol{{type Item;type Rows:Stream<Item=Self.Item>;}};
            type Rows<F> is{{f:F}};implement<{bounds}> Stream for Rows<F>{{type Item=B;}}
            type Db<F> is{{f:F}};implement<{bounds}> Factory for Db<F>{{type Item=B;type Rows=Rows<F>;}}
        "#);
        accept(&source);
    }
}
#[test]fn missing_deeper_callable_premise_is_not_synthesized(){
    reject(r#"module families;
        type Stream is protocol{type Item;};type Factory is protocol{type Item;type Rows:Stream<Item=Self.Item>;};
        type Rows<F> is{f:F};implement<B,G:fn(Int)->B,F:fn(Int)->G> Stream for Rows<F>{type Item=B;}
        type Db<F> is{f:F};implement<B,G,F:fn(Int)->G> Factory for Db<F>{type Item=B;type Rows=Rows<F>;}
    "#);
}
#[test]fn explicit_callable_where_premises_use_the_same_owner(){
    let source=source(":fn(Int)->B","fn(Int)->B","B");
    accept(&source.replace("implement<B,F:fn(Int)->B> Factory for Db<F>","implement<B,F> Factory for Db<F> where F:fn(Int)->B"));
}

#[test]fn callable_in_protocol_argument_retains_all_its_rigid_variables(){
    for result in ["B","Int"] {
        let source=format!(r#"module families;
            type Stream<F> is protocol{{type Item;}};
            type Factory<F> is protocol{{type Item;type Rows:Stream<F,Item=Self.Item>;}};
            type Rows is{{}};implement<B,F:fn(Int)->{result}> Stream<F> for Rows{{type Item=B;}}
            type Db is{{}};implement<B,F:fn(Int)->B> Factory<F> for Db{{type Item=B;type Rows=Rows;}}
        "#);
        if result=="B"{{accept(&source)}}else{{reject(&source)}}
    }
}
