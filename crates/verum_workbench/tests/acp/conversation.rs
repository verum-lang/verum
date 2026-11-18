use super::*;
use crate::{Config, Studio};
use std::fs;

struct Fixture {
    _project: tempfile::TempDir,
    home: tempfile::TempDir,
    studio: Studio,
    id: Uuid,
}

impl Fixture {
    async fn new(limit: Duration) -> Self {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("private.vr"),
            "distinct workspace bytes",
        )
        .unwrap();
        let executable = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/acp/provider.py");
        let studio = Studio::open_with_providers(
            Config {
                workspace: project.path().into(),
                verum: "unused".into(),
                proof: "unused".into(),
                codex: "unused".into(),
                port: 0,
            },
            None,
            executable.clone(),
        )
        .unwrap();
        let agent = Acp::connect_in(&executable, home.path(), limit)
            .await
            .unwrap();
        let id = agent.events.lock().await.connection();
        *studio.agy.lock().await = Some(Arc::new(agent));
        Self {
            _project: project,
            home,
            studio,
            id,
        }
    }
    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        self.studio
            .call(
                "agent.request",
                json!({"method":method,"connectionId":self.id,"params":params}),
            )
            .await
    }
    async fn login(&self) {
        let initial = self
            .studio
            .call("agent.connect", json!({"provider":"agy"}))
            .await
            .unwrap();
        assert_eq!(initial["connectionId"], json!(self.id));
        assert!(initial["account"].is_null());
        assert!(self.rpc("thread/start", json!({})).await.is_err());
        self.studio
            .call("agent.login", json!({"connectionId":self.id}))
            .await
            .unwrap();
        timeout(Duration::from_secs(3), async {
            while self.rpc("account/read", json!({})).await.unwrap()["account"].is_null() {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
    }
    async fn thread(&self, role: &str) -> Text {
        self.studio
            .call(
                "agent.request",
                json!({"method":"thread/start","role":role,"connectionId":self.id,"params":{}}),
            )
            .await
            .unwrap()["thread"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    async fn prompt(&self, thread: &str, text: &str) -> Result<Value> {
        self.rpc(
            "turn/start",
            json!({"threadId":thread,"input":[{"type":"text","text":text}]}),
        )
        .await
    }
    async fn events(&self, thread: &str) -> Value {
        self.studio
            .call(
                "agent.events",
                json!({"connectionId":self.id,"threadId":thread}),
            )
            .await
            .unwrap()
    }
    async fn terminal(&self, thread: &str, turn: &str) -> Value {
        timeout(Duration::from_secs(5), async {
            loop {
                let page = self.events(thread).await;
                if let Some(end) = page["events"].as_array().unwrap().iter().find(|e| {
                    e["message"]["method"] == "turn/completed"
                        && e["message"]["params"]["turn"]["id"] == turn
                }) {
                    return end["message"]["params"]["turn"].clone();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn scoped_text_turns_refuse_provider_tools_and_complete_in_wire_order() {
    let f = Fixture::new(TURN_TIMEOUT).await;
    f.login().await;
    let a = f.thread("author").await;
    let b = f.thread("reviewer").await;
    let first = f.prompt(&a, "hold").await.unwrap();
    assert!(f.prompt(&a, "busy").await.is_err());
    assert!(
        f.rpc(
            "turn/start",
            json!({"threadId":a,"input":[{"type":"resource_link","uri":"file:///private.vr"}]})
        )
        .await
        .is_err()
    );
    assert!(f.studio.call("agent.request",json!({"connectionId":Uuid::new_v4(),"method":"turn/start","params":{"threadId":a,"input":[{"type":"text","text":"stale"}]}})).await.is_err());
    let second = f.prompt(&b, "interleave").await.unwrap();
    for (thread, reply, text) in [
        (&a, first, "first author"),
        (&b, second, "reviewer complete"),
    ] {
        let turn = reply["turn"]["id"].as_str().unwrap();
        assert_eq!(f.terminal(thread, turn).await["status"], "completed");
        let page = f.events(thread).await;
        let all = page["events"].as_array().unwrap();
        assert!(!all.iter().any(|e| e.get("reviewId").is_some()));
        let completed: List<_> = all
            .iter()
            .filter(|e| e["message"]["method"] == "item/completed")
            .collect();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0]["message"]["params"]["item"]["text"], text);
        assert_eq!(
            all.iter()
                .filter(|e| e["message"]["method"] == "turn/completed")
                .count(),
            1
        );
    }
    let command = f.prompt(&a, "  /plugins").await.unwrap_err();
    assert!(command.to_string().contains("slash commands"));
    let exact = " π ≤ λ\r\n  ";
    let sent = f.prompt(&a, exact).await.unwrap();
    let turn = sent["turn"]["id"].as_str().unwrap();
    f.terminal(&a, turn).await;
    assert!(
        f.events(&a).await["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["message"]["params"]["turnId"] == turn
                && e["message"]["params"]["item"]["text"] == exact)
    );
}

#[tokio::test]
async fn interruption_failure_and_disconnect_never_complete_a_message() {
    let f = Fixture::new(TURN_TIMEOUT).await;
    f.login().await;
    let a = f.thread("reviewer").await;
    let pending = f.prompt(&a, "hold").await.unwrap();
    let id = pending["turn"]["id"].as_str().unwrap();
    assert!(
        f.rpc(
            "turn/interrupt",
            json!({"threadId":a,"turnId":Uuid::new_v4()})
        )
        .await
        .is_err()
    );
    f.rpc("turn/interrupt", json!({"threadId":a,"turnId":id}))
        .await
        .unwrap();
    assert_eq!(f.terminal(&a, id).await["status"], "interrupted");
    let fail = f.prompt(&a, "fail").await.unwrap();
    let failure = f.terminal(&a, fail["turn"]["id"].as_str().unwrap()).await;
    assert_eq!(failure["status"], "failed");
    assert!(
        failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains("-32603")
    );
    let expired = f.prompt(&a, "expired").await.unwrap();
    assert_eq!(
        f.terminal(&a, expired["turn"]["id"].as_str().unwrap())
            .await["status"],
        "failed"
    );
    assert!(f.rpc("account/read", json!({})).await.unwrap()["account"].is_null());
    f.login().await;
    let lost = f.prompt(&a, "disconnect").await.unwrap();
    let terminal = f.terminal(&a, lost["turn"]["id"].as_str().unwrap()).await;
    assert!(
        terminal["error"]["message"]
            .as_str()
            .unwrap()
            .contains("disconnected")
    );
    assert!(f.studio.connection_events(f.id).await.is_err());
    let page = f.events(&a).await.to_string();
    assert!(!page.contains("late text after cancellation"));
    assert!(!page.contains("item/completed"));
    assert!(!page.contains("untrusted provider detail"));
}

#[tokio::test]
async fn deadlines_and_unexpected_native_activity_retire_the_instance() {
    for (message, reason) in [("wait", "timed out"), ("native", "native tool activity")] {
        let f = Fixture::new(Duration::from_millis(200)).await;
        f.login().await;
        let thread = f.thread("author").await;
        let running = f.prompt(&thread, message).await.unwrap();
        let terminal = f
            .terminal(&thread, running["turn"]["id"].as_str().unwrap())
            .await;
        assert_eq!(terminal["status"], "failed");
        assert!(
            terminal["error"]["message"]
                .as_str()
                .unwrap()
                .contains(reason),
            "{terminal}"
        );
        assert!(f.studio.connection_events(f.id).await.is_err());
        assert!(f.prompt(&thread, "retired").await.is_err());
    }
}

#[tokio::test]
async fn changed_private_tool_policy_refuses_the_next_prompt() {
    let f = Fixture::new(TURN_TIMEOUT).await;
    f.login().await;
    let thread = f.thread("author").await;
    let first = f.prompt(&thread, "seen").await.unwrap();
    assert_eq!(
        f.terminal(&thread, first["turn"]["id"].as_str().unwrap())
            .await["status"],
        "completed"
    );
    let file = f.home.path().join(".gemini/antigravity-cli/settings.json");
    let mut settings: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    settings["permissions"]["allow"] = json!(["command(*)"]);
    fs::write(&file, serde_json::to_vec(&settings).unwrap()).unwrap();
    let before = f.events(&thread).await["cursor"].clone();
    assert!(f.prompt(&thread, "must stay with the host").await.is_err());
    assert_eq!(f.events(&thread).await["cursor"], before);
}

#[tokio::test]
async fn reconnect_checks_cached_oauth_and_consumes_the_prepared_session_once() {
    let f = Fixture::new(TURN_TIMEOUT).await;
    f.login().await;
    let executable = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/acp/provider.py");
    let resumed = Acp::connect_with_storage(&executable, f.home.path())
        .await
        .unwrap();
    assert!(resumed.account().await.unwrap()["account"].is_object());
    let role = json!("reviewer");
    let first = resumed
        .request("thread/start", &json!({}), Some(&role))
        .await
        .unwrap();
    let second = resumed
        .request("thread/start", &json!({}), Some(&role))
        .await
        .unwrap();
    assert_eq!(first["thread"]["id"], "s1");
    assert_eq!(second["thread"]["id"], "s2");
    assert_ne!(resumed.events.lock().await.connection(), f.id);
}
