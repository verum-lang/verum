//! Release-gate checks against the chosen serialized stdlib and its bytecode.
use std::{path::PathBuf, sync::Arc};
use verum_compiler::archive_ctx_loader::ArchiveCtxCache;
use verum_fast_parser::Parser;
use verum_vbc::{
    archive::read_archive_from_file,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

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
    let archive =
        PathBuf::from(std::env::var_os("VERUM_TEST_FROZEN_STDLIB").expect("candidate .vbca"));
    let archive = read_archive_from_file(archive).unwrap();
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
    assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(13));
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
