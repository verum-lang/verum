//! T2278: namespaces and unresolved members cannot manufacture bottom values.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn check(source: &str, providers: &[(&str, &str)]) -> List<Text> {
    let mut registry = verum_modules::ModuleRegistry::new();
    for (index, (owner, body)) in providers.iter().chain(std::iter::once(&("cog", source))).enumerate() {
        let ast = Parser::new(body).parse_module().expect("source grammar");
        let id = verum_modules::ModuleId::new(index as u32);
        let path = verum_modules::ModulePath::from_str(owner);
        let mut info = verum_modules::ModuleInfo::new(id, path.clone(), ast.clone(), verum_ast::FileId::new(index as u32), (*body).into());
        info.exports = verum_modules::extract_exports_from_module(&ast, id, &path).expect("declared exports");
        registry.register(info);
    }
    let ast = Parser::new(source).parse_module().expect("consumer grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("cog");
    checker.set_module_registry_direct(registry.clone());
    for item in &ast.items {
        if let ItemKind::Module(module) = &item.kind { checker.pre_register_module_public(module, "cog"); }
    }
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "cog", &registry) { errors.push(Text::from(format!("{error:?}"))); }
        }
    }
    errors.extend(checker.prepare_module_declarations(&ast).iter().map(|error| Text::from(format!("{error:?}"))));
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(Text::from(format!("{error:?}"))); }
    }
    errors.extend(checker.take_deferred_errors().iter().map(|error| Text::from(format!("{error:?}"))));
    errors.extend(checker.diagnostics().iter().filter(|d| d.is_error()).map(|error| Text::from(format!("{error:?}"))));
    errors
}
fn accepts(source: &str, providers: &[(&str, &str)]) {
    let errors = check(source, providers);
    assert!(errors.is_empty(), "{source}: {errors:?}");
}
fn unresolved(source: &str, name: &str) {
    let errors = check(source, &[]);
    assert!(errors.iter().any(|error| (error.contains("UnboundVariable") || (error.contains("E100") && error.contains("unbound variable:"))) && error.contains(name)), "expected unresolved {name}: {source}: {errors:?}");
    eprintln!("{name}: {errors:?}");
}

#[test]
fn standalone_namespaces_are_not_bottom_values() {
    unresolved("module api {public fn value()->Int {42}} fn probe() {let module_value=api;}", "api");
    unresolved("module external; fn probe() {let module_value=external;}", "external");
    unresolved("fn probe()->! { cog }", "cog");
    unresolved("module child {public fn probe()->! { super }}", "super");
}

#[test]
fn unresolved_rooted_values_and_unloaded_modules_report_the_source_name() {
    for (source, name) in [
        ("fn probe()->! { cog.absent }", "cog.absent"),
        ("module child {public fn probe()->! { super.absent }}", "super.absent"),
        ("module external; fn probe()->! { external.absent }", "external.absent"),
        ("module api {public const VALUE:Int=42;} fn probe()->! {api.absent}", "absent"),
    ] { unresolved(source, name); }
}

#[test]
fn unresolved_namespace_calls_cannot_fall_back_to_receiver_inference() {
    for (source, name) in [
        ("module api {public fn value()->Int {42}} fn probe()->! {cog.api.absent()}", "absent"),
        ("module parent {public module child {public fn probe()->! {super.absent()}}}", "super.absent"),
        ("module external; fn probe()->! {external.absent()}", "external.absent"),
    ] { unresolved(source, name); }
}

#[test]
fn unrelated_bare_functions_cannot_satisfy_a_qualified_value_request() {
    for (source, name) in [
        ("fn present()->Int {42} fn probe()->fn()->Int {cog.absent.present}", "cog.absent.present"),
        ("fn present()->Int {42} module child {public fn probe()->fn()->Int {super.absent.present}}", "super.absent.present"),
        ("fn present()->Int {42} fn probe()->fn()->Int {core.absent.present}", "core"),
    ] { unresolved(source, name); }
}

#[test]
fn declared_rooted_functions_constants_and_chains_preserve_their_actual_types() {
    for source in [
        "module api {public fn value()->Int {42}} fn probe()->Int {api.value()}",
        "module api {public fn value()->Int {42}} mount cog.api as chosen; fn probe()->Int {chosen.value()}",
        "public type Cell is {public value:Int}; implement Cell {public fn read(&self)->Int {self.value}} module api {public fn value()->Cell {Cell {value:42}}} fn probe()->Int {api.value().read()}",
        "module api {public fn value()->Int {42}} fn probe()->Int {cog.api.value()}",
        "module api {public const VALUE:Int=42;} fn probe()->Int {cog.api.VALUE}",
        "module parent {public fn value()->Int {42} public module child {public fn probe()->Int {super.value()}}}",
        "module parent {public const VALUE:Int=42; public module child {public fn probe()->Int {super.VALUE}}}",
        "public type Cell is {public value:Int}; implement Cell {public fn read(&self)->Int {self.value}} module api {public fn value()->Cell {Cell {value:42}}} fn probe()->Int {cog.api.value().read()}",
        "type Inner is {public value:Int}; type Outer is {public inner:Inner}; fn probe(x:Outer)->Int {x.inner.value}",
    ] { accepts(source, &[]); }
    for source in [
        "module api {public fn value()->Int {42}} fn probe()->Bool {cog.api.value()}",
        "module parent {public const VALUE:Int=42; public module child {public fn probe()->Bool {super.VALUE}}}",
    ] {
        let errors = check(source, &[]);
        assert!(errors.iter().any(|e| e.contains("Mismatch")), "lost declared result: {source}: {errors:?}");
    }
}

