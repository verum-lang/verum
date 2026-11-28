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
    check_module(&mut checker, &module)
}

fn check_module(checker: &mut TypeChecker, module: &verum_ast::Module) -> List<Text> {
    let mut errors = checker.prepare_module_declarations(module);
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

fn imported_diagnostics<const N: usize>(sources: [(&str, Text); N]) -> List<Text> {
    use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
    let mut registry = ModuleRegistry::new();
    for (owner, source) in sources {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let module = FastParser::new().parse_module_str(&source, file).unwrap();
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&module, id, &path).unwrap();
        let mut info = ModuleInfo::new(id, path, module, file, source);
        info.exports = exports;
        registry.register(info);
    }
    let module = registry.get_by_path("consumer").unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    check_module(&mut checker, &module.ast)
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
fn a_typed_exchange_returns_the_previous_slots_contents() {
    let declarations = r#"
        type Holder<T> is { public value:T, public marker:Int };
        fn exchange<T>(slot:&mut T, replacement:T)->T { @intrinsic("replace",slot,replacement) }
        fn forward<T>(replacement:T, slot:&mut T)->T { exchange(slot,replacement) }
    "#;
    for operation in ["exchange(&mut stored.value,replacement)", "forward(replacement,&mut stored.value)"] {
        let generic = format!("fn extract<T>(mut stored:Holder<T>, replacement:T)->T {{ {operation} }}");
        let errors = diagnostics(declarations, &generic);
        assert!(errors.is_empty(), "{generic}: {errors:?}");
        let caller = format!("fn extract(input:&Int,replacement:&Int)->&Int {{
            let mut stored=Holder{{value:input,marker:3}}; {operation}
        }}");
        let errors = diagnostics(declarations, &caller);
        assert!(errors.is_empty(), "{caller}: {errors:?}");
        let caller = format!("fn escape(replacement:&Int)->&Int {{
            let local=4; let mut stored=Holder{{value:&local,marker:3}}; {operation}
        }}");
        let errors = diagnostics(declarations, &caller);
        assert!(errors.iter().any(|error| error.contains("local")
            && error.contains("does not live long enough")), "{caller}: {errors:?}");
    }
}

#[test]
fn an_exchanging_signature_cannot_return_a_local_storage_address() {
    let errors = diagnostics(
        "type Holder<T> is { public value:T, public marker:Int };
         fn exchange<T>(slot:&mut Holder<T>, replacement:T)->&Int { &slot.marker }",
        "fn escape(mut stored:Holder<Int>)->&Int { exchange(&mut stored,7) }",
    );
    assert!(errors.iter().any(|error| error.contains("stored")
        && error.contains("does not live long enough")), "{errors:?}");
}

#[test]
fn a_forwarder_cannot_substitute_an_unrelated_value_for_the_old_slot() {
    let errors = diagnostics(
        "fn exchange<T>(slot:&mut T, replacement:T)->T { replacement }
         fn forward<T>(slot:&mut T, replacement:T)->T { exchange(slot,replacement) }",
        "fn escape(slot:&mut &Int)->&Int { let local=7; forward(slot,&local) }",
    );
    assert!(errors.iter().any(|error| error.contains("local") &&
        error.contains("does not live long enough")), "{errors:?}");
}

#[test]
fn exchanging_a_value_cannot_install_a_local_loan_in_caller_storage() {
    let errors = diagnostics(
        "fn exchange<T>(slot:&mut T, new_value:T)->T { @intrinsic(\"replace\",slot,new_value) }",
        "fn install(slot:&mut &Int) { let local=5; let old=exchange(slot,&local); }",
    );
    assert!(errors.iter().any(|error| error.contains("local") &&
        (error.contains("does not live long enough") || error.contains("shorter"))), "{errors:?}");
}

