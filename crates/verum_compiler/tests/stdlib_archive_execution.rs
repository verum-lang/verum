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
fn a_list_owns_its_transferred_cancellation_registration() {
    let source = r#"
        module app;
        mount core.async.cancellation.{CancellationToken, Registration};
        mount core.sync.atomic.{AtomicInt, MemoryOrdering};
        static OBSERVED: AtomicInt = AtomicInt.new(0);
        fn retain(regs: &mut List<Registration>, token: &CancellationToken) {
            let reg = token.register(|| { let _ = OBSERVED.fetch_add(1, MemoryOrdering.AcqRel); });
            regs.push(reg);
        }
        fn main() -> Int {
            let token = CancellationToken.new();
            let mut regs: List<Registration> = [];
            retain(&mut regs, &token);
            token.cancel();
            token.cancel();
            assert_eq(OBSERVED.load(MemoryOrdering.Acquire), 1);
            73
        }
    "#;
    assert_eq!(execute_from_archive(source), 73);
}

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
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn explicit_and_propagated_returns_release_inferred_lock_guards() {
    let source = r#"
        module app;
        mount core.sync.rwlock.RwLock;
        mount core.sync.mutex.Mutex;
        fn early(lock: &RwLock<Int>, active: Bool) -> Bool {
            let guard = lock.write().unwrap();
            if !active { return false; }
            true
        }
        fn require_active(active: Bool) -> Result<Int, Int> {
            if active { Result.Ok(17) } else { Result.Err(7) }
        }
        fn guarded(lock: &Mutex<Int>, active: Bool) -> Result<Int, Int> {
            let guard = lock.lock().unwrap();
            let value = require_active(active)?;
            Result.Ok(value)
        }
        fn main() -> Int {
            let write = RwLock.new(17);
            let mutex = Mutex.new(25);
            assert(!early(&write, false), "expected early return");
            {
                let read = write.try_read().unwrap();
                assert_eq(*read, 17);
            }
            assert(guarded(&mutex, false).is_err(), "expected residual return");
            {
                let exclusive = mutex.try_lock().unwrap();
                assert_eq(*exclusive, 25);
            }
            guarded(&mutex, true).unwrap()
        }
    "#;
    assert_eq!(execute_from_archive(source), 17);
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

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn supervisor_acknowledges_each_child_generation_before_shutdown() {
    let source = r#"
        module app;
        mount core.runtime.supervisor.{SupervisorHandle, SupervisorConfig, ChildSpec, SupervisorError};
        mount core.async.future.Future;
        mount core.async.poll.Poll;
        mount core.async.task.yield_now;
        mount core.async.waker.Context;

        mount core.sync.atomic.{AtomicInt, MemoryOrdering};
        static POLLS: AtomicInt = AtomicInt.new(0);
        type Worker is { pending: Bool };
        implement Future for Worker {
            type Output = Int;
            fn poll(&mut self, cx: &mut Context) -> Poll<Int> {
                POLLS.fetch_add(1, MemoryOrdering.SeqCst);
                if self.pending {
                    self.pending = false;
                    cx.waker().wake_by_ref();
                    return Poll.Pending;
                }
                Poll.Ready(17)
            }
        }
        fn make_worker() -> Worker { Worker { pending: true } }

        async fn main() -> Int {
            let supervisor = SupervisorHandle.new(SupervisorConfig.one_for_one("workers"));
            let child = supervisor.supervise<Worker, Int>(make_worker, ChildSpec.temporary("test")).await.unwrap();
            yield_now().await;
            assert_eq(POLLS.load(MemoryOrdering.SeqCst), 2);
            let status = supervisor.status();
            assert_eq(status.child_count, 1);
            assert_eq(status.running_count, 0);
            supervisor.restart(child).await.unwrap();
            yield_now().await;
            assert_eq(POLLS.load(MemoryOrdering.SeqCst), 4);
            supervisor.terminate(child).await.unwrap();
            supervisor.terminate(child).await.unwrap();
            assert_eq(supervisor.status().running_count, 0);

            supervisor.shutdown().await.unwrap();
            supervisor.shutdown().await.unwrap();
            assert(!supervisor.status().is_running, "shutdown must close admission");
            match supervisor.supervise<Worker, Int>(make_worker, ChildSpec.temporary("late")).await {
                Result.Err(SupervisorError.ShuttingDown) => supervisor.status().child_count,
                _ => -1,
            }
        }
    "#;
    assert_eq!(execute_from_archive(source), 1);
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn owned_and_borrowed_smart_pointers_reach_their_payload() {
    let source = r#"
        module app;
        mount core.base.memory.{Heap, Shared};
        type Pair is { first: Int, second: Int };
        fn borrowed(owner: &Shared<Pair>) -> Int { owner.first * 100 + owner.second }
        fn owned(owner: Shared<Pair>) -> Int { owner.first * 100 + owner.second }
        fn borrowed_heap(owner: &Heap<Pair>) -> Int { owner.first * 100 + owner.second }
        fn owned_heap(owner: Heap<Pair>) -> Int { owner.first * 100 + owner.second }
        fn main() -> Int {
            let owner: Shared<Pair> = Shared.new(Pair { first: 13, second: 17 });
            assert_eq(borrowed(&owner), 1317);
            assert_eq(owned(owner), 1317);
            let heap: Heap<Pair> = Heap.new(Pair { first: 13, second: 17 });
            assert_eq(borrowed_heap(&heap), 1317);
            owned_heap(heap)
        }
    "#;
    assert_eq!(execute_from_archive(source), 1317);
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
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive and loopback TCP"]
fn an_empty_tcp_accept_can_be_polled_and_then_selected() {
    let source = r#"
        module app;
        mount core.net.tcp.{TcpListener, TcpStream};
        mount core.net.addr.{Ipv4Addr, SocketAddrV4, SocketAddr};
        mount core.async.cancellation.CancellationToken;
        mount core.async.future.Future;
        mount core.async.waker.{Context, noop_waker};

        async fn main() -> Int {
            let listener = TcpListener.bind(SocketAddr.V4(SocketAddrV4.new(Ipv4Addr.localhost(), 0))).unwrap();
            let bound = listener.local_addr().unwrap();
            let token = CancellationToken.new();
            let mut accepting = listener.accept_future();
            let mut cancelling = token.cancelled();
            let waker = noop_waker();
            let mut cx = Context.from_waker(&waker);
            assert(accepting.poll(&mut cx).is_pending(), "empty listener must yield");
            assert(cancelling.poll(&mut cx).is_pending(), "uncancelled token must yield");
            let client = TcpStream.connect(bound).unwrap();
            let accepted = select biased {
                result = accepting.await => result.unwrap(),
                _ = cancelling.await => panic("unexpected cancellation"),
            };
            assert(accepted.1.port() == client.local_addr().unwrap().port());
            token.cancel();
            assert(listener.accept_cancellable(&token).await.is_err());
            73
        }
    "#;
    assert_eq!(execute_from_archive(source), 73);
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

fn compile_from_archive(source: &str) -> verum_vbc::module::VbcModule {
    let archive =
        PathBuf::from(std::env::var_os("VERUM_TEST_FROZEN_STDLIB").expect("candidate .vbca"));
    let archive = read_archive_from_file(archive).unwrap();
    let app = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new(&archive).apply_lazy_with_types(&mut codegen, &app, &[]).expect("valid archive");
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], ItemFailurePolicy::Strict)
        .unwrap();
    codegen.finalize_module_from_state().unwrap()
}

fn execute_from_archive(source: &str) -> i64 {
    let module = compile_from_archive(source);
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

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn archived_select_preserves_both_channel_receiver_witnesses() {
    let source = r#"
        module app;
        mount core.async.channel.{Sender, Receiver, RecvFut, bounded};
        mount core.async.select.{Either, select_either};

        async fn choose(ready_left: Bool) -> Int {
            let (left_tx, mut left_rx): (Sender<Byte>, Receiver<Byte>) = bounded<Byte>(1);
            let (right_tx, mut right_rx): (Sender<Int>, Receiver<Int>) = bounded<Int>(1);
            if ready_left {
                left_tx.try_send(31 as Byte).unwrap();
            } else {
                right_tx.try_send(73).unwrap();
            }
            match select_either<Maybe<Byte>, Maybe<Int>, RecvFut<Byte>, RecvFut<Int>>(
                left_rx.recv_fut(), right_rx.recv_fut()
            ).await {
                Either.Left(Maybe.Some(value)) => value as Int,
                Either.Right(Maybe.Some(value)) => value,
                _ => -1,
            }
        }

        async fn main() -> Int {
            let first = choose(true).await;
            let second = choose(false).await;
            first * 100 + second
        }
    "#;
    assert_eq!(execute_from_archive(source), 3173);
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn pool_methods_pass_the_session_declaration_to_the_error_mapping_call() {
    use verum_vbc::{bytecode::decode_instructions, instruction::Instruction, types::{TypeId, TypeRef}};

    let source = r#"
        module app;
        mount core.database.common.error.DbError;
        mount core.database.common.pool_impl.{ConnectionPool, PoolConfig};
        type Failure is Database(DbError);
        type Reader is { pool: ConnectionPool<Int> };
        implement Reader {
            fn open(limits: PoolConfig) -> Result<Reader, Failure> {
                let factory = || -> Result<Int, DbError> { Result.Ok(17) };
                let pool = ConnectionPool.launch(limits, factory).map_err(|error| Failure.Database(error))?;
                Result.Ok(Reader { pool })
            }
            fn checkout(&self) -> Result<Int, Failure> {
                let mut loan = self.pool.acquire().map_err(|error| Failure.Database(error))?;
                Result.Ok(*loan.conn_mut())
            }
        }
    "#;
    let module = compile_from_archive(source);
    for (caller, container) in [("Reader.checkout", "PooledConnection"), ("Reader.open", "ConnectionPool")] {
        let function = module.functions.iter().find(|function|
            module.get_string(function.name).is_some_and(|name| name.ends_with(caller))
        ).unwrap();
        let start = function.bytecode_offset as usize;
        let code = decode_instructions(&module.bytecode[start..start + function.bytecode_length as usize]).unwrap();
        let witnesses = code.windows(2).find_map(|pair| match pair {
            [Instruction::SetCallWitness { type_args }, Instruction::CallM { method_id, .. }]
                if module.get_string(verum_vbc::types::StringId(*method_id)) == Some("Result.map_err") => Some(type_args),
            _ => None,
        }).expect("witnesses of the actual Result.map_err call");
        let TypeRef::Instantiated { base, args } = &witnesses[0] else {
            panic!("{container}<Int> expected, got {:?}", witnesses[0]);
        };
        let declared_name = |id| module.get_string(module.get_type(id).unwrap().name).unwrap()
            .rsplit('.').next().unwrap();
        assert_eq!(declared_name(*base), container);
        assert_eq!(args.as_slice(), &[TypeRef::Concrete(TypeId::INT)]);
        let [_, TypeRef::Concrete(error), TypeRef::Concrete(failure), TypeRef::Function { params, return_type, contexts }] =
            witnesses.as_slice() else { panic!("{caller}: incomplete mapping witnesses: {witnesses:?}") };
        assert_eq!(declared_name(*error), "DbError");
        assert_eq!(declared_name(*failure), "Failure");
        assert_eq!(params.as_slice(), &[TypeRef::Concrete(*error)]);
        assert_eq!(**return_type, TypeRef::Concrete(*failure));
        assert!(contexts.is_empty());
    }
}

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn a_generic_guard_extracted_by_match_is_destroyed_on_early_return() {
    let source = r#"
        module app;
        mount core.sync.mutex.Mutex;
        type Payload<T> is { item: T };
        type Slot<Marker, T> is { marker: Marker, gate: Mutex<Payload<T>> };
        implement<Marker, T> Slot<Marker, T> {
            fn checkout(&self) -> Bool {
                loop {
                    let guard = match self.gate.lock() {
                        Result.Ok(held) => held,
                        Result.Err(_) => { return false; }
                    };
                    return true;
                }
            }
        }
        fn main() -> Int {
            let slot: Slot<Bool, Int> = Slot { marker: false, gate: Mutex.new(Payload { item: 17 }) };
            if slot.checkout() && slot.gate.try_lock().is_ok() { 17 } else { 0 }
        }
    "#;
    assert_eq!(execute_from_archive(source), 17);
}
