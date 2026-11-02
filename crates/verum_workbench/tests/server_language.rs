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
    let request = |source: &str| {
        json!({
            "method":"language.request", "params":{"path":"module.vr", "source": source}
        })
    };
    let initial = request(
        "mount helper.{answer};\ncontext Logger { fn log(message: Text); }\nfn report(value: Int) using [Logger] { print(value); }\nfn result() -> Int { answer() }",
    );

    let (status, denied) = post(host, None, &initial);
    assert_eq!(status, 403);
    assert!(denied["error"].is_string() && denied.get("result").is_none());

    let (status, body) = post(host, Some(token), &initial);
    assert_eq!(status, 200);
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
    let source = concat!(
        "public type Witness<A> is { public value: A };\n",
        "public type Pack is { public carrier: Type, public witness: Witness<carrier> };\n",
        "implement Pack {\n",
        "    fn replace(&mut self, y: Pack) { *self = y; }\n",
        "    fn inspect(&self, y: Pack) {}\n",
        "}\n",
        "fn edit(x: Pack, y: Pack) -> Pack {\n",
        "    /* 🧮 */ let w = x.witness;\n",
        "    /* 🔬 λ */ x.replace(y);\n",
        "    Pack { carrier: x.carrier, witness: w }\n",
        "}\n",
    );
    let (status, stale) = post(host, Some(token), &request(source));
    assert_eq!(status, 200);
    let snapshot = &stale["result"]["result"];
    let diagnostics = snapshot["diagnostics"].as_array().unwrap();
    let diagnostic = diagnostics
        .iter()
        .find(|d| {
            d["severity"] == 1
                && d["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("cannot mutate a receiver"))
        })
        .unwrap_or_else(|| panic!("missing receiver diagnostic: {diagnostics:?}"));
    let range = |line: usize, token: &str| {
        let prefix = source
            .lines()
            .nth(line)
            .unwrap()
            .split_once(token)
            .unwrap()
            .0;
        let start = prefix.encode_utf16().count();
        json!({"start":{"line":line,"character":start},
               "end":{"line":line,"character":start + token.encode_utf16().count()}})
    };
    assert_eq!(diagnostic["range"], range(8, "x"));
    let related = diagnostic["relatedInformation"].as_array().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0]["location"]["uri"], snapshot["uri"]);
    assert_eq!(related[0]["location"]["range"], range(7, "x.witness"));

    let (status, repaired) = post(
        host,
        Some(token),
        &request(&source.replace("x.replace(y)", "x.inspect(y)")),
    );
    assert_eq!(status, 200);
    let updated = &repaired["result"]["result"];
    assert_eq!(updated["uri"], snapshot["uri"]);
    assert_eq!(updated["parsed"], true);
    assert_ne!(updated["source_hash"], snapshot["source_hash"]);
    assert_eq!(updated["diagnostics"], json!([]));
}

fn post(host: &str, token: Option<&str>, body: &Value) -> (u16, Value) {
    let mut stream = TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let body = body.to_string();
    write!(stream, "POST /api HTTP/1.1\r\nHost: {host}\r\n").unwrap();
    if let Some(token) = token {
        write!(stream, "Authorization: Bearer {token}\r\n").unwrap();
    }
    write!(
        stream,
        "Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (header, body) = response.split_once("\r\n\r\n").unwrap();
    let status = header.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(body).unwrap())
}
