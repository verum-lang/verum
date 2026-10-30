use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn standalone_server_analyzes_without_cargos_stack_environment() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("helper.vr"),
        "public fn answer() -> Int { 42 }",
    )
    .unwrap();
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_verum-studio"))
            .arg("--workspace")
            .arg(root.path())
            .env_remove("RUST_MIN_STACK")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut output = BufReader::new(server.0.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    let url = line.trim().strip_prefix("Verum Studio: http://").unwrap();
    let (host, token) = url.split_once("/#").unwrap();
    let mut stream = TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let body = json!({
        "method":"language.request", "params":{
            "path":"module.vr",
            "source":"mount helper.{answer};\ncontext Logger { fn log(message: Text); }\nfn report(value: Int) using [Logger] { print(value); }\nfn result() -> Int { answer() }"
        }
    }).to_string();
    write!(stream, "POST /api HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    let body: Value = serde_json::from_str(body).unwrap();
    assert!(body.get("error").is_none(), "{body}");
    assert_eq!(body["result"]["result"]["parsed"], true);
    assert_eq!(body["result"]["result"]["bindings"][0]["name"], "answer");
    assert!(
        body["result"]["result"]["project_revision"]
            .as_str()
            .is_some_and(|revision| revision.len() == 64)
    );
    assert_eq!(body["result"]["result"]["diagnostics"], json!([]));
    assert_eq!(
        body["result"]["result"]["declarations"][1]["requirements"][0]["source"],
        "Logger"
    );
}
