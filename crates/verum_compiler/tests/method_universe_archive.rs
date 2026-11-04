//! Method-local universe obligations survive source, wire, archive and sidecar replay.
use verum_ast::Module;
use verum_common::{List, Shared, Text};
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::CoreMetadata};
use verum_vbc::{
    archive::{ArchiveBuilder, read_archive, write_archive},
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    serialize::serialize_module,
};

const OWNER: &str = "core.method_levels";
const SOURCE: &str = r#"
module core.method_levels;
public type Holder<T> is { public value:T };
public type Pair<A,B> is { public first:A, public second:B };
public type Wrap<T> is { public value:T };
implement<B> Pair<Int,B> { public fn keep<R>(value:B, result:R)->R { result } }
implement<T> Pair<T,T> { public fn equal_type(value:T)->T { value } }
implement<L,R> Pair<R,L> { public fn reorder(left:L,right:R)->(L,R) { (left,right) } }
implement<T> Pair<Wrap<T>,T> { public fn nested(value:T)->T { value } }
public type Other<T> is { public value:T };
public type Cell<N:meta Int> is { public value:() };
implement<T> Holder<T> {
 public fn retain<universe u>(&self, value:Type(u))->Type(u) { value }
 public fn retained<universe u>(value:Type(u))->Type(u) { value }
 public fn size<universe u,N:meta Int>(&self,cell:Cell<N>)->Int { N }
 public fn local<universe u,A>(&self,value:A)->A { value }
}
"#;

fn check(checker: &mut TypeChecker, ast: &Module) -> List<Text> {
    let mut errors: List<Text> = checker
        .prepare_module_declarations(ast)
        .into_iter()
        .map(|error| format!("{error:?}").into())
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
            .map(|error| format!("{error:?}").into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| format!("{diagnostic:?}").into()),
    );
    errors
}

fn checked_source(source: &str, owner: &str) -> Module {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_universe_poly_enabled(true);
    checker.set_current_module_path(owner);
    let errors = check(&mut checker, &ast);
    assert!(errors.is_empty(), "source checking: {errors:?}");
    ast
}

fn metadata(interface: bool) -> CoreMetadata {
    let ast = checked_source(SOURCE, OWNER);
    let mut producer = VbcCodegen::with_config(CodegenConfig::new(OWNER));
    let source = if interface {
        producer.collect_declaration_interface(&[&ast])
    } else {
        producer.compile_module(&ast)
    }
    .expect("ordinary source/interface producer");
    let wire = deserialize_module(&serialize_module(&source).unwrap()).unwrap();
    let mut builder = ArchiveBuilder::new();
    builder.add_module(OWNER, &wire, &[]).unwrap();
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let metadata = archive_to_core_metadata(&archive).expect("real archive publication");
    verum_types::core_metadata::decode_metadata(
        &verum_types::core_metadata::encode_metadata(&metadata).unwrap(),
    )
    .unwrap()
}

