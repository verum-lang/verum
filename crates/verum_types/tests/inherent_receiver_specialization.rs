//! Receiver applicability follows the selected declaration, including nested binders.
use verum_ast::{FileId, ItemKind};
use verum_common::{List};
use verum_fast_parser::FastParser;
use verum_modules::{extract_exports_from_module, ModuleId, ModuleInfo, ModulePath, ModuleRegistry};
use verum_types::{TypeChecker, TypeError};

fn checked(source: &str, providers: &[(&str, &str)]) -> (TypeChecker, List<TypeError>) {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("realm");
    checker.set_current_module_path("realm.consumer");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, body)) in providers.iter().chain(std::iter::once(&("realm.consumer", source))).enumerate() {
        let file = FileId::new(index as u32);
        let ast = FastParser::new().parse_module_str(body, file).expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        let mut module = ModuleInfo::new(id, path, ast, file, (*body).into());
        module.exports = exports;
        registry.register(module);
    }
    checker.set_module_registry_direct(registry.clone());
    let ast = FastParser::new().parse_module_str(source, FileId::new(providers.len() as u32)).expect("consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "realm.consumer", &registry) { errors.push(error); }
        }
    }
    errors.extend(checker.prepare_module_declarations(&ast));
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(error); }
    }
    errors.extend(checker.take_deferred_errors());
    assert!(checker.diagnostics().iter().all(|diagnostic| !diagnostic.is_error()), "{:?}", checker.diagnostics());
    (checker, errors)
}

fn accepts(source: &str, providers: &[(&str, &str)]) {
    let (_, errors) = checked(source, providers);
    assert!(errors.is_empty(), "{source}\n{errors:?}");
}

fn rejects_call(source: &str, providers: &[(&str, &str)]) {
    let (_, errors) = checked(source, providers);
    assert!(!errors.is_empty(), "incompatible receiver was accepted: {source}");
    for error in errors {
        assert!(matches!(error, TypeError::Mismatch { .. } | TypeError::MethodNotFound { .. }), "wrong refusal: {error:?}");
        assert!(error.to_diagnostic().primary_span().is_some(), "missing source attribution: {error:?}");
    }
}

const NESTED: &str = "type Inner<T> is { value:T }; type Shell<T> is { value:T }; implement<T> Shell<Inner<T>> { fn selected(&self)->Int { 73 } }";

#[test]
fn nested_binder_accepts_concrete_and_rigid_caller_arguments() {
    for caller in ["fn use_it(value:Shell<Inner<Int>>)->Int { value.selected() }", "fn use_it<A>(value:Shell<Inner<A>>)->Int { value.selected() }"] {
        accepts(&format!("{NESTED} {caller}"), &[]);
    }
}

#[test]
fn nested_binder_does_not_accept_the_wrong_constructor() {
    rejects_call(&format!("{NESTED} fn use_it(value:Shell<Int>)->Int {{ value.selected() }}"), &[]);
}

#[test]
fn nested_binder_supplies_a_real_method_suggestion() {
    let (_, errors) = checked(&format!("{NESTED} fn use_it(value:Shell<Inner<Int>>)->Int {{ value.selecte() }}"), &[]);
    assert!(!errors.is_empty());
    for error in errors {
        let TypeError::MethodNotFound { method, did_you_mean, .. } = error else { panic!("wrong refusal: {error:?}") };
        assert_eq!(method, "selecte");
        assert_eq!(did_you_mean.as_deref(), Some("selected"));
    }
}

#[test]
fn repeated_receiver_binder_is_one_type() {
    let declarations = "type Pair<A,B> is { left:A, right:B }; implement<T> Pair<T,T> { fn selected(&self)->Int { 73 } }";
    accepts(&format!("{declarations} fn use_it(value:Pair<Int,Int>)->Int {{ value.selected() }}"), &[]);
    rejects_call(&format!("{declarations} fn use_it(value:Pair<Int,Bool>)->Int {{ value.selected() }}"), &[]);
}

