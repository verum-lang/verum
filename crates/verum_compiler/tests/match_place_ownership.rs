//! Match-place inspection preserves ownership through the ordinary core producer.
use std::{fs, sync::Arc};
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{VbcModule, archive::read_archive_from_file, interpreter::Interpreter};

fn produce(source: &str) -> Result<VbcModule, Text> {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    fs::create_dir(&core).unwrap();
    fs::write(core.join("mod.vr"), source).unwrap();
    let output = directory.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    match CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core()
    {
        Ok(result) => {
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            Ok(read_archive_from_file(&output)
                .unwrap()
                .load_module("core")
                .unwrap())
        }
        Err(error) => {
            assert!(!output.exists(), "invalid ownership published an archive");
            Err(format!("{error:#}").into())
        }
    }
}

const TYPES: &str = "module core; type affine Token is {value:Int}; type Choice is Scalar(Int) | Owned(Token); type Holder is {choice:Choice};";

#[test]
fn selected_ref_payload_preserves_the_actual_owner_on_archive_and_wire() {
    let source = format!("{TYPES}
        fn inspect(value:&Holder)->Int {{match value.choice {{Choice.Scalar(n)=>n,Choice.Owned(ref token)=>token.value}}}}
        fn probe()->Int {{
            let owner=Holder{{choice:Choice.Owned(Token{{value:37}})}};
            let first=inspect(&owner);
            let second=inspect(&owner);
            first+second-1
        }}");
    let module = produce(&source).expect("complete checked source");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for (route, module) in [("archive", module), ("wire", wire)] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("core.probe").unwrap();
        assert!(!verum_vbc::stub_ranges::is_stub_id(entry.0));
        let mut vm = Interpreter::new(Arc::new(module));
        vm.run_global_ctors().unwrap();
        let result = vm
            .execute_function(entry)
            .unwrap_or_else(|e| panic!("{route}: {e:?}"));
        assert_eq!(result.try_as_i64(), Some(73), "{route}");
    }
}

#[test]
fn moved_payload_or_escaping_pattern_reference_cannot_publish_an_archive() {
    for (body, expected) in [
        (
            "fn invalid(value:&Holder)->Token {match value.choice {Choice.Owned(token)=>token,Choice.Scalar(_)=>Token{value:0}}}",
            "borrowed storage",
        ),
        (
            "fn invalid()->&Token {let local=Holder{choice:Choice.Owned(Token{value:73})}; match local.choice {Choice.Owned(ref token)=>token,Choice.Scalar(_)=>panic(\"empty\")}}",
            "E312",
        ),
    ] {
        let error = produce(&format!("{TYPES} {body}")).expect_err("invalid lifetime or ownership");
        assert!(error.contains(expected), "{body}: {error}");
    }
}
