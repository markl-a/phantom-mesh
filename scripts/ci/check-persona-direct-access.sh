#!/usr/bin/env bash
# B6 / T87 anti-pattern guard.
#
# Channel-adapter modules under core/src/openclaw/ (telegram.rs, whatsapp.rs,
# slack.rs) MUST resolve per-channel persona greetings via
# `PersonaDispatcher::channel_intro` (or the underlying `Persona::channel_intro`
# resolver), never by indexing `persona.channels.get(...)` directly.
#
# Direct indexing is silently buggy: a `[persona.channels.slack]` block with
# `intro_message = ""` would return `Some("")` and ship an empty greeting,
# instead of falling through to the top-level `intro_message`. See
# `core/src/openclaw/persona.rs::Persona::channel_intro` for the contract.
#
# This script fails the build if any *non-test, non-dispatcher* code in
# core/src/openclaw/ contains the forbidden access pattern.

set -euo pipefail

scan_dir="core/src/openclaw"

if [ ! -d "$scan_dir" ]; then
    # Feature is opt-in; if the module hasn't been added yet, nothing to check.
    echo "openclaw module not present — skipping persona anti-pattern check"
    exit 0
fi

# Files exempt from the check:
#   - persona.rs        : owns the `channels` field (defines the resolver).
#   - dispatch.rs       : the *only* sanctioned consumer of `persona.channels`.
exempt=("persona.rs" "dispatch.rs")

violations=0
while IFS= read -r -d '' file; do
    fname="$(basename "$file")"
    skip=0
    for e in "${exempt[@]}"; do
        if [ "$fname" = "$e" ]; then
            skip=1
            break
        fi
    done
    if [ "$skip" -eq 1 ]; then
        continue
    fi

    # Strip `#[cfg(test)]` modules — tests may exercise the raw field for
    # round-trip assertions, but the guard is about production code.
    # Cheap heuristic: ignore matches inside lines containing "test" markers
    # by scanning the whole file but filtering out lines after `mod tests`
    # markers is more involved; instead we just look for direct usage in
    # *any* production line. Test modules in our channel files use the
    # `PersonaDispatcher` API, not raw indexing, so the pattern below will
    # only fire on real violations.
    # Strip line-comments before matching, then look for the forbidden
    # expression. This lets module docs reference the anti-pattern by name
    # without tripping the guard, while still catching real call-site usage.
    if sed -E 's://.*$::' "$file" \
        | grep -nE 'persona[[:space:]]*\.[[:space:]]*channels[[:space:]]*\.[[:space:]]*get\b' \
            >/dev/null 2>&1; then
        echo "::error file=$file::B6/T87 anti-pattern: \`persona.channels.get(...)\` found — use \`PersonaDispatcher::channel_intro\` instead. See core/src/openclaw/dispatch.rs for the rationale."
        sed -E 's://.*$::' "$file" \
            | grep -nE 'persona[[:space:]]*\.[[:space:]]*channels[[:space:]]*\.[[:space:]]*get\b' || true
        violations=$((violations + 1))
    fi
done < <(find "$scan_dir" -type f -name '*.rs' -print0)

if [ "$violations" -gt 0 ]; then
    echo ""
    echo "FAILED: $violations file(s) bypass PersonaDispatcher."
    echo "Routing fix: replace \`persona.channels.get(name)...\` with"
    echo "  PersonaDispatcher::new(Some(&persona)).channel_intro(name)"
    echo "so empty-override fallback semantics stay intact."
    exit 1
fi

echo "OK: no direct persona.channels.get(...) usage in openclaw channel adapters."
