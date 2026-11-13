//! Aggregate return origins must distinguish copied payloads from borrowed storage.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn diagnostics(declaration: &str, caller: &str) -> List<Text> {
    let source =
        format!("type Payload is {{ public ptr:&Int, public marker:Int }}; {declaration} {caller}");
    let module = FastParser::new()
        .parse_module_str(&source, FileId::new(1))
        .unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("probe");
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| error.to_string().into())
        .collect()
}

#[test]
fn copied_payload_return_does_not_borrow_the_local_container_or_lookup_key() {
    let errors = diagnostics(
        "fn selected(source:&Payload, key:&Int)->Payload { Payload {ptr:source.ptr,marker:source.marker} }",
        "fn receive(input:&Int)->Payload { let container=Payload{ptr:input,marker:2}; let key=3; selected(&container,&key) }",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_returned_reference_to_the_lookup_key_keeps_its_real_loan() {
    let errors = diagnostics(
        "fn selected(source:&Payload, key:&Int)->Payload { Payload {ptr:key,marker:source.marker} }",
        "fn receive(container:&Payload)->Payload { let key=3; selected(container,&key) }",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("key") && e.contains("does not live long enough")),
        "{errors:?}"
    );
}

#[test]
fn a_returned_reference_to_container_storage_keeps_its_real_loan() {
    let errors = diagnostics(
        "fn selected(source:&Payload, key:&Int)->Payload { Payload {ptr:&source.marker,marker:*key} }",
        "fn receive(input:&Int)->Payload { let container=Payload{ptr:input,marker:2}; selected(&container,input) }",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("container") && e.contains("does not live long enough")),
        "{errors:?}"
    );
}

#[test]
fn a_payload_written_through_a_call_keeps_the_injected_loan() {
    let errors = diagnostics(
        "fn replace(target:&mut Payload, value:&Int) { target.ptr=value; } fn selected(source:&Payload)->Payload { Payload {ptr:source.ptr,marker:source.marker} }",
        "fn receive(input:&Int)->Payload { let mut container=Payload{ptr:input,marker:2}; { let local=3; replace(&mut container,&local); } selected(&container) }",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("does not live long enough")),
        "{errors:?}"
    );
}

#[test]
fn an_unknown_mutating_call_cannot_create_an_untracked_self_reference() {
    let errors = diagnostics(
        "fn replace(target:&mut Payload) { target.ptr=&target.marker; }",
        "fn receive(input:&Int)->Payload { let mut container=Payload{ptr:input,marker:2}; replace(&mut container); Payload{ptr:container.ptr,marker:container.marker} }",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("cannot establish the lifetime")
                || e.contains("does not live long enough")),
        "{errors:?}"
    );
}

#[test]
fn source_owned_readonly_calls_do_not_invalidate_stored_payloads() {
    let errors = diagnostics(
        "fn selected(source:&mut Payload, key:&Int)->Payload { Payload {ptr:source.ptr,marker:source.marker} }",
        "fn receive(input:&Int)->Payload { let mut container=Payload{ptr:input,marker:2}; let key=3; selected(&mut container,&key) }",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn dereferencing_an_invalidated_payload_cannot_hide_its_unknown_loans() {
    let errors = diagnostics(
        "fn replace(target:&mut Payload) { target.ptr=&target.marker; }",
        "fn receive(input:&Int)->Payload { let mut container=Payload{ptr:input,marker:2}; replace(&mut container); *(&container) }",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("cannot establish the lifetime")
                || e.contains("does not live long enough")),
        "{errors:?}"
    );
}

#[test]
fn method_writes_cannot_install_a_shorter_lived_reference() {
    let errors = diagnostics(
        "implement Payload { fn replace(&mut self,value:&Int) { self.ptr=value; } }",
        "fn receive(input:&Int)->Payload { let mut container=Payload{ptr:input,marker:2}; { let local=3; container.replace(&local); } Payload{ptr:container.ptr,marker:container.marker} }",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("does not live long enough")),
        "{errors:?}"
    );
}

