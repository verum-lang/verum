//! Only the selected source body's typed writes can preserve payload origins.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(declarations: &str, caller: &str) -> List<Text> {
    let source = format!(
        "type Cell is {{ public pointer:&Int, public value:Int }}; {declarations} {caller}"
    );
    module_errors("write_probe", &source)
}

fn module_errors(owner: &str, source: &str) -> List<Text> {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(1))
        .unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path(owner);
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
fn scalar_write_through_a_managed_field_retains_both_layers_of_loans() {
    let found = errors(
        "type Writer is { public target:&mut Cell }; implement Writer { fn set(&mut self, n:Int) { self.target.value=n; } }",
        "fn preserve(mut writer:Writer)->Writer { writer.set(1); writer.set(2); writer }",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn owned_slot_writes_retain_the_inserted_value_and_the_existing_slots() {
    let declarations = r#"
        type Deque<T> is { head:Int };
        unsafe fn place<T>(value:T, output:&mut Deque<T>, index:Int) {
            @intrinsic("deque_storage_write",output,index,value)
        }
    "#;
    for (caller, should_pass) in [
        (
            "fn preserve(mut queue:Deque<&Int>, value:&Int)->Deque<&Int> {unsafe {place(value,&mut queue,0);} queue}",
            true,
        ),
        (
            "fn preserve<T>(mut queue:Deque<T>, value:T)->Deque<T> {unsafe {place(value,&mut queue,0);} queue}",
            true,
        ),
        (
            "fn preserve<T>(mut queue:Deque<T>, first:T, second:T)->Deque<T> {unsafe {place(first,&mut queue,0);place(second,&mut queue,1);} queue}",
            true,
        ),
        (
            "fn retain(mut queue:Deque<&Int>)->Deque<&Int> {{let local=8; unsafe {place(&local,&mut queue,0);}} queue}",
            false,
        ),
        (
            "fn retain()->Deque<&Int> {let mut queue:Deque<&Int> = Deque{head:0}; let local=8; unsafe {place(&local,&mut queue,0);} queue}",
            false,
        ),
    ] {
        let source = format!("{declarations} {caller}");
        let found = module_errors("core.collections.deque", &source);
        if should_pass {
            assert!(found.is_empty(), "{caller}: {found:?}");
        } else {
            assert!(
                found
                    .iter()
                    .any(|e| e.contains("local") && e.contains("does not live long enough")),
                "{caller}: {found:?}"
            );
        }
    }
}

#[test]
fn an_incoming_generic_payload_cannot_lend_its_stack_slot_to_the_caller() {
    let found = errors("", "fn retain<T>(value:T)->&T {&value}");
    assert!(
        found
            .iter()
            .any(|error| error.contains("value") && error.contains("does not live long enough")),
        "{found:?}"
    );
}

#[test]
fn record_shorthand_copies_the_origins_of_slots_installed_since_construction() {
    let source = r#"
        type Deque<T> is {head:Int};
        type Holder is {items:Deque<&Int>};
        unsafe fn put<T>(items:&mut Deque<T>, n:Int, value:T) {@intrinsic("deque_storage_write",items,n,value)}
        fn capture()->Holder {
            let mut items:Deque<&Int> = Deque{head:0};
            let local=73;
            unsafe {put(&mut items,0,&local);}
            Holder{items}
        }
    "#;
    let found = module_errors("core.collections.deque", source);
    assert!(
        found
            .iter()
            .any(|e| e.contains("local") && e.contains("does not live long enough")),
        "{found:?}"
    );
}

#[test]
fn an_inline_slot_operation_checks_its_inputs_and_retains_the_moved_payload() {
    let declaration = "type Deque<T> is {head:Int};";
    for (function, expected) in [
        (
            r#"fn store<T>(mut items:Deque<T>, value:T)->Deque<T> {unsafe {@intrinsic("deque_storage_write",&mut items,0,value);} items}"#,
            None,
        ),
        (
            r#"fn store(mut items:Deque<&Int>)->Deque<&Int> {{let local=73; unsafe {@intrinsic("deque_storage_write",&mut items,0,&local);}} items}"#,
            Some("does not live long enough"),
        ),
        (
            r#"fn store(items:&Deque<Int>) {unsafe {@intrinsic("deque_storage_write",items,0,7);}}"#,
            Some("exclusive reference"),
        ),
        (
            r#"fn store(items:&mut Deque<Int>) {@intrinsic("deque_storage_write",items,0,7)}"#,
            Some("requires unsafe"),
        ),
        (
            r#"unsafe fn store(items:&mut Deque<Int>)->&Int {@intrinsic("deque_storage_write",items,0,7)}"#,
            Some("mismatch"),
        ),
    ] {
        let found = module_errors(
            "core.collections.deque",
            &format!("{declaration} {function}"),
        );
        match expected {
            Some(message) => assert!(
                found.iter().any(|error| error.contains(message)),
                "{function}: {found:?}"
            ),
            None => assert!(found.is_empty(), "{function}: {found:?}"),
        }
    }
}

#[test]
fn owned_slot_authority_requires_the_selected_carrier_and_complete_operation() {
    let caller = "fn preserve(mut queue:Deque<&Int>, value:&Int)->Deque<&Int> {unsafe {place(&mut queue,0,value);} queue}";
    for (owner, body) in [
        (
            "application",
            "@intrinsic(\"deque_storage_write\",output,index,value)",
        ),
        (
            "core.collections.deque",
            "@intrinsic(\"list_storage_write\",output,index,value)",
        ),
        (
            "core.collections.deque",
            "let copy=index; @intrinsic(\"deque_storage_write\",output,copy,value)",
        ),
    ] {
        let source = format!(
            "type Deque<T> is {{ pointer:&Int }}; unsafe fn place<T>(output:&mut Deque<T>, index:Int, value:T) {{{body}}} {caller}"
        );
        let found = module_errors(owner, &source);
        assert!(
            found
                .iter()
                .any(|e| e.contains("cannot establish the lifetime")),
            "{owner} {body}: {found:?}"
        );
    }
}

#[test]
fn an_explicit_input_does_not_cover_an_opaque_contexts_payload() {
    let declarations = r#"
        type Deque<T> is {head:Int};
        context Hidden {fn choose(seed:&Int)->&Int;}
        unsafe fn put<T>(items:&mut Deque<T>, index:Int, value:T) {@intrinsic("deque_storage_write",items,index,value)}
    "#;
    let checked = module_errors("core.collections.deque", declarations);
    assert!(checked.is_empty(), "{checked:?}");
    for store in [
        "unsafe {put(&mut items,0,Hidden.choose(seed));}",
        "let selected=Hidden.choose(seed); unsafe {put(&mut items,0,selected);}",
        "let mut selected=seed; selected=Hidden.choose(seed); unsafe {put(&mut items,0,selected);}",
    ] {
        let caller = format!(
            "{declarations} fn store(mut items:Deque<&Int>,seed:&Int)->Deque<&Int> using [Hidden] {{{store} items}}"
        );
        let found = module_errors("core.collections.deque", &caller);
        assert!(
            found
                .iter()
                .any(|e| e.contains("cannot establish the lifetime") && e.contains("items")),
            "{store}: {found:?}"
        );
    }
}

#[test]
fn installing_a_vacant_slot_does_not_observe_other_slots_after_an_opaque_write() {
    let declaration = r#"type Deque<T> is {head:Int}; unsafe fn put<T>(items:&mut Deque<T>, index:Int, value:T) {@intrinsic("deque_storage_write",items,index,value)}"#;
    let discard = "fn store(items:&mut Deque<&Int>, action:fn(&mut Deque<&Int>), value:&Int) {action(items); unsafe {put(items,0,value);}}";
    let found = module_errors(
        "core.collections.deque",
        &format!("{declaration} {discard}"),
    );
    assert!(found.is_empty(), "{found:?}");
    let read = "fn store(mut items:Deque<&Int>, action:fn(&mut Deque<&Int>), value:&Int)->Deque<&Int> {action(&mut items); unsafe {put(&mut items,0,value);} items}";
    let found = module_errors("core.collections.deque", &format!("{declaration} {read}"));
    assert!(found.iter().any(|error| error.contains("cannot establish the lifetime") && error.contains("items")), "{found:?}");
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
    // The guard keeps its exclusive loan of cell through the unknown cleanup
    // call. Reading cell is rejected at that borrow boundary, before a payload
    // lifetime could be reused. Do not require the later E312 diagnostic.
    assert!(
        found
            .iter()
            .any(|e| e.contains("cannot borrow `cell`") && e.contains("conflicting borrows")),
        "{found:?}"
    );
}

fn registered_errors<const N: usize>(sources: [(&str, Text); N]) -> List<Text> {
    use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
    let mut registry = ModuleRegistry::new();
    for (owner, source) in sources {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let ast = FastParser::new().parse_module_str(&source, file).unwrap();
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&ast, id, &path).unwrap();
        let mut info = ModuleInfo::new(id, path, ast, file, source);
        info.exports = exports;
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
    found
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|e| e.to_string().into())
        .collect()
}

fn declaration<'a>(source: &'a str, name: &str) -> &'a str {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(1))
        .unwrap();
    let item = module
        .items
        .iter()
        .find(|item| match &item.kind {
            verum_ast::ItemKind::Type(ty) => ty.name.as_str() == name,
            verum_ast::ItemKind::Function(function) => function.name.as_str() == name,
            _ => false,
        })
        .unwrap();
    // The item span starts at `type` / `fn`. Retain its source-line
    // modifiers so the ordinary mount checks its actual visibility.
    let start = source[..item.span.start as usize]
        .rfind('\n')
        .map_or(0, |line| line + 1);
    &source[start..item.span.end as usize]
}

