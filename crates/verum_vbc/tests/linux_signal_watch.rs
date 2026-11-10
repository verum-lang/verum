#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    interpreter::{Interpreter, InterpreterError},
    module::VbcModule,
    serialize::serialize_module,
    value::Value,
};

const GUEST: &str = r#"
    @intrinsic("linux_signal_install") fn acquire(signal: Int) -> Int;
    @intrinsic("linux_signal_take") fn drain(ticket: Int) -> Int;
    @intrinsic("linux_signal_restore") fn release(ticket: Int) -> Int;
    fn start(n: Int) -> Int { acquire(n) }
    fn read(n: Int) -> Int { let operation = drain; operation(n) }
    fn close(n: Int) -> Int { release(n) }
"#;
fn guest() -> Arc<VbcModule> {
    let vbc = VbcCodegen::new()
        .compile_module(&Parser::new(GUEST).parse_module().unwrap())
        .unwrap();
    Arc::new(deserialize_module(&serialize_module(&vbc).unwrap()).unwrap())
}
fn call(vm: &mut Interpreter, name: &str, n: i64) -> Result<i64, InterpreterError> {
    let id = vm.state.module.find_function_by_name(name).unwrap();
    vm.execute_function_with_args(id, &[Value::from_i64(n)])
        .map(|v| v.try_as_i64().expect("Int"))
}

