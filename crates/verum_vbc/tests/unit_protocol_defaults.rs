#![cfg(feature = "codegen")]

use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenErrorKind, CodegenResult, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    VbcModule,
};

fn compile(sources: &[&str]) -> CodegenResult<VbcModule> {
    let modules: List<_> = sources
        .iter()
        .map(|source| Parser::new(source).parse_module().unwrap())
        .collect();
    let modules: List<_> = modules.iter().collect();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("app"));
    compiler.collect_unit_declarations(&modules)?;
    compiler.compile_unit_items(&modules, ItemFailurePolicy::Strict)?;
    compiler.finalize_module_from_state()
}

fn run(sources: &[&str], expected: i64) {
    let module = compile(sources).unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    let entry = module.find_function_by_name("app.main").unwrap();
    assert_eq!(
        Interpreter::new(module.into())
            .execute_function(entry)
            .unwrap()
            .try_as_i64(),
        Some(expected)
    );
}

const PROVIDER: &str = r#"
    module provider;
    public type Measure is protocol {
        fn value(&self) -> Int;
        fn total(&self) -> Int { self.value() + 36 }
    };
    public type Marked is protocol extends Measure {};
"#;

#[test]
fn unit_body_entry_materializes_record_defaults_before_callers() {
    run(
        &[
            PROVIDER,
            r#"
        module app;
        mount provider.{Measure, Marked};
        type Key is { value: Int };
        implement Measure for Key { fn value(&self) -> Int { self.value } }
        implement Marked for Key {}
        fn main() -> Int { Key { value: 37 }.total() }
    "#,
        ],
        73,
    );
}

#[test]
fn generic_owner_and_bound_dispatch_keep_the_default_subject() {
    run(
        &[
            PROVIDER,
            r#"
        module app;
        mount provider.{Measure, Marked};
        type Holder<T> is { payload: T, marker: Int };
        implement<T> Measure for Holder<T> { fn value(&self) -> Int { self.marker } }
        implement<T> Marked for Holder<T> {}
        fn relay<T: Marked>(value: &T) -> Int { value.total() }
        fn main() -> Int {
            let number = Holder { payload: 3, marker: 37 };
            let flag = Holder { payload: true, marker: 37 };
            assert_eq(relay(&number), 73);
            flag.total()
        }
    "#,
        ],
        73,
    );
}

#[test]
fn explicit_late_override_keeps_its_owned_slot() {
    run(
        &[
            PROVIDER,
            r#"
        module app;
        mount provider.{Measure, Marked};
        type Key is { value: Int };
        implement Measure for Key { fn value(&self) -> Int { self.value } }
        implement Marked for Key {}
        implement Key { fn total(&self) -> Int { self.value } }
        fn main() -> Int { Key { value: 73 }.total() }
    "#,
        ],
        73,
    );
}

#[test]
fn a_same_named_foreign_protocol_does_not_replace_the_selected_default() {
    let foreign = r#"
        module foreign;
        public type Measure is protocol { fn total(&self) -> Int { 999 } };
    "#;
    let consumer = r#"
        module app;
        mount provider.Measure;
        type Key is { value: Int };
        implement Measure for Key { fn value(&self) -> Int { self.value } }
        fn main() -> Int { Key { value: 37 }.total() }
    "#;
    for modules in [[PROVIDER, foreign, consumer], [foreign, PROVIDER, consumer]] {
        run(&modules, 73);
    }
}

#[test]
fn shared_protocol_heads_do_not_share_receiver_obligations() {
    let provider = r#"
        module gate;
        public type Base is protocol {};
        public type Evidence is protocol {};
        public type Granted is protocol { fn token(&self) -> Int { 73 } };
        implement<T: Base + Evidence> Granted for T {}
    "#;
    for count in [1, 8, 32] {
        let mut consumer = verum_common::Text::from(
            "module app; mount gate.{Base, Evidence, Granted};\n",
        );
        let mut total = verum_common::Text::from("0");
        for index in 0..count {
            consumer.push_str(&format!(
                "type Item{index} is {{}}; implement Base for Item{index} {{}} \
                 implement Evidence for Item{index} {{}}\n"
            ));
            total.push_str(&format!(" + Item{index} {{}}.token()"));
        }
        consumer.push_str("type Missing is {}; implement Base for Missing {}\n");
        let valid = format!("{consumer} fn main() -> Int {{ {total} }}");
        let invalid = format!("{consumer} fn main() -> Int {{ Missing {{}}.token() }}");
        for consumer_first in [false, true] {
            let sources = |source| if consumer_first {
                [source, provider]
            } else {
                [provider, source]
            };
            run(&sources(valid.as_str()), count * 73);
            let error = compile(&sources(invalid.as_str())).unwrap_err();
            assert!(
                matches!(&error.kind, CodegenErrorKind::UndefinedFunction(name)
                    if name.ends_with("Missing.token")),
                "unproved receiver acquired another receiver's evidence: {error:?}"
            );
        }
    }
}

#[test]
fn missing_implementation_and_invalid_default_bodies_refuse_publication() {
    for (consumer, missing) in [
        (
            r#"module app; mount provider.Measure; type Key is { value: Int };
            fn main() -> Int { Key { value: 37 }.total() }"#,
            "Key.total",
        ),
        (
            r#"module app; type Bad is protocol { fn total(&self) -> Int { absent() } };
            type Key is { value: Int }; implement Bad for Key {}
            fn main() -> Int { 73 }"#,
            "absent",
        ),
    ] {
        let error = compile(&[PROVIDER, consumer]).unwrap_err();
        assert!(
            matches!(&error.kind, CodegenErrorKind::UndefinedFunction(name) if name == missing),
            "{error:?} for {consumer}"
        );
    }
}
