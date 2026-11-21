mod audio;
mod saved;
mod translation;
use serde_json::{json, Value};
use tezbar_extension_sdk::{
    no_view, unchanged, view, BoxFuture, CoreResult, Extension, ExtensionContext,
};

pub const ID: &str = "com.tezbar.native.google-translate";
pub struct Adapter {
    pub endpoint: String,
}
const ENDPOINT: &str = "https://translate.google.com/translate_a/single";

#[derive(Debug)]
enum HandlerError {
    Operation(String),
    Translation(String),
}

#[derive(Debug)]
struct TranslationFailure(String);

impl From<String> for TranslationFailure {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<TranslationFailure> for HandlerError {
    fn from(error: TranslationFailure) -> Self {
        Self::Translation(error.0)
    }
}

impl From<String> for HandlerError {
    fn from(message: String) -> Self {
        Self::Operation(message)
    }
}

impl From<&str> for HandlerError {
    fn from(message: &str) -> Self {
        Self::Operation(message.to_owned())
    }
}
const COMMANDS: &[&str] = &[
    "translate",
    "translate-form",
    "quick-translate",
    "quick-translate-view",
    "instant-translate-copy",
    "instant-translate-paste",
    "instant-translate-view",
];
fn string<'a>(v: &'a Value, key: &str, default: &'a str) -> &'a str {
    v[key].as_str().unwrap_or(default)
}
fn languages() -> Value {
    serde_json::from_str(include_str!("../resources/languages.json")).expect("language data")
}

fn render(command: &str, state: &mut Value) -> Value {
    if state["manager"] == true {
        return saved::render(state);
    }
    let revision = state["revision"].as_u64().unwrap_or(0) + 1;
    state["revision"] = json!(revision);
    let results = state["results"].as_array().cloned().unwrap_or_default();
    let mut actions = vec![];
    let mut targets = serde_json::Map::new();
    let mut rows = vec![];
    for (i, result) in results.iter().enumerate() {
        let mut ids = vec![];
        let verbs = if state["preferences"]["defaultAction"] == "paste" {
            [
                "paste",
                "copy",
                "original",
                "pronunciation",
                "open",
                "listen",
            ]
        } else {
            [
                "copy",
                "paste",
                "original",
                "pronunciation",
                "open",
                "listen",
            ]
        };
        let pronunciation = string(result, "pronunciation", "");
        for verb in verbs {
            if verb == "pronunciation" && pronunciation.trim().is_empty() {
                continue;
            }
            let id = format!("translate:{revision}:{i}:{verb}");
            let title = match verb {
                "copy" => "Copy Translation",
                "paste" => "Paste Translation",
                "listen" => "Play Text-To-Speech",
                "original" => "Copy Original Text",
                "pronunciation" => "Copy Pronunciation",
                _ => "Open in Google Translate",
            };
            actions.push(json!({"id":id,"title":title,"kind":if verb == "paste" { "paste" } else if verb == "open" { "open" } else if verb == "listen" { "custom" } else { "copy" }}));
            targets.insert(id.clone(), json!({"verb":verb,"result":result}));
            ids.push(id);
        }
        let text = string(result, "text", "");
        let markdown = if command.starts_with("quick-") && !pronunciation.trim().is_empty() {
            format!("{text}\n\n```\n{pronunciation}\n```")
        } else {
            text.to_owned()
        };
        rows.push(json!({"type":"List.Item","props":{"id":i.to_string(),"title":result["text"],"subtitle":format!("{} → {}",string(result,"from","auto"),string(result,"to","en")),"actionIds":ids},"children":[{"type":"List.Item.Detail","props":{"markdown":markdown}}]}));
    }
    state["targets"] = Value::Object(targets);
    if command == "translate-form" || state["editing"] == true {
        let mut from = vec![];
        let mut to = vec![];
        for lang in languages().as_array().unwrap() {
            let item = json!({"type":"Form.Dropdown.Item","props":{"value":lang["value"],"title":lang["title"]}});
            from.push(item.clone());
            if lang["value"] != "auto" {
                to.push(item);
            }
        }
        actions.insert(
            0,
            json!({"id":"translate:submit","title":"Translate","kind":"submit-form"}),
        );
        actions.insert(
            1,
            json!({"id":"translate:save-set","title":"Save Language Set","kind":"submit-form"}),
        );
        actions
            .push(json!({"id":"translate:manage","title":"Manage Language Sets","kind":"custom"}));
        return view(
            "Google Translate",
            json!({"type":"Form","children":[
                {"type":"Form.TextArea","props":{"id":"text","title":"Text","value":state["text"]}},
                {"type":"Form.Dropdown","props":{"id":"language_from","title":"From","value":state["from"]},"children":from},
                {"type":"Form.Dropdown","props":{"id":"language_to","title":"To","value":state["to"]},"children":to},
                {"type":"Form.TextField","props":{"id":"extra_targets","title":"Additional targets","value":state["extraTargets"].as_str().unwrap_or(""),"placeholder":"Comma-separated language codes, e.g. ru,kk,de (Quick Translate)"}},
                {"type":"Form.Description","props":{"title":"Translation","text":state["error"].as_str().unwrap_or_else(|| results.first().map(|r|string(r,"text","")).unwrap_or(""))}}
            ]}),
            actions,
        );
    }
    for (id, title) in [
        ("translate:previous", "Previous Language Set"),
        ("translate:next", "Next Language Set"),
        ("translate:toggle", "Toggle Full Text"),
        ("translate:swap", "Swap Languages"),
        ("translate:languages", "Change Languages"),
        ("translate:manage", "Manage Language Sets"),
    ] {
        actions.push(json!({"id":id,"title":title,"kind":"custom"}));
        for row in &mut rows {
            if let Some(ids) = row["props"]["actionIds"].as_array_mut() {
                ids.push(json!(id));
            }
        }
    }
    if rows.is_empty() {
        rows.push(json!({"type":"List.EmptyView","props":{"title":"Enter text to translate","description":string(state,"error","Language preferences select source and target languages.")}}));
    }
    view(
        "Google Translate",
        json!({"type":"List","props":{"searchText":state["text"],"searchBarPlaceholder":"Enter text to translate","__hasServerSearch":true,"isShowingDetail":state["showDetail"].as_bool().unwrap_or(true)},"children":rows}),
        actions,
    )
}

async fn translate_state(
    command: &str,
    state: &mut Value,
    endpoint: &str,
) -> Result<(), TranslationFailure> {
    state["error"] = Value::Null;
    let text = string(state, "text", "").trim();
    if text.is_empty() {
        state["results"] = json!([]);
        return Ok(());
    }
    let from = string(state, "from", "auto");
    let to = string(state, "to", "en");
    let prefs = &state["preferences"];
    let second = string(prefs, "lang2", "en");
    let fallback = if command.starts_with("instant-") {
        second
    } else {
        to
    };
    let proxy = string(prefs, "proxy", "");
    let client = translation::client(proxy)?;
    let first = translation::translate(&client, endpoint, text, from, to).await?;
    let mut results = vec![first];
    if matches!(command, "quick-translate" | "quick-translate-view")
        || (command == "translate"
            && state["extraTargets"]
                .as_str()
                .is_some_and(|v| !v.is_empty()))
    {
        let extra = string(state, "extraTargets", second);
        let targets = saved::validate_targets(from, to, extra)?;
        for target in targets.iter().skip(1) {
            results.push(translation::translate(&client, endpoint, text, from, target).await?);
        }
        if prefs["prioritizeCrossLanguage"] == true {
            results.sort_by_key(|r| {
                translation::same_language(string(r, "from", ""), string(r, "to", ""))
            });
        }
    } else if command == "translate" {
        if translation::same_language(string(&results[0], "from", ""), to)
            && fallback != to
            && fallback != "auto"
        {
            results[0] = translation::translate(&client, endpoint, text, from, fallback).await?;
        }
        let first = &results[0];
        results.push(
            translation::translate(
                &client,
                endpoint,
                if from == "auto" {
                    string(first, "text", "")
                } else {
                    text
                },
                string(first, "to", "en"),
                string(first, "from", "en"),
            )
            .await?,
        );
    } else if command.starts_with("instant-")
        && translation::same_language(string(&results[0], "from", ""), to)
        && second != to
        && second != "auto"
    {
        results[0] = translation::translate(&client, endpoint, text, from, fallback).await?;
    }
    state["results"] = json!(results);
    Ok(())
}

impl Extension for Adapter {
    fn id(&self) -> &'static str {
        ID
    }
    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
    fn commands(&self) -> &'static [&'static str] {
        COMMANDS
    }
    fn handle<'a>(
        &'a self,
        ctx: &'a ExtensionContext,
        command: &'a str,
        operation: &'a str,
        payload: &'a Value,
        state: &'a mut Value,
    ) -> BoxFuture<'a, CoreResult> {
        Box::pin(async move {
            let outcome = async {
            if !COMMANDS.contains(&command) {
                return Err("Unknown translation command".into());
            }
            if operation == "extension:dispose-session" {
                *state = json!({});
                return Ok(unchanged());
            }
            if operation == "extension:run-command" {
                let prefs = &payload["preferences"];
                *state = json!({"text":"","from":string(prefs,"langFrom","auto"),"to":string(prefs,"lang1","en"),"preferences":prefs,"results":[]});
                let store = tezbar_extension_sdk::read_store(ctx, "language-sets").await?;
                state["sets"] = store["sets"]
                    .as_array()
                    .map(|s| json!(s))
                    .unwrap_or(json!([]));
                if !command.starts_with("instant-") {
                    if let Some(selected) = store["selected"].as_object() {
                        let selected = Value::Object(selected.clone());
                        if saved::validate_targets(
                            string(&selected, "from", "auto"),
                            string(&selected, "to", "en"),
                            string(&selected, "extraTargets", ""),
                        )
                        .is_ok()
                        {
                            for key in ["from", "to", "extraTargets"] {
                                state[key] = selected[key].clone();
                            }
                        }
                    }
                }
                let launch_text = if command == "quick-translate-view" {
                    payload["launchContext"].get("text")
                        .map(|text| text.as_str().ok_or("Quick Translate launch text must be a string"))
                        .transpose()?
                } else {
                    None
                };
                let selected = if launch_text.is_none() && (command.starts_with("instant-")
                    || prefs["autoInput"].as_bool().unwrap_or(true)
                ) {
                    match ctx.host("host.selection.read", json!({})).await {
                        Ok(v) => v.as_str().unwrap_or("").to_owned(),
                        Err(e) if command.starts_with("instant-") => return Err(e.into()),
                        Err(_) => String::new(),
                    }
                } else {
                    String::new()
                };
                let input = if command.starts_with("instant-") {
                    selected.as_str()
                } else {
                    launch_text.or_else(|| payload["searchText"].as_str().filter(|s| !s.is_empty()))
                        .unwrap_or(&selected)
                };
                state["text"] = json!(input);
                if command.starts_with("instant-") && string(state, "text", "").trim().is_empty() {
                    return Err("No text selected. Please select text to translate.".into());
                }
            } else if operation == "extension:search-text-changed" && !command.starts_with("instant-") {
                state["text"] = json!(payload["searchText"]
                    .as_str()
                    .ok_or("Missing search text")?);
                tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            } else if operation == "extension:invoke-action" {
                let id = payload["actionId"].as_str().ok_or("Missing action ID")?;
                if id == "translate:previous" || id == "translate:next" {
                    let prefs = &state["preferences"];
                    let mut sets = vec![
                        json!({"from":string(prefs,"langFrom","auto"),"to":string(prefs,"lang1","en"),"extraTargets":string(prefs,"lang2","")}),
                    ];
                    sets.extend(state["sets"].as_array().cloned().unwrap_or_default());
                    let index = sets
                        .iter()
                        .position(|s| {
                            s["from"] == state["from"]
                                && s["to"] == state["to"]
                                && s["extraTargets"].as_str().unwrap_or("")
                                    == state["extraTargets"].as_str().unwrap_or("")
                        })
                        .unwrap_or(0);
                    let next = if id == "translate:next" {
                        (index + 1) % sets.len()
                    } else {
                        (index + sets.len() - 1) % sets.len()
                    };
                    let selected = sets[next].clone();
                    for key in ["from", "to", "extraTargets"] {
                        state[key] = selected[key].clone();
                    }
                    tezbar_extension_sdk::update_store(ctx, "language-sets", move |store| {
                        store["selected"] = selected;
                        Ok(())
                    })
                    .await?;
                    translate_state(command, state, &self.endpoint).await?;
                    return Ok(render(command, state));
                }
                if id == "translate:toggle" {
                    state["showDetail"] = json!(!state["showDetail"].as_bool().unwrap_or(true));
                    return Ok(render(command, state));
                }
                if id == "translate:swap" {
                    let from = string(state, "from", "auto");
                    let detected = string(&state["results"][0], "from", from);
                    if detected == "auto" {
                        return Err(
                            "Translate text before swapping an auto-detected language".into()
                        );
                    }
                    let to = state["to"].clone();
                    let from = json!(detected);
                    state["from"] = to;
                    state["to"] = from;
                    translate_state(command, state, &self.endpoint).await?;
                    return Ok(render(command, state));
                }
                if id == "translate:manage" {
                    state["manager"] = json!(true);
                    return Ok(render(command, state));
                }
                if id == "sets:back" {
                    state["manager"] = json!(false);
                    return Ok(render(command, state));
                }
                if id == "sets:add" {
                    state["manager"] = json!(false);
                    state["editing"] = json!(true);
                    return Ok(render(command, state));
                }
                if state["manager"] == true {
                    let target = state["setTargets"]
                        .get(id)
                        .ok_or("Unknown or stale language-set action")?
                        .clone();
                    let index = target["index"].as_u64().ok_or("Invalid language set")? as usize;
                    let sets = state["sets"].as_array().cloned().unwrap_or_default();
                    let selected = sets
                        .get(index)
                        .ok_or("Language set no longer exists")?
                        .clone();
                    if target["verb"] == "delete" {
                        let stored = tezbar_extension_sdk::update_store(
                            ctx,
                            "language-sets",
                            move |store| {
                                let mut sets =
                                    store["sets"].as_array().cloned().unwrap_or_default();
                                sets.retain(|set| set != &selected);
                                if store["selected"] == selected {
                                    store.as_object_mut().unwrap().remove("selected");
                                }
                                store["sets"] = json!(sets);
                                Ok(())
                            },
                        )
                        .await?;
                        state["sets"] = stored["sets"].clone();
                        return Ok(render(command, state));
                    }
                    for key in ["from", "to", "extraTargets"] {
                        state[key] = selected[key].clone();
                    }
                    tezbar_extension_sdk::update_store(ctx, "language-sets", move |store| {
                        store["selected"] = selected;
                        Ok(())
                    })
                    .await?;
                    state["manager"] = json!(false);
                    translate_state(command, state, &self.endpoint).await?;
                    return Ok(render(command, state));
                }
                if id == "translate:languages" {
                    state["editing"] = json!(true);
                    return Ok(render(command, state));
                }
                if id == "translate:submit" || id == "translate:save-set" {
                    let values = &payload["formValues"];
                    for (key, field) in [
                        ("text", "text"),
                        ("from", "language_from"),
                        ("to", "language_to"),
                    ] {
                        state[key] = json!(values[field].as_str().ok_or_else(|| HandlerError::Translation(format!("Missing form field: {field}")))?);
                    }
                    state["extraTargets"] = json!(values["extra_targets"].as_str().unwrap_or(""));
                    saved::validate_targets(
                        string(state, "from", "auto"),
                        string(state, "to", "en"),
                        string(state, "extraTargets", ""),
                    ).map_err(HandlerError::Translation)?;
                    let selected = json!({"from":state["from"],"to":state["to"],"extraTargets":state["extraTargets"]});
                    let saving = id == "translate:save-set";
                    let stored =
                        tezbar_extension_sdk::update_store(ctx, "language-sets", move |store| {
                            let mut sets = store["sets"].as_array().cloned().unwrap_or_default();
                            if saving && !sets.contains(&selected) {
                                if sets.len() >= 64 {
                                    return Err(
                                        "At most 64 saved language sets are supported".into()
                                    );
                                }
                                sets.push(selected.clone());
                            }
                            store["selected"] = selected;
                            store["sets"] = json!(sets);
                            Ok(())
                        })
                        .await?;
                    state["sets"] = stored["sets"].clone();
                    if id == "translate:save-set" {
                        state["editing"] = json!(false);
                        state["manager"] = json!(true);
                        return Ok(render(command, state));
                    }
                    state["editing"] = json!(false);
                } else {
                    let target = state["targets"]
                        .get(id)
                        .ok_or("Unknown or stale translation action")?
                        .clone();
                    let result = &target["result"];
                    let text = string(result, "text", "");
                    match target["verb"].as_str() {
                        Some("listen") => {
                            audio::speak(
                                ctx,
                                text,
                                string(result, "to", "en"),
                                string(&state["preferences"], "proxy", ""),
                            )
                            .await?;
                        }
                        Some("copy") => {
                            ctx.host("host.clipboard.write", json!({"text":text}))
                                .await?;
                        }
                        Some("original") => {
                            ctx.host("host.clipboard.write", json!({"text":result["original"]}))
                                .await?;
                        }
                        Some("pronunciation") => {
                            ctx.host(
                                "host.clipboard.write",
                                json!({"text":result["pronunciation"]}),
                            )
                            .await?;
                        }
                        Some("paste") => {
                            ctx.host("host.clipboard.pasteText", json!({"text":text}))
                                .await?;
                            return Ok(no_view("Translation pasted"));
                        }
                        Some("open") => {
                            let mut url =
                                reqwest::Url::parse("https://translate.google.com/").unwrap();
                            url.query_pairs_mut()
                                .append_pair("sl", string(result, "from", "auto"))
                                .append_pair("tl", string(result, "to", "en"))
                                .append_pair("text", string(result, "original", ""))
                                .append_pair("op", "translate");
                            ctx.host("host.shell.open", json!({"target":url.to_string()}))
                                .await?;
                        }
                        _ => return Err("Unsupported translation action".into()),
                    }
                    return Ok(render(command, state));
                }
            } else if operation == "extension:refresh-session" {
                return Ok(render(command, state));
            } else if operation == "extension:load-more" {
                return Ok(unchanged());
            } else {
                return Err("Unsupported translation operation".into());
            }
            translate_state(command, state, &self.endpoint).await?;
            if command.starts_with("instant-") {
                let text = string(&state["results"][0], "text", "").to_owned();
                if command.ends_with("copy") {
                    ctx.host("host.clipboard.write", json!({"text":text}))
                        .await?;
                    return Ok(no_view("Translation copied"));
                }
                if command.ends_with("paste") {
                    ctx.host("host.clipboard.pasteText", json!({"text":text}))
                        .await?;
                    return Ok(no_view("Translation pasted"));
                }
                return Ok(view(
                    "Translation",
                    json!({"type":"Detail","props":{"markdown":text}}),
                    vec![],
                ));
            }
            Ok(render(command, state))
            }.await;
            match outcome {
                Err(HandlerError::Translation(error)) => {
                    // Never retain an old translation or its copy/paste actions after a failed request.
                    state["results"] = json!([]);
                    state["targets"] = json!({});
                    state["error"] = json!(error);
                    if command == "translate-form" || state["editing"] == true {
                        Ok(render(command, state))
                    } else {
                        Ok(view(
                            "Google Translate request failed",
                            json!({"type":"Detail","props":{"markdown":error}}),
                            vec![],
                        ))
                    }
                }
                Err(HandlerError::Operation(error)) => Err(error),
                Ok(reply) => Ok(reply),
            }
        })
    }
}
fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    let result = rt.block_on(tezbar_extension_sdk::serve(Adapter {
        endpoint: ENDPOINT.into(),
    }));
    rt.shutdown_timeout(std::time::Duration::from_millis(500));
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use tezbar_extension_sdk::{ExtensionPaths, TestContext};
    #[tokio::test]
    async fn copy_paste_open_and_host_errors_use_authoritative_result() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = TestContext::new(
            ExtensionPaths {
                package_dir: dir.path().into(),
                data_dir: dir.path().join("data"),
                temp_dir: dir.path().join("temp"),
            },
            &["clipboard.write", "clipboard.pasteText", "shell.open"],
        );
        for (verb, method) in [
            ("copy", "host.clipboard.write"),
            ("paste", "host.clipboard.pasteText"),
            ("open", "host.shell.open"),
        ] {
            let mut state = json!({"text":"Привет & +","results":[{"original":"Привет & +","text":"Hello & +","from":"ru","to":"en","pronunciation":"privet"}],"preferences":{"defaultAction":"paste"}});
            let v = render("translate", &mut state);
            assert!(v["actions"][0]["id"].as_str().unwrap().ends_with(":paste"));
            let id = v["actions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["id"].as_str().unwrap().ends_with(&format!(":{verb}")))
                .unwrap()["id"]
                .clone();
            let payload = json!({"actionId":id,"text":"attacker-controlled"});
            ctx.run(
                Adapter {
                    endpoint: ENDPOINT.into(),
                }
                .handle(
                    &ctx.context,
                    "translate",
                    "extension:invoke-action",
                    &payload,
                    &mut state,
                ),
            )
            .await
            .unwrap();
            let calls = ctx.calls();
            let call = calls.last().unwrap();
            assert_eq!(call["method"], method);
            if verb == "open" {
                let url = reqwest::Url::parse(call["params"]["target"].as_str().unwrap()).unwrap();
                assert_eq!(
                    url.query_pairs().find(|(k, _)| k == "text").unwrap().1,
                    "Привет & +"
                );
            } else {
                assert_eq!(call["params"]["text"], "Hello & +");
            }
            let v = render("translate", &mut state);
            let id = v["actions"][0]["id"].clone();
            ctx.reply(Err("clipboard unavailable".into()));
            assert!(ctx
                .run(
                    Adapter {
                        endpoint: ENDPOINT.into()
                    }
                    .handle(
                        &ctx.context,
                        "translate",
                        "extension:invoke-action",
                        &json!({"actionId":id}),
                        &mut state
                    )
                )
                .await
                .is_err());
        }
    }
}

