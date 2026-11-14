//! Only the selected source body's typed writes can preserve payload origins.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(declarations: &str, caller: &str) -> List<Text> {
    let source = format!(
        "type Cell is {{ public pointer:&Int, public value:Int }}; {declarations} {caller}"
    );
    let module = FastParser::new()
        .parse_module_str(&source, FileId::new(1))
        .unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("write_probe");
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
        .map(|e| e.to_string().into())
        .collect()
}

#[test]
fn scalar_field_write_preserves_the_existing_reference_payload() {
    let found = errors(
        "fn update(cell:&mut Cell) { cell.value=73; }",
        "fn preserve(mut cell:Cell)->Cell { update(&mut cell); cell }",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn scalar_write_uses_the_declared_argument_slot() {
    let found = errors(
        "fn update(value:Int, unrelated:&Int, cell:&mut Cell) { cell.value=value; }",
        "fn preserve(mut cell:Cell)->Cell { let unrelated=4; update(73,&unrelated,&mut cell); cell }",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn nested_scalar_write_preserves_sibling_payloads() {
    let found = errors(
        "type Holder is { public cell:Cell }; fn update(holder:&mut Holder) { holder.cell.value=73; }",
        "fn preserve(mut holder:Holder)->Holder { update(&mut holder); holder }",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn scalar_receiver_write_uses_self_before_explicit_arguments() {
    let found = errors(
        "implement Cell { fn update(&mut self, value:Int, unrelated:&Int) { self.value=value; } }",
        "fn preserve(mut cell:Cell)->Cell { let unrelated=4; cell.update(73,&unrelated); cell }",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn immutable_callable_alias_preserves_the_selected_write_plan() {
    let found = errors(
        "fn update(cell:&mut Cell) { cell.value=73; }",
        "fn preserve(mut cell:Cell)->Cell { let selected=update; selected(&mut cell); cell }",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn stored_reference_write_keeps_the_injected_local_loan() {
    let found = errors(
        "fn update(cell:&mut Cell, source:&Int) { cell.value=73; cell.pointer=source; }",
        "fn preserve(input:&Int)->Cell { let mut cell=Cell{pointer:input,value:0}; { let local=73; update(&mut cell,&local); } cell }",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("local") && e.contains("does not live long enough")),
        "{found:?}"
    );
}

#[test]
fn unknown_callback_is_not_a_scalar_write_contract() {
    let found = errors(
        "fn update(cell:&mut Cell, action:fn(&mut Cell)) { cell.value=73; action(cell); }",
        "fn preserve(mut cell:Cell, action:fn(&mut Cell))->Cell { update(&mut cell,action); cell }",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("cannot establish the lifetime")),
        "{found:?}"
    );
}

#[test]
fn mutating_receiver_cannot_hide_a_shorter_lived_argument() {
    let found = errors(
        "implement Cell { fn update(&mut self,value:Int,source:&Int) { self.value=value; self.pointer=source; } }",
        "fn preserve(input:&Int)->Cell { let mut cell=Cell{pointer:input,value:0}; { let local=73; cell.update(73,&local); } cell }",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("local") && e.contains("does not live long enough")),
        "{found:?}"
    );
}

#[test]
fn changed_function_value_does_not_keep_the_previous_write_plan() {
    let found = errors(
        "fn update(cell:&mut Cell,source:&Int) { cell.value=73; } fn replace(cell:&mut Cell,source:&Int) { cell.pointer=source; }",
        "fn preserve(input:&Int)->Cell { let mut cell=Cell{pointer:input,value:0}; let mut action=update; action=replace; { let local=73; action(&mut cell,&local); } cell }",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("local") && e.contains("does not live long enough")),
        "{found:?}"
    );
}

#[test]
fn a_shared_alias_still_blocks_the_exclusive_scalar_write() {
    let found = errors(
        "fn update(cell:&mut Cell) { cell.value=73; }",
        "fn preserve(mut cell:Cell)->Int { let shared=&cell; update(&mut cell); shared.value }",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("cell") && e.contains("borrow")),
        "{found:?}"
    );
}

#[test]
fn a_context_input_is_not_mistaken_for_an_explicit_argument() {
    let declaration = "context Hidden { fn mutate(cell:&mut Cell); } fn update(cell:&mut Cell) using [Hidden] { cell.value=73; Hidden.mutate(cell); }";
    let source_errors = errors(declaration, "");
    assert!(source_errors.is_empty(), "{source_errors:?}");
    let found = errors(
        declaration,
        "fn preserve(mut cell:Cell)->Cell using [Hidden] { update(&mut cell); cell }",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("cannot establish the lifetime")),
        "{found:?}"
    );
}

#[test]
fn writing_through_another_reference_field_remains_conservative() {
    let found = errors(
        "type Holder is { public target:&mut Cell }; fn update(holder:&mut Holder) { holder.target.pointer=&holder.target.value; }",
        "fn preserve(mut holder:Holder)->Holder { update(&mut holder); holder }",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("cannot establish the lifetime")
                || e.contains("does not live long enough")),
        "{found:?}"
    );
}

#[test]
fn scalar_assignment_does_not_erase_previous_unknown_mutation() {
    let found = errors(
        "fn replace(cell:&mut Cell,source:&Int) { cell.pointer=source; } fn update(cell:&mut Cell) { cell.value=73; }",
        "fn preserve(input:&Int)->Cell { let mut cell=Cell{pointer:input,value:0}; {let local=4;replace(&mut cell,&local);} update(&mut cell); cell }",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("local") && e.contains("does not live long enough")),
        "{found:?}"
    );
}

#[test]
fn a_nominal_value_with_cleanup_is_not_a_primitive_scalar_write() {
    let declaration = "type Drop is protocol { fn drop(&mut self); }; type affine Deferred is {target:&mut Cell,source:&Int}; implement Drop for Deferred { fn drop(&mut self) {self.target.pointer=self.source;} } fn update(cell:&mut Cell, guard:Deferred) {cell.value=73;}";
    let found = errors(
        declaration,
        "fn preserve(input:&Int)->Cell { let mut cell=Cell{pointer:input,value:0}; let mut other=Cell{pointer:input,value:0}; {let local=4;let guard=Deferred{target:&mut cell,source:&local};update(&mut other,guard);} cell }",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("cell") && e.contains("cannot establish the lifetime")),
        "{found:?}"
    );
}

#[test]
fn renamed_source_mounts_keep_the_selected_owners_write_footprint() {
    use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
    for (write, accepted) in [
        ("cell.value=73;", true),
        ("cell.value=73;cell.pointer=key;", false),
    ] {
        for call in ["selected", "service.update"] {
            let mut registry = ModuleRegistry::new();
            for (owner, source) in [
                ("owner",format!("public type Cell is {{public pointer:&Int,public value:Int}}; public fn update(cell:&mut Cell,key:&Int)->Unit {{{write}}}")),
                ("decoy","public type Cell is {public pointer:&Int,public value:Int}; public fn update(cell:&mut Cell,key:&Int)->Unit {cell.pointer=key;}".into()),
                ("consumer",format!("mount decoy.*; mount owner.{{self as service,Cell as Carrier,update as selected}}; fn preserve(mut cell:Carrier)->Carrier {{let local=4;{call}(&mut cell,&local);cell}}")),
            ] {
                let id=registry.allocate_id();
                let file=FileId::new(id.as_u32());
                let ast=FastParser::new().parse_module_str(&source,file).unwrap();
                let path=ModulePath::from_str(owner);
                let exports=extract_exports_from_module(&ast,id,&path).unwrap();
                let mut info=ModuleInfo::new(id,path,ast,file,source.into());
                info.exports=exports;
                registry.register(info);
            }
            let module = registry.get_by_path("consumer").unwrap();
            let mut checker = TypeChecker::with_minimal_context();
            checker.register_primitives();
            checker.set_current_module_path("consumer");
            checker.set_module_registry_direct(registry);
            let mut found = checker.prepare_module_declarations(&module.ast);
            for item in &module.ast.items {
                if let Err(error) = checker.check_item(item) {
                    found.push(error);
                }
            }
            found.extend(checker.take_deferred_errors());
            let found: List<Text> = found
                .iter()
                .chain(checker.diagnostic_sources())
                .map(|e| e.to_string().into())
                .collect();
            if accepted {
                assert!(found.is_empty(), "{call}: {found:?}");
            } else {
                assert!(
                    found
                        .iter()
                        .any(|e| e.contains("local") && e.contains("does not live long enough")),
                    "{call}: {found:?}"
                );
            }
        }
    }
}

#[test]
fn generic_write_footprints_are_validated_at_the_actual_instantiation() {
    let declaration = "type Holder<T> is {public payload:T,public pointer:&Int}; fn update<T>(holder:&mut Holder<T>,value:T) {holder.payload=value;}";
    let accepted = errors(
        declaration,
        "fn preserve(mut holder:Holder<Int>)->Holder<Int> {update(&mut holder,73);holder}",
    );
    assert!(accepted.is_empty(), "{accepted:?}");
    let rejected = errors(
        declaration,
        "fn preserve(input:&Int)->Holder<&Int> {let mut holder=Holder{payload:input,pointer:input}; {let local=4;update(&mut holder,&local);} holder}",
    );
    assert!(
        rejected
            .iter()
            .any(|e| e.contains("local") && e.contains("does not live long enough")),
        "{rejected:?}"
    );
}

#[test]
fn a_scalar_field_read_does_not_retain_its_temporary_owner() {
    let found = errors(
        "fn update(cell:&mut Cell,source:&Cell) {cell.value=source.value;}",
        "fn preserve(input:&Int)->Cell {let mut cell=Cell{pointer:input,value:0}; {let local=4;let source=Cell{pointer:&local,value:73};update(&mut cell,&source);}cell}",
    );
    assert!(found.is_empty(), "{found:?}");
}


#[test]
fn compound_scalar_writes_preserve_unmodified_payloads() {
    let found = errors(
        "fn update(cell:&mut Cell) { cell.value+=1; }",
        "fn preserve(mut cell:Cell)->Cell { update(&mut cell); cell }",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn declared_sum_replacement_preserves_sibling_payloads() {
    let found = errors(
        "type Choice<T> is Empty | Full(T); type Holder is {public cell:Cell, public state:Choice<Int>}; fn reset(holder:&mut Holder) {holder.state=Choice.Empty;}",
        "fn preserve(mut holder:Holder)->Holder {reset(&mut holder);holder}",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn declared_sum_payload_replacement_preserves_sibling_payloads() {
    let found = errors(
        "type Choice<T> is Empty | Full(T); type Holder is {public cell:Cell, public state:Choice<Int>}; fn update(holder:&mut Holder, value:Int) {holder.state=Choice.Full(value);}",
        "fn preserve(mut holder:Holder)->Holder {update(&mut holder,73);holder}",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_sum_payload_cannot_hide_an_injected_reference() {
    let found = errors(
        "type Choice<T> is Empty | Full(T); type Holder is {public cell:Cell, public state:Choice<&Int>}; fn update(holder:&mut Holder, value:&Int) {holder.state=Choice.Full(value);}",
        "fn preserve(input:&Int)->Holder {let mut holder=Holder{cell:Cell{pointer:input,value:0},state:Choice.Empty}; {let local=73;update(&mut holder,&local);} holder}",
    );
    assert!(found.iter().any(|e| e.contains("local") && e.contains("does not live long enough")), "{found:?}");
}

#[test]
fn declared_sum_cleanup_cannot_invoke_an_unproved_destructor() {
    let found = errors(
        "type Drop is protocol {fn drop(&mut self);}; type Choice is Empty | Full(Int); implement Drop for Choice {fn drop(&mut self) {}} type Holder is {public cell:Cell,public state:Choice}; fn reset(holder:&mut Holder) {holder.state=Choice.Empty;}",
        "fn preserve(mut holder:Holder)->Holder {reset(&mut holder);holder}",
    );
    assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn a_sum_with_a_callback_payload_does_not_get_structural_cleanup() {
    let found = errors(
        "type Choice<T> is Empty | Full(T); type Holder is {public cell:Cell,public state:Choice<fn(&Int)->Int>}; fn reset(holder:&mut Holder) {holder.state=Choice.Empty;}",
        "fn preserve(mut holder:Holder)->Holder {reset(&mut holder);holder}",
    );
    assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn sum_writes_validate_the_whole_instantiated_payload() {
    for (payload, accepted) in [("Int", true), ("&Int", false)] {
        let found = errors(
            "type Choice<T> is Empty | Full(T); type Holder<T> is {public cell:Cell,public state:Choice<T>}; fn reset<T>(holder:&mut Holder<T>) {holder.state=Choice.Empty;}",
            &format!("fn preserve(mut holder:Holder<{payload}>)->Holder<{payload}> {{reset(&mut holder);holder}}"),
        );
        if accepted { assert!(found.is_empty(), "{found:?}"); }
        else { assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{found:?}"); }
    }
}

#[test]
fn variant_spelling_does_not_override_a_different_declared_owner() {
    let found = errors(
        "type Choice is Empty | Full(Int); type Other is Empty | Full(Int); type Holder is {public cell:Cell,public state:Choice}; fn reset(holder:&mut Holder) {holder.state=Other.Empty;}",
        "fn preserve(mut holder:Holder)->Holder {reset(&mut holder);holder}",
    );
    assert!(!found.is_empty(), "foreign owner was accepted");
}

#[test]
fn a_structural_tuple_write_cannot_bypass_its_selected_destructor() {
    let found = errors(
        "type Drop is protocol {fn drop(&mut self);}; implement Drop for (Int,Int) {fn drop(&mut self) {}} type Holder is {public cell:Cell,public pair:(Int,Int)}; fn update(holder:&mut Holder) {holder.pair=(1,2);}",
        "fn preserve(mut holder:Holder)->Holder {update(&mut holder);holder}",
    );
    assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn structural_cleanup_budget_exhaustion_keeps_the_unknown_write_rule() {
    // Flat source exercises the explicit work budget on an ordinary stack;
    // exhausting the proof is not evidence of reference-free storage.
    for (count, accepted) in [(64, true), (4097, false)] {
        let cases = (0..count).map(|index| format!("Case{index}")).collect::<List<_>>().join(" | ");
        let found = errors(
            &format!("type Many is {cases}; type Holder is {{public cell:Cell,public state:Many}}; fn reset(holder:&mut Holder) {{holder.state=Many.Case0;}}"),
            "fn preserve(mut holder:Holder)->Holder {reset(&mut holder);holder}",
        );
        if accepted { assert!(found.is_empty(), "{found:?}"); }
        else { assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{found:?}"); }
    }
}
