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
