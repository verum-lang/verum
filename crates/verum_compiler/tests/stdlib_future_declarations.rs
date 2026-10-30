//! Actual archive bootstrap preserves Future ownership across file modules.
use std::fs;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{Instruction, archive::read_archive_from_file, bytecode::decode_instructions};

#[test]
fn imported_future_outputs_and_contextual_owner_arguments_survive_bootstrap() {
    check_bootstrap(false);
}

#[test]
fn glob_imported_future_output_keeps_declaration_authority() {
    check_bootstrap(true);
}

fn check_bootstrap(glob: bool) {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for directory in ["values", "scheduler", "network"] {
        fs::create_dir_all(core.join(directory)).unwrap();
    }
    fs::write(core.join("mod.vr"), "module core; public module values; public module scheduler; public module network; public mount .values.Maybe;").unwrap();
    fs::write(
        core.join("values/mod.vr"),
        r#"
module core.values;
public type Result<T, E> is Ok(T) | Err(E);
public type Maybe<T> is None | Some(T);
public type StreamError is { code: Int };
public type IoError is StreamError;
"#,
    )
    .unwrap();
    fs::write(
        core.join("scheduler/mod.vr"),
        "module core.scheduler; public module future; public module select;",
    )
    .unwrap();
    fs::write(
        core.join("scheduler/future.vr"),
        r#"
module core.scheduler.future;
public type Poll<T> is Ready(T) | Pending;
public type Waker is { marker: Int };
public type Context is { waker: &Waker };
implement Context { public fn from_waker(waker: &Waker) -> Context { Context { waker: waker } } }
public fn noop_waker() -> Waker { Waker { marker: 0 } }
public type Thread is ();
implement Thread { public fn yield_now() {} }
public type Future is protocol {
    type Output;
    fn poll(&mut self, cx: &mut Context) -> Poll<Self.Output>;
};
"#,
    )
    .unwrap();
    fs::write(core.join("scheduler/select.vr"), r#"
module core.scheduler.select;
mount core.scheduler.future.{Future, Poll, Context, Waker, Thread, noop_waker};
mount core.Maybe;
public type Tagged<P, T> is { value: Maybe<T> };
implement<P, T> Future for Tagged<P, T> {
    type Output = Maybe<P>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Maybe<P>> { Poll.Ready(Maybe.None) }
}
public async fn choose<X, P, T>(x: X, value: T) -> Maybe<P> { Tagged { value: Maybe.Some(value) }.await }
"#).unwrap();
    fs::write(
        core.join("network/mod.vr"),
        "module core.network; public module tcp;",
    )
    .unwrap();
    fs::write(core.join("network/tcp.vr"), r#"
module core.network.tcp;
mount core.scheduler.future.{Future, Poll, Context, Waker, Thread, noop_waker};
mount core.values.{Result, IoError};
public type AcceptFuture<'a> is { value: &'a Int };
implement<'a> Future for AcceptFuture<'a> {
    type Output = Result<(Int, Int), IoError>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Result<(Int, Int), IoError>> {
        Poll.Ready(Result.Ok((*self.value, 9)))
    }
}
public async fn accept(value: &Int) -> Result<(Int, Int), IoError> { AcceptFuture { value: value }.await }
"#).unwrap();
    if glob {
        let path = core.join("network/tcp.vr");
        let source = fs::read_to_string(&path).unwrap().replace(
            "mount core.values.{Result, IoError};",
            "mount core.values.*;",
        );
        fs::write(path, source).unwrap();
    }
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core()
            .unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&output).unwrap();
    for (module, name) in [("core.scheduler", "choose"), ("core.network", "accept")] {
        let module = archive.load_module(module).unwrap();
        let function = module
            .functions
            .iter()
            .find(|f| {
                module
                    .strings
                    .get(f.name)
                    .is_some_and(|n| n == name || n.ends_with(&format!(".{name}")))
            })
            .unwrap();
        let code = decode_instructions(
            &module.bytecode[function.bytecode_offset as usize
                ..(function.bytecode_offset + function.bytecode_length) as usize],
        )
        .unwrap();
        let panics: Vec<_> = code
            .iter()
            .filter_map(|op| match op {
                Instruction::Panic { message_id } => {
                    module.strings.get(verum_vbc::StringId(*message_id))
                }
                _ => None,
            })
            .collect();
        assert!(
            function.has_source_body && panics.is_empty(),
            "{name}: {panics:?}"
        );
    }
}
