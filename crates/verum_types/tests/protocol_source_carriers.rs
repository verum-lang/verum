use verum_fast_parser::Parser;
use verum_types::TypeChecker;
fn errors(source: &str) -> Vec<String> {
    let ast=Parser::new(source).parse_module().unwrap();
    let mut checker=TypeChecker::new();checker.register_primitives();checker.set_current_module_path("demo");
    let mut errors:Vec<_>=checker.prepare_module_declarations(&ast).iter().map(|e|format!("{e:?}")).collect();
    errors.extend(checker.take_deferred_errors().iter().map(|e|format!("{e:?}")));errors
}
#[test] fn inherited_self_is_the_actual_receiver() {
    let e=errors("module demo; type Parent is protocol {fn same(&self,other:&Self)->Bool;}; type Child is protocol extends Parent {}; type Model is {n:Int}; implement Child for Model {fn same(&self,other:&Model)->Bool{true}}");
    assert!(e.is_empty(),"{e:#?}");
}
#[test] fn inherited_self_cannot_be_another_carrier() {
    let e=errors("module demo; type Parent is protocol {fn same(&self,other:&Self)->Bool;}; type Child is protocol extends Parent {}; type Model is {n:Int}; type Other is {n:Int}; implement Child for Model {fn same(&self,other:&Other)->Bool{true}}");
    assert!(e.iter().any(|e|e.contains("E427")),"{e:#?}");
}
#[test] fn forward_nominal_signature_uses_declaring_carrier() {
    let e=errors("module demo; type Driver is protocol {fn apply(&self,arg:Input)->Output;}; type Input is {n:Int}; type Output is {n:Bool}; type Model is (); implement Driver for Model {fn apply(&self,arg:Input)->Output{Output{n:true}}}");
    assert!(e.is_empty(),"{e:#?}");
}
#[test] fn forward_nominal_signature_rejects_same_shape_foreign_type() {
    let e=errors("module demo; type Driver is protocol {fn apply(&self,arg:Input)->Output;}; type Input is {n:Int}; type Other is {n:Int}; type Output is {n:Bool}; type Model is (); implement Driver for Model {fn apply(&self,arg:Other)->Output{Output{n:true}}}");
    assert!(e.iter().any(|e|e.contains("E427")),"{e:#?}");
}
#[test]
fn sum_implementation_self_retains_declaring_owner() {
    let e=errors("module demo; type Equality is protocol {fn equal(&self, other:&Self)->Bool;}; type Status is Ready | Failed(Int); implement Equality for Status {fn equal(&self,other:&Status)->Bool{true}}");
    assert!(e.is_empty(),"{e:#?}");
}
#[test]
fn sum_implementation_self_rejects_another_carrier() {
    let e=errors("module demo; type Equality is protocol {fn equal(&self, other:&Self)->Bool;}; type Status is Ready | Failed(Int); type Other is Ready | Failed(Int); implement Equality for Status {fn equal(&self,other:&Other)->Bool{true}}");
    assert!(e.iter().any(|e|e.contains("E427")),"{e:#?}");
}
