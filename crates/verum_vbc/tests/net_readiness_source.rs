//! Execute the shared network future, preserving its actual stdlib bodies and
//! declaration owners. The host sockets are fixtures, never readiness oracles.
#![cfg(all(feature = "codegen", any(target_os = "macos", target_os = "linux")))]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::time::Duration;
use verum_ast::{
    ItemKind, Module, TypeKind,
    decl::{ImplItemKind, ImplKind},
};
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    Value, VbcModule,
    codegen::{CodegenConfig, CodegenErrorKind, CodegenResult, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().expect("source grammar")
}

// Select dependencies by declaration, without replacing signatures or bodies.
// Readiness itself is always compiled in full, including all of its mounts.
fn declarations(source: &str, names: &[&str], methods: &[(&str, &[&str])]) -> Module {
    let mut module = parse(source);
    module.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Type(decl) => names.contains(&decl.name.as_str()),
        ItemKind::Protocol(decl) => names.contains(&decl.name.as_str()),
        ItemKind::Function(decl) => names.contains(&decl.name.as_str()),
        ItemKind::Impl(decl) => {
            let ImplKind::Inherent(ty) = &decl.kind else {
                return false;
            };
            let TypeKind::Path(path) = &ty.kind else {
                return false;
            };
            let Some(name) = path.as_ident() else {
                return false;
            };
            let Some((_, selected)) = methods.iter().find(|(owner, _)| *owner == name.as_str())
            else {
                return false;
            };
            decl.items.retain(|item| {
                matches!(&item.kind, ImplItemKind::Function(function)
                if selected.contains(&function.name.as_str()))
            });
            true
        }
        _ => false,
    });
    module
}

fn sources() -> List<Module> {
    let mut future = declarations(
        include_str!("../../../core/async/future.vr"),
        &["Future"],
        &[],
    );
    future
        .items
        .extend(parse("mount core.async.poll.Poll; mount core.async.waker.Context;").items);
    List::from_iter([
        declarations(include_str!("../../../core/async/poll.vr"), &["Poll"], &[]),
        future,
        declarations(
            include_str!("../../../core/async/waker.vr"),
            &["RawWakerVTable", "RawWaker", "Waker", "Context"],
            &[
                ("RawWakerVTable", &["new"]),
                ("RawWaker", &["new"]),
                ("Waker", &["from_raw", "wake_by_ref"]),
                ("Context", &["from_waker", "waker"]),
            ],
        ),
        declarations(
            include_str!("../../../core/sys/common.vr"),
            &["FileDesc"],
            &[("FileDesc", &["as_raw"])],
        ),
        declarations(
            include_str!("../../../core/sys/io_engine.vr"),
            &["IOInterest"],
            &[],
        ),
        declarations(
            include_str!("../../../core/intrinsics/runtime/os.vr"),
            &["__io_wait_readable_raw", "__io_wait_writable_raw"],
            &[],
        ),
        parse(include_str!("../../../core/net/readiness.vr")),
    ])
}

fn compile(consumer: &str) -> CodegenResult<VbcModule> {
    let mut modules = sources();
    modules.push(parse(consumer));
    let units: List<_> = modules.iter().collect();
    let mut codegen =
        VbcCodegen::with_config(CodegenConfig::new("readiness_fixture").with_validation());
    codegen.collect_unit_declarations(&units)?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&units, ItemFailurePolicy::Strict)?;
    let mut module = codegen.finalize_module_from_state()?;
    assert!(module.resolve_external_bands().is_empty());
    module.resolve_protocol_dispatch();
    Ok(module)
}

