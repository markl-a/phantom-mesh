//! FEATURE-MATRIX `## 4.` → backlog-spec planner core (pure). Parses the real
//! per-feature worklist under `## 4. Full feature table (by SPEC group)`, keeps
//! only PARTIAL/STUB rows, derives build `caps` from the `### Group:` header and
//! the spec-gate `capability` from the pillar column, and emits the `[spec]`
//! envelope `scripts/dev-loop/spec-gate.sh` validates. The shell planner renders
//! these per-target blocks to .toml + posts them to the backlog.
//!
//! The matrix has OTHER tables (pillar/track/platform rollups) outside section 4
//! with different columns — the parser is section-bounded: it only scans rows
//! AFTER the `## 4.` header and stops at the next level-2 `## ` header (or EOF).

/// Code-status of a target we still emit work for. We deliberately DROP
/// `MISSING` this round (greenfield work needs human scoping, not an auto-spec),
/// so only the two "code exists but unfinished" states remain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixStatus {
    Partial,
    Stub,
}

/// One buildable target distilled from a section-4 feature row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixTarget {
    /// Bare spec number as it appears in col0, e.g. `"21"`.
    pub spec: String,
    /// col1 feature/name.
    pub name: String,
    /// col2 pillar cell raw, e.g. `"P2"`, `"P2/P4"`, `"cross-cut"`.
    pub pillar: String,
    /// col3 track.
    pub track: String,
    /// col4 platform/component cell — used as the spec-gate `component`.
    pub component: String,
    /// Leading code-status word of col5 (PARTIAL or STUB only).
    pub status: MatrixStatus,
    /// Build capabilities DERIVED from the `### Group:` header (there is NO caps
    /// column in the real matrix), e.g. `["macos"]`, `["windows"]`,
    /// `["windows", "android"]`.
    pub caps: Vec<String>,
    /// The `### Group:` name this row was found under, e.g. `"platform-mac"`.
    pub group: String,
}

/// Extract the leading run of ASCII-alphabetic chars from a cell, uppercased.
/// `"PARTIAL (orchestration real…)"` → `"PARTIAL"`, `"PARTIAL/MISSING"` →
/// `"PARTIAL"`, `"DONE-thin-shim"` → `"DONE"`, `"UNKNOWN/PARTIAL"` → `"UNKNOWN"`.
fn leading_status_word(cell: &str) -> String {
    cell.trim()
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Map a `### Group:` header line to its bare group name: the text after
/// `### Group: ` up to the first ` (` (or end), trimmed. Returns `None` if the
/// line is not a group header.
fn group_name(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("### Group:")?;
    let rest = rest.trim();
    let name = match rest.split_once(" (") {
        Some((head, _)) => head,
        None => rest,
    };
    Some(name.trim().to_string())
}

/// Derive build capabilities from the group name (and, for the platform-other
/// group, the row's track/component cells which may flag android).
fn caps_for_group(group: &str, track: &str, component: &str) -> Vec<String> {
    if group.starts_with("platform-mac") {
        vec!["macos".to_string()]
    } else if group.starts_with("platform-ios") {
        vec!["ios".to_string()]
    } else if group.starts_with("platform-other") {
        let mut caps = vec!["windows".to_string()];
        let hay = format!("{track} {component}").to_ascii_lowercase();
        if hay.contains("android") {
            caps.push("android".to_string());
        }
        caps
    } else {
        // foundation / protocol-interop / system-life / system-work (shared-core,
        // cross-platform) and any unknown group → the 3 Windows nodes build Rust.
        vec!["windows".to_string()]
    }
}

/// Map a pillar cell to the spec-gate `capability` (one of sense|learn|dispatch;
/// `nudge` = ③proactive is out of scope this round, so it is never emitted). Uses
/// the FIRST `P1`/`P2`/`P3`/`P4` found anywhere in the string; defaults to
/// `dispatch` when no P-number is present (`cross-cut`, `meta`, …).
pub fn capability_for(pillar: &str) -> &'static str {
    let bytes = pillar.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'P' {
            match bytes[i + 1] {
                b'1' => return "dispatch",
                b'2' => return "sense",
                b'3' => return "learn",
                b'4' => return "dispatch",
                _ => {}
            }
        }
        i += 1;
    }
    "dispatch"
}

