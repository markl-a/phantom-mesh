//! SPEC-03 §8 — cold-launch decision tree (SYS-B local-first propagation).
//!
//! On every cold launch the app routes to exactly one destination derived from
//! three on-disk booleans — `identity_exists` / `cluster_paired` / `onboarded`.
//! The keystone invariant (SPEC-03 G3, `T-info-arch-cold-launch-truth-table`)
//! is **no dead-end**: every one of the 8 flag combinations reaches an
//! *actionable* screen (`onboarding-pick` or `chat`) — never an error / offline
//! screen, and the function is total (no panic, no missing arm).
//!
//! SYS-B (operator-locked 2026-06-13, `design-decisions-2026-06-13.md`): being
//! chat-capable is a LOCAL property. The fully-provisioned state reaches `chat`
//! on a **local cluster with no broker / phantom account**. The decision tree
//! deliberately takes NO "broker"/"account" input, so a broker can never become
//! a precondition for reaching chat (the login-first dead-end SYS-B removes).
//!
//! This Rust function is the single source of truth the spec verifies; the GUI
//! router (`app/src/App.tsx`) and the FFI surfaces mirror it.
//!
//! 中文: SPEC-03 §8 冷啟動決策真值表。三個旗標(identity 存在 / cluster 已配對 /
//! onboarded)決定唯一目的地,8 種組合全部有出口、無死路。SYS-B 本地優先:完整
//! 佈建的本機(本地 cluster、無 broker)直達 `chat`;函式不收 broker 參數,所以
//! 「能聊天」永遠不被帳號/broker 把關。

/// The three cold-launch state flags (SPEC-03 §8). All are derived from on-disk
/// state at launch time:
/// - `identity_exists`: `~/.phantom-mesh/keys/ed25519.{priv,pub}` present.
/// - `cluster_paired`: a `[cluster]` membership is configured (local or broker).
/// - `onboarded`: the first-run wizard reached its terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColdLaunchFlags {
    pub identity_exists: bool,
    pub cluster_paired: bool,
    pub onboarded: bool,
}

impl ColdLaunchFlags {
    /// Construct from the three booleans (ordering matches the SPEC-03 §8 table
    /// columns: identity, cluster, onboarded).
    pub fn new(identity_exists: bool, cluster_paired: bool, onboarded: bool) -> Self {
        Self { identity_exists, cluster_paired, onboarded }
    }
}

/// The cold-launch destination. Only `OnboardingPick` and `Chat` are ever
/// produced by [`cold_launch_route`] per SPEC-03 §8. The error variants exist so
/// the "no dead-end" invariant is a real, *falsifiable* property: a regression
/// that routed a cold launch to an error/offline screen would be representable
/// here — and is caught by the truth-table test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColdLaunchRoute {
    /// The onboarding picker — the user chooses one of the three local-first
    /// paths (mint identity / pair cluster / set provider). Always actionable.
    OnboardingPick,
    /// The chat screen — fully provisioned, chat-capable. Reached on a LOCAL
    /// cluster without a broker (SYS-B).
    Chat,
    /// Dead-end: offline error screen (must NEVER be a cold-launch target).
    ErrorOffline,
    /// Dead-end: permission error screen (must NEVER be a cold-launch target).
    ErrorPermission,
    /// Dead-end: not-found error screen (must NEVER be a cold-launch target).
    ErrorNotFound,
}

impl ColdLaunchRoute {
    /// The stable kebab-case route slug (matches SPEC-03 §7.1 `RouteSlug`).
    pub fn slug(self) -> &'static str {
        match self {
            ColdLaunchRoute::OnboardingPick => "onboarding-pick",
            ColdLaunchRoute::Chat => "chat",
            ColdLaunchRoute::ErrorOffline => "error-offline",
            ColdLaunchRoute::ErrorPermission => "error-permission",
            ColdLaunchRoute::ErrorNotFound => "error-not-found",
        }
    }

    /// A dead-end: an error screen with no forward action a fresh launch can
    /// take. The cold-launch tree must NEVER return one of these.
    pub fn is_dead_end(self) -> bool {
        matches!(
            self,
            ColdLaunchRoute::ErrorOffline
                | ColdLaunchRoute::ErrorPermission
                | ColdLaunchRoute::ErrorNotFound
        )
    }

    /// Whether this destination lets the user actually start chatting.
    pub fn is_chat_capable(self) -> bool {
        matches!(self, ColdLaunchRoute::Chat)
    }
}

