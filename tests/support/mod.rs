use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};

const BIN: &str = env!("CARGO_BIN_EXE_tezbar-google-translate");
const ID: &str = "com.tezbar.native.google-translate";
pub const GRANTS: &[&str] = &[
    "selection.read",
    "clipboard.write",
    "clipboard.pasteText",
    "shell.open",
    "window.visibility",
];

type Reply = Box<dyn FnMut(&str, &Value) -> Result<Value, String>>;

pub struct Peer {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    pub calls: Vec<(String, Value)>,
    reply: Reply,
    sequence: u64,
    _dir: tempfile::TempDir,
}

pub fn clipboard(text: &'static str) -> Reply {
    Box::new(move |method, _| match method {
        "host.selection.read" => Ok(json!(text)),
        _ => Ok(Value::Null),
    })
}

impl Peer {
    pub async fn start(grants: &[&str], reply: Reply) -> Self {
        Self::spawn(Command::new(BIN), grants, reply).await
    }
    pub async fn spawn(mut command: Command, grants: &[&str], reply: Reply) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let output = BufReader::new(child.stdout.take().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let mut peer = Self {
            child,
            input,
            output,
            calls: vec![],
            reply,
            sequence: 1,
            _dir: dir,
        };
        let hello = peer.frame().await;
        assert_eq!(hello["type"], "hello");
        assert_eq!(hello["id"], ID);
        assert_eq!(hello["protocol"], 1);
        assert_eq!(hello["version"], env!("CARGO_PKG_VERSION"));
        let root = peer._dir.path().to_path_buf();
        let init = peer
            .request(
                "initialize",
                json!({"protocol":1,"extensionId":ID,
                    "platform":if cfg!(windows) {"Windows"} else {"macOS"},
                    "packageDir":env!("CARGO_MANIFEST_DIR"),
                    "dataDir":root.join("data"),"tempDir":root.join("temp"),
                    "grantedCapabilities":grants,"reason":"command","migrationPaths":{}}),
            )
            .await
            .unwrap();
        assert_eq!(init["ready"], true);
        peer
    }
    pub async fn send(&mut self, value: Value) {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        self.input
            .as_mut()
            .unwrap()
            .write_all(&bytes)
            .await
            .unwrap();
    }
    pub async fn frame(&mut self) -> Value {
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(10), self.output.read_line(&mut line))
            .await
            .expect("protocol timeout")
            .unwrap();
        assert!(n > 0, "unexpected EOF");
        assert!(line.ends_with('\n'), "LF framing");
        serde_json::from_str(&line).unwrap()
    }
    /// Sends a host request and services reverse RPCs until its response.
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = format!("h:00000000-0000-4000-8000-{:012x}", self.sequence);
        self.sequence += 1;
        self.send(json!({"type":"request","id":id,"method":method,"params":params}))
            .await;
        loop {
            let frame = self.frame().await;
            match frame["type"].as_str() {
                Some("response") if frame["id"] == id.as_str() => {
                    return match frame.get("error") {
                        Some(error) => Err(error.clone()),
                        None => Ok(frame["result"].clone()),
                    };
                }
                Some("request") => {
                    let method = frame["method"].as_str().unwrap().to_owned();
                    assert!(frame["id"].as_str().unwrap().starts_with("e:"));
                    assert!(method.starts_with("host."), "non-typed RPC {method}");
                    let params = frame["params"].clone();
                    let result = (self.reply)(&method, &params);
                    self.calls.push((method, params));
                    let reply = match result {
                        Ok(value) => json!({"type":"response","id":frame["id"],"result":value}),
                        Err(message) => json!({"type":"response","id":frame["id"],
                            "error":{"code":"host_error","message":message}}),
                    };
                    self.send(reply).await;
                }
                Some("event") => {}
                other => panic!("unexpected frame {other:?}: {frame}"),
            }
        }
    }
    pub async fn open(
        &mut self,
        session: &str,
        command: &str,
        prefs: Value,
    ) -> Result<Value, Value> {
        self.request(
            "session.open",
            json!({"sessionId":session,"commandName":command,"preferences":prefs,"argumentValues":{}}),
        )
        .await
    }
    pub async fn call(
        &mut self,
        session: &str,
        operation: &str,
        payload: Value,
    ) -> Result<Value, Value> {
        self.request(
            "session.call",
            json!({"sessionId":session,"operation":operation,"payload":payload}),
        )
        .await
    }
    pub fn methods(&self) -> Vec<&str> {
        self.calls.iter().map(|(m, _)| m.as_str()).collect()
    }
    pub async fn shutdown(mut self) {
        self.request("shutdown", json!({})).await.unwrap();
        drop(self.input.take());
        let status = tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success());
    }
}