/// Parse the FEATURE-MATRIX markdown; return only the PARTIAL/STUB rows under
/// `## 4. Full feature table`. Section-bounded and group-aware: scanning starts
/// after the `## 4.` header and stops at the next level-2 `## ` header (or EOF).
pub fn parse_matrix_targets(md: &str) -> Vec<MatrixTarget> {
    let mut out = Vec::new();
    let mut in_section = false;
    let mut group = String::new();

    for line in md.lines() {
        let trimmed = line.trim();

        // Section boundary handling. We accept `## 4.` (and only that) as the
        // entry, and ANY other later `## ` header as the exit.
        if trimmed.starts_with("## ") {
            if trimmed.starts_with("## 4.") {
                in_section = true;
                continue;
            } else if in_section {
                break; // a later level-2 header ends section 4
            }
            continue;
        }
        if !in_section {
            continue;
        }

        if let Some(g) = group_name(trimmed) {
            group = g;
            continue;
        }

        if !trimmed.starts_with('|') {
            continue;
        }
        let cells: Vec<String> = trimmed
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect();
        if cells.len() < 7 {
            continue;
        }
        // col0 must be a bare integer — skips the `SPEC` header row, the `|---|`
        // separator, and the rare `(adj)` non-spec row.
        let Ok(_) = cells[0].parse::<u32>() else {
            continue;
        };

        let status = match leading_status_word(&cells[5]).as_str() {
            "PARTIAL" => MatrixStatus::Partial,
            "STUB" => MatrixStatus::Stub,
            _ => continue, // DONE / MISSING / UNKNOWN / REAL / … → not a target
        };

        let caps = caps_for_group(&group, &cells[3], &cells[4]);
        out.push(MatrixTarget {
            spec: cells[0].clone(),
            name: cells[1].clone(),
            pillar: cells[2].clone(),
            track: cells[3].clone(),
            component: cells[4].clone(),
            status,
            caps,
            group: group.clone(),
        });
    }
    out
}

/// Lower-kebab id of `spec-<spec>-<name>`, collapsing any non-alphanumeric run
/// to a single `-` and trimming. spec="21", name="Focus wire contract" →
/// `"spec-21-focus-wire-contract"`.
pub fn target_slug(t: &MatrixTarget) -> String {
    let raw = format!("spec-{}-{}", t.spec, t.name);
    let mut s = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            s.push('-');
            prev_dash = true;
        }
    }
    s.trim_matches('-').to_string()
}

/// Make a string safe to embed inside a single-line double-quoted TOML scalar:
/// drop any embedded newlines and replace `"` with `'` so the closing quote is
/// never reached early (spec-lib's `spec_val` strips at the first `"`).
fn one_line_quoted(s: &str) -> String {
    s.replace(['\n', '\r'], " ").replace('"', "'")
}

