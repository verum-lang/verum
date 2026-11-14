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

#[test]
fn implicit_local_cleanup_cannot_hide_a_payload_write() {
    let declarations = "type Drop is protocol {fn drop(&mut self);};
        type affine DeferredWrite is {target:&mut Payload,source:&Int};
        implement Drop for DeferredWrite {fn drop(&mut self) {self.target.ptr=self.source;}}
        fn replace_later(target:&mut Payload,source:&Int) {
            let pending=DeferredWrite{target,source}; ()
        }";
    assert!(
        diagnostics(declarations, "").is_empty(),
        "the destructor and callable must be valid"
    );
    let errors = diagnostics(
        declarations,
        "fn receive(input:&Int)->Payload {let mut container=Payload{ptr:input,marker:2};
         {let local=3;replace_later(&mut container,&local);}
         Payload{ptr:container.ptr,marker:container.marker}}",
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("does not live long enough")
                || error.contains("cannot establish the lifetime")),
        "{errors:?}"
    );
}

#[test]
fn owned_arguments_carry_mutable_storage_into_implicit_cleanup() {
    let declarations = "type Drop is protocol {fn drop(&mut self);};
        type affine DeferredWrite is {target:&mut Payload,source:&Int};
        implement Drop for DeferredWrite {fn drop(&mut self) {self.target.ptr=self.source;}}
        fn finish(pending:DeferredWrite) {}";
    assert!(
        diagnostics(declarations, "").is_empty(),
        "the destructor and callable must be valid"
    );
    let errors = diagnostics(
        declarations,
        "fn receive(input:&Int)->Payload {let mut container=Payload{ptr:input,marker:2};
         {let local=3;finish(DeferredWrite{target:&mut container,source:&local});}
         Payload{ptr:container.ptr,marker:container.marker}}",
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("does not live long enough")
                || error.contains("cannot establish the lifetime")),
        "{errors:?}"
    );
}

#[test]
fn reference_and_scalar_locals_have_closed_cleanup() {
    let errors = diagnostics(
        "fn selected(source:&mut Payload,key:&Int)->Payload {
            let alias=source; let marker=alias.marker;
            Payload{ptr:alias.ptr,marker}
        }",
        "fn receive(input:&Int)->Payload {let mut container=Payload{ptr:input,marker:2};
         let key=3;selected(&mut container,&key)}",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_user_deref_body_is_not_a_direct_field_derivation() {
    let declarations = "type Deref is protocol {type Target;fn deref(&self)->&Self.Target;};
        type Wrapper is {value:Payload};
        implement Deref for Wrapper {
            type Target=Payload;
            fn deref(&self)->&Payload {&self.value}
        }
        fn selected(source:&Wrapper,key:&Int)->Payload {
            Payload{ptr:source.ptr,marker:source.marker}
        }";
    assert!(
        diagnostics(declarations, "").is_empty(),
        "the selected Deref body must be valid"
    );
    let errors = diagnostics(
        declarations,
        "fn receive(input:&Int)->Payload {let local=3;
         let wrapper=Wrapper{value:Payload{ptr:&local,marker:2}};
         selected(&wrapper,input)}",
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("does not live long enough")),
        "{errors:?}"
    );
}

#[test]
fn an_unknown_callable_environment_invalidates_reachable_payload_facts() {
    let errors = diagnostics(
        "fn execute(operation:fn()) {operation();}",
        "fn receive(input:&Int)->Payload {let mut container=Payload{ptr:input,marker:2};
         {let local=3;execute(|| {container.ptr=&local;});}
         Payload{ptr:container.ptr,marker:container.marker}}",
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("does not live long enough")
                || error.contains("cannot establish the lifetime")),
        "{errors:?}"
    );
}