const PROBE: &str = r#"
    mount core.net.readiness.AsyncIoReady;
    mount core.sys.common.FileDesc;
    mount core.async.future.Future;
    mount core.async.poll.Poll;
    mount core.async.waker.{RawWakerVTable, RawWaker, Waker, Context};

    fn count_wake(data: &unsafe Byte) {
        // The caller retains this live counter until poll returns.
        unsafe {
            let counter = data as &unsafe mut Int;
            *counter = *counter + 1;
        }
    }
    fn discard(_data: &unsafe Byte) {}
    fn clone_waker(data: &unsafe Byte) -> RawWaker {
        RawWaker.new(data, RawWakerVTable.new(clone_waker, count_wake, count_wake, discard))
    }
    fn probe(fd: Int, write_ready: Bool) -> Int {
        let mut wakes = 0;
        let waker = Waker.from_raw(clone_waker((&mut wakes) as &unsafe Byte));
        let mut context = Context.from_waker(&waker);
        let mut ready = if write_ready { AsyncIoReady.writable(FileDesc(fd)) }
            else { AsyncIoReady.readable(FileDesc(fd)) };
        let result = match ready.poll(&mut context) { Poll.Ready(()) => 1, Poll.Pending => 0 };
        result + 10 * wakes
    }
"#;

fn compiled_and_archived(owner: &str) -> [VbcModule; 2] {
    let module = compile(&format!("module {owner};\n{PROBE}")).expect("shared readiness source");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    [module, wire]
}

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture bind");
    let client = TcpStream::connect(listener.local_addr().unwrap()).expect("fixture connect");
    let (server, _) = listener.accept().expect("fixture accept");
    (client, server)
}

#[test]
fn network_siblings_share_real_readiness_and_wake_only_pending_polls() {
    for owner in ["core.net.tcp", "core.net.unix"] {
        for module in compiled_and_archived(owner) {
            let probe = module
                .find_function_by_name(&format!("{owner}.probe"))
                .unwrap();
            let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
            let (mut client, mut server) = pair();
            let mut args = [
                Value::from_i64(server.as_raw_fd() as i64),
                Value::from_bool(false),
            ];
            assert_eq!(
                vm.execute_function_with_args(probe, &args)
                    .unwrap()
                    .try_as_i64(),
                Some(10),
                "an idle socket must be Pending and wake the driver once"
            );
            args[1] = Value::from_bool(true);
            assert_eq!(
                vm.execute_function_with_args(probe, &args)
                    .unwrap()
                    .try_as_i64(),
                Some(1),
                "a writable socket must complete without a wake"
            );
            args[1] = Value::from_bool(false);
            client.write_all(b"ready").expect("fixture send");
            server
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut bytes = [0; 5];
            assert!(server.peek(&mut bytes).expect("fixture arrival") > 0);
            assert_eq!(
                vm.execute_function_with_args(probe, &args)
                    .unwrap()
                    .try_as_i64(),
                Some(1),
                "queued data must be ready through the source future"
            );
            server.read_exact(&mut bytes).expect("fixture drain");
            assert_eq!(&bytes, b"ready");
            drop(client);
            assert_eq!(server.peek(&mut bytes).expect("fixture EOF"), 0);
            assert_eq!(
                vm.execute_function_with_args(probe, &args)
                    .unwrap()
                    .try_as_i64(),
                Some(1),
                "EOF must be ready so the transport can observe termination"
            );
        }
    }
}

#[test]
fn invalid_descriptors_fail_instead_of_reporting_pending_or_ready() {
    for module in compiled_and_archived("core.net.tcp") {
        let probe = module.find_function_by_name("core.net.tcp.probe").unwrap();
        for fd in [-1, i64::MAX] {
            for write_ready in [false, true] {
                let mut vm = Interpreter::try_new_validated(module.clone().into()).unwrap();
                let error = vm
                    .execute_function_with_args(
                        probe,
                        &[Value::from_i64(fd), Value::from_bool(write_ready)],
                    )
                    .unwrap_err();
                assert!(
                    error.to_string().contains("I/O readiness probe failed"),
                    "{error}"
                );
            }
        }
    }
}

#[test]
fn shared_readiness_does_not_become_a_public_or_cog_wide_api() {
    for owner in ["application", "core.io", "core.network", "other.net.tcp"] {
        let error = compile(&format!("module {owner};\n{PROBE}")).unwrap_err();
        let CodegenErrorKind::InvalidSourceMount(messages) = error.kind else {
            panic!("wrong refusal for {owner}: {error}");
        };
        assert!(
            messages.iter().any(
                |message| message.contains("core.net.readiness.AsyncIoReady")
                    && message.contains("inaccessible")
                    && message.contains(owner)
            ),
            "{messages:?}"
        );
    }
}
