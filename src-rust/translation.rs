use serde_json::{json, Value};
use std::time::Duration;
pub fn same_language(a: &str, b: &str) -> bool {
    !a.is_empty()
        && !b.is_empty()
        && a.split('-')
            .next()
            .unwrap()
            .eq_ignore_ascii_case(b.split('-').next().unwrap())
}
pub fn client(proxy: &str) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent(concat!(
            "Tezbar Google Translate/",
            env!("CARGO_PKG_VERSION")
        ));
    if !proxy.trim().is_empty() {
        builder =
            builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| "Invalid translation proxy URL")?);
    }
    builder
        .build()
        .map_err(|_| "Cannot initialize translation HTTP client".into())
}
pub fn parse(raw: Value, text: &str, from: &str, to: &str) -> Result<Value, String> {
    let segments = raw[0]
        .as_array()
        .ok_or("Invalid translation response: missing segments")?;
    let translated: String = segments.iter().filter_map(|s| s[0].as_str()).collect();
    if translated.is_empty() {
        return Err("Translation not found in response".into());
    }
    let detected = raw[2].as_str().filter(|v| !v.is_empty()).unwrap_or(from);
    if detected == "auto" {
        return Err("Translation response has no detected language".into());
    }
    let pronunciation = segments
        .iter()
        .filter_map(|s| {
            s[2].as_str()
                .filter(|p| !p.trim().is_empty())
                .or_else(|| s[3].as_str())
        })
        .collect::<String>();
    Ok(
        json!({"text":translated,"original":text,"from":detected,"to":to,"pronunciation":pronunciation}),
    )
}
pub async fn translate(
    client: &reqwest::Client,
    endpoint: &str,
    text: &str,
    from: &str,
    to: &str,
) -> Result<Value, String> {
    if text.chars().count() > 5000 {
        return Err("Google Translate accepts at most 5000 characters per translation".into());
    }
    let languages: Value =
        serde_json::from_str(include_str!("../resources/languages.json")).unwrap();
    let known = |code: &str| {
        languages
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["value"] == code)
    };
    if !known(from) || to == "auto" || !known(to) {
        return Err("Select a supported source and target language".into());
    }
    let mut url = reqwest::Url::parse(endpoint).map_err(|_| "Invalid translation endpoint")?;
    // The pinned vendor uses Node querystring for the URL (including %20 for spaces)
    // and URLSearchParams form encoding for the POST body.
    let options = format!("client=dict-chrome-ex&sl={from}&tl={to}&hl={to}&dt=at&dt=bd&dt=ex&dt=ld&dt=md&dt=qca&dt=rw&dt=rm&dt=ss&dt=t&ie=UTF-8&oe=UTF-8&otf=1&ssel=0&tsel=0&kc=7");
    let mut query = format!("{options}&q=");
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            query.push(char::from(b));
        } else {
            use std::fmt::Write;
            write!(query, "%{b:02X}").unwrap();
        }
    }
    let request = if url.as_str().len() + 1 + query.len() > 2048 {
        url.set_query(Some(&options));
        client
            .post(url)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded;charset=UTF-8",
            )
            .form(&[("q", text)])
    } else {
        url.set_query(Some(&query));
        client.get(url)
    };
    let mut response = request.send().await.map_err(|e| {
        format!(
            "Translation request to Google Translate failed: {}",
            if e.is_timeout() {
                "timeout"
            } else if e.is_connect() {
                "connection/TLS/proxy failure"
            } else {
                "transport failure"
            }
        )
    })?;
    if response.status().as_u16() == 429 {
        return Err("Google Translate returned HTTP 429 (rate limited / automated-query protection). No translation was returned. Try later or configure an authorized proxy.".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "Google Translate returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let mut bytes = vec![];
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Translation response interrupted")?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("Translation response exceeds 1 MiB limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value =
        serde_json::from_slice(&bytes).map_err(|_| "Google Translate returned invalid JSON")?;
    parse(value, text, from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_segments_and_pronunciation() {
        let v = parse(
            json!([
                [
                    ["Hello ", "Привет ", null],
                    ["world", "мир", null],
                    [null, null, "privet mir"]
                ],
                null,
                "ru"
            ]),
            "Привет мир",
            "auto",
            "en",
        )
        .unwrap();
        assert_eq!(v["text"], "Hello world");
        assert_eq!(v["pronunciation"], "privet mir");
        assert_eq!(v["from"], "ru");
    }
    #[test]
    fn rejects_malformed() {
        for raw in [json!({}), json!([[]]), json!([[[null, null]], null, "en"])] {
            assert!(parse(raw, "hello", "auto", "en").is_err());
        }
    }
    #[test]
    fn language_families() {
        assert!(same_language("EN-us", "en"));
        assert!(same_language("zh-CN", "zh-TW"));
        assert!(!same_language("", ""));
        assert!(!same_language("ru", "en"));
    }
    #[tokio::test]
    async fn bounds_input_and_languages() {
        let c = client("").unwrap();
        assert!(
            translate(&c, "http://127.0.0.1:1", &"x".repeat(5001), "auto", "en")
                .await
                .unwrap_err()
                .contains("5000")
        );
        assert!(translate(&c, "http://127.0.0.1:1", "hello", "auto", "auto")
            .await
            .is_err());
    }
    #[tokio::test]
    #[ignore = "actual external service"]
    async fn live_directions_and_proxy() {
        let c = client("").unwrap();
        let endpoint = "https://translate.google.com/translate_a/single";
        let mut records = vec![];
        for (text, from, to) in [
            ("Bonjour", "auto", "en"),
            ("Bonjour", "fr", "en"),
            ("Good morning", "en", "fr"),
            ("Bonjour", "fr", "ru"),
            ("Bonjour", "fr", "kk"),
        ] {
            let result = translate(&c, endpoint, text, from, to).await;
            records.push(json!({"input":text,"from":from,"to":to,"result":result}));
        }
        let proxy_error = translate(
            &client("http://127.0.0.1:1").unwrap(),
            endpoint,
            "Bonjour",
            "fr",
            "en",
        )
        .await
        .unwrap_err();
        assert!(proxy_error.contains("connection/TLS/proxy failure"));
        records.push(json!({"proxy":"http://127.0.0.1:1","error":proxy_error}));
        std::fs::write(
            "metadata/live-directions-proxy.json",
            serde_json::to_vec_pretty(&records).unwrap(),
        )
        .unwrap();
    }
    #[tokio::test]
    async fn rate_limit_is_not_translation_or_json_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/translate_a/single",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            socket.read(&mut buf).await.unwrap();
            socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 6\r\nConnection: close\r\n\r\n<html>").await.unwrap();
        });
        let error = translate(&client("").unwrap(), &endpoint, "Bonjour", "auto", "en")
            .await
            .unwrap_err();
        assert!(error.contains("HTTP 429"));
        assert!(error.contains("No translation was returned"));
        server.await.unwrap();
    }
    #[tokio::test]
    async fn actual_http_request_and_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            assert!(request.contains("client=dict-chrome-ex"));
            assert!(request.contains("sl=auto"));
            assert!(request.contains("tl=en"));
            assert!(request.contains("q="));
            let body = r#"[[["Hello","Привет",null]],null,"ru"]"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let v = translate(
            &client("").unwrap(),
            &format!("http://{address}/translate"),
            "Привет",
            "auto",
            "en",
        )
        .await
        .unwrap();
        assert_eq!(v["text"], "Hello");
        server.await.unwrap();
    }
}
