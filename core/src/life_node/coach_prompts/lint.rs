//! Coach output linter — shame-free (SPEC-23 G3) + medical-disclaimer (G9).
//!
//! BIG-GOAL Operational Principle #1: Coach tone never blames, sarcasms,
//! or judges. SPEC-23 §G3: reject "你又..." / "你終於..." / "you failed
//! again" style patterns (zh + en), **fail-closed** (one match rejects the
//! WHOLE review; the user sees "still cooking", never shaming output).
//! SPEC-23 §G9 (medical disclaimer, NG2): the coach must not emit
//! diagnosis / prescription advice — an output-side blocklist intercepts
//! EXPLICIT prescription/diagnosis wording ("處方" / "diagnosis" / "prescribe"
//! / "dosage"). Informal drug advice ("你該吃止痛藥") is NOT caught here (it
//! needs drug-name awareness, not substrings) — that is deferred to the
//! fixture-backed follow-up; see the SCOPE note on `MEDICAL_PATTERNS` below.
//!
//! Apply via `check(text)` on Coach prompt templates AND on generated
//! Coach output BEFORE delivery (file/Telegram/email/push).
//!
//! Precision matters: this gate is fail-closed, so an over-broad pattern
//! silently rejects clean reviews. Every pattern below is chosen to be
//! vanishingly unlikely in a supportive one-line "tomorrow try X" action
//! while still catching the shaming / medical-overreach it targets — and the
//! tests assert a battery of clean zh+en coach outputs all PASS.