fn consumer_errors(metadata: &CoreMetadata, tail: &str, eager: bool) -> List<Text> {
    let metadata = Shared::new(metadata.clone()).into_arc();
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_universe_poly_enabled(true);
    checker.set_current_module_path("consumer");
    let mut registry = verum_modules::ModuleRegistry::new();
    registry.register(verum_modules::ModuleInfo::new(
        verum_modules::ModuleId::new(0),
        verum_modules::ModulePath::from_str(OWNER),
        Parser::new("").parse_module().unwrap(),
        verum_ast::FileId::new(0),
        Text::new(),
    ));
    checker.set_module_registry_direct(registry);
    let ast = Parser::new(&format!("mount {OWNER}.{{Holder,Cell,Pair,Wrap}}; {tail}"))
        .parse_module()
        .expect("importing consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    check(&mut checker, &ast)
}

fn cases(consumer: &str, reason: Option<fn(&str) -> bool>) {
    for interface in [false, true] {
        let metadata = metadata(interface);
        for eager in [false, true] {
            let errors = consumer_errors(&metadata, consumer, eager);
            if let Some(reason) = reason {
                assert!(
                    errors.iter().any(|error| reason(error)),
                    "missing causal refusal, interface={interface} eager={eager} {consumer}: {errors:?}"
                );
            } else {
                assert!(
                    errors.is_empty(),
                    "interface={interface} eager={eager} {consumer}: {errors:?}"
                );
            }
        }
    }
}
#[test]
fn method_levels_survive_actual_archive_and_sidecar() {
    cases(
        "fn good(h:&Holder<Int>,c:Cell<7>)->Int{h.size<2,7>(c)} fn local(h:&Holder<Int>)->Bool{h.local<2,Bool>(true)}",
        None,
    );
    cases(
        "fn good(h:&Holder<Int>,x:Type(1))->Type(1){h.retain<1>(x)} fn two(h:&Holder<Int>,x:Type(2))->Type(2){h.retain<2>(x)}",
        None,
    );
    cases(
        "fn good(x:Type(1))->Type(1){Holder<Int>.retained<1>(x)}",
        None,
    );
}
#[test]
fn imported_method_cannot_erase_level_obligations() {
    cases(
        "fn bad(h:&Holder<Int>,x:Type(2))->Type(1){h.retain<2>(x)}",
        Some(|e| e.contains("Universe") || e.contains("universe")),
    );
    cases(
        "fn bad(h:&Holder<Int>,x:Type(2))->Type(2){h.retain<1>(x)}",
        Some(|e| e.contains("Universe") || e.contains("universe")),
    );
}

#[test]
fn altered_implementation_target_cannot_borrow_another_types_methods() {
    let mut metadata = metadata(false);
    let source = &mut metadata
        .inherent_implementations
        .get_mut(&Text::from("core.method_levels.Holder"))
        .expect("owned source")[0];
    let mut declaration = source.declaration().unwrap();
    let foreign = Parser::new("implement<T> Other<T> {}")
        .parse_module()
        .unwrap();
    let verum_ast::ItemKind::Impl(foreign) = &foreign.items[0].kind else {
        panic!("implementation")
    };
    declaration.kind = foreign.kind.clone();
    *source = verum_ast::source_implementation::AssociatedImplementationSource::new(
        OWNER.into(),
        &declaration,
    )
    .unwrap();
    let error =
        TypeChecker::validate_source_implementation_metadata(Shared::new(metadata).into_arc())
            .expect_err("foreign target cannot establish owner");
    assert!(format!("{error:?}").contains("target owner"), "{error:?}");
}

#[test]
fn erased_implementation_header_cannot_certify_method_metadata() {
    let mut metadata = metadata(false);
    metadata.inherent_implementations.clear();
    let error = verum_types::core_metadata::encode_metadata(&metadata)
        .expect_err("missing header must be refused at publication");
    assert!(
        error.contains("declaration-owned implementation header"),
        "{error}"
    );
}

#[test]
fn static_receiver_keeps_fixed_repeated_reordered_and_nested_arguments() {
    for source in [
        "fn good()->Bool { Pair<Int,Text>.keep<Bool>(\"x\",true) }",
        "fn good()->Bool { Pair<Bool,Bool>.equal_type(true) }",
        "fn good()->(Bool,Int) { Pair<Int,Bool>.reorder(true,42) }",
        "fn good()->Int { Pair<Wrap<Int>,Int>.nested(42) }",
    ] {
        cases(source, None);
    }
    for source in [
        "fn bad()->Bool { Pair<Bool,Text>.keep<Bool>(\"x\",true) }",
        "fn bad()->Bool { Pair<Bool,Int>.equal_type(true) }",
        "fn bad()->(Bool,Int) { Pair<Bool,Int>.reorder(true,42) }",
        "fn bad()->Int { Pair<Wrap<Bool>,Int>.nested(42) }",
    ] {
        cases(
            source,
            Some(|error| error.contains("Int") && error.contains("Bool")),
        );
    }
}