#[test]
fn inline_result_cannot_capture_a_private_type_from_the_callers_scope() {
    let source = "type Cell is {public value:Int}; module api {public fn make()->Cell {Cell {value:42}}} fn probe()->Int {api.make().value}";
    let errors = check(source, &[]);
    assert!(errors.iter().any(|e| e.contains("Cell")
        && (e.contains("TypeNotFound") || e.contains("VisibilityError"))), "{errors:?}");
}

#[test]
fn actual_source_reexports_externs_and_intrinsics_remain_declared_values() {
    accepts("mount facade.{value}; fn probe()->Int {value()}", &[
        ("origin", "public fn value()->Int {42}"),
        ("facade", "public mount origin.{value};"),
    ]);
    accepts("mount native.{close}; fn probe()->Int {close(1)}", &[
        ("native", "extern \"C\" {public fn close(fd:Int)->Int;}"),
    ]);
    accepts("mount native.{exit_process}; fn probe()->! {exit_process(0)}", &[
        ("native", "@intrinsic(\"exit_process\") public fn exit_process(code:Int)->!;"),
    ]);
    accepts("module native {@intrinsic(\"exit_process\") public fn exit_process(code:Int)->!;} fn probe()->! {native.exit_process(0)}", &[]);
}
const QUALIFIED_VALUE_DECLARATIONS: &str = r#"
public type State is | Cold | Warm;
public type Marker is protocol { fn marker(&self) -> Int; };
implement State {
    public fn degree(&self) -> Int {
        match self { State.Cold => 0, State.Warm => 1 }
    }
}
implement Marker for State {
    fn marker(&self) -> Int { self.degree() }
}
"#;

#[test]
fn qualified_variant_values_are_instance_method_receivers() {
    for expression in ["State.Cold.degree()", "State.Warm.marker()"] {
        accepts(
            &format!("{QUALIFIED_VALUE_DECLARATIONS}\nfn probe()->Int {{{expression}}}"),
            &[],
        );
        accepts(
            &format!("mount source.{{State, Marker}}; fn probe()->Int {{{expression}}}"),
            &[("source", QUALIFIED_VALUE_DECLARATIONS)],
        );
    }
    accepts(
        "mount source.{State as Phase, Marker}; fn probe()->Int {Phase.Warm.marker()}",
        &[("source", QUALIFIED_VALUE_DECLARATIONS)],
    );
}

#[test]
fn qualified_value_method_misses_and_bad_arguments_are_refused() {
    for (expression, expected) in [
        ("State.Cold.absent()", "MethodNotFound"),
        ("State.Cold.degree(7)", "WrongArgCount"),
        ("State.Warm.marker(false)", "WrongArgCount"),
    ] {
        let source = format!("{QUALIFIED_VALUE_DECLARATIONS}\nfn probe()->Int {{{expression}}}");
        let errors = check(&source, &[]);
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "expected {expected} for {source}: {errors:?}"
        );
    }
}

#[test]
fn rooted_constructors_preserve_declared_owners_and_generic_parameters() {
    for expression in ["cog.State.Cold", "cog.State.Warm"] {
        accepts(&format!("{QUALIFIED_VALUE_DECLARATIONS}\nfn probe()->State {{{expression}}}"), &[]);
    }
    accepts(&format!("{QUALIFIED_VALUE_DECLARATIONS}\nfn probe()->Int {{cog.State.Cold.degree()}}"), &[]);
    accepts("public type Mode is | idle | active; implement Mode { public fn marker(&self)->Int {19} } fn first()->Int {cog.Mode.idle.marker()} fn second()->Mode {cog.Mode.active}", &[]);
    accepts("public type Parcel<T> is | Empty | Full(T); fn first()->Parcel<Int> {cog.Parcel.Empty} fn second()->Parcel<Text> {cog.Parcel.Empty} fn third()->Parcel<Int> {cog.Parcel.Full(7)}", &[]);
    accepts("public type State is | Cold | Warm; mount other.{State as Foreign}; fn foreign()->Foreign {Foreign.Hot} fn probe()->State {cog.State.Cold}", &[("other", "public type State is | Hot | Cold(Int);")]);
    accepts("public type State is | Cold | Warm; type Phase is State; fn probe()->State {cog.Phase.Cold}", &[]);
}

#[test]
fn rooted_constructor_paths_cannot_borrow_missing_or_private_owners() {
    for source in [
        "mount other.State; fn probe()->State {cog.absent.State.Cold}",
        "public type State is | Cold | Warm; mount other.{State as Foreign}; fn probe()->State {cog.State.Hot}",
        "fn Cold()->Int {7} fn probe()->Int {cog.absent.State.Cold}",
    ] {
        let errors = check(source, &[("other", "public type State is | Cold | Hot;")]);
        assert!(errors.iter().any(|error| error.contains("UnboundVariable") || error.contains("TypeNotFound")), "{source}: {errors:?}");
    }
    let errors = check("fn probe() {let value=cog.hidden.State.Cold;}", &[("cog.hidden", "type State is | Cold | Warm;")]);
    assert!(errors.iter().any(|error| error.contains("VisibilityError") || error.contains("E202")), "{errors:?}");
}
