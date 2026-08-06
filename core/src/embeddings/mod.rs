//! Semantic memory primitives — the local-first embedding layer that upgrades
//! `spectyn recall` from lexical-only (FTS5 BM25 + file-store substring) to
//! **hybrid keyword + semantic**. This module owns the *pure* parts (trait,
//! cosine, brute-force top-k, f32-BLOB codec); the Ollama HTTP implementation
//! lives in [`ollama`].
//!
//! Design invariant (see the recall-engine design doc): the embedding vector is
//! a **derived index** — the canonical truth is always the age-encrypted event
//! blob. `events_emb` can be dropped + rebuilt (`spectyn recall --reindex`) at
//! any time. We embed only the already-PII-scrubbed `plaintext_summary` (the
//! same field FTS5 indexes), never the encrypted body.
//!
//! Vector store choice = **brute-force cosine over an f32 BLOB column in the
//! existing `events.sqlite`** — zero new Cargo deps. At personal scale (a few
//! thousand → tens of thousands of events) a pure-Rust dot-product loop is
//! sub-millisecond; an ANN index (hnsw / sqlite-vec) only pays off at 100k–1M+
//! vectors, which a single user never reaches.
//!
//! 中文: 語意記憶地基。把 recall 從純關鍵字升級成 hybrid。vector 只是 derived
//! index,canonical 真相永遠是加密 blob,events_emb 可隨時 reindex 重建。個人
//! 尺度下 brute-force cosine 就夠快,不上 ANN,且零新 Cargo dep。

pub mod ollama;

/// A typed embedding failure. Kept deliberately small + `String`-backed (like
/// `EventStoreError`) so the trait surface stays decoupled from reqwest/serde.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EmbedError {
    /// The embedding backend is unreachable (Ollama not running, connection
    /// refused, DNS, timeout). This is the **expected, non-fatal** case — the
    /// caller degrades to FTS5-only and never panics.
    #[error("embedding backend unavailable: {0}")]
    Unavailable(String),
    /// The backend answered but the response could not be parsed into vectors
    /// (bad JSON, missing `embedding` field, wrong shape).
    #[error("embedding response malformed: {0}")]
    BadResponse(String),
    /// The backend returned a non-2xx HTTP status (e.g. 404 = model not pulled).
    #[error("embedding backend error: {0}")]
    Backend(String),
    /// A vector came back with an unexpected dimension (model mismatch).
    #[error("embedding dim mismatch: expected {expected}, got {got}")]
    DimMismatch { expected: usize, got: usize },
}

