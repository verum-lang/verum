//! A reference does not manufacture methods or override its referent's declaration.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::VerumParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

fn errors(source: &str, providers: &[(&str, &str)]) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    for (owner, body) in providers.iter().chain(std::iter::once(&("cog", source))) {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let ast = VerumParser::new().parse_module_str(body, file).expect("grammar");
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&ast, id, &path).expect("exports");
        let mut info = ModuleInfo::new(id, path, ast, file, (*body).into());
        info.exports = exports;
        registry.register(info);
    }
    let module = registry.get_by_path("cog").unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    checker.register_primitives();
    checker.set_module_registry_direct(registry.clone());
    checker.set_current_module_path("cog");
    let mut errors: List<Text> = checker.prepare_module_declarations(&module.ast)
        .into_iter().map(|error| format!("{error:?}").into()).collect();
    for item in &module.ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(format!("{error:?}").into()); }
    }
    errors.extend(checker.take_deferred_errors().into_iter().map(|error| Text::from(format!("{error:?}"))));
    errors.extend(checker.diagnostic_sources().iter().map(|error| Text::from(format!("{error:?}"))));
    errors
}

#[test]
fn declared_inherent_metadata_names_keep_their_result_and_receiver() {
    for name in ["capabilities", "generation", "epoch", "stored_generation", "epoch_caps", "is_valid", "can_read", "can_write", "raw_ptr", "header_size", "allocation_generation", "to_checked", "to_unsafe", "as_ref", "is_null"] {
        for reference in ["&", "&checked ", "&unsafe "] {
            let source = format!("type Report is {{flag:Bool}}; type Item is {{value:Int}}; implement Item {{ public fn {name}(&self)->Report {{Report{{flag:true}}}} }} fn probe(value:{reference}Item)->Report {{value.{name}()}}");
            let found = errors(&source, &[]);
            assert!(found.is_empty(), "{source}: {found:?}");
        }
    }
}

#[test]
fn dynamic_and_bound_protocol_metadata_names_use_the_declared_result() {
    for parameter in ["value:&dyn Inspect", "value:&T"] {
        let generics = if parameter.contains("&T") { "<T:Inspect>" } else { "" };
        let source = format!("type Report is {{flag:Bool}}; type Inspect is protocol {{fn capabilities(&self)->Report;}}; fn probe{generics}({parameter})->Report {{value.capabilities()}}");
        let found = errors(&source, &[]);
        assert!(found.is_empty(), "{source}: {found:?}");
    }
}

#[test]
fn unknown_reference_metadata_names_are_not_invented() {
    for name in ["capabilities", "stored_generation", "epoch_caps", "raw_ptr", "header_size", "allocation_generation", "is_valid"] {
        let source = format!("fn probe(value:&Int) {{value.{name}();}}");
        let found = errors(&source, &[]);
        assert!(found.iter().any(|e| e.contains("MethodNotFound")), "{source}: {found:?}");
    }
}

#[test]
fn a_smart_pointer_homonym_does_not_acquire_reference_metadata_methods() {
    let found = errors("type Heap<T> is {value:T}; fn probe(value:Heap<Int>)->Int {value.capabilities()}", &[]);
    assert!(found.iter().any(|e| e.contains("MethodNotFound")), "{found:?}");
}

#[test]
fn a_metadata_spelling_does_not_bypass_method_arity() {
    let found = errors("type Item is {value:Int}; implement Item {public fn capabilities(&self, flag:Bool)->Int {self.value}} fn probe(value:&Item)->Int {value.capabilities()}", &[]);
    assert!(found.iter().any(|e| e.contains("WrongArgCount") && e.contains("expected: 1, actual: 0")), "{found:?}");
}

#[test]
fn a_metadata_spelling_does_not_turn_a_mutating_method_into_a_read() {
    let found = errors("type Item is {value:Int}; implement Item {public fn capabilities(&mut self)->Int {self.value=73;self.value}} fn probe(value:&Item)->Int {value.capabilities()}", &[]);
    assert!(found.iter().any(|e| e.contains("SharedReferenceWrite")), "{found:?}");
}

#[test]
fn renamed_method_owner_is_preserved_and_private_methods_stay_private() {
    let providers = [("provider", "public type Item is {public value:Int}; public type Report is {public flag:Bool}; implement Item {public fn capabilities(&self)->Report {Report{flag:true}} fn generation(&self)->Int {self.value}}")];
    let good = errors("mount provider.{Item as Selected,Report}; fn probe(value:&Selected)->Report {value.capabilities()}", &providers);
    assert!(good.is_empty(), "{good:?}");
    let private = errors("mount provider.Item as Selected; fn probe(value:&Selected)->Int {value.generation()}", &providers);
    assert!(private.iter().any(|e| e.contains("MethodNotFound") && e.contains("generation")), "{private:?}");
}

#[test]
fn a_foreign_metadata_method_does_not_satisfy_an_unrelated_receiver() {
    let providers = [("provider", "public type Item is {public value:Int}; implement Item {public fn capabilities(&self)->Int {self.value}}")];
    let found = errors("mount provider.Item as Other; type Item is {value:Int}; fn probe(value:&Item)->Int {value.capabilities()}", &providers);
    assert!(found.iter().any(|e| e.contains("MethodNotFound")), "{found:?}");
}

#[test]
fn structural_reference_conversions_cannot_upgrade_a_shared_outer_borrow() {
    for source in [
        "fn probe(value:&&mut Int)->&checked mut Int {value.to_checked_mut()}",
        "fn probe(value:&&checked mut Int)->&mut Int {value.to_managed_mut()}",
    ] {
        let found = errors(source, &[]);
        assert!(found.iter().any(|e| e.contains("SharedReferenceWrite") || e.contains("ImmutableToMutable")), "{source}: {found:?}");
    }
    let found = errors("fn probe(value:&mut Int)->&checked mut Int {value.to_checked_mut()}", &[]);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn declared_protocol_receiver_permissions_are_not_erased_by_a_metadata_spelling() {
    for (generics, parameter) in [("", "&dyn Inspect"), ("<T:Inspect>", "&T")] {
        let source = format!("type Inspect is protocol {{fn capabilities(&mut self)->Int;}}; fn probe{generics}(value:{parameter})->Int {{value.capabilities()}}");
        let found=errors(&source, &[]);
        assert!(found.iter().any(|e| e.contains("SharedReferenceWrite")), "{source}: {found:?}");
    }
}
