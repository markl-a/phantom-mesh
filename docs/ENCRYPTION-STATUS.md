# At-Rest Encryption Status (honest, per-store)

> Updated 2026-06-18. Purpose: the apex P4「加密為先」/ "encryption coverage closure" P1 deliverable —
> **document the real at-rest encryption status of every sensitive store so privacy claims are accurate, not misleading.**
> This is the SSOT for "what is encrypted at rest vs not, and why." Verified against origin/main code, not assumed.

## Posture

- **Crypto primitive (one, reused everywhere):** age v1, keyed by the device **EventKey** derived (HKDF-SHA256) from
  `~/.spectyn-mesh/identity.key` (ed25519). Sealing lives in `core/src/skillbank/memory_seal.rs` (`seal` / `open` / `is_sealed`);
  every encrypted store reuses it — **no per-store crypto is invented.**
- **Opt-in, default-OFF (for the toggleable stores):** the `SPECTYN_ENCRYPT_*` flags default OFF, so a shipping build is
  **byte-identical** to plaintext for existing users; turning a flag ON seals that store at rest. Reads are **fail-closed**
  (a sealed-but-undecryptable value returns an error — ciphertext is never surfaced as if it were the data).
- **Zero-knowledge cloud:** the optional broker/Relay only ever stores **sealed** blobs (the vault seal key itself is sealed).

## Status table

| Store | Holds | At-rest status | Flag / mechanism | Notes |
|---|---|---|---|---|
| `events/<id>/` (food, focus, notes, **multimodal captures** `modality_*.png`) | owned-memory events + their media | ✅ **Encrypted** (always) | age v1 via `EventStore::with_key` (E004) | The owned-memory media path IS sealed — read back through `store.read_file`. |
| `memory.db` (skill-memory FTS5 text/source) | owned-memory / skills | ✅ Encrypted (opt-in) | `SPECTYN_ENCRYPT_MEMORY` (P0-8) | FTS5 is fed a de-PII'd token form so keyword recall survives sealing; default-off. |
| `conversations/*.jsonl` | chat transcripts | ✅ Encrypted (opt-in) | `SPECTYN_ENCRYPT_CONVERSATIONS` | default-off. |
| `agents.toml` → `[providers.*].api_key` | provider **API keys** | ✅ Encrypted (opt-in) | `SPECTYN_ENCRYPT_AGENTS` (`skillbank/agents_seal.rs`) | default-off, fail-closed, seal-on-save / unseal-on-load with plaintext migration. Other secret-bearing fields (`[tools].*_api_key`, `[core].hub_api_key`) are **not yet** sealed — same pattern would extend. |
| Broker **vault seal key** (`WrappedVaultSealKey`) | the key that unlocks ZK-cloud blobs | ✅ Sealed | `broker_vault_wire` | The secret that actually unlocks data is protected. |
| `identity.key` (device root key) | the ed25519 device key | OS-keystore-wrapped where wired | Linux Secret Service ✅; **mac Keychain / win DPAPI / iOS Keychain / Android KeyStore = v0.7.0** | The root key; everything else derives from it. |
| `auth.json` (`broker_url` + `broker_token`) | broker auth token | ⬜ **Plaintext** | — | The broker is **zero-knowledge** — this token authenticates *to* the broker but does **not** unlock any data (the vault seal key, which does, is sealed). `spectyn logout` deletes it. Sealing it = operator-scope (bootstrap-order, low marginal value under ZK). |
| `captures/<ts>.png` (tool screenshots: `computer_use`, `image_gen`, simctl) | transient **tool output** | ⬜ Plaintext **by design** | — | Regular PNG artifacts with a user-overridable `path`, meant to be consumed by the user/other tools as images — **not owned-memory**. (Owned-memory captures live in `events/` and ARE encrypted, see row 1.) |
| Provider OAuth caches (`CodexAuth` / `GeminiCliAuth` / `GAuth`) | 3rd-party provider tokens | ⬜ Plaintext (external) | — | Managed by the respective provider CLIs, outside spectyn's store. |
| Logs (`events.jsonl` flight-recorder, `autoevolve.log`) | run/audit trail | ⬜ Plaintext | — | apex-④ plan: upgrade to **signed** (your-key) append-only; signing is the priority over encryption (you must be able to read "what it did"). Currently plaintext. |

Legend: ✅ encrypted · 🔐 OS-keystore-wrapped · ⬜ plaintext (with rationale).

## Honest claims (what you may and may NOT say)

- ✅ You MAY say: "owned-memory events + their media are encrypted at rest; opt-in flags seal memory.db / conversations /
  agents.toml API keys with your device key, fail-closed; the cloud is zero-knowledge."
- ❌ You may NOT say: "**everything** is encrypted at rest." It is not — see the ⬜ rows. The toggleable stores are
  **default-OFF** unless the user opts in.

## Verification

Each sealed store has at-rest tests asserting: sealed-on-disk (no plaintext leaks), round-trip (decrypts to original),
default-off byte-identical, and wrong/missing-key **fail-closed**. E.g. `core/tests/agents_toml_seal_at_rest.rs`
(round-trip / off-identical / missing-key / wrong-key), the memory_seal tests, and the kill-switch `spectyn data delete --all --yes`.

## Remaining (operator-scope, not autonomous)

- `auth.json`, `[tools]/[core]` secret fields, provider OAuth caches: seal vs explicitly-document-as-plaintext is a scope call.
- Log **signing** (apex-④) > log encryption.
- OS-keystore wrap for identity.key on mac / win / iOS / Android (v0.7.0).