/// Pluggable embedding source. Implementations turn text into fixed-dim float
/// vectors. `model_id` + `dim` are persisted alongside each stored vector so a
/// later model swap is detectable (stale rows are skipped, not silently
/// compared at the wrong dimension).
///
/// 中文: 可插拔的 embedding 來源。model_id + dim 入庫以偵測換模型造成的 stale。
pub trait EmbeddingProvider: Send + Sync {
    /// Stable identifier written into `events_emb.model_id` (e.g.
    /// `"nomic-embed-text"`). Used to detect stale rows after a model swap.
    fn model_id(&self) -> &str;
    /// The vector dimension this provider emits (e.g. 768 for nomic-embed-text).
    fn dim(&self) -> usize;
    /// Embed a batch of texts, returning one vector per input in order. A
    /// backend that has no batch endpoint may loop internally.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}

/// Cosine similarity of two equal-length vectors, in `[-1.0, 1.0]`. Returns
/// `0.0` for a length mismatch or a zero-magnitude vector (degenerate inputs
/// are "unrelated", never a panic / NaN). Identical vectors → `1.0`, orthogonal
/// → `0.0`, opposite → `-1.0`.
///
/// 中文: 餘弦相似度。長度不符或零向量回 0(視為不相關,不 panic / 不回 NaN)。
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Encode a `&[f32]` as little-endian bytes for the `events_emb.vec` BLOB
/// column. The inverse is [`decode_vec`]. Little-endian is fixed across
/// platforms so a db copied between machines decodes identically.
///
/// 中文: f32 向量 → little-endian bytes(存 BLOB)。跨平台固定 LE。
pub fn encode_vec(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Decode a little-endian f32 BLOB (written by [`encode_vec`]) back into a
/// `Vec<f32>`. Returns `None` if the byte length is not a multiple of 4 (a
/// truncated / corrupt row — skipped rather than panicking).
///
/// 中文: BLOB → f32 向量。長度非 4 倍數視為毀損回 None。
pub fn decode_vec(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(out)
}

/// Brute-force cosine top-k over the `events_emb` table. Decodes each row's
/// f32-BLOB vector, scores it against `qvec`, and returns the `k` highest by
/// cosine, descending. Rows whose stored `dim` differs from `qvec.len()` (a
/// model swap left them stale) are skipped — they can't be meaningfully
/// compared. Corrupt BLOBs (bad byte length) are skipped too.
///
/// Personal-scale brute force: a few thousand → tens of thousands of dot
/// products is sub-millisecond in pure Rust, so no ANN index is needed.
///
/// 中文: events_emb 全表 brute-force cosine top-k。dim 不符 / BLOB 毀損的列跳過。
pub fn brute_force_topk(
    qvec: &[f32],
    conn: &rusqlite::Connection,
    k: usize,
) -> rusqlite::Result<Vec<(String, f32)>> {
    let mut stmt = conn.prepare("SELECT event_id, dim, vec FROM events_emb")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    let mut scored: Vec<(String, f32)> = Vec::new();
    for r in rows {
        let (event_id, dim, blob) = r?;
        if dim as usize != qvec.len() {
            continue; // stale row from a different model / dimension
        }
        let Some(vec) = decode_vec(&blob) else {
            continue; // corrupt BLOB — skip, don't panic
        };
        if vec.len() != qvec.len() {
            continue;
        }
        scored.push((event_id, cosine(qvec, &vec)));
    }
    // Sort by cosine descending; NaN can't appear (cosine returns finite f32),
    // but guard with total_cmp anyway for a stable, panic-free ordering.
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(k);
    Ok(scored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_is_one_orthogonal_is_zero_opposite_is_minus_one() {
        let a = vec![1.0, 2.0, 3.0];
        // identical → 1.0
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-6, "identical = 1.0");
        // orthogonal → 0.0
        let x = vec![1.0, 0.0];
        let y = vec![0.0, 1.0];
        assert!(cosine(&x, &y).abs() < 1e-6, "orthogonal = 0.0");
        // opposite → -1.0
        let p = vec![1.0, 2.0, 3.0];
        let n = vec![-1.0, -2.0, -3.0];
        assert!((cosine(&p, &n) + 1.0).abs() < 1e-6, "opposite = -1.0");
    }

    #[test]
    fn cosine_degenerate_inputs_return_zero_not_nan() {
        assert_eq!(cosine(&[], &[]), 0.0, "empty = 0");
        assert_eq!(cosine(&[1.0, 2.0], &[1.0]), 0.0, "length mismatch = 0");
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0, "zero magnitude = 0");
    }

    #[test]
    fn f32_blob_round_trips() {
        let v = vec![0.0f32, -1.5, 3.14159, 1e-7, -2_000_000.0, f32::MIN_POSITIVE];
        let bytes = encode_vec(&v);
        assert_eq!(bytes.len(), v.len() * 4, "4 bytes per f32");
        let back = decode_vec(&bytes).expect("decode");
        assert_eq!(v, back, "exact round-trip (bit-identical f32)");
    }

    #[test]
    fn decode_rejects_unaligned_blob() {
        assert!(decode_vec(&[0u8, 1, 2]).is_none(), "3 bytes is not a multiple of 4");
        assert_eq!(decode_vec(&[]).unwrap(), Vec::<f32>::new(), "empty decodes to empty");
    }

    #[test]
    fn brute_force_topk_ranks_by_cosine_and_skips_stale_dims() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE events_emb(event_id TEXT PRIMARY KEY, dim INTEGER, \
             model_id TEXT, vec BLOB, indexed_at INTEGER);",
        )
        .unwrap();
        let insert = |id: &str, v: &[f32]| {
            conn.execute(
                "INSERT INTO events_emb(event_id, dim, model_id, vec, indexed_at) \
                 VALUES(?, ?, 'test', ?, 0)",
                rusqlite::params![id, v.len() as i64, encode_vec(v)],
            )
            .unwrap();
        };
        // Query is [1,0]. "near" points the same way; "far" points away; "stale"
        // is a different dim and must be skipped entirely.
        insert("near", &[1.0, 0.1]);
        insert("far", &[-1.0, 0.0]);
        insert("orth", &[0.0, 1.0]);
        insert("stale", &[1.0, 0.0, 0.0]); // dim=3, query is dim=2

        let q = vec![1.0f32, 0.0];
        let top = brute_force_topk(&q, &conn, 2).unwrap();
        assert_eq!(top.len(), 2, "k=2 caps the result");
        assert_eq!(top[0].0, "near", "highest cosine first");
        assert!(top[0].1 > top[1].1, "descending by cosine");
        // "stale" (dim=3) was never compared.
        assert!(top.iter().all(|(id, _)| id != "stale"), "stale dim skipped");
    }
}
