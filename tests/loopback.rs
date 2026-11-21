//! The harness-free test executable also serves as its child SDK worker. Only
//! this fixture injects a loopback endpoint. Production always uses Google.
#[allow(dead_code, unused_imports)]
#[path = "../src-rust/main.rs"]
mod port;
#[allow(dead_code)]
mod support;

use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use support::{clipboard, Peer, GRANTS};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::Command,
};

type Log = Arc<Mutex<Vec<(String, String)>>>;

async fn respond(socket: TcpStream, log: &Log) {
    let mut io = BufReader::new(socket);
    let mut line = String::new();
    io.read_line(&mut line).await.unwrap();
    assert!(line.len() < 2100, "long URI passed to service");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap().to_string();
    let url = reqwest::Url::parse(&format!("http://localhost{}", parts.next().unwrap())).unwrap();
    let mut length = 0;
    let mut content_type = String::new();
    let mut agent = String::new();
    loop {
        line.clear();
        assert!(
            io.read_line(&mut line).await.unwrap() > 0,
            "incomplete headers"
        );
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').unwrap();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().unwrap(),
            "content-type" => content_type = value.trim().to_string(),
            "user-agent" => agent = value.trim().to_string(),
            _ => {}
        }
    }
    assert_eq!(
        agent,
        concat!("Tezbar Google Translate/", env!("CARGO_PKG_VERSION"))
    );
    let param = |key: &str| {
        url.query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, v)| v.into_owned())
    };
    assert_eq!(param("client").as_deref(), Some("dict-chrome-ex"));
    let target = param("tl").unwrap();
    let q = if method == "POST" {
        assert!(param("q").is_none());
        assert_eq!(
            content_type,
            "application/x-www-form-urlencoded;charset=UTF-8"
        );
        let mut body = vec![0; length];
        io.read_exact(&mut body).await.unwrap();
        let mut form = reqwest::Url::parse("http://localhost").unwrap();
        form.set_query(Some(std::str::from_utf8(&body).unwrap()));
        let pairs: Vec<_> = form.query_pairs().collect();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "q");
        pairs[0].1.to_string()
    } else {
        assert_eq!(method, "GET");
        param("q").unwrap()
    };
    log.lock().unwrap().push((method, q.clone()));
    let body = match q.as_str() {
        "malformed" => "[]".into(),
        "empty segments" => "[[]]".into(),
        "empty body" => String::new(),
        "missing language" => r#"[[["text"]]]"#.into(),
        "oversized" => "x".repeat(1024 * 1024 + 1),
        "interrupted" => "[".into(),
        _ => {
            let mut segments = vec![json!([format!("result {target}"), q, null])];
            if target == "de" {
                segments.push(json!([null, null, null, "source romanization"]));
            }
            json!([segments, null, "fr"]).to_string()
        }
    };
    let advertised = body.len() + usize::from(q == "interrupted");
    io.write_all(
        format!("HTTP/1.1 200 OK\r\nContent-Length: {advertised}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    io.write_all(body.as_bytes()).await.unwrap();
}

async fn peer(port: u16, selection: &'static str) -> Peer {
    let mut worker = Command::new(std::env::current_exe().unwrap());
    worker
        .env("TEZBAR_FIXTURE_PORT", port.to_string())
        .env("NO_PROXY", "*")
        .env("no_proxy", "*");
    Peer::spawn(worker, GRANTS, clipboard(selection)).await
}

async fn invoke(peer: &mut Peer, session: &str, id: &Value) -> Value {
    peer.call(session, "extension:invoke-action", json!({"actionId":id}))
        .await
        .unwrap()
}

async fn rows(peer: &mut Peer) {
    let mut reply = peer.request("session.open", json!({"sessionId":"rows","commandName":"quick-translate-view",
        "searchText":"wrong","launchContext":{"text":"forwarded"},"preferences":{"lang1":"en","lang2":"de"}})).await.unwrap();
    let rows = reply["root"]["children"].as_array().unwrap();
    assert_eq!(reply["root"]["type"], "List");
    assert_eq!(rows.len(), 2);
    for (i, text) in ["result en", "result de"].iter().enumerate() {
        assert_eq!(rows[i]["type"], "List.Item");
        let detail = &rows[i]["children"][0];
        assert_eq!(detail["type"], "List.Item.Detail");
        assert!(detail.get("children").is_none());
        assert!(detail["props"]["markdown"]
            .as_str()
            .unwrap()
            .starts_with(*text));
    }
    assert!(rows[1]["children"][0]["props"]["markdown"]
        .as_str()
        .unwrap()
        .contains("source romanization"));
    assert!(!rows[0]["props"]["actionIds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().unwrap().ends_with(":pronunciation")));
    let ids = rows[1]["props"]["actionIds"].as_array().unwrap();
    for id in ids {
        assert!(reply["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == *id));
    }
    for action in ["previous", "next", "toggle", "swap", "languages", "manage"] {
        assert!(ids.contains(&json!(format!("translate:{action}"))));
    }
    let copy = ids[0].clone();
    assert!(copy.as_str().unwrap().ends_with(":copy"));
    reply = invoke(peer, "rows", &copy).await;
    let pronunciation = reply["root"]["children"][1]["props"]["actionIds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v.as_str().unwrap().ends_with(":pronunciation"))
        .unwrap();
    let reply = invoke(peer, "rows", pronunciation).await;
    let language = reply["root"]["children"][1]["props"]["actionIds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| *v == "translate:languages")
        .unwrap();
    assert_eq!(invoke(peer, "rows", language).await["root"]["type"], "Form");
    assert_eq!(
        peer.methods(),
        ["host.clipboard.write", "host.clipboard.write"]
    );
    assert_eq!(peer.calls[0].1["text"], "result de");
    assert_eq!(peer.calls[1].1["text"], "source romanization");
}

async fn submit(peer: &mut Peer, text: &str) -> Value {
    peer.call("form","extension:invoke-action",json!({"actionId":"translate:submit","formValues":{"text":text,"language_from":"auto","language_to":"en"}})).await.unwrap()
}

async fn failures(peer: &mut Peer) {
    peer.open("form", "translate-form", json!({"autoInput":false}))
        .await
        .unwrap();
    let count = peer.calls.len();
    for (input, error) in [
        ("malformed", "missing segments"),
        ("empty segments", "Translation not found"),
        ("empty body", "invalid JSON"),
        ("missing language", "no detected language"),
        ("oversized", "1 MiB"),
        ("interrupted", "interrupted"),
    ] {
        let good = submit(peer, "good").await;
        let old = good["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["title"] == "Copy Translation")
            .unwrap()["id"]
            .clone();
        let failed = submit(peer, input).await;
        assert_eq!(failed["root"]["children"][0]["props"]["value"], input);
        assert!(failed["root"]["children"][4]["props"]["text"]
            .as_str()
            .unwrap()
            .contains(error));
        let stale = peer
            .call("form", "extension:invoke-action", json!({"actionId":old}))
            .await
            .unwrap_err();
        assert!(stale["message"].as_str().unwrap().contains("stale"));
        let refresh = peer
            .call("form", "extension:refresh-session", json!({}))
            .await
            .unwrap();
        for view in [failed, refresh] {
            assert!(view["root"]["children"][4]["props"]["text"]
                .as_str()
                .unwrap()
                .contains(error));
            assert!(view["actions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|a| !a["id"].as_str().unwrap().ends_with(":copy")));
        }
        assert_eq!(peer.calls.len(), count);
    }
}

async fn instant(port: u16, log: &Log, selection: &'static str) {
    let mut p = peer(port, selection).await;
    for cmd in [
        "instant-translate-copy",
        "instant-translate-paste",
        "instant-translate-view",
    ] {
        let before = log.lock().unwrap().len();
        let calls = p.calls.len();
        let reply = p.request("session.open",json!({"sessionId":cmd,"commandName":cmd,"searchText":"unrelated","preferences":{"autoInput":false,"lang1":"en"}})).await;
        assert_eq!(p.calls[calls].0, "host.selection.read");
        if selection.is_empty() {
            assert!(reply.unwrap_err()["message"]
                .as_str()
                .unwrap()
                .contains("No text selected"));
            assert_eq!(log.lock().unwrap().len(), before);
            assert_eq!(p.calls.len(), calls + 1);
        } else {
            let reply = reply.unwrap();
            assert_eq!(
                &log.lock().unwrap()[before..],
                &[("GET".to_owned(), selection.to_owned())]
            );
            if cmd.ends_with("view") {
                assert_eq!(reply["root"]["props"]["markdown"], "result en");
                assert_eq!(p.calls.len(), calls + 1);
            } else {
                assert_eq!(p.calls.len(), calls + 2);
                assert_eq!(
                    p.calls[calls + 1].0,
                    if cmd.ends_with("copy") {
                        "host.clipboard.write"
                    } else {
                        "host.clipboard.pasteText"
                    }
                );
                assert_eq!(p.calls[calls + 1].1["text"], "result en");
            }
        }
    }
    p.shutdown().await;
}

#[tokio::main(worker_threads = 2)]
async fn main() {
    if let Ok(value) = std::env::var("TEZBAR_FIXTURE_PORT") {
        let port: u16 = value.parse().unwrap();
        tezbar_extension_sdk::serve(port::Adapter {
            endpoint: format!("http://127.0.0.1:{port}/translate_a/single"),
        })
        .await
        .unwrap();
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Log::default();
    let records = log.clone();
    let server = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            respond(socket, &records).await;
        }
    });
    let mut p = peer(port, "wrong selection").await;
    rows(&mut p).await;
    assert_eq!(
        *log.lock().unwrap(),
        vec![("GET".to_owned(), "forwarded".to_owned()); 2]
    );
    failures(&mut p).await;
    let base = format!("http://127.0.0.1:{port}/translate_a/single?client=dict-chrome-ex&sl=auto&tl=en&hl=en&dt=at&dt=bd&dt=ex&dt=ld&dt=md&dt=qca&dt=rw&dt=rm&dt=ss&dt=t&ie=UTF-8&oe=UTF-8&otf=1&ssel=0&tsel=0&kc=7&q=");
    for (encoded_len, method) in [(2048, "GET"), (2049, "POST")] {
        let text = "a".repeat(encoded_len - base.len());
        let reply = submit(&mut p, &text).await;
        assert_eq!(reply["root"]["children"][4]["props"]["text"], "result en");
        assert_eq!(
            log.lock().unwrap().last(),
            Some(&(method.to_string(), text))
        );
    }
    let long = "界 +&".repeat(400);
    let result = submit(&mut p, &long).await;
    assert_eq!(result["root"]["children"][4]["props"]["text"], "result en");
    assert_eq!(log.lock().unwrap().last(), Some(&("POST".to_owned(), long)));
    p.shutdown().await;
    instant(port, &log, "selection").await;
    instant(port, &log, "").await;
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