#[test]
fn imported_list_storage_write_retains_only_the_source_payload() {
    let list = declaration(include_str!("../../../core/collections/list.vr"), "List");
    let write = declaration(
        include_str!("../../../core/intrinsics/memory.vr"),
        "list_storage_write",
    );
    let imports = "mount core.collections.list.List; mount core.intrinsics.memory.{list_storage_write as put};";
    for (caller, accepted) in [
        (
            "fn store<T>(mut items:List<T>, value:T)->List<T> {unsafe {put(&mut items,0,value);} items}",
            true,
        ),
        (
            "fn store(mut items:List<&Int>)->List<&Int> {{let local=73; unsafe {put(&mut items,0,&local);}} items}",
            false,
        ),
    ] {
        let found = registered_errors([
            ("core.collections.list", list.into()),
            (
                "core.intrinsics.memory",
                format!("mount core.collections.list.List; {write}").into(),
            ),
            ("consumer", format!("{imports} {caller}").into()),
        ]);
        if accepted {
            assert!(found.is_empty(), "{found:?}");
        } else {
            assert!(
                found
                    .iter()
                    .any(|e| e.contains("local") && e.contains("does not live long enough")),
                "{found:?}"
            );
        }
    }
}

#[test]
fn a_checked_slot_read_transfers_existing_loans_without_borrowing_its_local_owner() {
    let list = declaration(include_str!("../../../core/collections/list.vr"), "List");
    let memory = include_str!("../../../core/intrinsics/memory.vr");
    let read = declaration(memory, "list_storage_read");
    let write = declaration(memory, "list_storage_write");
    let imports = "mount core.collections.list.List; mount core.intrinsics.memory.{list_storage_read as read_slot, list_storage_write as put};";
    for (caller, expected) in [
        (
            "unsafe fn extract<T>(items:List<T>)->T {unsafe {read_slot(&items,0)}}",
            None,
        ),
        (
            "unsafe fn extract(items:List<&Int>)->&Int {unsafe {read_slot(&items,0)}}",
            None,
        ),
        (
            "unsafe fn move_slot<T>(items:List<T>,mut out:List<T>)->List<T> {unsafe {put(&mut out,0,read_slot(&items,0));} out}",
            None,
        ),
        (
            "unsafe fn move_slot<T>(items:List<T>,mut out:List<T>)->List<T> {let old=unsafe {read_slot(&items,0)}; unsafe {put(&mut out,0,old);} out}",
            None,
        ),
        (
            "unsafe fn extract(mut items:List<&Int>)->&Int {let local=73; unsafe {put(&mut items,0,&local); read_slot(&items,0)}}",
            Some("local"),
        ),
        (
            "unsafe fn extract(mut items:List<&Int>, opaque:fn(&mut List<&Int>))->&Int {opaque(&mut items); unsafe {read_slot(&items,0)}}",
            Some("cannot establish the lifetime"),
        ),
    ] {
        let found = registered_errors([
            ("core.collections.list", list.into()),
            (
                "core.intrinsics.memory",
                format!("mount core.collections.list.List; {read} {write}").into(),
            ),
            ("consumer", format!("{imports} {caller}").into()),
        ]);
        match expected {
            None => assert!(found.is_empty(), "{caller}: {found:?}"),
            Some(text) => assert!(
                found.iter().any(|e| e.contains(text)),
                "{caller}: {found:?}"
            ),
        }
    }
}