#[test]
fn exchanging_with_a_type_supplied_default_preserves_an_owned_payload() {
    let errors = diagnostics(
        r#"type Fresh is protocol { fn fresh()->Self; };
        type Holder<T> is { public value:T };
        fn exchange<T>(slot:&mut T, replacement:T)->T { @intrinsic("replace",slot,replacement) }
        fn take<T:Fresh>(slot:&mut T)->T { exchange(slot,T.fresh()) }"#,
        "fn select<T:Fresh>(mut owned:Holder<T>)->T { take(&mut owned.value) }",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn exchanging_one_field_keeps_its_previous_borrow_separate_from_its_sibling() {
    let source = r#"type Pair is { public left:&Int, public right:&Int };
        fn exchange<T>(slot:&mut T, value:T)->T { @intrinsic("replace",slot,value) }"#;
    let good = diagnostics(source, "fn chosen(from:&Int, incoming:&Int)->&Int {
        let short=9; let mut pair=Pair{left:from,right:&short}; exchange(&mut pair.left,incoming)
    }");
    assert!(good.is_empty(), "{good:?}");
    for write in ["pair.right=&short;", "{ let alias=&mut pair; alias.right=&short; }"] {
        let good = diagnostics(source, &format!("fn chosen(from:&Int, incoming:&Int)->&Int {{
            let short=9; let mut pair=Pair{{left:from,right:from}}; {write}
            exchange(&mut pair.left,incoming)
        }}"));
        assert!(good.is_empty(), "{write}: {good:?}");
    }
    let bad = diagnostics(source, "fn chosen(from:&Int, incoming:&Int)->&Int {
        let short=9; let mut pair=Pair{left:&short,right:from}; exchange(&mut pair.left,incoming)
    }");
    assert!(bad.iter().any(|error| error.contains("short") && error.contains("does not live long enough")), "{bad:?}");
}

#[test]
fn writing_a_short_loan_through_an_alias_discards_old_field_provenance() {
    let declarations = r#"type Pair is { public left:&Int, public right:&Int };
        fn exchange<T>(slot:&mut T, value:T)->T { @intrinsic("replace",slot,value) }"#;
    for caller in [
        "fn chosen(from:&Int,incoming:&Int)->&Int {
            let short=5; let mut pair=Pair{left:from,right:from};
            { let alias=&mut pair; alias.left=&short; }
            exchange(&mut pair.left,incoming)
        }",
        "fn chosen(from:&Int,incoming:&Int)->&Int {
            let mut pair=Pair{left:from,right:from};
            { let short=5; let alias=&mut pair; alias.left=&short; }
            exchange(&mut pair.left,incoming)
        }",
        "fn chosen(from:&Int,incoming:&Int)->&Int {
            let short=5; let mut pair=Pair{left:from,right:from};
            { let alias=&mut pair; let inner=&mut *alias; inner.left=&short; }
            exchange(&mut pair.left,incoming)
        }",
        "fn chosen(from:&Int,incoming:&Int)->&Int {
            let short=5; let mut pair=Pair{left:from,right:from};
            { let alias=&mut pair; { let pair=Pair{left:from,right:from}; alias.left=&short; } }
            exchange(&mut pair.left,incoming)
        }",
    ] {
        let errors = diagnostics(declarations, caller);
        assert!(errors.iter().any(|error| error.contains("short") && error.contains("does not live long enough")), "{caller}: {errors:?}");
    }
    let errors = diagnostics(declarations, "type Through is {public target:&mut Pair};
        implement Through { fn store(&mut self) { let short=5; self.target.left=&short; } }");
    assert!(errors.iter().any(|error| error.contains("short") && error.contains("does not live long enough")), "{errors:?}");
}

#[test]
fn writing_a_managed_loan_through_a_raw_pointer_cannot_reuse_old_field_provenance() {
    let scalar = diagnostics("", "fn write(raw:*mut Int, value:Int) {
        unsafe { *raw=value; };
    }");
    assert!(scalar.is_empty(), "{scalar:?}");
    let errors = diagnostics(
        r#"type Pair is { public left:&Int, public right:&Int };
        fn exchange<T>(slot:&mut T, value:T)->T { @intrinsic("replace",slot,value) }"#,
        "fn chosen(from:&Int,incoming:&Int)->&Int {
            let short=5; let mut pair=Pair{left:from,right:from};
            unsafe { let raw=(&mut pair) as *mut Pair; (*raw).left=&short; }
            exchange(&mut pair.left,incoming)
        }",
    );
    assert!(errors.iter().any(|error| error.contains("does not live long enough")
        || error.contains("cannot establish the original carrier")), "{errors:?}");
}

