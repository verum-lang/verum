use verum_ast::{FileId, Module};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().expect("actual grammar")
}
fn registry(owner: &str, source: &str) -> ModuleRegistry {
    let ast = parse(source);
    let id = ModuleId::new(1);
    let path = ModulePath::from_str(owner);
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), FileId::new(0), source.into());
    info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
    let mut registry = ModuleRegistry::new();
    registry.register(info);
    registry
}
fn checker() -> TypeChecker {
    let mut c = TypeChecker::new();
    c.register_primitives();
    c.set_current_module_path("demo");
    c
}
fn check(c: &mut TypeChecker, source: &str) -> List<Text> {
    let start = c.diagnostics().len();
    let ast = parse(source);
    let mut errors: List<Text> = c
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|e| e.to_string().into())
        .collect();
    for item in &ast.items {
        if let Err(e) = c.check_item(item) {
            errors.push(e.to_string().into());
        }
    }
    errors.extend(
        c.take_deferred_errors()
            .into_iter()
            .map(|e| e.to_string().into()),
    );
    errors.extend(
        c.diagnostics()
            .iter()
            .skip(start)
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}").into()),
    );
    errors
}
fn accept(c: &mut TypeChecker, source: &str) {
    let e = check(c, source);
    assert!(e.is_empty(), "{source}\n{e:#?}");
}

const BROKEN: &str =
    "type Driver is protocol {fn apply(&self,arg:Input)->Int;}; type Input is {value:Missing};";
fn refuse_foreign(c: &mut TypeChecker) {
    c.set_current_module_path("foreign");
    let errors = check(
        c,
        "type ForeignDriver is protocol {fn apply(&self,arg:Input)->Int;};",
    );
    assert!(
        !errors.is_empty(),
        "failed source type supplied a foreign protocol"
    );
}
#[test]
fn failed_preparation_cannot_supply_foreign_protocol() {
    let mut c = checker();
    assert!(!check(&mut c, BROKEN).is_empty());
    refuse_foreign(&mut c);
}
#[test]
fn rejected_direct_recursive_declaration_cannot_supply_foreign_function() {
    let mut c = checker();
    let ast = parse("type Input is {bad:fn(Input)->Int};");
    let verum_ast::ItemKind::Type(decl) = &ast.items[0].kind else {
        panic!()
    };
    let error = c
        .register_type_declaration(decl)
        .expect_err("negative recursive occurrence");
    assert!(error.to_string().contains("positiv"), "{error}");
    c.set_current_module_path("foreign");
    assert!(!check(&mut c, "fn take(value:Input)->Int{1}").is_empty());
}
#[test]
fn failed_two_pass_registration_cannot_supply_foreign_function() {
    let mut c = checker();
    let ast = parse("type Input is {value:Missing};");
    c.register_all_type_names(&ast.items);
    assert!(
        c.resolve_all_type_definitions(&ast.items)
            .iter()
            .any(|r| r.is_err())
    );
    c.set_current_module_path("foreign");
    assert!(!check(&mut c, "fn take(value:Input)->Int{1}").is_empty());
}
#[test]
fn failed_declaration_does_not_replace_other_owner_binding() {
    let mut c = checker();
    c.set_current_module_path("foreign");
    accept(&mut c, "type Input is {value:Int};");
    c.set_current_module_path("demo");
    assert!(!check(&mut c, BROKEN).is_empty());
    c.set_current_module_path("foreign");
    accept(
        &mut c,
        "type ForeignDriver is protocol {fn apply(&self,arg:Input)->Int;};fn take(value:Input)->Int{value.value}",
    );
}
#[test]
fn corrected_declaration_retries_after_failure() {
    let mut c = checker();
    assert!(!check(&mut c, BROKEN).is_empty());
    accept(
        &mut c,
        "type Input is {value:Int};type Good is protocol {fn apply(&self,arg:Input)->Int;};",
    );
}
#[test]
fn recursive_and_forward_records_preserve_provisional_identity() {
    let mut c = checker();
    accept(
        &mut c,
        "type Node is {next:&Node};type Left is {right:&Right};type Right is {left:&Left};",
    );
}
#[test]
fn declared_owner_and_method_generics_remain_available() {
    let mut c = checker();
    accept(
        &mut c,
        "type Holder<T> is {value:T};type Transform<A> is protocol {fn apply<B>(&self,value:Holder<A>,other:B)->B;};",
    );
}
#[test]
fn failed_lazy_group_does_not_publish_dependency_success() {
    let broken = "module demo;type Entry is protocol {fn use_input(&self,arg:Input)->Int;};type Input is {link:&Other,missing:Missing};type Other is {link:&Input};";
    let corrected = broken.replace("missing:Missing", "missing:Int");
    let mut c = checker();
    c.set_module_registry_direct(registry("demo", broken));
    assert!(!check(&mut c, broken).is_empty());
    for key in ["demo.Input", "demo.Other"] {
        assert!(
            c.resolved_type(key).is_none(),
            "failed recursive group published {key}"
        );
    }
    c.set_module_registry_direct(registry("demo", &corrected));
    accept(&mut c, &corrected);
}

