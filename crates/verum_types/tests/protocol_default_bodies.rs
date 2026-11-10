use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn module(source: &str) -> verum_ast::Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("actual grammar")
}
fn checker() -> TypeChecker {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("fixture");
    checker.set_current_module_path("fixture.main");
    checker
}
fn check(checker: &mut TypeChecker, source: &str) -> List<Text> {
    let diagnostic_start = checker.diagnostics().len();
    let ast = module(source);
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|e| format!("{e:?}").into())
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .skip(diagnostic_start)
            .filter(|e| e.is_error())
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}
fn accepted(source: &str) {
    let errors = check(&mut checker(), source);
    assert!(errors.is_empty(), "{source}\n{errors:?}");
}
fn rejected(source: &str) {
    let errors = check(&mut checker(), source);
    assert!(!errors.is_empty(), "invalid body admitted: {source}");
}
#[test]
fn ordinary_and_protocol_results_use_the_same_judgement() {
    rejected("fn wrong()->Bool{42}");
    rejected("type Rule is protocol { fn wrong()->Bool{42} };");
    accepted("type Rule is protocol { fn right()->Bool{true} };");
    accepted("type Rule is protocol { fn required()->Bool; };");
}
#[test]
fn protocol_body_statements_are_checked() {
    rejected("type Rule is protocol { fn wrong()->Bool{let bad: Bool = 42; true} };");
    rejected("type Rule is protocol { fn wrong()->Bool{@const{let bad: Bool=42; bad}} };");
}
#[test]
fn self_is_rigid_and_keeps_the_exact_protocol_contract() {
    accepted("type Rule is protocol { fn identity(self)->Self{self} };");
    rejected("type Rule is protocol { fn invent()->Self{42} };");
    accepted(
        "type Rule is protocol { fn required(&self)->Bool; fn derived(&self)->Bool{self.required()} };",
    );
    rejected(
        "type Rule is protocol { fn required(&self)->Int; fn derived(&self)->Bool{self.required()} };",
    );
}
#[test]
fn owner_parameters_remain_universal_in_each_default_body() {
    accepted("type Rule<T> is protocol { fn identity(value:T)->T{value} };");
    rejected("type Rule<T> is protocol { fn invent()->T{42} };");
    rejected("type Rule<T> is protocol { fn narrow(value:T)->Int{value} };");
}
#[test]
fn method_parameters_and_owner_shadowing_remain_independent() {
    accepted("type Rule<T> is protocol { fn identity<T>(value:T)->T{value} };");
    accepted("type Rule<T> is protocol { fn identity<U>(value:U)->U{value} };");
    rejected("type Rule<T> is protocol { fn wrong<U>(value:U)->T{value} };");
}
#[test]
fn method_bounds_authorize_only_the_declared_operations() {
    accepted(
        "type Read is protocol {fn read(&self)->Bool;} type Rule is protocol {fn value<U: Read>(value:U)->Bool{value.read()}};",
    );
    rejected(
        "type Read is protocol {fn read(&self)->Bool;} type Rule is protocol {fn value<U>(value:U)->Bool{value.read()}};",
    );
}
#[test]
fn associated_receiver_types_are_available_after_the_complete_header() {
    accepted("type Rule is protocol {fn identity(value:Self.Item)->Self.Item{value} type Item;};");
    rejected("type Rule is protocol {fn wrong(value:Self.Item)->Bool{value} type Item;};");
}
#[test]
fn failed_body_does_not_publish_locals_or_owner_parameters() {
    let mut checker = checker();
    assert!(
        !check(
            &mut checker,
            "type Rule<T> is protocol {fn wrong(value:T)->Bool{let local_value=42; value}};"
        )
        .is_empty()
    );
    assert!(check(&mut checker, "fn identity<T>(value:T)->T{value}").is_empty());
    assert!(!check(&mut checker, "fn probe()->Int{local_value}").is_empty());
}

