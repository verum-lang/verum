//! Source declarations preserve unknown computational properties across imports and method resolution.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

fn errors(consumer: &str, providers: &[(&str, &str)]) -> List<Text> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("demo");
    checker.set_current_module_path("demo.main");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in providers
        .iter()
        .chain(std::iter::once(&("demo.main", consumer)))
        .enumerate()
    {
        let file_id = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file_id)
            .expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), file_id, (*source).into());
        module.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(module);
    }
    checker.set_module_registry_direct(registry.clone());
    let ast = FastParser::new()
        .parse_module_str(consumer, FileId::new(providers.len() as u32))
        .expect("consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(e) = checker.process_import(import, "demo.main", &registry) {
                errors.push(format!("{e:?}").into());
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(d) => checker.register_type_declaration(d),
            ItemKind::Impl(d) => checker.register_impl_block(d),
            ItemKind::Function(d) => checker.register_function_signature(d),
            ItemKind::Context(_) => checker.check_item(item),
            _ => continue,
        };
        if let Err(e) = result {
            errors.push(format!("{e:?}").into());
        }
    }
    for item in &ast.items {
        if matches!(item.kind, ItemKind::Context(_)) {
            continue;
        }
        if let Err(e) = checker.check_item(item) {
            errors.push(format!("{e:?}").into());
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
            .filter(|e| e.is_error())
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}
fn assert_contract_rejected(errors: &List<Text>, message: &str) {
    assert!(!errors.is_empty(), "{message}");
    assert!(
        errors
            .iter()
            .all(|error| !error.contains("UnboundVariable") && !error.contains("UndefinedContext")),
        "unrelated rejection: {errors:?}"
    );
}
const PROVIDER: &str = r#"module demo.api;
async fn task() -> Int { 7 }
public fn work() -> Int { task() }
public async fn explicit() -> Int { 7 }
"#;
#[test]
fn ordinary_import_call_remains_valid() {
    let e = errors(
        "module demo.main; mount demo.api.work; fn probe()->Int {work()}",
        &[("demo.api", PROVIDER)],
    );
    assert!(e.is_empty(), "{e:?}");
}
#[test]
fn ordinary_import_cannot_fabricate_nonasync_callback_evidence() {
    let e = errors(
        "module demo.main; mount demo.api.work; fn consume(callback:fn()->Int)->Int { callback() } fn probe()->Int {consume(work)}",
        &[("demo.api", PROVIDER)],
    );
    assert_contract_rejected(
        &e,
        "imported ordinary function declaration is not NonAsync evidence",
    );
}
#[test]
fn explicit_async_import_is_not_a_nonasync_callback() {
    let e = errors(
        "module demo.main; mount demo.api.explicit; fn consume(callback:fn()->Int)->Int { callback() } fn probe()->Int {consume(explicit)}",
        &[("demo.api", PROVIDER)],
    );
    assert_contract_rejected(&e, "explicit async is not NonAsync");
}
#[test]
fn ordinary_import_fits_an_unrestricted_callback() {
    let e = errors(
        "module demo.main; mount demo.api.work; fn consume(callback:async fn()->Int)->Int { callback() } fn probe()->Int {consume(work)}",
        &[("demo.api", PROVIDER)],
    );
    assert!(e.is_empty(), "{e:?}");
}
#[test]
fn context_method_call_remains_valid() {
    let e = errors(
        "module demo.main; context Worker { fn run()->Int; } fn probe()->Int using [worker: Worker] { worker.run() }",
        &[],
    );
    assert!(e.is_empty(), "{e:?}");
}
#[test]
fn method_resolver_does_not_invent_nonasync_evidence() {
    use verum_types::method_resolution::{DefaultMethodResolver, MethodResolver};
    use verum_types::{Type, computational_properties::PropertyRequirement};
    let ast = FastParser::new()
        .parse_module_str(
            "implement Worker { fn run(&self)->Int { task() } }",
            FileId::new(0),
        )
        .unwrap();
    let ItemKind::Impl(implementation) = &ast.items[0].kind else {
        panic!()
    };
    let receiver = Type::Named {
        path: verum_ast::Path::single(verum_ast::Ident::new("Worker", verum_ast::Span::dummy())),
        args: List::new(),
    };
    let mut resolver = DefaultMethodResolver::new();
    resolver.register_inherent_impl("Worker", implementation);
    let resolved = resolver
        .resolve_method(&receiver, "run", &[], false)
        .unwrap();
    let Type::Function {
        property_requirement,
        properties,
        ..
    } = &resolved.signature.ty
    else {
        panic!()
    };
    assert!(properties.is_none());
    assert!(
        !PropertyRequirement::source_callable(false).satisfied_by(property_requirement, None),
        "unknown method body is not NonAsync evidence: {property_requirement:?}"
    );
}

fn context_capture_errors(is_async: bool) -> List<Text> {
    let marker = if is_async { "async " } else { "" };
    let source = format!(
        "module demo.main; context Worker {{ fn run()->Int; }} \
         fn capture()->({marker}fn()->Int) using [Worker] {{ Worker.run }}"
    );
    let ast = FastParser::new()
        .parse_module_str(&source, FileId::new(0))
        .expect("context capture grammar");
    let capture = ast
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Function(decl) if decl.name.name.as_str() == "capture" => Some(decl),
            _ => None,
        })
        .expect("expected capture declaration");
    assert_eq!(
        capture.contexts.len(),
        1,
        "using must bind the outer declaration"
    );
    assert_eq!(
        capture.contexts[0].path.as_ident().unwrap().name.as_str(),
        "Worker"
    );
    let verum_ast::TypeKind::Function {
        is_async: parsed_async,
        contexts,
        ..
    } = &capture.return_type.as_ref().unwrap().kind
    else {
        panic!("expected callable result")
    };
    assert_eq!(*parsed_async, is_async);
    assert!(
        contexts.requirements.is_empty(),
        "returned callable has no injected contexts"
    );
    errors(&source, &[])
}

#[test]
fn context_method_does_not_fabricate_nonasync_evidence() {
    let e = context_capture_errors(false);
    assert_contract_rejected(&e, "ordinary context method is not NonAsync evidence");
    assert!(e.iter().any(|error| error.contains("Mismatch")), "{e:?}");
}
#[test]
fn context_method_can_be_an_unrestricted_callback() {
    let e = context_capture_errors(true);
    assert!(e.is_empty(), "{e:?}");
}
