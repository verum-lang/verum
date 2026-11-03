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

#[test]
fn inline_reexport_matches_the_declaring_generic_sum_after_bootstrap() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (file, source) in [
        (
            "mod.vr",
            "module core; public module values; public module reader;
             public module surface { public mount super.values.*; }",
        ),
        (
            "values/mod.vr",
            "module core.values; public type Choice<T> is Missing | Found(T);",
        ),
        (
            "reader/mod.vr",
            "module core.reader; mount core.{Choice};
             fn read(value: core.values.Choice<Int>) -> Int {
                 match value { Choice.Missing => 3, Choice.Found(n) => n }
             }
             public fn check_both() -> Int { read(Choice.Missing) + read(Choice.Found(8)) }",
        ),
    ] {
        let path = core.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let archive = bootstrap(&core);
    assert_eq!(
        execute(&archive, "module app; mount core.reader.check_both; fn main() -> Int { check_both() }"),
        11
    );
}

#[test]
fn sibling_private_functions_keep_their_call_and_result_owners() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    let codecs = core.join("codecs");
    fs::create_dir_all(&codecs).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module codecs;").unwrap();
    fs::write(codecs.join("mod.vr"), "module core.codecs; public module left; public module right;").unwrap();
    for (file, sum, first, second, value) in [
        ("left", "Amber", "Kept", "Gone", 3),
        ("right", "Blue", "Seen", "Absent", 4),
    ] {
        fs::write(codecs.join(format!("{file}.vr")), format!(
            "module core.codecs.{file};
             public type {sum} is {first}(Int) | {second};
             fn decode_item() -> {sum} {{ {sum}.{first}({value}) }}
             public fn read_{file}() -> Int {{
                 let item = decode_item();
                 match item {{ {sum}.{first}(x) => x, {sum}.{second} => 0 }}
             }}"
        )).unwrap();
    }
    let archive = bootstrap(&core);
    assert_eq!(
        execute(
            &archive,
            "module app; mount core.codecs.left.read_left; mount core.codecs.right.read_right;
             fn main() -> Int { read_left() * 10 + read_right() }",
        ),
        34
    );
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
public type IoResult<T> is Result<T, StreamError>;
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
public type Complete<T> is { value: T };
implement<T> Future for Complete<T> {
    type Output = T;
    fn poll(&mut self, _cx: &mut Context) -> Poll<T> { Poll.Ready(self.value) }
}
public fn complete<T>(value: T) -> Complete<T> { Complete { value: value } }
"#,
    )
    .unwrap();
    fs::write(core.join("scheduler/select.vr"), r#"
module core.scheduler.select;
mount core.scheduler.future.{Future, Poll, Context, Waker, Thread, noop_waker};
mount core.Maybe;
mount core.values.{IoResult, Result};
public type WriteValue<T> is { payload: T, count: Int };
implement<T> Future for WriteValue<T> {
    type Output = IoResult<Int>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<IoResult<Int>> {
        Poll.Ready(Result.Ok(self.count))
    }
}
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
mount core.scheduler.select.WriteValue;
public type AcceptFuture<'a> is { value: &'a Int };
implement<'a> Future for AcceptFuture<'a> {
    type Output = Result<(Int, Int), IoError>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Result<(Int, Int), IoError>> {
        Poll.Ready(Result.Ok((*self.value, 9)))
    }
}
public async fn forward() -> Int {
    let value = core.scheduler.future.complete(37).await;
    let write: WriteValue<Int> = WriteValue { payload: value, count: value };
    select { result = write.await => { match result { Result.Ok(count) => count, _ => 0 } } }
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
    let archive = bootstrap(&core);
    for (module, name) in [
        ("core.scheduler", "choose"),
        ("core.network", "accept"),
        ("core.network", "forward"),
    ] {
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
    assert_eq!(
        execute(
            &archive,
            "module app; mount core.network.tcp.forward; async fn main() -> Int { forward().await }",
        ),
        37
    );
}

fn bootstrap(core: &std::path::Path) -> verum_vbc::archive::VbcArchive {
    let output = core.parent().unwrap().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(core).with_output(&output))
            .compile_core()
            .unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    read_archive_from_file(&output).unwrap()
}

