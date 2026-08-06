//! Local-first [`EmbeddingProvider`] backed by a locally-running Ollama at
//! `127.0.0.1:11434`. Reuses the existing `reqwest` client + the
//! `providers_wire::block_on_async` sync→async bridge — **zero new Cargo deps**.
//!
//! Embeds via `POST /api/embeddings {"model": "<id>", "prompt": "<text>"}` →
//! `{"embedding": [..768 floats..]}`. Ollama's classic embeddings endpoint takes
//! one prompt per call, so a batch loops (fine at personal scale + capture-time
//! best-effort indexing).
//!
//! This is the MVP source: compute + data both stay on the user's machine, so it
//! keeps the local-first moat. When Ollama is not running, every call returns
//! [`EmbedError::Unavailable`] and the caller degrades to FTS5-only — capture and
//! recall NEVER block on or panic over a missing embedder.
//!
//! 中文: 本機 Ollama 的 EmbeddingProvider。複用既有 reqwest + block_on_async,零新
//! dep。Ollama 不在時回 Unavailable,呼叫端降級為純 FTS5,不擋 capture / recall。

use super::{EmbedError, EmbeddingProvider};

/// Default Ollama embeddings model — 768-dim, ~137M params, local. Matches the
/// model the design pins (`ollama pull nomic-embed-text`).
pub const DEFAULT_MODEL: &str = "nomic-embed-text";
/// `nomic-embed-text` emits 768-dimensional vectors.
pub const DEFAULT_DIM: usize = 768;
/// Default local Ollama base URL. Overridable via `OLLAMA_HOST` (matches the
/// env var Ollama's own CLI honours).
const DEFAULT_BASE: &str = "http://127.0.0.1:11434";

/// An [`EmbeddingProvider`] that calls a local Ollama embeddings endpoint.
pub struct OllamaEmbedder {
    model: String,
    dim: usize,
    base_url: String,
    timeout_ms: u64,
}

impl Default for OllamaEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

impl OllamaEmbedder {
    /// Construct with the default `nomic-embed-text` / 768-dim config, honouring
    /// `OLLAMA_HOST` for the base URL if set.
    pub fn new() -> Self {
        let base_url = std::env::var("OLLAMA_HOST")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|h| normalize_base(&h))
            .unwrap_or_else(|| DEFAULT_BASE.to_string());
        OllamaEmbedder {
            model: DEFAULT_MODEL.to_string(),
            dim: DEFAULT_DIM,
            base_url,
            // Embeddings are small; 30s is generous and matches the provider
            // background-class budget.
            timeout_ms: 30_000,
        }
    }

    /// Override the model + dimension (e.g. `all-minilm` / 384). Keeps the
    /// trait pluggable without a second impl.
    pub fn with_model(mut self, model: impl Into<String>, dim: usize) -> Self {
        self.model = model.into();
        self.dim = dim;
        self
    }

    fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(self.timeout_ms.max(1000)))
            .build()
            .map_err(|e| EmbedError::Unavailable(format!("reqwest build: {e}")))?;
        let url = format!("{}/api/embeddings", self.base_url);
        let body = serde_json::json!({ "model": self.model, "prompt": text });

        // Reuse the crate-wide sync→async bridge (handles being inside or
        // outside an ambient tokio runtime). Drive BOTH the send and the body
        // read inside a SINGLE `block_on_async` so the response future never
        // straddles two runtime entries (which can panic with "context is being
        // shutdown" when an ambient short-lived runtime is torn down between the
        // two calls).
        let sent: Result<(u16, String), reqwest::Error> =
            crate::providers_wire::block_on_async(async {
                let resp = client.post(&url).json(&body).send().await?;
                let status = resp.status().as_u16();
                let txt = resp.text().await?;
                Ok((status, txt))
            });
        let (status, txt) =
            sent.map_err(|e| EmbedError::Unavailable(format!("ollama request failed: {e}")))?;
        if !(200..300).contains(&status) {
            // 404 here usually means the model isn't pulled.
            return Err(EmbedError::Backend(format!(
                "ollama {} for model `{}`: {}",
                status,
                self.model,
                txt.chars().take(200).collect::<String>()
            )));
        }
        let json: serde_json::Value = serde_json::from_str(&txt)
            .map_err(|e| EmbedError::BadResponse(format!("parse: {e}")))?;
        let arr = json
            .get("embedding")
            .and_then(|v| v.as_array())
            .ok_or_else(|| EmbedError::BadResponse("missing `embedding` array".into()))?;
        let vec: Vec<f32> = arr
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();
        if vec.is_empty() {
            return Err(EmbedError::BadResponse("empty embedding".into()));
        }
        if vec.len() != self.dim {
            return Err(EmbedError::DimMismatch {
                expected: self.dim,
                got: vec.len(),
            });
        }
        Ok(vec)
    }
}