#[test]
fn taking_a_slot_requires_its_exclusive_owner_and_retains_the_moved_values_loans() {
    let ring = include_str!("../../../core/collections/deque.vr");
    let take = declaration(ring, "deque_storage_take");
    let write = declaration(ring, "deque_storage_write");
    let owner = "type Deque<T> is {head:Int};";
    for (caller, expected) in [
        (
            "unsafe fn extract<T>(mut items:Deque<T>)->T {unsafe {deque_storage_take(&mut items,0)}}",
            None,
        ),
        (
            r#"unsafe fn extract<T>(mut items:Deque<T>)->T {@intrinsic("deque_storage_take",&mut items,0)}"#,
            None,
        ),
        (
            "unsafe fn extract(mut items:Deque<&Int>)->&Int {let local=73; unsafe {deque_storage_write(&mut items,0,&local); deque_storage_take(&mut items,0)}}",
            Some("local"),
        ),
        (
            r#"unsafe fn extract(items:&Deque<Int>)->Int {@intrinsic("deque_storage_take",items,0)}"#,
            Some("exclusive reference"),
        ),
        (
            r#"fn extract(items:&mut Deque<Int>)->Int {@intrinsic("deque_storage_take",items,0)}"#,
            Some("requires unsafe"),
        ),
        (
            r#"unsafe fn extract(items:&mut Deque<Int>)->Bool {@intrinsic("deque_storage_take",items,0)}"#,
            Some("mismatch"),
        ),
        (
            r#"unsafe fn extract(items:&mut Deque<Int>)->Int {@intrinsic("deque_storage_take",items,true)}"#,
            Some("mismatch"),
        ),
    ] {
        let found = module_errors(
            "core.collections.deque",
            &format!("{owner} {take} {write} {caller}"),
        );
        match expected {
            None => assert!(found.is_empty(), "{caller}: {found:?}"),
            Some(text) => assert!(
                found.iter().any(|e| e.contains(text)),
                "{caller}: {found:?}"
            ),
        }
    }
}