#[cfg(test)]
mod parity_tests {
    use super::*;
    #[tokio::test]
    async fn explicit_reverse_uses_original_auto_reverse_uses_translation_and_multiple_targets_stay_ordered(
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (command, from, extra, second_query) in [
            ("translate", "ru", "", "q=original"),
            ("translate", "auto", "", "q=translated"),
            ("translate", "auto", "de", "tl=de"),
            ("quick-translate", "auto", "de", "tl=de"),
            ("quick-translate-view", "auto", "de", "tl=de"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                for i in 0..2 {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut buf = [0; 8192];
                    let n = socket.read(&mut buf).await.unwrap();
                    let request = String::from_utf8_lossy(&buf[..n]);
                    if i == 1 {
                        assert!(request.contains(second_query), "{request}");
                    }
                    let body = r#"[[["translated","original",null]],null,"ru"]"#;
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
                }
            });
            let mut state = json!({"text":"original","from":from,"to":"en","extraTargets":extra,"preferences":{"lang2":"de"}});
            translate_state(command, &mut state, &format!("http://{address}/"))
                .await
                .unwrap();
            assert_eq!(state["results"].as_array().unwrap().len(), 2);
            server.await.unwrap();
        }
    }
    #[test]
    fn audio_and_detail_actions_are_real_and_stale_ids_expire() {
        let mut state = json!({"results":[{"text":"hello","to":"en","from":"ru"}]});
        let first = render("translate", &mut state);
        let selected_actions = first["root"]["children"][0]["props"]["actionIds"]
            .as_array()
            .unwrap();
        assert!(selected_actions
            .iter()
            .any(|id| id == "translate:languages"));
        assert!(selected_actions.iter().any(|id| id == "translate:manage"));
        assert!(selected_actions[0].as_str().unwrap().ends_with(":copy"));
        assert!(first["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["title"] == "Play Text-To-Speech"));
        let old = first["actions"][0]["id"].as_str().unwrap().to_owned();
        render("translate", &mut state);
        assert!(state["targets"].get(old).is_none());
    }
}
