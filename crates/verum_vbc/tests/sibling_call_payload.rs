//! Inferred call results carry the callee's nominal parameters across source scopes.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

const PROVIDER: &str = r#"
module sample.page;
public type Receipt<T, E> is Accepted(T) | Refused(E);
public type Header is { size: Int };
public type Fault is Missing | Invalid(Int);
public fn read(n: Int) -> Receipt<Header, Fault> {
    if n < 0 { return Receipt.Refused(Fault.Invalid(n)); }
    Receipt.Accepted(Header { size: n })
}
"#;

const CONSUMER: &str = r#"
module sample.check;
mount sample.page.{Receipt, read};
type Fault is Invalid(Int) | Missing;
fn check(n: Int) -> Int {
    let value = read(n);
    match value {
        Receipt.Accepted(h) => h.size,
        Receipt.Refused(e) => match e { Missing => 0, Invalid(i) => i },
    }
}
fn probe() -> Int { check(19) - check(-7) }
"#;

#[test]
fn sibling_result_uses_callee_declarations_for_generic_payloads() {
    let provider = Parser::new(PROVIDER).parse_module().unwrap();
    let consumer = Parser::new(CONSUMER).parse_module().unwrap();
    let files = [&consumer, &provider];
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("sample"));
    codegen.collect_unit_declarations(&files).unwrap();
    codegen
        .compile_unit_items(&files, ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module().unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    for module in [module, decoded] {
        let module = Arc::new(module);
        let probe = module
            .functions
            .iter()
            .find(|f| {
                module
                    .strings
                    .get(f.name)
                    .is_some_and(|name| name == "sample.check.probe")
            })
            .unwrap();
        let result = Interpreter::new(module.clone())
            .execute_function(probe.id)
            .unwrap();
        assert!(result.is_int());
        assert_eq!(result.as_i64(), 26);
    }
}

#[test]
fn iterator_binds_the_declared_element_of_a_qualified_call_result() {
    let sources = [
        "module core.base.iterator; public type IntoIterator is protocol { type Item; };",
        "module core.collections.list; mount core.base.iterator.IntoIterator;
         public type List<T> is { len: Int, cap: Int, ptr: &unsafe T };
         implement<T> IntoIterator for List<T> { type Item = T; }",
        "module sample.types; public type Result<T, E> is Ok(T) | Err(E);",
        "module sample.provider; mount sample.types.Result; mount core.collections.list.List;
         public type Addr is V4(Int) | V6(Int);
         public type Other is V6(Int) | V4(Int);
         public fn addresses(n: Int) -> Result<core.collections.list.List<sample.provider.Addr>, Int> { Result.Err(n) }",
        "module sample.consumer; mount sample.types.Result; mount sample.provider.addresses;
         fn total(n: Int) -> Result<Int, Int> {
             let result = addresses(n)?;
             let mut total = 0;
             for address in result { if let V4(value) = address { total = total + value; } }
             Result.Ok(total)
         }",
    ];
    for (owner, accepted) in [("core.base.iterator", true), ("sample.counterfeit", false)] {
        let modules: Vec<_> = sources
            .iter()
            .map(|source| {
                Parser::new(&source.replace("core.base.iterator", owner))
                    .parse_module()
                    .unwrap()
            })
            .collect();
        let files: Vec<_> = modules.iter().collect();
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("sample"));
        codegen.collect_unit_declarations(&files).unwrap();
        let result = codegen.compile_unit_items(&files, ItemFailurePolicy::Strict);
        if accepted {
            result.unwrap();
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("no declared tag for variant pattern 'V4'")
            );
        }
    }
}

#[test]
fn a_returned_iterator_keeps_its_borrowed_item_across_sibling_scopes() {
    let sources = [
        r#"module core.base.iterator; public type Maybe<T> is None | Some(T);
public type Iterator is protocol { type Item; fn next(&mut self) -> Maybe<Self.Item>; }
public type IntoIterator is protocol { type Item; }"#,
        r#"module core.unit.values;
mount core.base.iterator.{Iterator,IntoIterator,Maybe};
public type Stage is Cancel | Save(Int);
public type Decoy is Save(Int) | Cancel;
public type Cursor<T> is { current: T, done: Bool };
implement<T> Iterator for Cursor<T> {
    type Item = &T;
    fn next(&mut self) -> Maybe<&T> {
        if self.done { return Maybe.None; }
        self.done = true; Maybe.Some(&self.current)
    }
}
implement<T> IntoIterator for Cursor<T> { type Item = &T; }
public type Container<A,B> is { selected: A, unrelated: B };
implement<A,B> Container<A,B> { public fn iter(&self) -> Cursor<A> { Cursor { current: self.selected, done: false } } }"#,
        r#"module core.unit.consumer; mount core.unit.values.{Stage,Decoy,Container};
public fn run(rows: Container<Stage, Decoy>) -> Int {
    for stage in rows.iter() {
        match stage { Save(x) => return *x, Cancel => return 3 }
    }
    0
}
public fn probe() -> Int { assert_eq(run(Container { selected: Stage.Save(41), unrelated: Decoy.Cancel }),41); assert_eq(run(Container{selected:Stage.Cancel,unrelated: Decoy.Save(98)}),3); 1 }"#,
    ];
    let modules: Vec<_> = sources.iter().map(|source| Parser::new(source).parse_module().unwrap()).collect();
    let files: Vec<_> = modules.iter().collect();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("core.unit"));
    cg.collect_unit_declarations(&files).unwrap();
    cg.resolve_pending_imports();
    cg.compile_unit_items(&files, ItemFailurePolicy::Strict).unwrap();
    let module = cg.finalize_module().unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [module, verum_vbc::deserialize::deserialize_module(&wire).unwrap()] {
        let entry = module.find_function_by_name("core.unit.consumer.probe").unwrap();
        assert_eq!(Interpreter::new(Arc::new(module)).execute_function(entry).unwrap().as_i64(), 1);
    }
}