/// Render a backlog spec .toml with the `[spec]` envelope spec-gate validates.
///
/// CRITICAL: spec-lib.sh cannot read a `"""` multi-line string nor a multi-line
/// array — `acceptance`, `caps`, and `scope_allow` are ALL single-line, and the
/// acceptance text is escaped so it stays a valid single-line quoted scalar.
pub fn render_spec_toml(t: &MatrixTarget, scope_allow: &[String]) -> String {
    let capability = capability_for(&t.pillar);
    let component = one_line_quoted(&t.component);
    let caps = t
        .caps
        .iter()
        .map(|c| format!("\"{}\"", one_line_quoted(c)))
        .collect::<Vec<_>>()
        .join(", ");
    let scope = scope_allow
        .iter()
        .map(|p| format!("\"{}\"", one_line_quoted(p)))
        .collect::<Vec<_>>()
        .join(", ");
    let status = match t.status {
        MatrixStatus::Partial => "PARTIAL",
        MatrixStatus::Stub => "STUB",
    };
    // Single-line acceptance: spec number + feature name + status + the standard
    // constraints. MUST stay one line (spec-lib.sh reads up to the first quote).
    let acceptance = one_line_quoted(&format!(
        "SPEC-{spec} ({name}) is currently {status}. Bring it to DONE+TESTED. \
         Constraints: edit ONLY files in scope_allow; verify-first; TDD red->green; \
         real exit-code gate; ENGLISH in code; do not touch unrelated files.",
        spec = t.spec,
        name = t.name,
    ));
    format!(
        "[spec]\n\
         capability = \"{capability}\"\n\
         component = \"{component}\"\n\
         caps = [{caps}]\n\
         scope_allow = [{scope}]\n\
         max_files = 3\n\
         acceptance = \"{acceptance}\"\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // A realistic fixture mirroring the real file: an earlier non-section-4
    // table (pillar rollup) that MUST be ignored, then `## 4.` with two real
    // group blocks (platform-mac, system-life) plus a platform-other row whose
    // track mentions android, plus DONE/MISSING rows that must be skipped, then
    // a later `## 5.` header that must end the scan.
    const FIXTURE: &str = "\
## 2. Pillar rollup
| pillar | done | partial |
|---|---|---|
| P2 | 3 | 4 |

## 4. Full feature table (by SPEC group)

### Group: platform-mac (SPEC-40..41)
| SPEC | feature | pillar | track | platform | code status | test status |
|---|---|---|---|---|---|---|
| 40 | menu-bar tray | P2 | Mac | macos-app | PARTIAL (tray real; popover TODO) | WEAK (smoke only) |
| 41 | launchd helper | P1 | Mac | macos-app | STUB (plist gen stub) | NONE |
| 40 | code-sign + notarize | P1 | Mac | macos-app | DONE (real notarize) | DONE+TESTED |

### Group: system-life (SPEC-20..25)
| SPEC | feature | pillar | track | platform | code status | test status |
|---|---|---|---|---|---|---|
| 21 | Focus wire contract | P2 | Life | shared-core | PARTIAL | WEAK (serde + smoke) |
| 22 | Habit \"quoted\" data | P2 | Life | shared-core | MISSING | NONE |

### Group: platform-other-server-test (SPEC-33/34/44/45/46/50/51/52/60/61/62/63)
| SPEC | feature | pillar | track | platform | code status | test status |
|---|---|---|---|---|---|---|
| 50 | android worker bridge | P4 | android | android-app | PARTIAL (jni real; lifecycle TODO) | WEAK |

## 5. Critical gaps
| spec | gap |
|---|---|
| 99 | not a real row | P2 | x | y | PARTIAL | z |
";

    #[test]
    fn keeps_only_partial_and_stub_rows_in_section_4() {
        let rows = parse_matrix_targets(FIXTURE);
        // platform-mac: 40 PARTIAL + 41 STUB (40 DONE skipped);
        // system-life: 21 PARTIAL (22 MISSING skipped);
        // platform-other: 50 PARTIAL.
        // The pillar-rollup table (section 2) and the section-5 row are ignored.
        assert_eq!(rows.len(), 4, "got {rows:#?}");
        let specs: Vec<&str> = rows.iter().map(|r| r.spec.as_str()).collect();
        assert_eq!(specs, vec!["40", "41", "21", "50"]);
    }

    #[test]
    fn platform_mac_row_derives_macos_caps_and_fields() {
        let rows = parse_matrix_targets(FIXTURE);
        let r = &rows[0];
        assert_eq!(r.spec, "40");
        assert_eq!(r.name, "menu-bar tray");
        assert_eq!(r.component, "macos-app");
        assert_eq!(r.group, "platform-mac");
        assert_eq!(r.caps, vec!["macos".to_string()]);
        assert_eq!(r.status, MatrixStatus::Partial);
        // the STUB row
        assert_eq!(rows[1].status, MatrixStatus::Stub);
    }

    #[test]
    fn system_life_row_derives_windows_caps() {
        let rows = parse_matrix_targets(FIXTURE);
        let r = rows.iter().find(|r| r.spec == "21").unwrap();
        assert_eq!(r.group, "system-life");
        assert_eq!(r.caps, vec!["windows".to_string()]);
        assert_eq!(r.name, "Focus wire contract");
    }

    #[test]
    fn platform_other_android_row_has_windows_and_android_caps() {
        let rows = parse_matrix_targets(FIXTURE);
        let r = rows.iter().find(|r| r.spec == "50").unwrap();
        assert!(r.caps.contains(&"windows".to_string()), "{:?}", r.caps);
        assert!(r.caps.contains(&"android".to_string()), "{:?}", r.caps);
    }

    #[test]
    fn capability_maps_first_p_number_else_dispatch() {
        assert_eq!(capability_for("P2"), "sense");
        assert_eq!(capability_for("P3"), "learn");
        assert_eq!(capability_for("P4/P1"), "dispatch");
        assert_eq!(capability_for("P1/P4"), "dispatch");
        assert_eq!(capability_for("cross-cut"), "dispatch");
        assert_eq!(capability_for("meta"), "dispatch");
    }

    #[test]
    fn slug_is_lower_kebab_of_spec_and_name() {
        let t = MatrixTarget {
            spec: "21".into(),
            name: "Focus wire contract".into(),
            pillar: "P2".into(),
            track: "Life".into(),
            component: "shared-core".into(),
            status: MatrixStatus::Partial,
            caps: vec!["windows".into()],
            group: "system-life".into(),
        };
        assert_eq!(target_slug(&t), "spec-21-focus-wire-contract");
    }

    #[test]
    fn rendered_spec_is_single_line_and_gate_shaped() {
        let t = MatrixTarget {
            spec: "21".into(),
            name: "Focus wire \"contract\"".into(), // intentionally has a quote
            pillar: "P2".into(),
            track: "Life".into(),
            component: "shared-core".into(),
            status: MatrixStatus::Partial,
            caps: vec!["windows".into()],
            group: "system-life".into(),
        };
        let toml = render_spec_toml(&t, &["core/src/*focus*".to_string()]);
        assert!(toml.contains("capability = \"sense\""), "{toml}");
        assert!(toml.contains("component = \"shared-core\""), "{toml}");
        assert!(toml.contains("caps = [\"windows\"]"), "{toml}");
        assert!(
            toml.contains("scope_allow = [\"core/src/*focus*\"]"),
            "{toml}"
        );

        // Find the acceptance line and assert it is a SINGLE line that starts
        // and ends with `"`, with no `"""` and no interior newline.
        let acc = toml
            .lines()
            .find(|l| l.starts_with("acceptance = "))
            .expect("acceptance line present");
        assert!(!acc.contains("\"\"\""), "no triple-quote: {acc}");
        let val = acc.strip_prefix("acceptance = ").unwrap();
        assert!(val.starts_with('"') && val.ends_with('"'), "{val}");
        // The quote inside the source name must have been neutralised to a single
        // quote so the scalar is valid (only the two delimiter quotes remain).
        assert_eq!(
            val.matches('"').count(),
            2,
            "exactly the 2 delimiters: {val}"
        );
        assert!(!acc.contains('\n'));
    }
}