/// Shame-free (G3) + medical-disclaimer (G9) lint over Coach output.
///
/// Returns `Ok(())` for clean text; `Err(reason)` on the first match. The
/// `reason` is prefixed `shame leakage` or `medical disclaimer` so callers can
/// map to the SPEC-23 §degraded reasons (`shame_leakage` / `medical_disclaimer_hit`)
/// and includes the byte offset (into the lower-cased haystack) for debugging.
/// Matching is ASCII-case-insensitive so capitalised LLM output ("Prescription")
/// is still caught; CJK patterns are unaffected by case folding.
pub fn check(text: &str) -> Result<(), String> {
    // SPEC-23 G3 — shame-free patterns (zh + en). HIGH-PRECISION only: every
    // entry is chosen so it cannot fire on a supportive one-line action.
    // NOTE the `還不去` / `還不快` narrowing: the old bare `還不` was a latent
    // false positive — it is a substring of the common POSITIVE phrases `還不錯`
    // (pretty good) / `還不賴` (not bad), so a clean review like "今天的選擇還不錯"
    // was being silently rejected by this fail-closed gate. The imperative-shame
    // forms ("還不去運動" / "還不快去睡") are matched specifically instead.
    const SHAME_PATTERNS: &[(&str, &str)] = &[
        ("你又", "blame: \"你又...\" implies recurring failure"),
        ("你終於", "sarcasm: \"你終於...\" implies prior repeated failure"),
        ("你居然", "judgment: \"你居然...\" implies disbelief at user's choice"),
        ("你怎麼又", "compound-blame: \"你怎麼又...\""),
        ("還不去", "imperative-shame: \"還不去...\" (commanding tone)"),
        ("還不快", "imperative-shame: \"還不快...\" (commanding tone)"),
        ("you failed again", "blame: recurring-failure framing"),
        ("you never learn", "judgment: implies the user is hopeless"),
        ("shame on you", "shaming: direct shaming"),
        ("you should be ashamed", "shaming: direct shaming"),
        ("what's wrong with you", "judgment: belittling framing"),
    ];
    // SPEC-23 G9 / NG2 — medical-disclaimer blocklist. Coach gives NO diagnosis /
    // prescription advice; the safe phrasing ("talk to your doctor" / "建議跟醫生
    //談") is deliberately NOT blocked (it contains none of these). HIGH-PRECISION
    // only: these terms appear in explicit prescription/diagnosis language but not
    // in benign small-actions OR in medication-adherence reminders.
    //
    // SCOPE (honest): this catches *explicit* prescription/diagnosis wording. It
    // does NOT yet catch informal drug advice like "你該吃止痛藥" (which needs
    // drug-name awareness, not substrings) — broadening that safely requires the
    // SPEC-23 G3/G9 mandated 100-shaming / 100-clean CI fixture + product
    // sign-off; tracked as a follow-up. The system-prompt medical disclaimer
    // (templates.rs) remains the primary guard; this is defence-in-depth.
    const MEDICAL_PATTERNS: &[(&str, &str)] = &[
        ("處方", "medical: prescription (處方)"),
        ("診斷", "medical: diagnosis (診斷)"),
        ("prescription", "medical: prescription"),
        ("prescribe", "medical: prescribing"),
        ("diagnos", "medical: diagnose/diagnosis"),
        ("dosage", "medical: dosage advice"),
    ];
    let lower = text.to_lowercase();
    for (pat, why) in SHAME_PATTERNS {
        if let Some(idx) = lower.find(pat) {
            return Err(format!("shame leakage at byte offset {}: {}", idx, why));
        }
    }
    for (pat, why) in MEDICAL_PATTERNS {
        if let Some(idx) = lower.find(pat) {
            return Err(format!("medical disclaimer at byte offset {}: {}", idx, why));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn rejects_shame_patterns_zh_and_en() {
        // SPEC-23 G3: all 10 zh+en shame patterns must fail-closed.
        let dirty = [
            "你又吃垃圾食物了",
            "你終於做了一件正確的事",
            "你居然會考慮這個選項",
            "你怎麼又熬夜",
            "還不去運動",
            "還不快去睡",
            "You failed again at your focus blocks.",
            "You never learn, do you?",
            "Honestly, shame on you.",
            "You should be ashamed of that choice.",
            "What's wrong with you?",
        ];
        for s in &dirty {
            let r = check(s);
            assert!(r.is_err(), "expected reject for: {}", s);
            assert!(
                r.unwrap_err().starts_with("shame leakage"),
                "shame reason for: {}",
                s
            );
        }
    }

    #[test]
    fn rejects_medical_disclaimer_patterns_zh_and_en() {
        // SPEC-23 G9 / NG2: no diagnosis / prescription / treatment advice.
        let dirty = [
            "這是醫生開的處方",
            "我的診斷是偏頭痛",
            "Here is a prescription for you.",
            "I would prescribe rest and ibuprofen.",
            "My diagnosis: you have a migraine.",
            "Increase the dosage to 200mg.",
        ];
        for s in &dirty {
            let r = check(s);
            assert!(r.is_err(), "expected reject for: {}", s);
            assert!(
                r.unwrap_err().starts_with("medical disclaimer"),
                "medical reason for: {}",
                s
            );
        }
    }

    #[test]
    fn accepts_clean_supportive_output_zh_and_en() {
        // Fail-closed gate: clean coach output (incl. the DESIRED medical-safe
        // phrasing "talk to your doctor" / "建議跟醫生談") must all PASS — these
        // guard against false positives that would silently drop a clean review.
        let clean = [
            "今天三餐熱量在目標範圍內",
            "明天可以試試早上 10 分鐘散步",
            "Caesar salad 是 fat_loss 軌道內的好選擇",
            "今天比昨天的選擇好在哪",
            // Regression: "還不錯"/"還不賴" (pretty good / not bad) are POSITIVE —
            // the old bare `還不` pattern wrongly rejected these. Must pass now.
            "今天的選擇還不錯，明天保持",
            "你的專注時段進步還不賴",
            // G9 desired phrasing: defer to a doctor, no prescription — must pass.
            "連續三天頭痛是顯著的，建議跟你的醫生提一下",
            "Try a 10-minute walk tomorrow morning.",
            "You kept three focus blocks today — nice work.",
            "Three days of headaches is notable; consider mentioning it to your doctor.",
            "Tomorrow, aim for a glass of water before each meal.",
            "",
        ];
        for s in &clean {
            assert!(check(s).is_ok(), "expected accept for: {}", s);
        }
    }
}