#[test]
fn incorrect_signal_declaration_is_rejected_before_inlining() {
    for source in [
        "fn wrong(x: Bool) -> Int;",
        "fn wrong(x: Int) -> UInt64;",
        "fn wrong() -> Int;",
    ] {
        let source = format!(r#"@intrinsic("linux_signal_take") {source}"#);
        let error = VbcCodegen::new()
            .compile_module(&Parser::new(&source).parse_module().unwrap())
            .unwrap_err();
        assert!(
            error.to_string().contains("requires (Int) -> Int"),
            "{error}"
        );
    }
}

#[test]
fn direct_and_callable_wire_operations_gate_the_signal_not_the_ticket() {
    use verum_vbc::interpreter::permission::{PermissionDecision, PermissionScope};
    for (name, operand) in [("start", 10), ("read", 74), ("close", 138)] {
        let mut vm = Interpreter::new(guest());
        vm.state.set_permission_policy(|scope, signal| {
            assert_eq!((scope, signal), (PermissionScope::Process, 10));
            PermissionDecision::Deny
        });
        assert!(
            matches!(call(&mut vm, name, operand), Err(InterpreterError::Panic { message })
            if message == "permission denied: Process(10)")
        );
    }
}

#[cfg(all(
    target_os = "linux",
    target_pointer_width = "64",
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
#[test]
fn real_delivery_ownership_and_exact_disposition_restoration() {
    use std::{
        mem::MaybeUninit,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static PRIOR: AtomicUsize = AtomicUsize::new(0);
    extern "C" fn prior(_: i32) {
        PRIOR.fetch_add(1, Ordering::Relaxed);
    }
    const SIG: i32 = libc::SIGUSR1;
    // The OS API is an independent observer of the complete kernel disposition.
    unsafe fn current() -> libc::sigaction {
        let mut action = MaybeUninit::zeroed();
        assert_eq!(
            unsafe { libc::sigaction(SIG, std::ptr::null(), action.as_mut_ptr()) },
            0
        );
        unsafe { action.assume_init() }
    }
    let original = unsafe { current() };
    let mut installed: libc::sigaction = unsafe { std::mem::zeroed() };
    installed.sa_sigaction = prior as *const () as usize;
    installed.sa_flags = libc::SA_RESTART;
    unsafe {
        libc::sigemptyset(&mut installed.sa_mask);
        libc::sigaddset(&mut installed.sa_mask, libc::SIGUSR2);
        assert_eq!(libc::sigaction(SIG, &installed, std::ptr::null_mut()), 0);
    }
    let expected = unsafe { current() };
    let mut vm = Interpreter::new(guest());
    let mut other = Interpreter::new(vm.state.module.clone());
    for signal in [0, 65, 9, 9, 19] {
        assert_eq!(call(&mut vm, "start", signal).unwrap(), -22);
    }
    let ticket = call(&mut vm, "start", SIG.into()).unwrap();
    assert!(ticket > 0);
    assert_eq!(call(&mut other, "start", SIG.into()).unwrap(), -16);
    assert_eq!(call(&mut other, "close", ticket).unwrap(), -9);
    assert_eq!(unsafe { libc::raise(SIG) }, 0);
    assert_eq!(
        std::thread::spawn(|| unsafe { libc::raise(SIG) })
            .join()
            .unwrap(),
        0
    );
    assert_eq!(call(&mut vm, "read", ticket).unwrap(), 2);
    assert_eq!(call(&mut vm, "read", ticket).unwrap(), 0);
    assert_eq!(call(&mut vm, "close", ticket).unwrap(), 0);
    let later = call(&mut vm, "start", SIG.into()).unwrap();
    assert!(later > ticket);
    assert_eq!(call(&mut vm, "read", ticket).unwrap(), -9);
    assert_eq!(call(&mut vm, "close", ticket).unwrap(), -9);
    vm.reset();
    let third = call(&mut other, "start", SIG.into()).unwrap();
    assert!(third > later);
    drop(other);
    let after = unsafe { current() };
    assert_eq!(after.sa_sigaction, expected.sa_sigaction);
    assert_eq!(after.sa_flags, expected.sa_flags);
    assert_eq!(
        after.sa_restorer.map(|f| f as usize),
        expected.sa_restorer.map(|f| f as usize)
    );
    for n in 1..=64 {
        assert_eq!(unsafe { libc::sigismember(&after.sa_mask, n) }, unsafe {
            libc::sigismember(&expected.sa_mask, n)
        });
    }
    assert_eq!(unsafe { libc::raise(SIG) }, 0);
    assert_eq!(PRIOR.load(Ordering::Relaxed), 1);
    assert_eq!(
        unsafe { libc::sigaction(SIG, &original, std::ptr::null_mut()) },
        0
    );
}

#[test]
fn public_watch_and_its_real_errno_path_run_from_wire() {
    use verum_ast::cfg::TargetConfig;
    use verum_ast::{ItemKind, TypeKind, decl::ImplKind};
    use verum_vbc::codegen::CodegenConfig;
    let source = include_str!("../../../core/sys/linux/signal_watch.vr").to_owned()
        + r#"
        fn unopened() -> Int {
            let mut w = SignalWatch.new();
            match w.take() { Result.Err(e) => { if e.code != 9 { return 1; } }, Result.Ok(_) => return 2 };
            match w.restore() { Result.Ok(()) => 73, Result.Err(_) => 2 }
        }
        fn lifecycle() -> Int {
            let mut w = SignalWatch.new();
            match unsafe { w.install(28) } { Result.Ok(()) => {}, Result.Err(_) => return 1 };
            match unsafe { w.install(28) } { Result.Ok(()) => {}, Result.Err(_) => return 2 };
            match unsafe { w.install(10) } { Result.Ok(()) => return 3, Result.Err(e) => { if e.code != 16 { return 10; } } };
            match w.take() { Result.Ok(0) => {}, _ => return 4 };
            match w.restore() { Result.Ok(()) => {}, Result.Err(_) => return 5 };
            match w.take() { Result.Ok(_) => 6, Result.Err(_) => 73 }
        }
    "#;
    let target = if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let ast = Parser::new(&source)
        .parse_module()
        .expect("actual public source plus caller");
    let config = CodegenConfig::new("core.sys.linux.signal_watch")
        .with_validation()
        .with_target(TargetConfig::for_target(target));
    // This unit exercises the public watcher and the transitive source it uses
    // from sys.common; unrelated platform services are outside its test scope.
    let mut errors = Parser::new(include_str!("../../../core/sys/common.vr"))
        .parse_module()
        .unwrap();
    errors.items.retain(|item| match &item.kind {
        ItemKind::Type(t) => t.name.as_str() == "OSError",
        ItemKind::Impl(i) => matches!(&i.kind, ImplKind::Inherent(t)
            if matches!(&t.kind, TypeKind::Path(p) if p.last_segment_name() == "OSError")),
        ItemKind::Function(f) => f.name.as_str() == "errno_to_message",
        ItemKind::Module(_) => true,
        _ => false,
    });
    let errno = Parser::new(include_str!("../../../core/sys/linux/errno.vr"))
        .parse_module()
        .unwrap();
    let files = [
        ("core.sys.common", &errors),
        ("core.sys.linux.errno", &errno),
        ("core.sys.linux.signal_watch", &ast),
    ];
    let mut codegen = VbcCodegen::with_config(config);
    codegen.initialize();
    codegen.collect_owned_unit_declarations(&files).unwrap();
    codegen
        .compile_owned_unit_items(&files, verum_vbc::codegen::ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module().unwrap();
    let mut vm = Interpreter::new(Arc::new(
        deserialize_module(&serialize_module(&module).unwrap()).unwrap(),
    ));
    let entry = |name: &str| {
        module
            .find_function_by_name(&format!("core.sys.linux.signal_watch.{name}"))
            .unwrap()
    };
    assert_eq!(
        vm.execute_function(entry("unopened")).unwrap().try_as_i64(),
        Some(73)
    );
    if cfg!(all(
        target_os = "linux",
        target_pointer_width = "64",
        any(target_arch = "aarch64", target_arch = "x86_64")
    )) {
        assert_eq!(
            vm.execute_function(entry("lifecycle"))
                .unwrap()
                .try_as_i64(),
            Some(73)
        );
    }
}