#[test]
fn scalar_only_destinations_do_not_acquire_a_stored_reference_contract() {
    let errors = diagnostics(
        "fn replace(target:&mut Int,value:&Int) {*target=*value;}",
        "fn receive(input:&Int)->Int {let mut target=0; {let local=3; replace(&mut target,&local);} target}",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn reassigned_function_values_do_not_reuse_another_bodys_return_rule() {
    let declarations = "fn readonly(target:&mut Payload,source:&Int) {} fn replace(target:&mut Payload,source:&Int) {target.ptr=source;}";
    assert!(
        diagnostics(declarations, "").is_empty(),
        "the declared bodies must be valid"
    );
    let errors = diagnostics(
        declarations,
        "fn receive(input:&Int)->Payload {let mut container=Payload{ptr:input,marker:2}; let mut operation=readonly; operation=replace; {let local=3; operation(&mut container,&local);} Payload{ptr:container.ptr,marker:container.marker}}",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("does not live long enough")
                || e.contains("cannot establish the lifetime")),
        "{errors:?}"
    );
}

#[test]
fn a_complete_assignment_restores_the_new_values_payload_provenance() {
    let errors = diagnostics(
        "fn touch(target:&mut Payload) {target.marker=4;} fn selected(source:&Payload)->Payload {Payload{ptr:source.ptr,marker:source.marker}}",
        "fn receive(input:&Int)->Payload {let mut container=Payload{ptr:input,marker:2}; touch(&mut container); container=Payload{ptr:input,marker:9}; selected(&container)}",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn conditional_replacement_does_not_erase_the_unwritten_branch() {
    let errors = diagnostics(
        "fn touch(target:&mut Payload) {target.ptr=&target.marker;}",
        "fn receive(input:&Int,change:Bool)->Payload {let mut container=Payload{ptr:input,marker:2}; touch(&mut container); if change {container=Payload{ptr:input,marker:9};} Payload{ptr:container.ptr,marker:container.marker}}",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("cannot establish the lifetime")
                || e.contains("does not live long enough")),
        "{errors:?}"
    );
}

#[test]
fn renamed_imports_keep_the_selected_owners_return_rule() {
    use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
    for (payload, accepted) in [("source.ptr", true), ("key", false)] {
        for call in ["choose", "service.selected"] {
            let mut registry = ModuleRegistry::new();
            for (owner, source) in [
            ("owner", format!("public type Payload is {{public ptr:&Int,public marker:Int}}; public fn selected(source:&Payload,key:&Int)->Payload {{Payload{{ptr:{payload},marker:source.marker}}}}")),
            ("decoy", "public type Payload is {public ptr:&Int,public marker:Int}; public fn selected(source:&Payload,key:&Int)->Payload {Payload{ptr:key,marker:source.marker}}".into()),
            ("consumer", format!("mount decoy.*; mount owner.{{self as service,Payload as Carrier,selected as choose}}; fn receive(input:&Int)->Carrier {{let container=Carrier{{ptr:input,marker:2}}; let key=3; {call}(&container,&key)}}")),
        ] {
            let id = registry.allocate_id();
            let file = FileId::new(id.as_u32());
            let module = FastParser::new().parse_module_str(&source, file).unwrap();
            let path = ModulePath::from_str(owner);
            let exports = extract_exports_from_module(&module, id, &path).unwrap();
            let mut info = ModuleInfo::new(id, path, module, file, source.into());
            info.exports = exports;
            registry.register(info);
        }
            let module = registry.get_by_path("consumer").unwrap();
            let mut checker = TypeChecker::with_minimal_context();
            checker.register_primitives();
            checker.set_current_module_path("consumer");
            checker.set_module_registry_direct(registry);
            let mut errors = checker.prepare_module_declarations(&module.ast);
            for item in &module.ast.items {
                if let Err(error) = checker.check_item(item) {
                    errors.push(error);
                }
            }
            errors.extend(checker.take_deferred_errors());
            let errors: List<Text> = errors
                .iter()
                .chain(checker.diagnostic_sources())
                .map(|error| error.to_string().into())
                .collect();
            if accepted {
                assert!(errors.is_empty(), "{errors:?}");
            } else {
                assert!(
                    errors.iter().any(|error| error.contains("key")
                        && error.contains("does not live long enough")),
                    "{errors:?}"
                );
            }
        }
    }
}