/// SPEC-03 §8 cold-launch decision tree. Total over all 8 flag combinations.
///
/// Only the fully-provisioned state (identity + cluster + onboarded) goes
/// straight to `chat`; every other combination funnels to the onboarding
/// picker, which is itself actionable (never a dead-end). Per SYS-B this holds
/// on a LOCAL cluster — no broker / account is an input, so chat is never gated
/// on one.
pub fn cold_launch_route(flags: ColdLaunchFlags) -> ColdLaunchRoute {
    match (flags.identity_exists, flags.cluster_paired, flags.onboarded) {
        // Fully provisioned → straight to chat. This holds on a LOCAL cluster
        // with no broker (SYS-B): no broker/account is an input, so chat is
        // never gated on one.
        (true, true, true) => ColdLaunchRoute::Chat,
        // Every other state funnels to the picker (the user chooses one of the
        // three local-first paths) — actionable, never a dead-end.
        _ => ColdLaunchRoute::OnboardingPick,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Enumerate the 8 combinations of the three booleans (SPEC-03 §8).
    fn all_eight_combos() -> Vec<ColdLaunchFlags> {
        let mut v = Vec::with_capacity(8);
        for identity_exists in [false, true] {
            for cluster_paired in [false, true] {
                for onboarded in [false, true] {
                    v.push(ColdLaunchFlags::new(identity_exists, cluster_paired, onboarded));
                }
            }
        }
        v
    }

    /// THE KEYSTONE (SPEC-03 G3 / `T-info-arch-cold-launch-truth-table`): every
    /// one of the 8 cold-launch flag combinations reaches an actionable screen.
    /// No combination is a dead-end (error/offline), and the function is total.
    #[test]
    fn cold_launch_truth_table_has_no_dead_end() {
        let combos = all_eight_combos();
        assert_eq!(combos.len(), 8, "must cover all 8 combinations");
        for f in combos {
            let r = cold_launch_route(f);
            assert!(
                !r.is_dead_end(),
                "SPEC-03 §8 dead-end for {f:?} → {r:?} (every cold-launch state must be actionable)"
            );
            assert!(
                matches!(r, ColdLaunchRoute::OnboardingPick | ColdLaunchRoute::Chat),
                "SPEC-03 §8 must route to onboarding-pick or chat for {f:?}, got {r:?}"
            );
        }
    }

    /// Exact per-row expectation from the SPEC-03 §8 decision table: 7 rows →
    /// `onboarding-pick`, only the all-true row → `chat`.
    #[test]
    fn cold_launch_matches_spec_03_section_8_table() {
        // (identity_exists, cluster_paired, onboarded) → expected route
        let cases = [
            ((false, false, false), ColdLaunchRoute::OnboardingPick),
            ((false, false, true), ColdLaunchRoute::OnboardingPick),
            ((false, true, false), ColdLaunchRoute::OnboardingPick),
            ((false, true, true), ColdLaunchRoute::OnboardingPick),
            ((true, false, false), ColdLaunchRoute::OnboardingPick),
            ((true, false, true), ColdLaunchRoute::OnboardingPick),
            ((true, true, false), ColdLaunchRoute::OnboardingPick),
            ((true, true, true), ColdLaunchRoute::Chat),
        ];
        for ((id, cl, ob), want) in cases {
            let got = cold_launch_route(ColdLaunchFlags::new(id, cl, ob));
            assert_eq!(got, want, "SPEC-03 §8 row (identity={id}, cluster={cl}, onboarded={ob})");
            assert_eq!(got.slug(), want.slug());
        }
    }

    /// SYS-B local-first lock: the fully-provisioned LOCAL node reaches a
    /// chat-capable destination. There is NO broker/account input to the
    /// decision, so being chat-capable can never be gated on a broker.
    #[test]
    fn chat_capability_is_local_first_no_broker_required() {
        let fully_local = ColdLaunchFlags::new(true, true, true);
        let r = cold_launch_route(fully_local);
        assert!(
            r.is_chat_capable(),
            "SYS-B: a fully-provisioned local node (no broker) must reach chat"
        );
        assert_eq!(r, ColdLaunchRoute::Chat);
    }

    /// The picker (the destination for the 7 non-provisioned states) is itself
    /// actionable; only the error screens are dead-ends.
    #[test]
    fn picker_and_chat_are_actionable_errors_are_dead_ends() {
        assert!(!ColdLaunchRoute::OnboardingPick.is_dead_end());
        assert!(!ColdLaunchRoute::Chat.is_dead_end());
        assert!(ColdLaunchRoute::ErrorOffline.is_dead_end());
        assert!(ColdLaunchRoute::ErrorPermission.is_dead_end());
        assert!(ColdLaunchRoute::ErrorNotFound.is_dead_end());
    }
}