#[test]
fn a_borrowed_receiver_can_return_its_own_type_with_different_provenance() {
    let declaration = "type Duplicate is protocol {fn duplicate(&self)->Self;};";
    let caller = "fn receive(input:&Int)->Payload {
        let local=Payload{ptr:input,marker:2}; local.duplicate()
    }";
    for (pointer, borrows_local) in [("self.ptr", false), ("&self.marker", true)] {
        let implementation = format!("{declaration}
            implement Duplicate for Payload {{
                fn duplicate(&self)->Self {{Payload{{ptr:{pointer},marker:self.marker}}}}
            }}");
        let errors = diagnostics(&implementation, caller);
        if borrows_local {
            assert!(errors.iter().any(|error| error.contains("local")
                && error.contains("does not live long enough")), "{errors:?}");
        } else {
            assert!(errors.is_empty(), "{errors:?}");
        }
    }
}

#[test]
fn typed_raw_stores_retain_the_value_in_the_addressed_record_field() {
    for operation in ["ptr_write", "ptr_write_volatile", "intrinsic_ptr_write"] {
        let declarations = format!(r#"type Pair is {{ public left:&Int, public right:&Int }};
            fn exchange<T>(slot:&mut T, value:T)->T {{ @intrinsic("replace",slot,value) }}
            unsafe fn store<T>(value:T, destination:*mut T) {{ @intrinsic("{operation}",destination,value) }}"#);
        for field in ["left", "right"] {
            for store in [
                format!(r#"@intrinsic("{operation}",(&mut pair.{field}) as *mut &Int,&short)"#),
                format!("store(&short,(&mut pair.{field}) as *mut &Int)"),
                format!("let raw=(&mut pair.{field}) as *mut &Int; let alias=raw; store(&short,alias)"),
            ] {
                let source = format!("fn caller(from:&Int,incoming:&Int)->&Int {{
                    let short=5; let mut pair=Pair{{left:from,right:from}};
                    unsafe {{ {store}; }}; exchange(&mut pair.left,incoming)
                }}");
                let errors = diagnostics(&declarations, &source);
                if field == "left" {
                    assert!(errors.iter().any(|error| error.contains("short") && error.contains("does not live long enough")),
                        "{store}: {errors:?}");
                } else {
                    assert!(errors.is_empty(), "{store}: {errors:?}");
                }
            }
        }
    }
}

#[test]
fn a_raw_pointer_argument_cannot_hide_writes_by_an_opaque_callee() {
    let declarations = r#"type Pair is { public left:&Int, public right:&Int };
        fn exchange<T>(slot:&mut T, value:T)->T { @intrinsic("replace",slot,value) }
        fn selected(slot:*mut &Int, value:&Int) {}"#;
    let errors = diagnostics(declarations, "fn caller(selected:fn(*mut &Int,&Int),from:&Int,incoming:&Int)->&Int {
        let short=5; let mut pair=Pair{left:from,right:from};
        unsafe { selected((&mut pair.left) as *mut &Int,&short); };
        exchange(&mut pair.left,incoming)
    }");
    assert!(errors.iter().any(|error| error.contains("does not live long enough")
        || error.contains("cannot establish the lifetime")), "{errors:?}");
    let bad = diagnostics("", r#"fn caller(raw:*mut &Int) { let short=5;
        unsafe { @intrinsic("ptr_write",raw,&short); }; }"#);
    assert!(bad.iter().any(|error| error.contains("cannot establish the original carrier")
        || error.contains("does not live long enough")), "{bad:?}");
    let bad = diagnostics("", r#"fn caller(raw:*const Int) { unsafe { @intrinsic("ptr_write",raw,2); }; }"#);
    assert!(bad.iter().any(|error| error.contains("requires a writable pointer")), "{bad:?}");
}

#[test]
fn an_imported_raw_store_exposes_its_unit_result_and_addressed_slot() {
    let provider = r#"public unsafe fn put<T>(value:T, address:*mut T) {
        @intrinsic("ptr_write",address,value)
    }"#;
    for field in ["left", "right"] {
        let consumer = format!(r#"mount owner.{{put as write}};
            type Pair is {{public left:&Int,public right:&Int}};
            fn exchange<T>(slot:&mut T,value:T)->T {{ @intrinsic("replace",slot,value) }}
            fn read(from:&Int,incoming:&Int)->&Int {{
                let short=5; let mut pair=Pair{{left:from,right:from}};
                unsafe {{write(&short,(&mut pair.{field}) as *mut &Int);}};
                exchange(&mut pair.left,incoming)
            }}"#);
        let errors = imported_diagnostics([("owner", provider.into()), ("consumer", consumer.into())]);
        if field == "left" {
            assert!(errors.iter().any(|error| error.contains("short")
                && error.contains("does not live long enough")), "{errors:?}");
        } else {
            assert!(errors.is_empty(), "{errors:?}");
        }
    }
    let wrong = imported_diagnostics([
        ("owner", provider.into()),
        ("consumer", "mount owner.put; fn wrong(address:*mut Int)->Int {unsafe {put(2,address)}}".into()),
    ]);
    assert!(wrong.iter().any(|error| error.contains("Unit") && error.contains("Int")), "{wrong:?}");
}

#[test]
fn writing_a_reference_slot_discards_its_previous_exact_identity() {
    for (operation, escapes) in [
        (r#"@intrinsic("ptr_write",address,&short)"#, true),
        (r#"@intrinsic("ptr_write",address,from)"#, false),
        ("*address=&short", true),
        ("*address=from", false),
        ("callback(address,&short)", true),
    ] {
        let caller = format!("fn caller(from:&Int, callback:fn(*mut &Int,&Int))->&Int {{
            let short=5; let mut slot=from;
            unsafe {{ let address=(&mut slot) as *mut &Int; {operation}; }};
            let result=slot; result
        }}");
        let errors = diagnostics("", &caller);
        if escapes {
            assert!(errors.iter().any(|error| error.contains("does not live long enough")
                || error.contains("cannot establish the lifetime")), "{operation}: {errors:?}");
        } else {
            assert!(errors.is_empty(), "{operation}: {errors:?}");
        }
    }
}

#[test]
fn retiring_a_raw_pointer_in_a_closed_callee_preserves_the_pointee() {
    for permission in ["*mut", "*volatile mut"] {
        let declarations = format!(r#"
            type Pair is {{ public left:&Int, public right:&Int }};
            fn exchange<T>(slot:&mut T, value:T)->T {{ @intrinsic("replace",slot,value) }}
            fn ptr_write(slot:{permission} &Int, value:&Int) {{}}
            fn forward(slot:{permission} &Int, value:&Int) {{ ptr_write(slot,value); }}
        "#);
        for selected in ["ptr_write", "forward"] {
            let caller = format!("fn caller(from:&Int,incoming:&Int)->&Int {{
                let short=5; let mut pair=Pair{{left:from,right:from}};
                unsafe {{ {selected}((&mut pair.left) as {permission} &Int,&short); }};
                exchange(&mut pair.left,incoming)
            }}");
            let errors = diagnostics(&declarations, &caller);
            assert!(errors.is_empty(), "{permission} {selected}: {errors:?}");
        }
    }
}

#[test]
fn retiring_a_raw_pointer_does_not_discharge_a_sibling_parameters_destructor() {
    let declarations = "type Drop is protocol {fn drop(&mut self);};
        type affine DeferredWrite is {target:&mut Payload,source:&Int};
        implement Drop for DeferredWrite {fn drop(&mut self) {self.target.ptr=self.source;}}
        fn finish(address:*mut Int, pending:DeferredWrite) {}";
    assert!(diagnostics(declarations, "").is_empty());
    let errors = diagnostics(declarations, "fn caller(input:&Int)->Payload {
        let mut cell=0; let mut container=Payload{ptr:input,marker:2};
        {let local=3; finish((&mut cell) as *mut Int,DeferredWrite{target:&mut container,source:&local});}
        Payload{ptr:container.ptr,marker:container.marker}
    }");
    assert!(errors.iter().any(|error| error.contains("does not live long enough")
        || error.contains("cannot establish the lifetime")), "{errors:?}");
}

#[test]
fn branch_results_keep_only_the_loans_in_the_returned_payload() {
    for (source, escapes) in [("input", false), ("&local", true)] {
        let caller = format!("fn receive(input:&Int,choose:Bool)->Payload {{
            let local=3;
            if choose {{
                let temporary=Payload{{ptr:&local,marker:1}};
                Payload{{ptr:{source},marker:temporary.marker}}
            }} else {{Payload{{ptr:input,marker:2}}}}
        }}");
        let errors = diagnostics("", &caller);
        if escapes {
            assert!(errors.iter().any(|error| error.contains("local")
                && error.contains("does not live long enough")), "{errors:?}");
        } else {
            assert!(errors.is_empty(), "{errors:?}");
        }
    }
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
    for (payload, accepted) in [("source.ptr", true), ("key", false)] {
        for call in ["choose", "service.selected"] {
            let errors = imported_diagnostics([
                ("owner", format!("public type Payload is {{public ptr:&Int,public marker:Int}}; public fn selected(source:&Payload,key:&Int)->Payload {{Payload{{ptr:{payload},marker:source.marker}}}}").into()),
                ("decoy", "public type Payload is {public ptr:&Int,public marker:Int}; public fn selected(source:&Payload,key:&Int)->Payload {Payload{ptr:key,marker:source.marker}}".into()),
                ("consumer", format!("mount decoy.*; mount owner.{{self as service,Payload as Carrier,selected as choose}}; fn receive(input:&Int)->Carrier {{let container=Carrier{{ptr:input,marker:2}}; let key=3; {call}(&container,&key)}}").into()),
            ]);
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
fn whole_parameter_transfer_preserves_payload_provenance() {
    let declarations = "fn forward<T>(value:T)->T {value}";
    let errors = diagnostics(
        declarations,
        "fn receive(input:&Int)->Payload {
            let container=Payload{ptr:input,marker:2}; forward(container)
        }",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn whole_parameter_transfer_does_not_erase_a_local_payload_loan() {
    for (declarations, call) in [
        ("fn forward<T>(value:T)->T {value}", "forward(container)"),
        (
            "implement Payload {fn forward(&self,value:Payload)->Payload {value}}",
            "container.forward(container)",
        ),
    ] {
        assert!(diagnostics(declarations, "").is_empty());
        let errors = diagnostics(
            declarations,
            &format!("fn receive()->Payload {{
                let local=3; let container=Payload{{ptr:&local,marker:2}};
                {call}
            }}"),
        );
        assert!(
            errors.iter().any(|error|
                error.contains("local") && error.contains("does not live long enough")),
            "{call}: {errors:?}"
        );
    }
}

#[test]
fn transferred_parameter_does_not_erase_another_parameters_cleanup() {
    let declarations = "type Drop is protocol {fn drop(&mut self);};
        type affine DeferredWrite is {target:&mut Payload,source:&Int};
        implement Drop for DeferredWrite {fn drop(&mut self) {self.target.ptr=self.source;}}
        fn forward_with_cleanup<T>(value:T,pending:DeferredWrite)->T {value}";
    assert!(diagnostics(declarations, "").is_empty());
    let errors = diagnostics(
        declarations,
        "fn receive(input:&Int)->Payload {let mut container=Payload{ptr:input,marker:2};
         {let local=3; forward_with_cleanup(7,DeferredWrite{target:&mut container,source:&local});}
         Payload{ptr:container.ptr,marker:container.marker}}",
    );
    assert!(
        errors.iter().any(|error| error.contains("does not live long enough")
            || error.contains("cannot establish the lifetime")),
        "{errors:?}"
    );
}

#[test]
fn transferred_parameter_does_not_erase_local_cleanup() {
    let declarations = "type Drop is protocol {fn drop(&mut self);};
        type affine DeferredWrite is {target:&mut Payload,source:&Int};
        implement Drop for DeferredWrite {fn drop(&mut self) {self.target.ptr=self.source;}}
        fn forward_with_cleanup<T>(value:T,target:&mut Payload,source:&Int)->T {
            let pending=DeferredWrite{target,source}; value
        }";
    assert!(diagnostics(declarations, "").is_empty());
    let errors = diagnostics(
        declarations,
        "fn receive(input:&Int)->Payload {let mut container=Payload{ptr:input,marker:2};
         {let local=3; forward_with_cleanup(7,&mut container,&local);}
         Payload{ptr:container.ptr,marker:container.marker}}",
    );
    assert!(
        errors.iter().any(|error| error.contains("does not live long enough")
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
    // A reference-valued intermediate can point outside both holder slots.
    let indirect = "type Holder is {item:&mut Payload};
        implement Payload {fn replace(&mut self,source:&Int) {self.ptr=source;}}
        fn selected(a:&mut Holder,b:&mut Holder,first:Bool)->&mut Holder {if first {a} else {b}}";
    let errors = diagnostics(indirect,
        "fn receive(a:&mut Holder,b:&mut Holder,first:Bool) {
         let local=3; selected(a,b,first).item.replace(&local); }");
    assert!(errors.iter().any(|error| error.contains("does not live long enough")
        || error.contains("cannot establish the lifetime")), "{errors:?}");
}

#[test]
fn a_selected_helper_retains_the_loans_of_either_conditional_result() {
    let caller = "fn receive(input:&Int, flag:Bool)->Payload {
        let local=3; let container=Payload{ptr:input,marker:1};
        selected(&container, &local, flag)
    }";
    for (left, right, escapes) in [
        ("source.ptr", "source.ptr", false),
        ("key", "source.ptr", true),
        ("source.ptr", "key", true),
    ] {
        let helper = format!("fn selected(source:&Payload, key:&Int, flag:Bool)->Payload {{
            if flag {{ Payload{{ptr:{left},marker:1}} }}
            else {{ Payload{{ptr:{right},marker:2}} }}
        }}");
        let errors = diagnostics(&helper, caller);
        if escapes {
            assert!(errors.iter().any(|error| error.contains("local")
                && error.contains("does not live long enough")), "{errors:?}");
        } else {
            assert!(errors.is_empty(), "{errors:?}");
        }
    }
}

#[test]
fn an_opaque_field_write_preserves_only_disjoint_inline_payloads() {
    let declarations = "type Branch is { left: Payload, right: Payload };
        type Bundle is { nested: Branch, other: Payload };
        fn update(slot: &mut Payload);
        fn replace_all(slot: &mut Bundle);";
    for projection in ["value.nested.right.ptr", "value.other.ptr"] {
        let source = format!("fn read(mut value:Bundle)->&Int {{
            update(&mut value.nested.left); {projection} }}");
        let errors = diagnostics(declarations, &source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
    let errors = diagnostics(declarations, "fn incoming()->Payload;
        fn after_assignment(mut value:Bundle)->&Int {
            value.nested.left=incoming(); value.nested.right.ptr }");
    assert!(errors.is_empty(), "{errors:?}");
    let errors = diagnostics(declarations, "fn through_alias(mut value:Bundle)->&Int {
        { let target=&mut value.nested.left; update(target); }
        value.other.ptr }");
    assert!(errors.is_empty(), "{errors:?}");
    for (result, expression) in [
        ("&Int", "value.nested.left.ptr"),
        ("Branch", "value.nested"),
        ("Bundle", "value"),
    ] {
        let source = format!("fn read(mut value:Bundle)->{result} {{
            update(&mut value.nested.left); {expression} }}");
        let errors = diagnostics(declarations, &source);
        assert!(errors.iter().any(|error| error.contains("cannot establish the lifetime")),
            "{source}: {errors:?}");
    }
    let errors = diagnostics(declarations, "fn read(mut value:Bundle)->&Int {
        replace_all(&mut value); value.other.ptr }");
    assert!(errors.iter().any(|error| error.contains("cannot establish the lifetime")), "{errors:?}");
}

#[test]
fn disjoint_field_access_keeps_its_own_local_loan_and_opaque_capture_writes() {
    let declarations = "type Pair is { changed: Payload, kept: Payload };
        fn update(value:&mut Payload);";
    let errors = diagnostics(declarations, "fn dangling(input:&Int)->&Int {
        let short=19; let mut value=Pair {
            changed:Payload{ptr:input, marker:1}, kept:Payload{ptr:&short, marker:2}
        };
        update(&mut value.changed); value.kept.ptr }");
    assert!(errors.iter().any(|error| error.contains("short") && error.contains("does not live long enough")),
        "{errors:?}");

    for body in [
        "fn replace_pair(slot:&mut Pair);
         implement Pair { fn after_whole_write(&mut self)->&Int {
             replace_pair(self); self.kept.ptr } }",
        "fn callback_write(mut value:Pair, callback:fn(&mut Payload))->&Int {
            callback(&mut value.kept); value.kept.ptr }",
        "fn captured_write(mut value:Pair)->&Int {
            { let change=|| update(&mut value.kept); change(); } value.kept.ptr }",
    ] {
        let errors = diagnostics(declarations, body);
        assert!(errors.iter().any(|error| error.contains("cannot establish the lifetime")
            || error.contains("borrow")), "{body}: {errors:?}");
    }
}

#[test]
fn generic_storage_handoff_is_independent_of_branch_order() {
    let declarations = "type Error<T> is { guard:T };
        implement<T> Error<T> { fn make(guard:T)->Error<T> { Error { guard } } }
        type Response<T> is Yes(T) | No(Error<T>);
        type Envelope<T> is { poisoned:Bool, data:T };";
    for (yes, no) in [
        ("Response.No(Error.make(self.data))", "Response.Yes(self.data)"),
        ("Response.Yes(self.data)", "Response.No(Error.make(self.data))"),
    ] {
        let body = format!("implement<T> Envelope<T> {{ fn take(self)->Response<T> {{
            if self.poisoned {{ {yes} }} else {{ {no} }} }} }}");
        let errors = diagnostics(declarations, &body);
        assert!(errors.is_empty(), "{body}: {errors:?}");
    }
}

#[test]
fn an_uncertain_write_reaches_only_its_possible_successors() {
    let declarations = "type Carrier is { item:&Int };
        fn change(p:&mut Carrier);
        fn writes(p:&mut Carrier)->Bool;";
    let accepted = diagnostics(declarations,
        "fn inspect(mut p:Carrier, flag:Bool, anchor:&Int)->&Int {
            if flag { change(&mut p); return anchor; } p.item }");
    assert!(accepted.is_empty(), "{accepted:?}");
    for body in [
        "fn after(mut p:Carrier, flag:Bool)->&Int {
            if flag { change(&mut p); } p.item }",
        "fn after_else(mut p:Carrier, flag:Bool)->&Int {
            if flag {} else { change(&mut p); } p.item }",
        "fn guard(mut p:Carrier, flag:Bool, anchor:&Int)->&Int {
            if flag && writes(&mut p) { return anchor; } p.item }",
    ] {
        let errors = diagnostics(declarations, body);
        assert!(errors.iter().any(|error| error.contains("cannot establish the lifetime")),
            "{body}: {errors:?}");
    }
}