#[test]
fn dynamic_list_push_retains_the_inserted_value_in_its_canonical_owner() {
    let source = include_str!("../../../core/collections/list.vr");
    let list = declaration(source, "List");
    let body = source.split_once("    public fn push(").unwrap().1;
    let end = body.find("\n    }").unwrap();
    let method = format!("implement<T> List<T> {{ public fn push({} }} }}", &body[..end]);
    for (provider, caller, expected) in [
        ("core.collections.list", "fn collect<T>(mut items:List<T>, a:T, b:T)->List<T> {items.push(a);items.push(b);items}", None),
        ("core.collections.list", "fn collect(mut items:List<&Int>, a:&Int)->List<&Int> {items.push(a);items}", None),
        ("core.collections.list", "fn collect(mut items:List<&Int>)->List<&Int> {{let local=11;items.push(&local);}items}", Some("local")),
        ("core.collections.list", "fn collect(mut items:List<&Int>, a:&Int,f:fn(&mut List<&Int>))->List<&Int> {f(&mut items);items.push(a);items}", Some("cannot establish the lifetime")),
        ("other", "fn collect<T>(mut items:List<T>, a:T, b:T)->List<T> {items.push(a);items.push(b);items}", Some("cannot establish the lifetime")),
    ] {
        let found = registered_errors([
            (provider, format!("{list} {method}").into()),
            ("consumer", format!("mount {provider}.List; {caller}").into()),
        ]);
        match expected {
            None => assert!(found.is_empty(), "{provider}: {caller}: {found:?}"),
            Some(text) => assert!(found.iter().any(|error| error.contains(text)), "{provider}: {caller}: {found:?}"),
        }
    }
    let source = "type State is {public cursor:Int}; type Parser is {}; implement Parser { fn read(&self, state:&mut State)->&Int {&state.cursor} }";
    let retain = "fn collect(parser:&Parser, state:&mut State, mut items:List<&Int>)->List<&Int> {let next=parser.read(state);items.push(next);state.cursor=state.cursor+1;items}";
    let found = registered_errors([
        ("core.collections.list", format!("{list} {method}").into()),
        ("consumer", format!("mount core.collections.list.List; {source} {retain}").into()),
    ]);
    assert!(found.iter().any(|error| error.contains("cannot borrow `state`") && error.contains("conflicting borrows")), "{found:?}");

    let extension = "type Push<T> is protocol { fn push(&mut self, value:T); }; implement<T> Push<T> for List<T> { fn push(&mut self, value:T) { self.len = self.len + 1; } }";
    let found = registered_errors([
        ("core.collections.list", format!("{list} {extension}").into()),
        ("consumer", "mount core.collections.list.{List,Push}; fn collect<T>(mut items:List<T>, a:T, b:T)->List<T> {items.push(a);items.push(b);items}".into()),
    ]);
    assert!(found.iter().any(|error| error.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn a_taken_caller_owned_element_does_not_borrow_the_ring_metadata() {
    let ring = include_str!("../../../core/collections/deque.vr");
    let take = declaration(ring, "deque_storage_take");
    let put = declaration(ring, "deque_storage_write");
    let owner = "type Deque<T> is {head:Int};";
    let selected = "fn advance<T>(items:&mut Deque<T>)->T {let moved=unsafe {deque_storage_take(items,0)}; items.head=1; moved}";
    let owned = "implement<T> Deque<T> { fn consume(mut self)->T {let moved=unsafe {deque_storage_take(&mut self,0)}; self.head=1; moved} }";
    let found = module_errors("core.collections.deque", &format!("{owner} {take} {put} {selected} {owned}"));
    assert!(found.is_empty(), "{found:?}");
    let retaining = "implement<T> Deque<T> { fn invalid(self)->&Int {&self.head} }";
    let found = module_errors("core.collections.deque", &format!("{owner} {take} {retaining}"));
    assert!(found.iter().any(|e| e.contains("self") && e.contains("does not live long enough")), "{found:?}");
}

#[test]
fn renamed_source_mounts_keep_the_selected_owners_write_footprint() {
    for (write, accepted) in [
        ("cell.value=73;", true),
        ("cell.value=73;cell.pointer=key;", false),
    ] {
        for call in ["selected", "service.update"] {
            let found = registered_errors([
                ("owner",format!("public type Cell is {{public pointer:&Int,public value:Int}}; public fn update(cell:&mut Cell,key:&Int)->Unit {{{write}}}").into()),
                ("decoy","public type Cell is {public pointer:&Int,public value:Int}; public fn update(cell:&mut Cell,key:&Int)->Unit {cell.pointer=key;}".into()),
                ("consumer",format!("mount decoy.*; mount owner.{{self as service,Cell as Carrier,update as selected}}; fn preserve(mut cell:Carrier)->Carrier {{let local=4;{call}(&mut cell,&local);cell}}").into()),
            ]);
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
fn imported_field_overwrites_check_linear_values_at_calls_and_at_function_value_formation() {
    let owner = "public type Holder<T> is {public payload:T}; public fn update<T>(holder:&mut Holder<T>,value:T){holder.payload=value;}";
    let imports = "mount owner.{Holder,update as replace}; type linear Ticket is {id:Int};";
    for (caller, expected) in [
        (
            "fn good(holder:&mut Holder<Int>) {replace(holder,73);}",
            None,
        ),
        ("fn good()->fn(&mut Holder<Int>,Int)->Unit {replace}", None),
        (
            "fn bad(holder:&mut Holder<Ticket>, value:Ticket) {replace(holder,value);}",
            Some("linear"),
        ),
        (
            "fn bad()->fn(&mut Holder<Ticket>,Ticket)->Unit {replace}",
            Some("linear"),
        ),
        (
            "fn hidden<T>()->fn(&mut Holder<T>,T)->Unit {replace}",
            Some("cannot overwrite"),
        ),
    ] {
        let found = registered_errors([
            ("owner", owner.into()),
            ("consumer", format!("{imports} {caller}").into()),
        ]);
        match expected {
            Some(text) => assert!(
                found
                    .iter()
                    .any(|e| e.contains("overwrite") && e.contains(text)),
                "{caller}: {found:?}"
            ),
            None => assert!(found.is_empty(), "{caller}: {found:?}"),
        }
    }
}

#[test]
fn overwriting_a_field_preserves_owned_linear_siblings_and_rejects_linear_sum_payloads() {
    for (declarations, caller, rejected) in [
        (
            "type linear Ticket is {id:Int}; type Holder<T> is {public payload:T,public count:Int}; fn reset<T>(h:&mut Holder<T>) {h.count=0;}",
            "fn good(h:&mut Holder<Ticket>){reset(h);}",
            false,
        ),
        (
            "type linear Ticket is {id:Int}; type Choice<T> is Empty | Full(T); type Holder<T> is {public state:Choice<T>}; fn reset<T>(h:&mut Holder<T>) {h.state=Choice.Empty;}",
            "fn bad(h:&mut Holder<Ticket>){reset(h);}",
            true,
        ),
        (
            "type linear Ticket is {id:Int}; type Inner<T> is {public payload:T,public ticket:Ticket}; type Holder<T> is {public part:Inner<T>};",
            "fn bad<T>(h:&mut Holder<T>, incoming:Inner<T>){h.part=incoming;}",
            true,
        ),
    ] {
        let found = module_errors("overwrite_probe", &format!("{declarations} {caller}"));
        if rejected {
            assert!(
                found
                    .iter()
                    .any(|e| e.contains("overwrite") && e.contains("linear")),
                "{caller}: {found:?}"
            );
        } else {
            assert!(found.is_empty(), "{caller}: {found:?}");
        }
    }
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
fn immutable_text_cleanup_preserves_sibling_payloads() {
    let found = errors(
        "type Choice<T> is Empty | Full(T); type Holder is {public cell:Cell, public state:Choice<Text>}; fn reset(holder:&mut Holder) {holder.state=Choice.Empty;}",
        "fn preserve(mut holder:Holder)->Holder {reset(&mut holder);holder}",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn sum_reset_preserves_payloads_through_borrowed_receivers() {
    for caller in [
        "fn preserve(holder:&mut Holder) {holder.reset();let pointer=holder.cell.pointer;}",
        "fn preserve<T>(holder:&mut Holder,value:T)->T {holder.reset();let pointer=holder.cell.pointer;value}",
        "async fn preserve(holder:&mut Holder) {holder.reset();let pointer=holder.cell.pointer;}",
    ] {
        let found = errors(
            "type Choice<T> is Empty | Full(T); type Holder is {public cell:Cell,public state:Choice<Text>}; implement Holder {fn reset(&mut self) {self.cell.value=0;self.state=Choice.Empty;}}",
            caller,
        );
        assert!(found.is_empty(), "{caller}: {found:?}");
    }
}

#[test]
fn shared_method_reborrow_does_not_become_a_payload_write() {
    let found = errors(
        "implement Cell {fn inspect(&self)->Int {if self.value>0 {self.value} else {0}}}",
        "fn preserve(cell:&mut Cell)->Int {let observed=cell.inspect();*cell.pointer+observed}",
    );
    assert!(found.is_empty(), "{found:?}");

    for caller in [
        "fn preserve(mut holder:Holder)->Holder {{let borrowed=&mut holder;borrowed.inspect();}holder}",
        "fn preserve(holder:&mut Holder)->fn()->Int {holder.inspect();holder.action}",
    ] {
        let found = errors(
            "type Holder is {public value:Int,public action:fn()->Int}; implement Holder {fn inspect(&self)->Bool {self.value>0}}",
            caller,
        );
        assert!(found.is_empty(), "{caller}: {found:?}");
    }
}

#[test]
fn closed_write_can_read_through_a_shared_reborrow() {
    let found = errors(
        "fn update(destination:&mut Cell,source:&Cell) {destination.value=source.value;}",
        "fn preserve(mut destination:Cell,source:&mut Cell)->Cell {update(&mut destination,source);destination}",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn shared_reborrow_proof_cannot_grant_exclusive_permission() {
    let found = errors(
        "fn update(cell:&mut Cell) {cell.value=73;}",
        "fn preserve(cell:&Cell) {update(cell);}",
    );
    assert!(!found.is_empty(), "shared input gained exclusive permission");
}

#[test]
fn a_text_homonym_cannot_borrow_the_primitive_lifecycle() {
    // Bare `Text` is primitive grammar; qualify the user declaration to test
    // its actual nominal identity rather than the primitive spelling.
    let found = errors(
        "type Text is {public action:fn()->Int}; type Choice<T> is Empty | Full(T); type Holder is {public cell:Cell,public state:Choice<write_probe.Text>}; fn reset(holder:&mut Holder) {holder.state=Choice.Empty;}",
        "fn preserve(mut holder:Holder)->Holder {reset(&mut holder);holder}",
    );
    assert!(found.iter().any(|error| error.contains("cannot establish the lifetime")), "{found:?}");
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