#[test]
fn pinned_value_and_declared_value_binder_keep_their_domains() {
    let declarations = "type Slot<const N:Int> is { value:Int }; implement Slot<3> { fn selected(&self)->Int { 73 } }";
    accepts(&format!("{declarations} fn use_it(value:Slot<3>)->Int {{ value.selected() }}"), &[]);
    rejects_call(&format!("{declarations} fn use_it(value:Slot<4>)->Int {{ value.selected() }}"), &[]);
    accepts("type Slot<const N:Int> is { value:Int }; implement<const N:Int> Slot<N> { fn selected(&self)->Int { N } } fn use_it(value:Slot<3>)->Int { value.selected() }", &[]);
}

#[test]
fn callable_arguments_preserve_the_declared_async_bound() {
    let declarations = "type Carrier<F> is { callback:F }; implement Carrier<fn()->Int> { fn selected(&self)->Int { 73 } }";
    accepts(&format!("{declarations} fn use_it(value:Carrier<fn()->Int>)->Int {{ value.selected() }}"), &[]);
    rejects_call(&format!("{declarations} fn use_it(value:Carrier<async fn()->Int>)->Int {{ value.selected() }}"), &[]);
}

#[test]
fn renamed_receiver_keeps_the_owner_and_foreign_homonyms_do_not() {
    let providers = [("realm.owner", "public type Shell<T> is { public value:T }; implement<T> Shell<T> { public fn selected(&self)->Int { 73 } }"), ("realm.other", "public type Shell<T> is { public value:T };")];
    let mounts = "mount realm.owner.Shell as Owned; mount realm.other.Shell as Foreign;";
    accepts(&format!("{mounts} fn use_it(value:Owned<Int>)->Int {{ value.selected() }}"), &providers);
    rejects_call(&format!("{mounts} fn use_it(value:Foreign<Int>)->Int {{ value.selected() }}"), &providers);
}

#[test]
fn concrete_nested_arguments_and_reference_permissions_are_invariant() {
    let declarations = "type Inner<T> is { value:T }; type Shell<T> is { value:T }; implement Shell<Inner<Int>> { fn selected(&self)->Int { 73 } }";
    accepts(&format!("{declarations} fn use_it(value:Shell<Inner<Int>>)->Int {{ value.selected() }}"), &[]);
    rejects_call(&format!("{declarations} fn use_it(value:Shell<Inner<Bool>>)->Int {{ value.selected() }}"), &[]);
    let declarations = "type Shell<T> is { value:T }; implement Shell<&mut Int> { fn selected(&self)->Int { 73 } }";
    accepts(&format!("{declarations} fn use_it(value:Shell<&mut Int>)->Int {{ value.selected() }}"), &[]);
    rejects_call(&format!("{declarations} fn use_it(value:Shell<&Int>)->Int {{ value.selected() }}"), &[]);
}

#[test]
fn true_aliases_do_not_erase_foreign_nested_owners() {
    let providers = [("realm.owner", "public type Token is { public value:Int }; public type Shell<T> is { public value:T }; implement Shell<Token> { public fn selected(&self)->Int { 73 } }"), ("realm.other", "public type Token is { public value:Int };")];
    let mounts = "mount realm.owner.{Shell,Token as Owned}; mount realm.other.Token as Foreign; type Alias is Owned;";
    accepts(&format!("{mounts} fn use_it(value:Shell<Alias>)->Int {{ value.selected() }}"), &providers);
    rejects_call(&format!("{mounts} fn use_it(value:Shell<Foreign>)->Int {{ value.selected() }}"), &providers);
}

#[test]
fn filtering_keeps_the_selected_implementations_protocol_obligations() {
    let declarations = "type Required is protocol {}; type Token is {}; implement Required for Token {} type Shell<T> is { value:T }; implement<T:Required> Shell<T> { fn selected(&self)->Int { 73 } }";
    accepts(&format!("{declarations} fn use_it(value:Shell<Token>)->Int {{ value.selected() }}"), &[]);
    accepts(&format!("{declarations} fn use_it<A:Required>(value:Shell<A>)->Int {{ value.selected() }}"), &[]);
    let (_, errors) = checked(&format!("{declarations} fn use_it(value:Shell<Int>)->Int {{ value.selected() }}"), &[]);
    assert!(errors.iter().any(|error| error.to_diagnostic().code()==Some("E405")), "{errors:?}");
}

