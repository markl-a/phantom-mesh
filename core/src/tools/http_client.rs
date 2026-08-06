use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    ClientBuilder,
};
use std::str::FromStr;
use std::time::Duration;

const MAX_BODY: usize = 8000;

fn build_header_map(headers: Option<&serde_json::Value>) -> HeaderMap {
    let mut map = HeaderMap::new();
    if let Some(serde_json::Value::Object(obj)) = headers {
        for (k, v) in obj {
            let val_str = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if let (Ok(name), Ok(value)) =
                (HeaderName::from_str(k), HeaderValue::from_str(&val_str))
            {
                map.insert(name, value);
            }
        }
    }
    map
}

fn format_response(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &str,
) -> String {
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body_truncated = if body.len() > MAX_BODY {
        format!(
            "{}\n[... truncated, {} total chars ...]",
            crate::tools::floor_char_boundary(body, MAX_BODY),
            body.len()
        )
    } else {
        body.to_string()
    };

    let mut out = format!(
        "HTTP {} {}\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    if !content_type.is_empty() {
        out.push_str(&format!("Content-Type: {}\n", content_type));
    }
    out.push_str("---\n");
    out.push_str(&body_truncated);
    out
}

pub async fn get(args: &serde_json::Value) -> String {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => return "ERROR: missing required parameter 'url'".to_string(),
    };
    // T7b T13-N6: SSRF guard. Blocks loopback / private / link-local hosts
    // unless SPECTYN_FETCH_ALLOW_LOCAL=1 is set.
    if let Err(e) = crate::tools::urlguard::validate_url(&url) {
        return format!("ERROR: {}", e);
    }
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);
    let extra_headers = args.get("headers");

    let client = match ClientBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .use_rustls_tls()
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("ERROR: failed to build HTTP client: {}", e),
    };

    let mut req = client.get(&url);
    let hmap = build_header_map(extra_headers);
    if !hmap.is_empty() {
        req = req.headers(hmap);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let resp_headers = resp.headers().clone();
            match resp.text().await {
                Ok(body) => {
                    if status.is_success() {
                        format_response(status, &resp_headers, &body)
                    } else {
                        format!(
                            "ERROR: HTTP {} {}\nURL: {}",
                            status.as_u16(),
                            status.canonical_reason().unwrap_or(""),
                            url
                        )
                    }
                }
                Err(e) => format!("ERROR: failed to read response body: {}", e),
            }
        }
        Err(e) => format!("ERROR: request failed: {}", e),
    }
}

pub async fn post(args: &serde_json::Value) -> String {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => return "ERROR: missing required parameter 'url'".to_string(),
    };
    // T7b T13-N6: SSRF guard. Blocks loopback / private / link-local hosts
    // unless SPECTYN_FETCH_ALLOW_LOCAL=1 is set.
    if let Err(e) = crate::tools::urlguard::validate_url(&url) {
        return format!("ERROR: {}", e);
    }
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);
    let extra_headers = args.get("headers");
    let body_json = args.get("body");
    let body_text = args.get("body_text").and_then(|v| v.as_str());

    let client = match ClientBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .use_rustls_tls()
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("ERROR: failed to build HTTP client: {}", e),
    };

    let mut req = client.post(&url);

    let hmap = build_header_map(extra_headers);
    if !hmap.is_empty() {
        req = req.headers(hmap);
    }

    req = if let Some(json_val) = body_json {
        req.json(json_val)
    } else if let Some(text) = body_text {
        req.header(reqwest::header::CONTENT_TYPE, "text/plain")
            .body(text.to_string())
    } else {
        req.header(reqwest::header::CONTENT_LENGTH, "0")
    };

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let resp_headers = resp.headers().clone();
            match resp.text().await {
                Ok(body) => {
                    if status.is_success() {
                        format_response(status, &resp_headers, &body)
                    } else {
                        format!(
                            "ERROR: HTTP {} {}\nURL: {}",
                            status.as_u16(),
                            status.canonical_reason().unwrap_or(""),
                            url
                        )
                    }
                }
                Err(e) => format!("ERROR: failed to read response body: {}", e),
            }
        }
        Err(e) => format!("ERROR: request failed: {}", e),
    }
}
