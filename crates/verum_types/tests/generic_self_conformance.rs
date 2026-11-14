//! A protocol's Self marker retains its exact generic implementation receiver.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    errors_with_providers(source, &[])
}
fn errors_with_providers(source: &str, providers: &[(&str, &str)]) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("valid grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    checker.set_current_module_path("sample");
    let mut registry = verum_modules::ModuleRegistry::new();
    for (index, (owner, body)) in providers
        .iter()
        .chain(std::iter::once(&("sample", source)))
        .enumerate()
    {
        let file = verum_ast::FileId::new(index as u32);
        let ast = verum_fast_parser::FastParser::new()
            .parse_module_str(body, file)
            .unwrap();
        let id = verum_modules::ModuleId::new(index as u32);
        let path = verum_modules::ModulePath::from_str(owner);
        let mut info =
            verum_modules::ModuleInfo::new(id, path.clone(), ast.clone(), file, (*body).into());
        info.exports = verum_modules::extract_exports_from_module(&ast, id, &path).unwrap();
        registry.register(info);
    }
    checker.set_module_registry_direct(registry.clone());
    let mut found = List::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Mount(mount) = &item.kind {
            if let Err(error) = checker.process_import(mount, "sample", &registry) {
                found.push(error);
            }
        }
    }
    found.extend(checker.prepare_module_declarations(&module));
    found.extend(
        module
            .items
            .iter()
            .filter_map(|item| checker.check_item(item).err()),
    );
    found.extend(checker.take_deferred_errors());
    found
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|e| format!("{e:?}").into())
        .collect()
}
fn accept(source: &str) {
    let found = errors(source);
    assert!(found.is_empty(), "{source}\n{found:#?}");
}
fn reject(source: &str) {
    let found = errors(source);
    assert!(
        found.iter().any(|e| e.contains("E427")),
        "{source}\n{found:#?}"
    );
}
#[test]
fn generic_receiver_can_spell_its_own_return_carrier() {
    accept(
        "module sample; type Holder<T> is {value:T}; type Echo is protocol {fn echo(self)->Self;}; implement<T> Echo for Holder<T> {fn echo(self)->Holder<T>{self}}",
    );
}
#[test]
fn generic_receiver_can_spell_its_own_parameter_carrier() {
    accept(
        "module sample; type Holder<T> is {value:T}; type Inspect is protocol {fn inspect(&self, other:&Self)->Unit;}; implement<T> Inspect for Holder<T> {fn inspect(&self, other:&Holder<T>)->Unit{}}",
    );
}
#[test]
fn inverse_generic_receiver_is_not_a_fresh_parameter() {
    accept(
        "module sample; type Holder<T> is {value:T}; type Inspect<A> is protocol {fn inspect(&self, other:&Holder<A>)->Unit;}; implement<T> Inspect<T> for Holder<T> {fn inspect(&self, other:&Self)->Unit{}}",
    );
}
#[test]
fn a_shared_generic_alias_keeps_the_same_receiver() {
    accept(
        "module sample; type Holder<T> is {value:T}; type Alias<T> is Holder<T>; type Inspect is protocol {fn inspect(&self, other:&Self)->Unit;}; implement<T> Inspect for Alias<T> {fn inspect(&self, other:&Holder<T>)->Unit{}}",
    );
}
#[test]
fn generic_receiver_arguments_cannot_be_reordered() {
    reject(
        "module sample; type Pair<A,B> is {first:A,second:B}; type Inspect is protocol {fn inspect(&self, other:&Self)->Unit;}; implement<A,B> Inspect for Pair<A,B> {fn inspect(&self, other:&Pair<B,A>)->Unit{}}",
    );
}
#[test]
fn self_does_not_identify_a_foreign_generic_owner() {
    let source = "module sample; mount foreign.{Holder as Foreign}; type Holder<T> is {value:T}; type Inspect is protocol {fn inspect(&self, other:&Self)->Unit;}; implement<T> Inspect for Holder<T> {fn inspect(&self, other:&Foreign<T>)->Unit{}}";
    let found = errors_with_providers(
        source,
        &[(
            "foreign",
            "module foreign; public type Holder<T> is {value:T};",
        )],
    );
    assert!(
        found.iter().any(|error| error.contains("E427")),
        "{found:#?}"
    );
    assert!(
        !found.iter().any(|error| error.contains("TypeNotFound")),
        "foreign declaration must be resolved: {found:#?}"
    );
}
#[test]
fn self_cannot_specialize_an_unconstrained_receiver_argument() {
    reject(
        "module sample; type Holder<T> is {value:T}; type Inspect is protocol {fn inspect(&self, other:&Self)->Unit;}; implement<T> Inspect for Holder<T> {fn inspect(&self, other:&Holder<Int>)->Unit{}}",
    );
}
#[test]
fn inverse_self_cannot_replace_a_different_declared_application() {
    reject(
        "module sample; type Holder<T> is {value:T}; type Inspect<A> is protocol {fn inspect(&self, other:&Holder<Int>)->Unit;}; implement<T> Inspect<T> for Holder<T> {fn inspect(&self, other:&Self)->Unit{}}",
    );
}

#[test]
fn a_mounted_generic_receiver_preserves_its_declaring_owner() {
    let source = "module sample; mount foreign.{Holder as Local}; type Echo is protocol {fn echo(self)->Self;}; implement<T> Echo for Local<T> {fn echo(self)->Local<T>{self}}";
    let found = errors_with_providers(
        source,
        &[(
            "foreign",
            "module foreign; public type Holder<T> is {value:T};",
        )],
    );
    assert!(found.is_empty(), "{found:#?}");
}
