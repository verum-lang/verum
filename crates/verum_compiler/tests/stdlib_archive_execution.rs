//! Release-gate checks against the chosen serialized stdlib and its bytecode.
use std::{path::PathBuf, sync::Arc};
use verum_compiler::archive_ctx_loader::ArchiveCtxCache;
use verum_fast_parser::Parser;
use verum_vbc::{
    archive::read_archive_from_file,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn bounded_channel_preserves_payloads_and_destroys_generic_endpoints() {
    let source = r#"
        module app;
        mount core.async.channel.{Sender, Receiver, bounded};
        mount core.time.duration.Duration;
        fn main() -> Int {
            let (sender, mut receiver): (Sender<Int>, Receiver<Int>) = bounded<Int>(1);
            if sender.send_timeout(13, Duration.zero()).is_err() { return 1; }
            let full = match sender.send_timeout(17, Duration.from_millis(3)) {
                Result.Err(e) => e.0,
                Result.Ok(()) => {return 2;}
            };
            let first = receiver.try_recv().unwrap();
            receiver.close();
            let closed = match sender.send_timeout(19, Duration.zero()) {
                Result.Err(e) => e.0,
                Result.Ok(()) => {return 3;}
            };
            assert(first == 13, "queue payload lost");
            assert(full == 17, "timed-out payload lost");
            assert(closed == 19, "disconnected payload lost");
            first * 10000 + full * 100 + closed
        }
    "#;
    assert_eq!(execute_from_archive(source), 13 * 10000 + 17 * 100 + 19);
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn returning_a_lock_guard_holds_and_releases_one_acquisition() {
    let source = r#"
        module app;
        mount core.sync.rwlock.{RwLock, RwLockReadGuard, RwLockWriteGuard};
        mount core.sync.mutex.{Mutex, MutexGuard, LockResult, TryLockResult};
        mount core.base.result.Result;
        fn main() -> Int {
            let lock = RwLock.new(17);
            {
                let held = lock.read().unwrap();
                if lock.try_write().is_ok() { return 1; }
                if *held != 17 { return 2; }
            }
            {
                let held = lock.try_write().unwrap();
                if lock.try_read().is_ok() { return 3; }
                if *held != 17 { return 4; }
            }
            let mutex = Mutex.new(25);
            {
                let held = mutex.lock().unwrap();
                if mutex.try_lock().is_ok() { return 5; }
                if *held != 25 { return 6; }
            }
            let read = lock.try_read().unwrap();
            let exclusive = mutex.try_lock().unwrap();
            *read + *exclusive
        }
    "#;
    assert_eq!(execute_from_archive(source), 42);
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive and loopback TCP"]
fn weft_overload_sends_the_complete_http_response() {
    let source = r#"
        module app;
        mount core.net.weft.listener.{PlainHttp1Runner, ConnectionRunner};
        mount core.net.weft.router.Router;
        mount core.net.weft.connection.ConnectionConfig;
        mount core.net.tcp.{TcpStream, TcpListener};
        mount core.net.addr.{Ipv4Addr, SocketAddrV4, SocketAddr};
        mount core.async.cancellation.CancellationToken;
        mount core.sync.atomic.AtomicBool;
        async fn main() -> Int {
            let listener = TcpListener.bind(SocketAddr.V4(SocketAddrV4.new(Ipv4Addr.localhost(), 0))).unwrap();
            let mut client = TcpStream.connect(listener.local_addr().unwrap()).unwrap();
            let (server, _) = listener.accept_async().await.unwrap();
            let runner = PlainHttp1Runner.new(
                Heap.new(Router.new()), CancellationToken.new(),
                Shared.new(AtomicBool.new(false)), ConnectionConfig.default(),
            );
            runner.refuse_overloaded(server).await;
            let expected = "HTTP/1.1 503 Service Unavailable\r\nconnection: close\r\ncontent-length: 11\r\n\r\noverloaded\n".as_bytes();
            let mut size = 0;
            while size < expected.len() {
                let mut bytes: List<Byte> = List.with_capacity(256);
                let count = client.read_async(&mut bytes).await.unwrap();
                if count <= 0 { return -1; }
                for byte in bytes {
                    if size >= expected.len() || byte != expected[size] { return -2; }
                    size = size + 1;
                }
            }
            size
        }
    "#;
    assert_eq!(execute_from_archive(source), 86);
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn rwlock_methods_preserve_the_declared_lock_result() {
    let source = r#"
        module app;
        mount core.sync.rwlock.{RwLock, RwLockReadGuard, RwLockWriteGuard};
        mount core.sync.mutex.LockResult;
        mount core.base.result.Result;
        fn main() -> Int {
            let first = RwLock.new(17);
            let second = RwLock.new(25);
            let read = match first.read() {
                Result.Ok(guard) => *guard, Result.Err(_) => -100,
            };
            let write = match second.write() {
                Result.Ok(guard) => *guard, Result.Err(_) => -100,
            };
            read + write
        }
    "#;
    assert_eq!(execute_from_archive(source), 42);
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn supervisor_constructors_use_declared_ownership() {
    let source = r#"
        module app;
        mount core.runtime.supervisor.{SupervisorHandle, SupervisorConfig};
        fn main() -> Int {
            let root = SupervisorHandle.root(SupervisorConfig.one_for_one("root"));
            let child = SupervisorHandle.new(SupervisorConfig.rest_for_one("child"));
            if root.id().raw() != 0 { return 1; }
            if child.id().raw() == 0 { return 2; }
            87
        }
    "#;
    assert_eq!(execute_from_archive(source), 87);
}

#[cfg(unix)]
#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive and a Unix socket"]
fn cancelled_accept_preserves_the_pending_unix_connection() {
    let directory = tempfile::tempdir_in("/tmp").unwrap();
    let socket = directory.path().join("server.sock");
    let source = r#"
        module app;
        mount core.net.unix.{UnixListener, UnixStream};
        mount core.async.cancellation.CancellationToken;
        async fn main() -> Int {
            let path: Text = "__SOCKET_PATH__".into();
            let listener = match UnixListener.bind(&path) { Result.Ok(v) => v, Result.Err(_) => {return 1;} };
            let client = match UnixStream.connect(&path) { Result.Ok(v) => v, Result.Err(_) => {return 2;} };
            let token = CancellationToken.new();
            token.cancel();
            if listener.accept_cancellable(&token).await.is_ok() { return 3; }
            let (server, peer) = match listener.accept_async().await { Result.Ok(v) => v, Result.Err(_) => {return 4;} };
            if peer.is_some() || client.peer_addr().is_none() || server.peer_addr().is_some() { return 5; }
            85
        }
    "#;
    assert_eq!(execute_from_archive(&source.replace("__SOCKET_PATH__", socket.to_str().unwrap())), 85);
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn cancellation_observers_see_publication_and_linked_children() {
    let source = r#"
        module app;
        mount core.async.cancellation.{CancellationToken, CancelReason};
        fn main() -> Int {
            let parent = CancellationToken.new();
            let before = parent.child_source();
            let observed_parent = parent.clone();
            let mark = CancellationToken.new();
            let callback_mark = mark.clone();
            let registration = parent.register(move || {
                if observed_parent.is_cancelled() { callback_mark.cancel(); }
            });
            parent.cancel();
            if !mark.is_cancelled() || !before.is_cancelled() { return 1; }
            let after = parent.child_source();
            if !after.is_cancelled() { return 2; }
            match before.token().reason() {
                Maybe.Some(CancelReason.ParentCancelled) => 81,
                _ => 3,
            }
        }
    "#;
    assert_eq!(execute_from_archive(source), 81);
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive and loopback TCP"]
fn cancelled_accept_preserves_the_pending_tcp_connection() {
    let source = r#"
        module app;
        mount core.net.tcp.{TcpListener, TcpStream};
        mount core.net.addr.{Ipv4Addr, SocketAddrV4, SocketAddr};
        mount core.async.cancellation.CancellationToken;
        async fn main() -> Int {
            let listener = TcpListener.bind(SocketAddr.V4(SocketAddrV4.new(Ipv4Addr.localhost(), 0))).unwrap();
            let bound = listener.local_addr().unwrap();
            let client = TcpStream.connect(bound).unwrap();
            let token = CancellationToken.new();
            token.cancel();
            let refused = listener.accept_cancellable(&token).await;
            if refused.is_ok() { return 1; }
            let (server, _) = listener.accept_async().await.unwrap();
            if client.peer_addr().unwrap().port() != bound.port() { return 2; }
            if server.peer_addr().unwrap().port() != client.local_addr().unwrap().port() { return 3; }
            83
        }
    "#;
    assert_eq!(execute_from_archive(source), 83);
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn sqlite_snapshot_checks_execute_from_the_published_archive() {
    let source = r#"
        module app;
        mount core.database.sqlite.native.l1_pager.snapshot.{SnapshotError, snapshot_new, ensure_fresh};
        fn main() -> Int {
            let snapshot = snapshot_new(11, 0, 0);
            let fresh = match ensure_fresh(&snapshot, 11) {
                Result.Ok(_) => 5,
                Result.Err(_) => 0,
            };
            let stale = match ensure_fresh(&snapshot, 19) {
                Result.Err(error) => match error {
                    SnapshotError.Stale(now, old) => now - old,
                    _ => 0,
                },
                Result.Ok(_) => 0,
            };
            fresh + stale
        }
    "#;
    assert_eq!(execute_from_archive(source), 13);
}

#[cfg(unix)]
#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn shell_child_polls_both_pipes_and_consumes_its_exit_status() {
    let source = r#"
        module app;
        mount core.io.process.{Command, Child, ExitStatus, Stdio};
        fn main() -> Int {
            let mut command = Command.new("/bin/sh".into());
            command.arg("-c".into());
            command.arg("printf A; printf B >&2; exit 7".into());
            command.stdout(Stdio.Piped);
            command.stderr(Stdio.Piped);
            let mut child = command.spawn().unwrap();
            let mut output = 0;
            let mut errors = 0;
            let mut code = -1;
            let mut polls = 0;
            while polls < 100000 {
                polls = polls + 1;
                if code < 0 {
                    if let Maybe.Some(status) = child.try_wait().unwrap() { code = status.code(); }
                }
                if let Maybe.Some(bytes) = child.read_stdout_ready(16).unwrap() {
                    for byte in bytes { output = output * 256 + byte as Int; }
                }
                if let Maybe.Some(bytes) = child.read_stderr_ready(16).unwrap() {
                    for byte in bytes { errors = errors * 256 + byte as Int; }
                }
                if code >= 0 && child.output_is_closed() {
                    return code * 65536 + output * 256 + errors;
                }
            }
            child.close_output();
            if code < 0 { let _ = child.kill(); let _ = child.wait(); }
            -1
        }
    "#;
    assert_eq!(execute_from_archive(source), 7 * 65536 + 65 * 256 + 66);
}

fn execute_from_archive(source: &str) -> i64 {
    let archive =
        PathBuf::from(std::env::var_os("VERUM_TEST_FROZEN_STDLIB").expect("candidate .vbca"));
    let archive = read_archive_from_file(archive).unwrap();
    let app = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new().apply_lazy_with_types(&archive, &mut codegen, &app, &[]);
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let main = module.find_function_by_name("app.main").unwrap();
    let mut vm = Interpreter::new(Arc::new(module));
    vm.run_global_ctors().unwrap();
    vm.execute_function(main).unwrap().try_as_i64().expect("integer result")
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn composed_simplex_actions_execute_from_the_published_archive() {
    let source = r#"
        module app;
        mount core.math.simplicial.{Simplex, simplex_standard};
        mount core.math.infinity_category.{FunctorAction, compose_functors, identity_functor, functor_action};
        mount core.math.examples.{PointSimplicialSet, IdentityInfinityFunctor};

        fn main() -> Int {
            let s = simplex_standard(2);
            let add: FunctorAction<PointSimplicialSet, PointSimplicialSet> =
                FunctorAction.from_map(|x: &Simplex| simplex_standard(x.dimension() + 1));
            let twice: FunctorAction<PointSimplicialSet, PointSimplicialSet> =
                FunctorAction.from_map(|x: &Simplex| simplex_standard(x.dimension() * 2));
            let lifted = functor_action<PointSimplicialSet, PointSimplicialSet, IdentityInfinityFunctor>();
            assert(lifted.preserves_faces(&s, 0));
            assert(lifted.preserves_degeneracies(&s, 0));
            assert(!add.preserves_faces(&s, 0));
            assert(!add.preserves_degeneracies(&s, 0));
            let identity = identity_functor<PointSimplicialSet>();
            assert_eq(identity.apply(&s).dimension(), 2);
            compose_functors(twice, add).apply(&s).dimension()
        }
    "#;
    assert_eq!(execute_from_archive(source), 6);
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn archived_function_ids_are_unique_within_each_module() {
    let path = std::env::var_os("VERUM_TEST_FROZEN_STDLIB").expect("candidate .vbca");
    let archive = read_archive_from_file(path).unwrap();
    assert!(!archive.index.is_empty());
    for (index, entry) in archive.index.iter().enumerate() {
        let module = archive.load_module_by_index(index).unwrap();
        let mut owners = std::collections::HashMap::with_capacity(module.functions.len());
        for function in &module.functions {
            let name = module.get_string(function.name).expect("function name");
            if let Some(previous) = owners.insert(function.id, name) {
                panic!("{} assigns {:?} to both {previous} and {name}", entry.name, function.id);
            }
            let end = u64::from(function.bytecode_offset) + u64::from(function.bytecode_length);
            assert!(end <= module.bytecode.len() as u64, "{} has an invalid body range for {name}", entry.name);
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn darwin_path_slices_refuse_truncation_before_opening_a_file() {
    let source = r#"
        module app;
        mount core.sys.darwin.libsystem.{safe_open, safe_close, O_RDONLY};
        fn main() -> Int {
            let embedded: [Byte; 3] = [47, 0, 97];
            let overlong: [Byte; 4096] = [97 as Byte; 4096];
            let nul_terminated: [Byte; 2] = [47, 0];
            assert_eq(safe_open(&embedded[..], O_RDONLY, 0).unwrap_err().code, 22);
            assert_eq(safe_open(&overlong[..], O_RDONLY, 0).unwrap_err().code, 63);
            safe_close(safe_open(&nul_terminated[..], O_RDONLY, 0).unwrap()).unwrap();
            1
        }
    "#;
    assert_eq!(execute_from_archive(source), 1);
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn watcher_reports_the_registered_file_and_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("observed file");
    std::fs::write(&file, []).unwrap();
    let source = r#"
        module app;
        mount core.sys.fs_watch.{FsWatcher, FsEventKind};
        mount core.sys.darwin.libsystem.{safe_open, safe_close, safe_ftruncate, O_RDWR};
        fn main() -> Int {
            let path: Text = FILE_NAME;
            let mut watcher = FsWatcher.new().unwrap();
            watcher.add(&path).unwrap();
            assert_eq(watcher.wait(0).unwrap().len(), 0);
            let writer = safe_open(path.as_bytes(), O_RDWR, 0).unwrap();
            safe_ftruncate(writer, 7).unwrap();
            let events = watcher.wait(1000).unwrap();
            assert_eq(events.len(), 1);
            assert_eq(events[0].path, path);
            match events[0].kind { FsEventKind.Modified => {}, _ => panic("expected modification") }
            watcher.remove(&path).unwrap();
            safe_ftruncate(writer, 15).unwrap();
            safe_close(writer).unwrap();
            watcher.wait(0).unwrap().len()
        }
    "#.replace("FILE_NAME", &format!("{:?}", file.to_str().unwrap()));
    assert_eq!(execute_from_archive(&source), 0);
    assert_eq!(std::fs::metadata(file).unwrap().len(), 15);
}