#[test]
fn lexical_wildcards_preserve_reexported_nominal_witnesses_without_selecting_a_sibling() {
    use verum_vbc::types::TypeRef;

    let source = [
        "module sample.storage; public type Parcel<A, B> is { first: A, second: B };
         type Secret is { value: Int }; public(crate) type Local is { value: Int };",
        "module sample.other; public type Parcel<T> is { other: T };",
        "module sample.facade; public mount sample.storage.*;",
        "module sample.consumer; mount sample.facade.*; mount sample.storage.*;\n\
         fn forward<T>(item: Parcel<Int, T>) -> Parcel<Int, T> { item }
         fn restricted(item: Local) -> Int { 1 }",
    ];
    let files: Vec<_> = source.iter().map(|s| Parser::new(s).parse_module().unwrap()).collect();
    for reverse in [false, true] {
        let mut files: Vec<_> = files.iter().collect();
        if reverse { files.reverse(); }
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("sample"));
        compiler.collect_unit_declarations(&files).unwrap();
        compiler.compile_unit_items(&files, ItemFailurePolicy::Strict).unwrap();
        let module = compiler.finalize_module().unwrap();
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        for module in [module, verum_vbc::deserialize::deserialize_module(&wire).unwrap()] {
        let own_type = module.types.iter().find(|t| {
            module.get_string(t.name) == Some("Parcel")
                && t.origin_module.and_then(|name| module.get_string(name)) == Some("sample.storage")
        }).unwrap().id;
        let function = |name: &str| module.get_function(module.find_function_by_name(name).unwrap()).unwrap();
        let forward = function("sample.consumer.forward");
        let expected = TypeRef::Instantiated { base: own_type, args: vec![
            TypeRef::Concrete(verum_vbc::types::TypeId::INT), TypeRef::Generic(forward.type_params[0].id),
        ] };
        assert_eq!(forward.semantic_parameter_types().unwrap()[0].as_ref(), Some(&expected));
        assert_eq!(forward.semantic_return.as_ref(), Some(&expected));
        assert!(function("sample.consumer.restricted").semantic_parameter_types().unwrap()[0].is_some());
        }
    }
}

#[test]
fn inaccessible_and_ambiguous_glob_witnesses_refuse_instead_of_erasing_signatures() {
    use verum_vbc::codegen::CodegenErrorKind;

    for (consumer, denied) in [
        ("module sample.consumer; mount sample.facade.*; fn locked(item: Secret) -> Int { 2 }", "Secret"),
        ("module outside.client; mount sample.facade.*; fn hidden(item: Secret) -> Int { 4 }", "Secret"),
        ("module outside.client; mount sample.facade.*; fn leaked(item: Local) -> Int { 3 }", "Local"),
        ("module sample.conflict; mount sample.facade.*; mount sample.other.*; fn unresolved<T>(item: Parcel<T>) -> Int { 0 }", "Parcel"),
    ] {
        let sources = [
            "module sample.storage; public type Parcel<A, B> is { first: A, second: B }; type Secret is { value: Int }; public(crate) type Local is { value: Int };",
            "module sample.other; public type Parcel<T> is { other: T };",
            "module sample.facade; public mount sample.storage.*;",
            consumer,
        ];
        let modules: Vec<_> = sources.iter().map(|source| Parser::new(source).parse_module().unwrap()).collect();
        for reverse in [false, true] {
            let mut files: Vec<_> = modules.iter().collect();
            if reverse { files.reverse(); }
            let mut compiler = VbcCodegen::with_config(CodegenConfig::new("sample"));
            let error = compiler.collect_unit_declarations(&files).unwrap_err();
            assert!(
                matches!(&error.kind, CodegenErrorKind::InvalidSourceSignature(message) if message.contains(denied)),
                "{consumer}: {error:?}"
            );
        }
    }
}


#[test]
fn unmounted_same_leaf_does_not_replace_a_scalar_argument() {
    use verum_vbc::types::{TypeId, TypeRef};
    let files = [
        "module sample.foreign; public type Byte is { foreign: Int };",
        "module sample.generic; public type Boxed<T> is { value: T };",
        "module sample.use; mount sample.generic.Boxed;
         fn implicit(x: Boxed<Byte>) -> Int { 1 }",
        "module sample.explicit; mount sample.generic.Boxed; mount sample.foreign.Byte;
         fn named(x: Boxed<Byte>) -> Int { 2 }",
    ].map(|s| Parser::new(s).parse_module().unwrap());
    let modules: Vec<_> = files.iter().collect();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("sample"));
    compiler.collect_unit_declarations(&modules).unwrap();
    compiler.compile_unit_items(&modules, ItemFailurePolicy::Strict).unwrap();
    let module = compiler.finalize_module().unwrap();
    let argument = |name| {
        let func = module.get_function(module.find_function_by_name(name).unwrap()).unwrap();
        let Some(TypeRef::Instantiated { args, .. }) = &func.semantic_parameter_types().unwrap()[0] else {
            panic!("missing nominal parameter in {name}");
        };
        args[0].base_type_id().unwrap()
    };
    assert_eq!(argument("sample.use.implicit"), TypeId::BYTE);
    let explicit = module.get_type(argument("sample.explicit.named")).unwrap();
    assert_eq!(explicit.origin_module.and_then(|id| module.get_string(id)), Some("sample.foreign"));
}
