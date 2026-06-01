//! V9 — ICU MessageFormat placeholder parity gate (Phase E Wave 18 /
//! SPEC-60 §8.9 / SPEC-05 §3.1 G3).
//!
//! Goal of this gate (from
//! `docs/superpowers/PHASE-E-INTEGRATION-TEST-PLAN.md` §3.9 b):
//!   - For every catalog key, the *set* of ICU placeholder names that
//!     appear in `zh-TW` must equal the set that appears in `en`.
//!
//! Why this matters
//! ----------------
//! ICU MessageFormat strings like `"Hello {name}"` (en) and `"你好
//! {name}"` (zh-TW) round-trip cleanly through the runtime resolver
//! because both reference the same `name` argument. The moment one
//! locale drops the placeholder — e.g. zh-TW = `"你好"` — the runtime
//! either renders a stranded literal or (worse) throws at format time
//! and falls back to the raw key. Either outcome violates SPEC-05 G3
//! ("plural / select / number / date render correctly") and breaks the
//! "no shame leak" half of the shame-free lint (SPEC-01 design
//! principle #5 — a hardcoded English-style fallback in zh-TW UI is a
//! shame-leak by definition).
//!
//! Placeholder grammar covered
//! ---------------------------
//! This file parses the subset of ICU MessageFormat that ships in
//! v0.6.0 catalogs:
//!   - simple: `{name}` / `{count}`
//!   - typed plural: `{count, plural, one {# meal} other {# meals}}`
//!   - typed select: `{role, select, admin {...} other {...}}`
//! For all three, the *name* (`name`, `count`, `role`) is what must
//! agree across locales — the inner branches do not have to align
//! word-for-word, but the placeholder *name set* must.
//!
//! Future locales (`ja`, `ko`, `de` — SPEC-05 §3.3 OoS1) will reuse
//! this same lint via the `pairwise_placeholder_diff` helper.
//!
//! No new Cargo deps: `serde_json` + `regex` are already in
//! `core/Cargo.toml`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use regex::Regex;

/// Resolve a fixture under `core/tests/fixtures/i18n/<name>` via
/// `CARGO_MANIFEST_DIR` so the test is invariant to cwd.
fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("i18n");
    p.push(name);
    p
}