#[test]
fn filtering_keeps_the_selected_implementations_callable_obligations() {
    let declarations = "type Shell<F> is { callback:F }; implement<F:fn()->Int> Shell<F> { fn selected(&self)->Int { 73 } }";
    accepts(&format!("{declarations} fn use_it(value:Shell<fn()->Int>)->Int {{ value.selected() }}"), &[]);
    let (_, errors) = checked(&format!("{declarations} fn use_it(value:Shell<fn()->Bool>)->Int {{ value.selected() }}"), &[]);
    assert!(errors.iter().any(|error| matches!(error, TypeError::Mismatch { .. })), "{errors:?}");
}

#[test]
fn equivalent_refinements_are_location_independent_but_not_erased() {
    let declarations = "type Shell<T> is { value:T }; implement Shell<Int {self>0}> { fn selected(&self)->Int { 73 } }";
    accepts(&format!("{declarations} fn use_it(value:Shell<Int {{self>0}}>)->Int {{ value.selected() }}"), &[]);
    rejects_call(&format!("{declarations} fn use_it(value:Shell<Int {{self>=0}}>)->Int {{ value.selected() }}"), &[]);
    rejects_call(&format!("{declarations} fn use_it(value:Shell<Int>)->Int {{ value.selected() }}"), &[]);
}

#[test]
fn caller_owned_parameters_cannot_be_specialized_by_a_candidate() {
    let declarations = "type Shell<T> is { value:T }; implement Shell<Int> { fn selected(&self)->Int { 73 } }";
    accepts(&format!("{declarations} fn use_it()->Int {{ let value=Shell {{value:3}}; value.selected() }}"), &[]);
    rejects_call(&format!("{declarations} fn use_it<A>(value:Shell<A>)->Int {{ value.selected() }}"), &[]);
}

#[test]
fn receiver_filter_does_not_instantiate_method_owned_arguments() {
    let declarations = "type Shell<T> is { value:T }; implement<T> Shell<T> { fn selected<U>(&self,item:U)->U { item } fn index<N:meta Int>(&self)->Int { N } }";
    accepts(&format!("{declarations} fn use_it(value:Shell<Bool>)->Int {{ value.selected<Int>(73) }}"), &[]);
    accepts(&format!("{declarations} fn use_it(value:Shell<Bool>)->Int {{ value.index<73>() }}"), &[]);
}

const PUBLIC_PROGRAM: &str = "module realm.consumer; public type Inner<T> is { public value:T }; public type Shell<T> is { public value:T }; implement<T> Shell<Inner<T>> { public fn selected(&self)->Int { 73 } } public fn main()->Int { let value=Shell {value:Inner {value:9}}; value.selected() }";

#[test]
fn complete_public_program_keeps_the_nested_receiver() {
    accepts(PUBLIC_PROGRAM, &[]);
    rejects_call(&PUBLIC_PROGRAM.replace("value:Inner {value:9}", "value:true"), &[]);
}


fn receiver_metadata(receiver: &str, generics: &[&str]) -> verum_types::core_metadata::CoreMetadata {
    use verum_common::{ResourceDiscipline, Text};
    use verum_types::core_metadata::*;
    let mut metadata = CoreMetadata::default();
    let parameter = |name: &str, id| GenericParam {
        name: name.into(), bounds: List::new(), default: None,
        type_bounds: List::new(), pid: Some(id),
    };
    metadata.types.insert("realm.Cell".into(), TypeDescriptor {
        source_declaration: None, name: "Cell".into(), module_path: "realm".into(),
        origin_module_path: None, generic_params: List::from_single(parameter("Item", 0)),
        kind: TypeDescriptorKind::Record { fields: List::new() }, size: None,
        alignment: None, methods: List::from_single("selected".into()),
        implements: List::new(), decl_span: None, is_public: true,
        is_transparent_wrapper: false, resource_discipline: ResourceDiscipline::Unrestricted,
    });
    metadata.type_declaration_order.push("realm.Cell".into());
    metadata.functions.insert("realm.Cell.selected".into(), FunctionDescriptor {
        source_signature: None, name: "Cell.selected".into(), module_path: "realm".into(),
        origin_module_path: None,
        generic_params: generics.iter().enumerate().map(|(id, name)| parameter(name, id as u16)).collect(),
        params: List::from_single(ParamDescriptor {
            name: "self".into(), ty: "Unit".into(), declared_ty: receiver.into(),
            has_default: false, default_literal: None,
        }),
        return_type: "Int".into(), contexts: List::new(), is_async: false,
        is_unsafe: false, intrinsic_id: None, parent_type: Some("realm.Cell".into()),
        impl_generic_names: generics.iter().map(|name| Text::from(*name)).collect(),
        is_const: false, decl_span: None, is_public: true,
        explicit_type_param_ids: Some(List::new()),
    });
    serde_json::from_slice(&serde_json::to_vec(&metadata).unwrap()).unwrap()
}