impl EmbeddingProvider for OllamaEmbedder {
    fn model_id(&self) -> &str {
        &self.model
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.embed_one(t)?);
        }
        Ok(out)
    }
}

/// Default Ollama port, appended when `OLLAMA_HOST` gives a bare host.
const DEFAULT_PORT: u16 = 11434;

/// Normalize a user-supplied `OLLAMA_HOST` into a full client base URL. Accepts
/// `host:port`, a bare host, or a full `http(s)://...` URL; strips a trailing
/// slash so `format!("{base}/api/embeddings")` is always well-formed.
///
/// Two robustness fixes for the common case where `OLLAMA_HOST` is set to the
/// *server* bind value rather than a client target:
///  • `0.0.0.0` (the "listen on all interfaces" bind) is rewritten to
///    `127.0.0.1` — a client cannot *connect* to `0.0.0.0`.
///  • a bare host with no `:port` gets the default Ollama port appended.
fn normalize_base(h: &str) -> String {
    let raw = h.trim().trim_end_matches('/');
    // Split off an optional scheme so we can inspect the authority.
    let (scheme, authority) = if let Some(rest) = raw.strip_prefix("http://") {
        ("http://", rest)
    } else if let Some(rest) = raw.strip_prefix("https://") {
        ("https://", rest)
    } else {
        ("http://", raw)
    };
    // Separate host[:port] (ignore any path — Ollama base never carries one).
    let (hostport, _path) = authority.split_once('/').unwrap_or((authority, ""));
    let (host, port) = match hostport.rsplit_once(':') {
        // `:` could be inside an IPv6 literal; only treat the tail as a port if
        // it parses as a number.
        Some((hp, p)) if p.parse::<u16>().is_ok() => (hp, Some(p.to_string())),
        _ => (hostport, None),
    };
    // A wildcard bind address is not a valid connect target → loopback.
    let host = if host == "0.0.0.0" || host == "::" || host.is_empty() {
        "127.0.0.1"
    } else {
        host
    };
    let port = port.unwrap_or_else(|| DEFAULT_PORT.to_string());
    format!("{scheme}{host}:{port}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_nomic_768() {
        let e = OllamaEmbedder::new();
        assert_eq!(e.model_id(), "nomic-embed-text");
        assert_eq!(e.dim(), 768);
    }

    #[test]
    fn normalize_base_forms() {
        // host:port passes through (with scheme added).
        assert_eq!(normalize_base("127.0.0.1:11434"), "http://127.0.0.1:11434");
        assert_eq!(normalize_base("http://host:1/"), "http://host:1");
        // bare host → default Ollama port appended.
        assert_eq!(normalize_base("myhost"), "http://myhost:11434");
        // a server wildcard bind is rewritten to loopback (can't *connect* to it).
        assert_eq!(normalize_base("0.0.0.0"), "http://127.0.0.1:11434");
        assert_eq!(normalize_base("0.0.0.0:11434"), "http://127.0.0.1:11434");
        // https host with no port still gets the default port.
        assert_eq!(normalize_base("https://x.example/"), "https://x.example:11434");
    }

    /// REAL embed against local Ollama. `#[ignore]`d (network/env-dependent) —
    /// run with `cargo test -p spectyn-mesh ollama_real -- --ignored --nocapture`.
    /// Proves a true 768-dim vector comes back (not a mock).
    #[ignore = "needs local Ollama + nomic-embed-text — run via --ignored"]
    #[test]
    fn ollama_real_embed_returns_768d() {
        let e = OllamaEmbedder::new();
        let out = e.embed(&["hello".to_string()]).expect("ollama embed");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 768, "nomic-embed-text is 768-dim");
        // A real embedding is not all-zero.
        assert!(out[0].iter().any(|&x| x != 0.0), "vector is non-trivial");
    }
}