fn load_catalog(name: &str) -> BTreeMap<String, String> {
    let path = fixture_path(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    serde_json::from_str::<BTreeMap<String, String>>(&raw)
        .unwrap_or_else(|e| panic!("fixture {} is not a flat catalog: {e}", path.display()))
}

/// Extract the *set* of ICU placeholder names from a translation
/// string. Matches the simple `{name}` form as well as the typed
/// `{name, plural, ...}` / `{name, select, ...}` forms by anchoring on
/// the first identifier after `{`. Returns a `BTreeSet` so set
/// equality is order-independent and diff output is deterministic.
fn placeholder_names(s: &str) -> BTreeSet<String> {
    // `[a-z_][a-z0-9_]*` matches the ICU argument-name grammar
    // (lowercase + underscore + digit, leading non-digit) — sufficient
    // for v0.6.0 keys which are all snake_case. We anchor on the first
    // identifier after `{` and require it to be followed by `}` (simple
    // form) or `,` (typed `{name, plural|select|...}` form). We do NOT try
    // to consume the typed suffix: ICU branches contain nested braces
    // (e.g. `{# item}`), which a `[^{}]*` run can't span — that bug made
    // every plural/select form fail to match. Capturing just the name is
    // all the parity lint needs; branch content is irrelevant.
    let re = Regex::new(r"\{([a-z_][a-z0-9_]*)\s*[},]")
        .expect("static regex compiles");
    re.captures_iter(s)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Walk both catalogs over the intersection of their key sets and
/// return a sorted list of `(key, zh_only, en_only)` triples for every
/// key whose placeholder sets diverge. Keys present in only one
/// catalog are out of scope for this lint — that's the parity test's
/// job (see `v9_i18n_string_parity.rs`).
fn pairwise_placeholder_diff(
    zh: &BTreeMap<String, String>,
    en: &BTreeMap<String, String>,
) -> Vec<(String, BTreeSet<String>, BTreeSet<String>)> {
    let mut out = Vec::new();
    for (key, zh_value) in zh {
        if let Some(en_value) = en.get(key) {
            let zh_set = placeholder_names(zh_value);
            let en_set = placeholder_names(en_value);
            if zh_set != en_set {
                let zh_only: BTreeSet<String> = zh_set.difference(&en_set).cloned().collect();
                let en_only: BTreeSet<String> = en_set.difference(&zh_set).cloned().collect();
                out.push((key.clone(), zh_only, en_only));
            }
        }
    }
    out
}

// ─── 1/5 — placeholder extractor handles all three ICU forms ───────────────

/// Pin the grammar this lint actually covers. If a future PR
/// introduces a placeholder form we do not parse (e.g. spaced names,
/// nested formats), this test must be updated *first* — otherwise the
/// parity lint will silently green-light a real mismatch.
#[test]
fn v9_placeholder_extractor_covers_icu_subset() {
    assert_eq!(
        placeholder_names("Hello {name}"),
        BTreeSet::from(["name".to_string()]),
        "simple {{name}} form"
    );

    assert_eq!(
        placeholder_names("You have {count, plural, one {# item} other {# items}}"),
        BTreeSet::from(["count".to_string()]),
        "typed plural form — only the name is captured, branches ignored"
    );

    assert_eq!(
        placeholder_names("Role: {role, select, admin {Admin} other {User}}"),
        BTreeSet::from(["role".to_string()]),
        "typed select form"
    );

    assert_eq!(
        placeholder_names("Hello {name}, you have {count} items"),
        BTreeSet::from(["name".to_string(), "count".to_string()]),
        "multiple distinct placeholders"
    );

    assert_eq!(
        placeholder_names("No placeholders here"),
        BTreeSet::<String>::new(),
        "plain string yields the empty set"
    );
}

// ─── 2/5 — happy path: balanced catalog passes ─────────────────────────────

/// The shipped baseline fixture pair (`zh-TW.json` + `en.json`) must
/// have zero placeholder drift. This is the gate we will repoint at
/// the production catalog once `app/src/lib/i18n/` ships (SPEC-05 §6.1
/// Stage 4 of `tauri_wire`).
#[test]
fn v9_baseline_catalogs_have_placeholder_parity() {
    let zh = load_catalog("zh-TW.json");
    let en = load_catalog("en.json");

    let diff = pairwise_placeholder_diff(&zh, &en);
    assert!(
        diff.is_empty(),
        "baseline catalogs must have zero placeholder drift, got: {diff:?}"
    );
}

// ─── 3/5 — detection: missing placeholder in one locale is caught ──────────

/// `coach.greeting` drops `{name}` in zh-TW; `error.auth_failed` adds
/// `{code}` in zh-TW that en lacks. Both shapes must be flagged.
/// This is the load-bearing test for the gate itself.
#[test]
fn v9_placeholder_lint_catches_missing_and_extra_placeholders() {
    let zh = load_catalog("zh-TW_placeholder_mismatch.json");
    let en = load_catalog("en_placeholder_mismatch.json");

    let diff = pairwise_placeholder_diff(&zh, &en);
    assert!(
        !diff.is_empty(),
        "placeholder-mismatch fixtures must surface at least one drift"
    );

    // Index the diff by key for ergonomic assertions.
    let by_key: BTreeMap<&String, &(String, BTreeSet<String>, BTreeSet<String>)> =
        diff.iter().map(|t| (&t.0, t)).collect();

    // Case A: zh-TW dropped `{name}` from `coach.greeting`.
    let (_, zh_only, en_only) = by_key
        .get(&"coach.greeting".to_string())
        .expect("coach.greeting must be flagged");
    assert!(
        zh_only.is_empty() && en_only.contains("name"),
        "coach.greeting: expected en_only={{name}}, got zh_only={zh_only:?} en_only={en_only:?}"
    );

    // Case B: zh-TW added `{code}` to `error.auth_failed`.
    let (_, zh_only, en_only) = by_key
        .get(&"error.auth_failed".to_string())
        .expect("error.auth_failed must be flagged");
    assert!(
        zh_only.contains("code") && en_only.is_empty(),
        "error.auth_failed: expected zh_only={{code}}, got zh_only={zh_only:?} en_only={en_only:?}"
    );

    // Sanity: `coach.daily_diff` has `{count}` on both sides, must NOT be flagged.
    assert!(
        !by_key.contains_key(&"coach.daily_diff".to_string()),
        "coach.daily_diff has matched placeholders and should pass"
    );
}

// ─── 4/5 — order-independence: {a}{b} == {b}{a} ────────────────────────────

/// Placeholder *order* may legitimately differ between locales because
/// natural-language word order does — e.g. en "{name} is {age}" vs
/// zh-TW "{age} 歲的 {name}". The lint asserts on the placeholder
/// *set*, not the sequence, so this case must pass.
#[test]
fn v9_placeholder_lint_is_order_independent() {
    let mut zh: BTreeMap<String, String> = BTreeMap::new();
    let mut en: BTreeMap<String, String> = BTreeMap::new();
    zh.insert("about.bio".to_string(), "{age} 歲的 {name}".to_string());
    en.insert("about.bio".to_string(), "{name} is {age}".to_string());

    let diff = pairwise_placeholder_diff(&zh, &en);
    assert!(
        diff.is_empty(),
        "placeholder order divergence must NOT be flagged (set equality only), got: {diff:?}"
    );
}

// ─── 5/5 — duplicate placeholders collapse to the same set ─────────────────

/// A locale may use the same placeholder twice — e.g. en "Welcome
/// {name}, glad to meet you {name}" — while the other uses it once.
/// Because the lint compares *sets*, this must not trigger a false
/// positive: the underlying argument list is still `{name}` on both
/// sides.
#[test]
fn v9_placeholder_lint_treats_duplicates_as_set_membership() {
    let mut zh: BTreeMap<String, String> = BTreeMap::new();
    let mut en: BTreeMap<String, String> = BTreeMap::new();
    zh.insert("welcome.repeat".to_string(), "你好 {name}".to_string());
    en.insert(
        "welcome.repeat".to_string(),
        "Welcome {name}, glad to meet you {name}".to_string(),
    );

    let diff = pairwise_placeholder_diff(&zh, &en);
    assert!(
        diff.is_empty(),
        "duplicate placeholders within one locale must not trigger drift, got: {diff:?}"
    );
}