#[test]
fn failed_direct_alias_does_not_supply_a_foreign_signature() {
    let mut c = checker();
    let ast = parse("type Alias is fn(Missing)->Int;");
    let verum_ast::ItemKind::Type(declaration) = &ast.items[0].kind else {
        panic!()
    };
    assert!(c.register_type_declaration(declaration).is_err());
    assert!(c.resolved_type("demo.Alias").is_none());
    c.set_current_module_path("foreign");
    assert!(
        !check(
            &mut c,
            "type Consumer is protocol {fn take(&self,value:Alias)->Int;};"
        )
        .is_empty()
    );
}

#[test]
fn failed_protocol_does_not_publish_partial_method_evidence() {
    let mut c = checker();
    let ast =
        parse("type Invalid is protocol {fn first(&self)->Int;fn bad(&self,value:Missing);};");
    let verum_ast::ItemKind::Type(declaration) = &ast.items[0].kind else {
        panic!()
    };
    assert!(c.register_type_declaration(declaration).is_err());
    assert!(c.resolved_type("demo.Invalid").is_none());
    assert!(
        !check(
            &mut c,
            "type Model is ();implement Invalid for Model {fn first(&self)->Int{1}}"
        )
        .is_empty()
    );
    accept(
        &mut c,
        "type Valid is protocol {fn first(&self)->Int;};implement Valid for Model {fn first(&self)->Int{1}}",
    );
}

fn failed_replacement(c: &mut TypeChecker) {
    let ast = parse("type Input is {bad:fn(Input)->Int};");
    let verum_ast::ItemKind::Type(decl) = &ast.items[0].kind else {
        panic!()
    };
    assert!(c.register_type_declaration(decl).is_err());
}
#[test]
fn failed_replacement_preserves_original_accepted_authority() {
    let mut c = checker();
    accept(&mut c, "type Input is {value:Int};");
    failed_replacement(&mut c);
    accept(
        &mut c,
        "type Read is protocol {fn read(&self,input:Input)->Int;};fn take(input:Input)->Int{input.value}",
    );
    assert!(!check(&mut c, "fn wrong(input:Input)->Bool{input.value}").is_empty());
}
#[test]
fn original_can_be_registered_again_after_failed_replacement() {
    let mut c = checker();
    accept(&mut c, "type Input is {value:Int};");
    failed_replacement(&mut c);
    accept(
        &mut c,
        "type Input is {value:Int};fn take(input:Input)->Int{input.value}",
    );
}
#[test]
fn changed_valid_declaration_can_replace_after_failure() {
    let mut c = checker();
    accept(&mut c, "type Input is {value:Int};");
    failed_replacement(&mut c);
    accept(
        &mut c,
        "type Input is {value:Bool};fn take(input:Input)->Bool{input.value}",
    );
    assert!(!check(&mut c, "fn wrong(input:Input)->Int{input.value}").is_empty());
}
#[test]
fn unrelated_failed_declaration_preserves_completed_dependency() {
    let mut c = checker();
    accept(&mut c, "type Input is {value:Int};");
    assert!(!check(&mut c, "type Broken is {input:Input,missing:Missing};").is_empty());
    accept(
        &mut c,
        "type Read is protocol {fn read(&self,input:Input)->Int;};",
    );
}
#[test]
fn failed_other_owner_must_preserve_a_foreign_forward_placeholder() {
    let mut c = checker();
    c.set_current_module_path("foreign");
    let ast = parse("type Input is {value:Int};");
    let verum_ast::ItemKind::Type(decl) = &ast.items[0].kind else {
        panic!()
    };
    c.register_type_name_only(decl);
    let before = c.resolved_type("Input").cloned();
    assert!(before.is_some());
    c.set_current_module_path("demo");
    failed_replacement(&mut c);
    assert_eq!(
        c.resolved_type("Input").cloned(),
        before,
        "foreign forward carrier was deleted"
    );
    c.set_current_module_path("foreign");
    let mut stack = List::new();
    assert!(c.resolve_type_definition(decl, &mut stack).is_ok());
    accept(&mut c, "fn take(input:Input)->Int{input.value}");
}

#[test]
fn failed_changed_source_cannot_reuse_old_completed_body() {
    let mut c = checker();
    accept(&mut c, "type Input is {value:Int};");
    failed_replacement(&mut c);
    c.set_module_registry_direct(registry("demo", "type Input is {bad:fn(Input)->Int};"));
    let errors = check(
        &mut c,
        "type Consumer is protocol {fn read(&self,input:Input)->Int;};",
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("conflicting source declarations")),
        "{errors:#?}"
    );
}

#[test]
fn forward_placeholder_unification_preserves_exact_owner() {
    use verum_types::{Type, Unifier};
    let span = verum_ast::Span::default();
    let nominal = |owner: &str| Type::Named {
        path: verum_ast::Path::new(
            owner
                .split('.')
                .map(|part| verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(part, span)))
                .collect(),
            span,
        ),
        args: List::new(),
    };
    let pending = Type::Placeholder {
        name: "demo.Input".into(),
        span,
    };
    for (other, accepted) in [
        (nominal("demo.Input"), true),
        (nominal("foreign.Input"), false),
        (nominal("Input"), false),
    ] {
        for reversed in [false, true] {
            let mut unifier = Unifier::new();
            let result = if reversed {
                unifier.unify(&other, &pending, span)
            } else {
                unifier.unify(&pending, &other, span)
            };
            assert_eq!(
                result.is_ok(),
                accepted,
                "{pending:?} vs {other:?}: {result:?}"
            );
        }
    }
}