fn execute(archive: &verum_vbc::archive::VbcArchive, source: &str) -> i64 {
    let app = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    let mut codegen =
        verum_vbc::codegen::VbcCodegen::with_config(verum_vbc::codegen::CodegenConfig::new("app"));
    verum_compiler::archive_ctx_loader::ArchiveCtxCache::new().apply_lazy_with_types(
        archive,
        &mut codegen,
        &app,
        &[],
    );
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], verum_vbc::codegen::ItemFailurePolicy::Strict)
        .unwrap();
    let app = codegen.finalize_module_from_state().unwrap();
    let main = app.find_function_by_name("app.main").unwrap();
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(app))
        .execute_function(main)
        .unwrap();
    assert!(value.is_int());
    value.as_i64()
}

#[test]
fn cyclic_dependency_completes_owned_variant_and_record_fields() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (path, source) in [
        (
            "mod.vr",
            "module core; public module values; public module workers; public module clock; public module coordination;",
        ),
        (
            "values/mod.vr",
            "module core.values; public type Maybe<T> is None | Some(T); public type Slots<T> is { len: Int }; implement<T> Slots<T> { public fn empty() -> Slots<T> { Slots { len: 0 } } }",
        ),
        (
            "coordination/mod.vr",
            "module core.coordination; public type Mutex<T> is { data: T }; implement<T> Mutex<T> { public fn new(data: T) -> Mutex<T> { Mutex { data: data } } }",
        ),
        (
            "clock/mod.vr",
            "module core.clock; public module instant; public mount .instant.{Instant};",
        ),
        (
            "clock/instant.vr",
            r#"
            module core.clock.instant;
            mount core.workers.cancellation.CancelReason;
            public type Instant is { nanos: Int };
            implement Instant { public fn record_reason(&self, _reason: CancelReason) -> Int { self.nanos } }
        "#,
        ),
        (
            "workers/mod.vr",
            "module core.workers; public module cancellation;",
        ),
        (
            "workers/cancellation.vr",
            r#"
            module core.workers.cancellation;
            mount core.clock.Instant;
            mount core.coordination.Mutex;
            mount core.values.{Maybe, Slots};
            public type CancelReason is Cancelled | Timeout { deadline: Instant };
            public type Flag is { reason: Mutex<Maybe<CancelReason>>, observed: Mutex<Slots<Instant>> };
            public fn probe() -> Int {
                let flag = Flag { reason: Mutex.new(Maybe.None), observed: Mutex.new(Slots.empty()) };
                match flag.reason.data { Maybe.None => 79, Maybe.Some(_) => 0 }
            }
        "#,
        ),
    ] {
        let file = core.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, source).unwrap();
    }
    let archive = bootstrap(&core);
    let module = archive.load_module("core.workers").unwrap();
    let ty = |name: &str| {
        module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some(name))
            .unwrap()
    };
    let reason = ty("CancelReason");
    let timeout = reason
        .variants
        .iter()
        .find(|v| module.get_string(v.name) == Some("Timeout"))
        .unwrap();
    let clock = timeout.fields[0]
        .declaration_type
        .as_ref()
        .unwrap()
        .base_type_id()
        .unwrap();
    assert_eq!(
        module.get_string(module.get_type(clock).unwrap().origin_module.unwrap()),
        Some("core.clock.instant")
    );
    let flag = ty("Flag");
    let mutex = flag.fields[0]
        .declaration_type
        .as_ref()
        .unwrap()
        .base_type_id()
        .unwrap();
    assert_eq!(
        module.get_string(module.get_type(mutex).unwrap().origin_module.unwrap()),
        Some("core.coordination")
    );
    let probe = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.rsplit('.').next() == Some("probe"))
        })
        .unwrap();
    let bytecode = &module.bytecode
        [probe.bytecode_offset as usize..(probe.bytecode_offset + probe.bytecode_length) as usize];
    let witnesses = decode_instructions(bytecode)
        .unwrap()
        .into_iter()
        .filter_map(|op| match op {
            Instruction::CallG { type_args, .. } => Some(type_args),
            _ => None,
        })
        .collect::<Vec<_>>();
    let instant = verum_vbc::types::TypeRef::Concrete(clock);
    let slots = flag.fields[1].declaration_type.as_ref().unwrap();
    let verum_vbc::types::TypeRef::Instantiated { args, .. } = slots else {
        panic!("expected Mutex<Slots<Instant>>")
    };
    assert!(
        witnesses
            .iter()
            .any(|args| args.as_slice() == [instant.clone()]),
        "{witnesses:?}"
    );
    assert!(
        witnesses
            .iter()
            .any(|actual| actual.as_slice() == args.as_slice()),
        "{witnesses:?}"
    );
    assert_eq!(
        execute(
            &archive,
            "module app; mount core.workers.cancellation.probe; fn main() -> Int { probe() }"
        ),
        79
    );
}

