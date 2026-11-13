//! Checked scalar assignment preserves values across archive serialization.
use std::{fs, sync::Arc};
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{VbcModule, archive::read_archive_from_file, interpreter::Interpreter};

fn produce(source: &str) -> Result<VbcModule, Text> {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    fs::write(core.join("mod.vr"), source).unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core();
    match result {
        Ok(result) => {
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            Ok(read_archive_from_file(&output)
                .unwrap()
                .load_module("core")
                .unwrap())
        }
        Err(error) => {
            assert!(!output.exists(), "rejected source published an archive");
            Err(format!("{error:#}").into())
        }
    }
}

fn execute(source: &str, expected: i64) {
    let module = produce(source).unwrap_or_else(|error| panic!("source:\n{source}\n{error}"));
    verum_vbc::validate::validate_module(&module).expect("strict producer validation");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize actual module"),
    )
    .expect("deserialize actual module");
    for (route, module) in [("produced", module), ("wire", wire)] {
        verum_vbc::validate::validate_module(&module).expect("strict execution validation");
        let entry = module
            .find_function_by_name("core.probe")
            .expect("actual probe declaration");
        assert!(
            !verum_vbc::stub_ranges::is_stub_id(entry.0),
            "probe is a forward stub"
        );
        let mut vm = Interpreter::new(Arc::new(module));
        vm.run_global_ctors().expect("actual global initialization");
        let result = vm
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("{route}: {error:?}"));
        assert_eq!(
            result.try_as_i64(),
            Some(expected),
            "{route}: receiver/body dispatch changed"
        );
    }
}

#[test]
fn borrowed_variant_payloads_update_successive_fields() {
    execute(
        r#"
        module core;
        type Number is Value(Int64) | Missing;
        type Cursor is { first:Int64, second:Int64 };
        implement Cursor {
            fn update(&mut self, input:&Number) {
                let mut value:Int64=0_i64;
                match input { Value(n)=>{value=n;} Missing=>{} }
                self.first=value;
                self.second=value+1_i64;
            }
            fn total(&self)->Int {(self.first+self.second) as Int}
        }
        public fn probe()->Int {
            let input=Number.Value(37_i64);
            let mut cursor=Cursor {first:0_i64,second:0_i64};
            cursor.update(&input);
            cursor.total()
        }
    "#,
        75,
    );
}

#[test]
fn copied_reference_payload_survives_its_intermediate_owner() {
    execute(
        r#"
        module core;
        type Sample is { ptr: &Int, total: Int };
        fn forward(value: Sample) -> Sample {
            let alias = &value;
            Sample { ptr: alias.ptr, total: alias.total }
        }
        public fn probe() -> Int {
            let n = 71;
            let copied = forward(Sample { ptr: &n, total: 2 });
            *copied.ptr + copied.total
        }
        "#,
        73,
    );
}
