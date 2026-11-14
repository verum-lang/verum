//! Archive consumers retain allocation ownership separately from element loans.
use std::sync::Arc;
use verum_common::{List, Text};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::archive::read_archive_from_file;

#[test]
fn archived_owned_container_declarations_preserve_element_and_buffer_loans() {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(
        core.join("mod.vr"),
        r#"
        module core;
        public module collections {
            public module list {
                public type List<T> is {len:Int,cap:Int,ptr:&unsafe T};
            }
        }
        public module base {
            public module memory {
                public type affine Heap<T> is {ptr:&unsafe T};
                public type Shared<T> is {ptr:&unsafe T};
            }
        }
        public module foreign {
            public type List<T> is {ptr:&unsafe T};
            public type Heap<T> is {ptr:&unsafe T};
            public type Shared<T> is {ptr:&unsafe T};
        }
    "#,
    )
    .unwrap();
    let output = directory.path().join("containers.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let compiled =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core()
            .unwrap_or_else(|error| panic!("{error}\n{}", session.format_diagnostics()));
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    let archive = read_archive_from_file(&output).unwrap();
    let metadata = verum_compiler::archive_metadata::archive_to_core_metadata(&archive).unwrap();
    for eager in [false, true] {
        for (mount, body, accepted) in [
            (
                "core.collections.list.List",
                "fn preserve(mut values:List<Int>, change:fn(&mut List<Int>))->List<Int> {change(&mut values); values}",
                true,
            ),
            (
                "core.collections.list.List",
                "fn invalid(mut values:List<&Int>, change:fn(&mut List<&Int>))->List<&Int> {change(&mut values); values}",
                false,
            ),
            (
                "core.collections.list.List",
                "fn invalid(values:List<Int>, read:fn(&List<Int>)->&Int)->&Int {let local=values; read(&local)}",
                false,
            ),
            (
                "core.base.memory.{Heap,Shared}",
                "fn preserve(value:Shared<Heap<Int>>)->Shared<Heap<Int>> {let first=value; let second=value; first}",
                true,
            ),
            (
                "core.base.memory.Heap",
                "fn preserve(mut values:Heap<Int>, change:fn(&mut Heap<Int>))->Heap<Int> {change(&mut values); values}",
                true,
            ),
            (
                "core.base.memory.Shared",
                "fn preserve(mut values:Shared<Int>, change:fn(&mut Shared<Int>))->Shared<Int> {change(&mut values); values}",
                true,
            ),
            (
                "core.base.memory.Heap",
                "fn invalid(mut values:Heap<&Int>, change:fn(&mut Heap<&Int>))->Heap<&Int> {change(&mut values); values}",
                false,
            ),
            (
                "core.base.memory.Shared",
                "fn invalid(mut values:Shared<&Int>, change:fn(&mut Shared<&Int>))->Shared<&Int> {change(&mut values); values}",
                false,
            ),
            (
                "core.foreign.Heap",
                "fn invalid(mut values:Heap<Int>, change:fn(&mut Heap<Int>))->Heap<Int> {change(&mut values); values}",
                false,
            ),
            (
                "core.foreign.Shared",
                "fn invalid(mut values:Shared<Int>, change:fn(&mut Shared<Int>))->Shared<Int> {change(&mut values); values}",
                false,
            ),
            (
                "core.foreign.List",
                "fn invalid(mut values:List<Int>, change:fn(&mut List<Int>))->List<Int> {change(&mut values); values}",
                false,
            ),
        ] {
            let mut checker = TypeChecker::with_minimal_context();
            checker.register_builtins();
            let metadata = Arc::new(metadata.clone());
            if eager {
                checker.load_core_eager(metadata);
            } else {
                checker.set_core_metadata(metadata);
            }
            checker.set_current_module_path("consumer");
            let source = format!("mount {mount}; {body}");
            let module = Parser::new(&source).parse_module().unwrap();
            let mut errors: List<_> = checker.prepare_module_declarations(&module);
            errors.extend(
                module
                    .items
                    .iter()
                    .filter_map(|item| checker.check_item(item).err()),
            );
            errors.extend(checker.take_deferred_errors());
            let messages: List<Text> = errors
                .iter()
                .chain(checker.diagnostic_sources())
                .map(|error| error.to_string().into())
                .collect();
            if accepted {
                assert!(messages.is_empty(), "eager={eager} {source}: {messages:?}");
            } else {
                assert!(
                    messages
                        .iter()
                        .any(|error| error.contains("does not live long enough")
                            || error.contains("cannot establish the lifetime")),
                    "eager={eager} {source}: {messages:?}"
                );
            }
        }
    }
}