#[test]
fn empty_collection_borrows_only_the_declared_element_type() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    // The canonical collection's runtime constructor and bootstrap dependencies
    // follow a different path than an ordinary user-defined static method.
    for path in [
        "base",
        "mem",
        "sys",
        "sys/linux",
        "sys/windows",
        "sys/darwin",
        "intrinsics",
        "text",
        "collections",
        "consumer",
    ] {
        fs::create_dir_all(core.join(path)).unwrap();
        fs::write(
            core.join(path).join("mod.vr"),
            format!("module core.{};", path.replace('/', ".")),
        )
        .unwrap();
    }
    fs::write(
        core.join("mod.vr"),
        "module core; public module collections; public module consumer;",
    )
    .unwrap();
    fs::write(
        core.join("collections/mod.vr"),
        "module core.collections; public module list;",
    )
    .unwrap();
    fs::write(core.join("collections/list.vr"), "module core.collections.list; public type List<T> is { len: Int }; implement<T> List<T> { public fn new() -> List<T> { List { len: 0 } } }").unwrap();
    fs::write(
        core.join("consumer/mod.vr"),
        r#"
        module core.consumer;
        mount core.collections.list.List;
        public type Slot<T> is { value: T };
        implement<T> Slot<T> { public fn new(x: T) -> Slot<T> { Slot { value: x } } }
        public type Batch is { clocks: Slot<List<Int>>, names: Slot<List<Text>> };
        public fn batches<T>(_unrelated: T) -> Batch {
            Batch { clocks: Slot.new(List.new()), names: Slot.new(List.new()) }
        }
    "#,
    )
    .unwrap();
    let archive = bootstrap(&core);
    let module = archive.load_module("core.consumer").unwrap();
    let batches = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.rsplit('.').next() == Some("batches"))
        })
        .unwrap();
    let bytecode = &module.bytecode[batches.bytecode_offset as usize
        ..(batches.bytecode_offset + batches.bytecode_length) as usize];
    let calls: Vec<_> = decode_instructions(bytecode)
        .unwrap()
        .into_iter()
        .filter_map(|op| match op {
            Instruction::CallG { type_args, .. } => Some(type_args),
            _ => None,
        })
        .collect();
    let batch = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Batch"))
        .unwrap();
    let expected: Vec<_> = batch
        .fields
        .iter()
        .map(|field| {
            let Some(verum_vbc::types::TypeRef::Instantiated { args, .. }) =
                &field.declaration_type
            else {
                panic!("Slot field missed its declared value type");
            };
            args.clone()
        })
        .collect();
    use verum_vbc::types::{TypeId, TypeRef};
    assert_eq!(
        expected,
        [TypeId::INT, TypeId::TEXT].map(|element| {
            vec![TypeRef::Instantiated {
                base: TypeId::LIST,
                args: vec![TypeRef::Concrete(element)],
            }]
        })
    );
    assert_eq!(calls, expected);
}