fn metadata_errors(receiver: &str, generics: &[&str], argument: &str, eager: bool) -> List<TypeError> {
    check_receiver_metadata(receiver_metadata(receiver, generics), argument, eager)
}

fn check_receiver_metadata(metadata: verum_types::core_metadata::CoreMetadata, argument: &str, eager: bool) -> List<TypeError> {
    let metadata = std::sync::Arc::new(metadata);
    let mut checker = if eager { TypeChecker::new_with_core_eager(metadata) }
        else { TypeChecker::new_with_core(metadata) };
    checker.register_primitives();
    let source = format!("fn probe(value: realm.Cell<{argument}>) -> Int {{ value.selected() }}");
    let module = FastParser::new().parse_module_str(&source, FileId::new(0)).unwrap();
    checker.register_stdlib_types_for_module(&module);
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) { errors.push(error); }
    }
    errors.extend(checker.take_deferred_errors());
    assert!(checker.diagnostics().iter().all(|diagnostic| !diagnostic.is_error()), "{:?}", checker.diagnostics());
    errors
}

#[test]
fn metadata_preserves_the_actual_declared_receiver_and_its_binders() {
    for eager in [false, true] {
        for (receiver, generics, argument) in [
            ("&Cell<&mut Int>", &[][..], "&mut Int"),
            ("&realm.Cell<&mut Int>", &[][..], "&mut Int"),
            ("&Cell<T>", &["T"][..], "Int"),
            ("&Cell<T>", &["T"][..], "Bool"),
        ] {
            let errors = metadata_errors(receiver, generics, argument, eager);
            assert!(errors.is_empty(), "{receiver}, {argument}, eager={eager}: {errors:?}");
        }
    }
}

#[test]
fn metadata_receiver_permissions_are_not_argument_coercions() {
    for eager in [false, true] {
        let errors = metadata_errors("&Cell<&mut Int>", &[], "&Int", eager);
        assert!(errors.iter().any(|error| matches!(error, TypeError::MethodNotFound { method, .. } if method == "selected")), "{errors:?}");
    }
}

#[test]
fn absent_or_undeclared_metadata_receiver_cannot_authorize_a_candidate() {
    for eager in [false, true] {
        for receiver in ["", "Unit", "&Self", "&Cell", "&Cell<__opaque_type_14>", "&Cell<T>", "&other.Cell<Int>"] {
            let errors = metadata_errors(receiver, &[], "Int", eager);
            assert!(errors.iter().any(|error| matches!(error, TypeError::MethodNotFound { method, .. } if method == "selected")),
                "receiver={receiver:?}, eager={eager}: {errors:?}");
        }
    }
}


#[test]
fn metadata_method_local_binder_is_not_an_implementation_binder() {
    for eager in [false, true] {
        let mut metadata = receiver_metadata("&Cell<T>", &["T"]);
        metadata.functions.get_mut(&"realm.Cell.selected".into()).unwrap().impl_generic_names.clear();
        let errors = check_receiver_metadata(metadata, "Int", eager);
        assert!(errors.iter().any(|error| matches!(error, TypeError::MethodNotFound { method, .. } if method == "selected")), "{errors:?}");
    }
}

#[test]
fn legacy_metadata_cannot_erase_declared_protocol_premises() {
    for eager in [false, true] {
        let mut metadata = receiver_metadata("&Cell<T>", &["T"]);
        metadata.functions.get_mut(&"realm.Cell.selected".into()).unwrap().generic_params[0]
            .bounds.push("realm.Missing".into());
        let errors = check_receiver_metadata(metadata, "Int", eager);
        assert!(errors.iter().any(|error| matches!(error, TypeError::MethodNotFound { method, .. } if method == "selected")), "{errors:?}");
    }
}
