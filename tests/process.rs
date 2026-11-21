//! Drives the real `tezbar-google-translate` executable over stdio JSONL pipes. The test
//! plays the Tezbar host: it answers typed `host.*` reverse RPCs from a script
//! and records them. No real clipboard, paste, browser or window is touched.
mod support;
use serde_json::json;
use support::{clipboard, Peer, GRANTS};
#[tokio::test]
async fn real_protocol_views_and_form() {
    let mut peer = Peer::start(GRANTS, clipboard("")).await;
    for command in [
        "translate",
        "quick-translate",
        "quick-translate-view",
        "translate-form",
    ] {
        let v = peer
            .open(command, command, json!({"autoInput":false}))
            .await
            .unwrap();
        assert_eq!(v["mode"], "view");
        assert_eq!(
            v["root"]["type"],
            if command == "translate-form" {
                "Form"
            } else {
                "List"
            }
        );
        if command != "translate-form" {
            let edited = peer
                .call(
                    command,
                    "extension:invoke-action",
                    json!({"actionId":"translate:languages"}),
                )
                .await
                .unwrap();
            assert_eq!(edited["root"]["type"], "Form");
            let error=peer.call(command,"extension:invoke-action",json!({"actionId":"translate:submit","formValues":{"text":"hello","language_from":"bogus","language_to":"en"}})).await.unwrap();
            assert!(error["root"]["children"][4]["props"]["text"]
                .as_str()
                .unwrap()
                .contains("supported"));
        }
        peer.request("session.dispose", json!({"sessionId":command}))
            .await
            .unwrap();
    }
    assert!(peer.calls.is_empty());
    peer.shutdown().await;
}

#[tokio::test]
async fn quick_results_preserve_explicit_empty_context_without_reading_selection() {
    let mut peer = Peer::start(GRANTS, clipboard("selection must stay private")).await;
    let reply = peer
        .request(
            "session.open",
            json!({
                "sessionId":"forwarded-empty", "commandName":"quick-translate-view",
                "preferences":{"autoInput":true}, "argumentValues":{},
                "searchText":"stale initiating search",
                "launchContext":{"text":"","requestId":"fresh-empty"}
            }),
        )
        .await
        .unwrap();
    assert_eq!(reply["root"]["type"], "List");
    assert_eq!(reply["root"]["props"]["searchText"], "");
    assert!(peer.calls.is_empty());
    let error = peer
        .request(
            "session.open",
            json!({
                "sessionId":"invalid-context", "commandName":"quick-translate-view",
                "preferences":{"autoInput":true}, "argumentValues":{},
                "launchContext":{"text":["invalid"]}
            }),
        )
        .await
        .unwrap_err();
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("launch text must be a string"));
    assert!(peer.calls.is_empty());
    peer.shutdown().await;
}

#[tokio::test]
async fn quick_results_use_forwarded_text_and_never_replace_it_with_selection() {
    let mut peer = Peer::start(GRANTS, clipboard("unrelated selection")).await;
    let reply = peer
        .request(
            "session.open",
            json!({
                "sessionId":"forwarded-text", "commandName":"quick-translate-view",
                "preferences":{"autoInput":true,"proxy":"http://127.0.0.1:1"},
                "argumentValues":{}, "searchText":"stale initiating search",
                "launchContext":{"text":"forwarded Bonjour","requestId":"fresh-text"}
            }),
        )
        .await
        .unwrap();
    assert!(reply.to_string().contains("connection/TLS/proxy failure"));
    let refreshed = peer
        .call("forwarded-text", "extension:refresh-session", json!({}))
        .await
        .unwrap();
    assert_eq!(
        refreshed["root"]["props"]["searchText"],
        "forwarded Bonjour"
    );
    assert!(peer.calls.is_empty());
    peer.shutdown().await;
}

#[tokio::test]
async fn instant_commands_require_selection_and_propagate_denial() {
    let mut peer = Peer::start(GRANTS, clipboard("")).await;
    for command in [
        "instant-translate-copy",
        "instant-translate-paste",
        "instant-translate-view",
    ] {
        let e = peer.open(command, command, json!({})).await.unwrap_err();
        assert!(e["message"].as_str().unwrap().contains("No text selected"));
    }
    assert_eq!(peer.methods(), vec!["host.selection.read"; 3]);
    peer.shutdown().await;
    let mut denied = Peer::start(&[], clipboard("hello")).await;
    assert!(denied
        .open("denied", "instant-translate-copy", json!({}))
        .await
        .is_err());
    assert!(denied.calls.is_empty());
    denied.shutdown().await;
}

#[tokio::test]
async fn stale_actions_and_disposed_sessions_fail() {
    let mut peer = Peer::start(GRANTS, clipboard("")).await;
    peer.open("s", "translate", json!({"autoInput":false}))
        .await
        .unwrap();
    assert!(peer
        .call(
            "s",
            "extension:invoke-action",
            json!({"actionId":"translate:1:0:copy"})
        )
        .await
        .is_err());
    peer.request("session.dispose", json!({"sessionId":"s"}))
        .await
        .unwrap();
    assert!(peer
        .call("s", "extension:refresh-session", json!({}))
        .await
        .is_err());
    peer.shutdown().await;
}

