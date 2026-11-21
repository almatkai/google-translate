use serde_json::{json, Value};
pub fn validate_targets(from: &str, to: &str, extra: &str) -> Result<Vec<String>, String> {
    let languages = super::languages();
    let known = |code: &str| {
        languages
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["value"] == code)
    };
    if !known(from) || !known(to) || to == "auto" {
        return Err("Select supported source and target languages".into());
    }
    let mut result = vec![to.to_owned()];
    for code in extra.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        if !known(code) || code == "auto" {
            return Err(format!("Unsupported target language: {code}"));
        }
        if !result.iter().any(|v| v == code) {
            result.push(code.to_owned());
        }
    }
    if result.len() > 8 {
        return Err("At most eight target languages per set are supported".into());
    }
    Ok(result)
}
pub fn render(state: &mut Value) -> Value {
    let revision = state["revision"].as_u64().unwrap_or(0) + 1;
    state["revision"] = json!(revision);
    let mut targets = serde_json::Map::new();
    let mut rows = vec![];
    let mut actions = vec![
        json!({"id":"sets:add","title":"Add Language Set","kind":"custom"}),
        json!({"id":"sets:back","title":"Back to Translation","kind":"custom"}),
    ];
    for (i, set) in state["sets"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let mut ids = vec![];
        for verb in ["select", "delete"] {
            let id = format!("sets:{revision}:{i}:{verb}");
            targets.insert(id.clone(), json!({"index":i,"verb":verb}));
            actions.push(json!({"id":id,"title":if verb=="select"{"Use Language Set"}else{"Delete Language Set"},"kind":"custom"}));
            ids.push(id);
        }
        ids.extend(["sets:add".to_owned(), "sets:back".to_owned()]);
        rows.push(json!({"type":"List.Item","props":{"id":i.to_string(),"title":format!("{} → {}",super::string(set,"from","auto"),super::string(set,"to","en")),"subtitle":super::string(set,"extraTargets",""),"actionIds":ids}}));
    }
    if rows.is_empty() {
        rows.push(json!({"type":"List.EmptyView","props":{"title":"No saved language sets","description":"Add a language set to reuse it across translation sessions."}}));
    }
    state["setTargets"] = Value::Object(targets);
    tezbar_extension_sdk::view(
        "Language Sets",
        json!({"type":"List","children":rows}),
        actions,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_validation() {
        assert_eq!(
            validate_targets("auto", "en", "ru, kk,ru").unwrap(),
            vec!["en", "ru", "kk"]
        );
        assert!(validate_targets("xx", "en", "").is_err());
        assert!(validate_targets("auto", "auto", "").is_err());
        assert!(validate_targets("auto", "en", "ru,kk,de,fr,it,es,ja,ko").is_err());
    }
}
