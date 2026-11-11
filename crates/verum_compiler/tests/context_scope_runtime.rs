//! Lexical contexts and recovery handlers through archive production and VBC execution.
//! The fixture is an independent module; it does not stand in for a full stdlib.
use std::{fs, sync::Arc};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{
    archive::read_archive_from_file, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module, validate::validate_module,
};

fn execute(functions: &str, expected: i64) {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("core");
    fs::create_dir(&source).unwrap();
    let entry = source.join("mod.vr");
    fs::write(
        &entry,
        format!(
            "module core;
             public context Gate {{fn read() -> Int;}}
             public type Channel is {{public value: Int}};
             implement Channel {{public fn read(&self) -> Int {{self.value}}}}
             fn read() -> Int using [Gate] {{Gate.read()}}
             {functions}"
        ),
    )
    .unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: entry,
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&source).with_output(&output))
            .compile_core()
            .expect("source compilation");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let module = read_archive_from_file(&output)
        .unwrap()
        .load_module("core")
        .unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for (route, module) in [("produced", module), ("wire", wire)] {
        validate_module(&module).expect("strict VBC validation");
        let probe = module
            .find_function_by_name("core.probe")
            .expect("compiled probe");
        assert!(!verum_vbc::stub_ranges::is_stub_id(probe.0));
        let mut vm = Interpreter::new(Arc::new(module));
        vm.run_global_ctors().expect("global initialization");
        let value = vm
            .execute_function(probe)
            .unwrap_or_else(|error| panic!("{route}: {error:?}"));
        assert_eq!(value.try_as_i64(), Some(expected), "{route}");
    }
}

#[test]
fn exact_refined_coverage_executes_through_the_produced_archive() {
    execute(
        "fn choose(value: Int{it >= 0 && it <= 100}) -> Int {
        match value {0..=100 => value + 1}}
        public fn probe() -> Int {choose(0) + choose(50) + choose(100)}",
        153,
    );
}

#[test]
fn nested_providers_restore_the_outer_value() {
    execute(
        "public fn probe() -> Int {
            provide Gate = Channel {value: 10};
            let mut result = read();
            {provide Gate = Channel {value: 20}; result = result * 100 + read();}
            result = result * 100 + read();
            provide Gate = Channel {value: 30} in {result = result * 100 + read();}
            result * 100 + read()
        }",
        1020103010,
    );
}

#[test]
fn early_returns_restore_the_callers_provider() {
    for body in [
        "provide Gate = Channel {value: 20} in {return read();}",
        "{provide Gate = Channel {value: 20}; return read();}",
    ] {
        execute(
            &format!(
                "fn inner() -> Int using [Gate] {{{body}}}
                 public fn probe() -> Int {{
                     provide Gate = Channel {{value: 10}};
                     inner() * 100 + read()
                 }}"
            ),
            2010,
        );
    }
}

#[test]
fn nested_functions_use_the_provider_at_the_call_site() {
    execute(
        "public fn probe() -> Int {
            provide Gate = Channel {value: 10};
            fn nested() -> Int using [Gate] {read()}
            let mut result = nested();
            provide Gate = Channel {value: 20} in {result = result * 100 + nested();}
            result * 100 + nested()
        }",
        102010,
    );
}

#[test]
fn recovery_handlers_preserve_shadowed_values_and_obey_guards() {
    execute(
        "public fn probe() -> Int {
            let hidden = 7;
            let value: Int = try {throw 3;} recover |hidden| {hidden + 1};
            hidden * 10 + value
        }",
        74,
    );
    execute(
        "public fn probe() -> Int {
            try {throw 7;} recover {_ if false => 99, caught => caught + 1}
        }",
        8,
    );
    execute(
        "public fn probe() -> Int {
            try {try {throw 7;} recover {_ if false => 99}}
            recover |original| {original + 1}
        }",
        8,
    );
    execute(
        "public fn probe() -> Int {
            try {throw (3, 4);} recover |(left, right)| {left * 10 + right}
        }",
        34,
    );
}