#[tokio::test]
async fn saved_language_sets_survive_sessions_and_delete() {
    let mut peer = Peer::start(GRANTS, clipboard("")).await;
    peer.open("save", "translate-form", json!({"autoInput":false}))
        .await
        .unwrap();
    let manager=peer.call("save","extension:invoke-action",json!({"actionId":"translate:save-set","formValues":{"text":"","language_from":"auto","language_to":"kk","extra_targets":"ru,en"}})).await.unwrap();
    assert_eq!(manager["title"], "Language Sets");
    assert_eq!(manager["root"]["children"][0]["props"]["subtitle"], "ru,en");
    peer.request("session.dispose", json!({"sessionId":"save"}))
        .await
        .unwrap();
    let form = peer
        .open(
            "load",
            "translate-form",
            json!({"autoInput":false,"lang1":"de"}),
        )
        .await
        .unwrap();
    assert_eq!(form["root"]["children"][2]["props"]["value"], "kk");
    let manager = peer
        .call(
            "load",
            "extension:invoke-action",
            json!({"actionId":"translate:manage"}),
        )
        .await
        .unwrap();
    let actions = manager["actions"].as_array().unwrap();
    let delete = actions
        .iter()
        .find(|a| a["title"] == "Delete Language Set")
        .unwrap()["id"]
        .clone();
    let empty = peer
        .call(
            "load",
            "extension:invoke-action",
            json!({"actionId":delete}),
        )
        .await
        .unwrap();
    assert_eq!(empty["root"]["children"][0]["type"], "List.EmptyView");
    assert!(peer
        .call(
            "load",
            "extension:invoke-action",
            json!({"actionId":delete})
        )
        .await
        .is_err());
    peer.shutdown().await;
}

#[tokio::test]
async fn native_detail_toggle_and_language_cycle_persist_without_host_effects() {
    let mut peer = Peer::start(GRANTS, clipboard("")).await;
    let opened = peer
        .open(
            "cycle",
            "translate",
            json!({"autoInput":false,"langFrom":"ru","lang1":"en","lang2":"kk"}),
        )
        .await
        .unwrap();
    assert_eq!(opened["root"]["props"]["isShowingDetail"], true);
    let toggled = peer
        .call(
            "cycle",
            "extension:invoke-action",
            json!({"actionId":"translate:toggle"}),
        )
        .await
        .unwrap();
    assert_eq!(toggled["root"]["props"]["isShowingDetail"], false);
    peer.call(
        "cycle",
        "extension:invoke-action",
        json!({"actionId":"translate:next"}),
    )
    .await
    .unwrap();
    let form = peer
        .call(
            "cycle",
            "extension:invoke-action",
            json!({"actionId":"translate:languages"}),
        )
        .await
        .unwrap();
    assert_eq!(form["root"]["children"][3]["props"]["value"], "kk");
    assert!(peer.calls.is_empty());
    peer.shutdown().await;
}

#[tokio::test]
#[ignore = "contacts the actual Google service; records real results, never fabricates a translation"]
async fn live_service_six_commands() {
    let mut evidence = vec![];
    for command in [
        "translate",
        "translate-form",
        "quick-translate",
        "instant-translate-copy",
        "instant-translate-paste",
        "instant-translate-view",
    ] {
        let mut peer = Peer::start(GRANTS, clipboard("Bonjour")).await;
        let mut reply = peer
            .open(
                "live",
                command,
                json!({"autoInput":true,"langFrom":"auto","lang1":"en","lang2":"ru"}),
            )
            .await
            .unwrap();
        if command == "translate-form" {
            reply = peer.call("live", "extension:invoke-action", json!({"actionId":"translate:submit","formValues":{"text":"Bonjour","language_from":"auto","language_to":"en","extra_targets":"ru"}})).await.unwrap();
        }
        if command == "translate-form" && !reply.to_string().contains("HTTP 429") {
            let action = reply["actions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["title"] == "Play Text-To-Speech")
                .unwrap()["id"]
                .clone();
            let tts = peer
                .call(
                    "live",
                    "extension:invoke-action",
                    json!({"actionId":action}),
                )
                .await;
            std::fs::write(
                "metadata/live-tts.json",
                serde_json::to_vec_pretty(&json!({"result":tts})).unwrap(),
            )
            .unwrap();
        }
        let encoded = reply.to_string();
        if encoded.contains("HTTP 429") {
            assert_eq!(reply["mode"], "view");
            assert!(!peer.methods().contains(&"host.clipboard.write"));
            assert!(!peer.methods().contains(&"host.clipboard.pasteText"));
            assert!(!peer.methods().contains(&"host.shell.open"));
        } else {
            assert!(!encoded.contains("request failed"), "{encoded}");
        }
        evidence.push(json!({"command":command,"reply":reply,"hostCalls":peer.calls}));
        peer.shutdown().await;
    }
    std::fs::write(
        "metadata/live-service-six.json",
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn failed_transport_is_visible_and_never_copies_or_pastes() {
    for command in [
        "translate-form",
        "instant-translate-copy",
        "instant-translate-paste",
        "instant-translate-view",
    ] {
        let mut peer = Peer::start(GRANTS, clipboard("Bonjour")).await;
        let mut reply = peer
            .open(
                "failure",
                command,
                json!({"autoInput":true,"proxy":"http://127.0.0.1:1"}),
            )
            .await
            .unwrap();
        if command == "translate-form" {
            reply = peer.call("failure", "extension:invoke-action", json!({"actionId":"translate:submit","formValues":{"text":"Bonjour","language_from":"fr","language_to":"en"}})).await.unwrap();
        }
        assert_eq!(reply["mode"], "view");
        assert!(reply.to_string().contains("connection/TLS/proxy failure"));
        assert!(!peer.methods().contains(&"host.clipboard.write"));
        assert!(!peer.methods().contains(&"host.clipboard.pasteText"));
        assert!(!peer.methods().contains(&"host.shell.open"));
        assert!(!reply["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["title"] == "Copy Translation"));
        peer.shutdown().await;
    }
}