#[test]
fn ordinary_generic_literals_need_construction_evidence() {
    rejected("type Rule is protocol {}; fn invent<T:Rule>()->T{42}");
    rejected("type Rule is protocol {}; fn invent<T:Rule>()->T{-1}");
    accepted("type Rule is protocol {}; fn construct<T:Rule>(make:fn(Int)->T)->T{make(42)}");
    accepted("fn ordinary()->Int{42}");
    accepted(
        "type Construct is protocol {fn make(value:Int)->Self;} fn construct<T:Construct>()->T{T.make(42)}",
    );
}
#[test]
fn unresolved_method_calls_need_actual_declaration_evidence() {
    rejected(
        "type Read is protocol {fn read(&self)->Bool;} fn value<U>(value:U)->Bool{value.read()}",
    );
    rejected("fn value<U>(value:U)->Bool{value.not_a_declared_method()}");
    accepted(
        "type Read is protocol {fn read(&self)->Bool;} fn value<U:Read>(value:U)->Bool{value.read()}",
    );
    accepted(
        "type Read is protocol {fn read(&self)->Bool;} fn value<U:Read>(value:U)->Bool{let input=value; input.read()}",
    );
}
#[test]
fn owner_bounds_authorize_default_body_operations() {
    accepted(
        "type Read is protocol {fn read(&self)->Bool;} type Rule<T:Read> is protocol {fn value(value:T)->Bool{value.read()}};",
    );
    rejected(
        "type Read is protocol {fn read(&self)->Bool;} type Rule<T> is protocol {fn value(value:T)->Bool{value.read()}};",
    );
}

fn imported_errors(consumer: &str, providers: &[(&str, &str)]) -> List<Text> {
    use verum_modules::{
        ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    };
    let mut checker = checker();
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in providers
        .iter()
        .chain(std::iter::once(&("fixture.main", consumer)))
        .enumerate()
    {
        let id = ModuleId::new(index as u32);
        let file = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file)
            .expect("provider grammar");
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, (*source).into());
        info.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(info);
    }
    checker.set_module_registry_direct(registry.clone());
    let ast = module(consumer);
    let mut errors = List::new();
    for item in &ast.items {
        if let verum_ast::ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "fixture.main", &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    errors.extend(check(&mut checker, consumer));
    errors
}
#[test]
fn default_body_checks_actual_imported_field_permissions() {
    for public in [true, false] {
        let provider = format!(
            "public type Secret is {{ {} value: Int }};",
            if public { "public" } else { "private" }
        );
        let errors = imported_errors(
            "mount fixture.secret.{Secret}; type Rule is protocol {fn value(input: Secret)->Int{input.value}};",
            &[("fixture.secret", &provider)],
        );
        assert_eq!(errors.is_empty(), public, "{provider}\n{errors:?}");
    }
}
#[test]
fn default_self_uses_its_exact_same_named_parent() {
    let providers = [
        (
            "fixture.left",
            "public type Read is protocol {fn read(&self)->Bool;};",
        ),
        (
            "fixture.right",
            "public type Read is protocol {fn read(&self)->Int;};",
        ),
    ];
    for (owner, accepted) in [("left", true), ("right", false)] {
        let source = format!(
            "mount fixture.{owner}.{{Read}}; type Rule is protocol extends Read {{fn value(&self)->Bool{{self.read()}}}};"
        );
        let errors = imported_errors(&source, &providers);
        assert_eq!(errors.is_empty(), accepted, "{source}\n{errors:?}");
    }
}

#[test]
fn error_restores_outer_alias_and_function_contract() {
    let mut checker = checker();
    assert!(check(&mut checker, "type T is Bool; fn shared()->Bool{true}").is_empty());
    assert!(
        !check(
            &mut checker,
            "type Rule<T> is protocol {fn shared(value:T)->Bool{value}};"
        )
        .is_empty()
    );
    let errors = check(&mut checker, "fn after()->T{shared()}");
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn required_contexts_are_checked_in_default_bodies() {
    accepted(
        "context Service {fn read()->Bool;} type Rule is protocol {fn derived()->Bool using [Service]{Service.read()}};",
    );
    rejected(
        "context Service {fn read()->Bool;} type Rule is protocol {fn derived()->Bool{Service.read()}};",
    );
}

#[test]
fn ordinary_body_parameters_shadow_module_aliases_lexically() {
    rejected("type T is Bool; fn wrong<T>(value:T)->Bool{value}");
    accepted("type T is Bool; fn identity<T>(value:T)->T{value} fn outer()->T{true}");
}

#[test]
fn inferred_locals_can_retain_the_enclosing_receivers_associated_type() {
    let declarations = "
        type Slot<T> is Missing | Found(T);
        type Feed is protocol {
            type Element;
            fn pull(&mut self) -> Slot<Self.Element>;
            fn final_item(mut self) -> Slot<Self.Element> {
                let mut saved = Slot.Missing;
                while let Slot.Found(x) = self.pull() { saved = Slot.Found(x); }
                saved
            }
        };
    ";
    let errors = check(&mut checker(), declarations);
    assert!(errors.is_empty(), "{errors:?}");

    let unconstrained = "type Slot<T> is Missing | Found(T); type Probe is protocol { fn f(&self) -> Int { let loose = Slot.Missing; 0 } };";
    let errors = check(&mut checker(), unconstrained);
    assert!(errors.iter().any(|error| error.contains("E404")), "{errors:?}");
}