#[test]
fn a_closed_callback_does_not_erase_unrelated_payload_facts() {
    let errors = diagnostics(
        "fn execute(operation:fn(Int)->Int) {operation(3);}",
        "fn receive(input:&Int)->Payload {let container=Payload{ptr:input,marker:2};
         execute(|value| value);
         Payload{ptr:container.ptr,marker:container.marker}}",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn empty_record_layouts_preserve_nominal_identity() {
    let declarations = "type First is {}; type Second is {};
        fn consume(first:First) {}";
    let valid = diagnostics(
        declarations,
        "fn receive(input:&Int)->Payload {let container=Payload{ptr:input,marker:2};
         consume(First{}); Payload{ptr:container.ptr,marker:container.marker}}",
    );
    assert!(valid.is_empty(), "{valid:?}");
    let invalid = diagnostics(declarations, "fn wrong(second:Second) {consume(second);}");
    assert!(
        invalid.iter().any(|error| error.contains("mismatch")),
        "{invalid:?}"
    );
}


#[test]
fn projected_mutable_receivers_preserve_unrelated_payload_origins() {
    let declarations = "type Holder is {item:Payload}; type Outer is {holder:Holder};
        type HolderRef is &mut Holder;
        implement Payload {fn update(&mut self) {self.marker=73;}}";
    for caller in [
        "fn receive(target:&mut Payload,separate:&Payload)->&Int {target.update(); separate.ptr}",
        "fn receive(target:&mut Holder,separate:&Payload)->&Int {target.item.update(); separate.ptr}",
        "fn receive(target:HolderRef,separate:&Payload)->&Int {target.item.update(); separate.ptr}",
        "fn receive(target:&checked mut Holder,separate:&Payload)->&Int {target.item.update(); separate.ptr}",
        "fn receive(target:&unsafe mut Holder,separate:&Payload)->&Int {target.item.update(); separate.ptr}",
        "fn receive(target:&mut Outer,separate:&Payload)->&Int {target.holder.item.update(); separate.ptr}",
        "implement Holder {fn receive(&mut self,separate:&Payload)->&Int {self.item.update(); separate.ptr}}",
    ] {
        let errors = diagnostics(declarations, caller);
        assert!(errors.is_empty(), "{caller}: {errors:?}");
    }
}

#[test]
fn projected_mutable_receivers_do_not_erase_real_shared_loans() {
    let declarations = "type Holder is {item:Payload};
        implement Payload {fn update(&mut self) {self.marker=73;}}";
    for caller in [
        "fn receive(target:&Holder)->&Int {target.item.update(); target.item.ptr}",
        "fn receive(target:&mut Holder)->&Int {let held=&target.item; target.item.update(); held.ptr}",
    ] {
        let errors = diagnostics(declarations, caller);
        assert!(errors.iter().any(|error| error.contains("borrow")
            || error.contains("shared reference")), "{caller}: {errors:?}");
    }
}

#[test]
fn projected_mutable_receivers_keep_stored_local_origins() {
    let declarations = "type Holder is {item:Payload};
        implement Payload {fn replace(&mut self,source:&Int) {self.ptr=source;}}";
    for caller in [
        "fn receive(input:&Int)->Holder {let mut target=Holder{item:Payload{ptr:input,marker:2}};
         {let local=3;target.item.replace(&local);} target}",
        "fn receive(target:&mut Holder) { {let local=3;target.item.replace(&local);} let held=target.item.ptr; }",
        "fn receive(input:&Int)->Holder {let mut target=Holder{item:Payload{ptr:input,marker:2}};
         {let local=3;let alias=&mut target;alias.item.replace(&local);} target}",
    ] {
        let errors = diagnostics(declarations, caller);
        assert!(errors.iter().any(|error| error.contains("does not live long enough")
            || error.contains("cannot establish the lifetime")), "{caller}: {errors:?}");
    }
}


#[test]
fn computed_write_receivers_do_not_gain_an_unproved_storage_identity() {
    let declarations = "type Holder is {item:Payload};
        implement Payload {fn replace(&mut self,source:&Int) {self.ptr=source;}}
        fn selected(a:&mut Holder,b:&mut Holder,first:Bool)->&mut Holder {if first {a} else {b}}";
    let errors = diagnostics(declarations,
        "fn receive(input:&Int,first:Bool)->Holder {
         let mut a=Holder{item:Payload{ptr:input,marker:1}};
         let mut b=Holder{item:Payload{ptr:input,marker:2}};
         {let local=3;selected(&mut a,&mut b,first).item.replace(&local);} a}");
    assert!(errors.iter().any(|error| error.contains("does not live long enough")
        || error.contains("cannot establish the lifetime")), "{errors:?}");
}
