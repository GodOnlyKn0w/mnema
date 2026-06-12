/// Context-command family: cmd_context plus pure context projection helpers.
/// Moved from main.rs (Layer 4c-context refactor).
use crate::journal::*;
use crate::projection;
use crate::shorten;

// ── Context projection ───────────────────────────────────
// Context projection layer.
// MUST NOT shell out to tasktree subcommands.
// Uses projection::project_strands() directly.
// See protocols/system-prompt-design.md §三 for rationale.

/// Pairing result for a single LogEntry index: the index of the friction
/// entry it resolves (when the entry is `[fixed]`), or the index of the
/// `[fixed]` entry that resolves it (when the entry is `[friction]`).
/// Used by `pair_frictions` to communicate which entries are paired.
///
/// Returned as a bitset: paired_friction_indices + paired_fixed_indices so
/// callers need only one pass.
pub(crate) struct FrictionPairing {
    /// Indices (into the log slice) of [friction] entries that are paired.
    pub(crate) paired_friction: std::collections::HashSet<usize>,
    /// Indices (into the log slice) of [fixed] entries that are paired.
    pub(crate) paired_fixed: std::collections::HashSet<usize>,
    /// For each paired friction index, the truncated content for the scar line.
    /// Key: friction log index; Value: "<friction truncated 50> → fixed"
    pub(crate) scar_content: std::collections::HashMap<usize, String>,
    /// Dangling fix references: (log_index_of_fixed_entry, fixes= prefix string).
    /// A dangling fix is a [fixed] entry whose `fixes=<prefix>` does not match
    /// any [friction] entry in this log (nonexistent id or non-friction target).
    /// These emit W075 warnings; the [fixed] entry is not folded.
    pub(crate) dangling_fixes: Vec<(usize, String)>,
}

/// Compute friction↔fixed pairing for a strand's log entries.
///
/// Rules (deterministic, engine-enforced):
///   1. Explicit reference only: if a `[fixed]` entry's content contains a token
///      `fixes=<prefix>` (prefix >= 8 hex chars), match the first unpaired
///      `[friction]` entry in the same log whose `append_id` starts with that
///      prefix.
///   2. `[fixed]` with no `fixes=` token: treated as a plain annotation —
///      not folded, not paired, no warning. Degrades gracefully.
///   3. `[fixed]` with a `fixes=` prefix that matches no [friction] entry:
///      dangling fix — recorded in `dangling_fixes` for W075 emission.
///      The [fixed] entry is not folded (exposed as normal annotation).
///   4. `[friction]` entries with no corresponding `[fixed]` remain unpaired
///      and are exposed full-text (live strand) or folded (closed strand).
///   5. Strict 1-1: one [fixed] pairs with at most one [friction].
///
/// Proximity inference is intentionally absent — the close-command footgun
/// taught that implicit matching creates ambiguous history. All pairing is
/// explicit via `fixes=`.
///
/// This function is pure: it only reads `log`, never writes to the journal.
pub(crate) fn pair_frictions(log: &[projection::LogEntry]) -> FrictionPairing {
    use std::collections::{HashMap, HashSet};

    // append_id → log index, for explicit `fixes=` lookup.
    // We index all friction entries by their append_id for O(n) lookup.
    let mut friction_by_append_id: Vec<(String, usize)> = Vec::new();

    let mut paired_friction: HashSet<usize> = HashSet::new();
    let mut paired_fixed: HashSet<usize> = HashSet::new();
    let mut scar_content: HashMap<usize, String> = HashMap::new();
    let mut dangling_fixes: Vec<(usize, String)> = Vec::new();

    // First pass: collect friction indices and their append_ids
    for (idx, entry) in log.iter().enumerate() {
        if entry.content.starts_with("[friction]") {
            if let Some(ref aid) = entry.append_id {
                if !aid.is_empty() {
                    friction_by_append_id.push((aid.clone(), idx));
                }
            }
        }
    }

    // Second pass: process [fixed] entries — explicit fixes= only
    for (idx, entry) in log.iter().enumerate() {
        if !entry.content.starts_with("[fixed]") {
            continue;
        }

        // Extract the first `fixes=<prefix>` token (if any)
        let fixes_prefix: Option<String> = {
            let body = entry.content.trim_start_matches("[fixed]").trim();
            let mut found = None;
            for token in body.split_whitespace() {
                if let Some(prefix) = token.strip_prefix("fixes=") {
                    if prefix.len() >= 8 {
                        found = Some(prefix.to_string());
                    }
                    break; // only first fixes= token, even if too short
                }
            }
            found
        };

        let prefix = match fixes_prefix {
            None => continue, // no fixes= token → plain annotation, skip
            Some(p) => p,
        };

        // Find the first unpaired friction whose append_id starts with prefix
        let matched: Option<usize> = friction_by_append_id.iter().find_map(|(aid, fidx)| {
            if aid.starts_with(prefix.as_str()) && !paired_friction.contains(fidx) {
                Some(*fidx)
            } else {
                None
            }
        });

        match matched {
            Some(fidx) => {
                // Build scar content: friction text truncated at 50 chars → fixed
                let friction_body = log[fidx].content
                    .trim_start_matches("[friction]")
                    .trim();
                let truncated: String = friction_body.chars().take(50).collect();
                let scar = format!("{} → fixed", truncated);

                paired_friction.insert(fidx);
                paired_fixed.insert(idx);
                scar_content.insert(fidx, scar);
            }
            None => {
                // fixes= prefix matched nothing → dangling fix
                dangling_fixes.push((idx, prefix));
            }
        }
    }

    FrictionPairing { paired_friction, paired_fixed, scar_content, dangling_fixes }
}

/// Pure projection for context (testable without stdout capture).
///
/// Exposure axis (scaffolding ADR-0002): what still binds the future is
/// exposed by default. [friction] entries on a live (registered) strand are
/// included full-text; on a closed strand they fold into `friction_folded`
/// (a scar, not a disappearance — retrieve with `show`). `--exclude-friction`
/// drops them entirely: hiding is an explicit choice, exposure the default.
/// `include_observations`: when true, [progress]/[observed]/[check] entries are
/// exposed full-text (tail folding disabled). When false (default), only the most
/// recent entry per marker type is kept; the rest are counted in `folded_counts`.
pub(crate) fn build_context_strands(
    strands: &[projection::ProjectedStrand],
    target_type: &str,
    covers: &[String],
    since_offset: Option<usize>,
    exclude_friction: bool,
    include_observations: bool,
) -> Vec<ContextStrandOutput> {
    // Filter strands by type
    let mut matching: Vec<&projection::ProjectedStrand> = strands
        .iter()
        .filter(|s| s.strand_type.as_deref() == Some(target_type))
        .collect();

    // Filter by --since-offset
    if let Some(so) = since_offset {
        matching.retain(|s| s.last_offset() > so);
    }

    // Build output structures
    let mut output_strands: Vec<ContextStrandOutput> = Vec::new();

    // Observation-class markers subject to tail-folding
    const OBS_MARKERS: [&str; 3] = ["[progress]", "[observed]", "[check]"];

    for strand in &matching {
        // Collect [covers] entries (only entries that START with [covers])
        let covers_list: Vec<String> = strand
            .log
            .iter()
            .filter(|e| e.content.starts_with("[covers]"))
            .map(|e| e.content.trim_start_matches("[covers]").trim().to_string())
            .collect();

        // --covers filter: check if any [covers] entry contains one of the paths
        if !covers.is_empty() {
            let has_match = covers_list.iter().any(|c| {
                covers.iter().any(|p| c.contains(p.as_str()))
            });
            if !has_match {
                continue;
            }
        }

        let strand_is_live = strand.state() == "registered";
        let mut friction_folded = 0usize;

        // ── A. friction↔fixed pairing (live strands only) ──────────────
        // On closed strands, all friction folds via friction_folded count; pairing
        // doesn't affect that (the line is dead regardless).
        let pairing = if strand_is_live && !exclude_friction {
            pair_frictions(&strand.log)
        } else {
            FrictionPairing {
                paired_friction: std::collections::HashSet::new(),
                paired_fixed: std::collections::HashSet::new(),
                scar_content: std::collections::HashMap::new(),
                dangling_fixes: Vec::new(),
            }
        };
        // ── W075: emit dangling fix warnings ───────────────────────────
        // A [fixed] entry with fixes=<prefix> that matched no [friction] entry.
        // Fired on every projection pass where the dangling fix is present.
        // This is a precision-first check: no false positives (only exact mismatches).
        for (fix_idx, prefix) in &pairing.dangling_fixes {
            let fix_entry = &strand.log[*fix_idx];
            eprintln!(
                "W075: [fixed] fixes={} in strand {} does not match any [friction] entry \
                 (append_id offset {}) (tasktree explain W075)",
                &prefix[..prefix.len().min(12)],
                shorten(&strand.id),
                fix_entry.offset,
            );
        }
        let friction_paired = pairing.paired_friction.len();

        // ── B. observation-class tail-folding pre-pass ──────────────────
        // For each obs marker, find the index of the LAST occurrence in the log
        // (that is the tail to keep). All earlier occurrences are folded.
        let mut last_obs_idx: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        if !include_observations {
            for (idx, entry) in strand.log.iter().enumerate() {
                for &om in &OBS_MARKERS {
                    if entry.content.starts_with(om) {
                        last_obs_idx.insert(om, idx);
                    }
                }
            }
        }

        // ── C. Build entry list ─────────────────────────────────────────
        let mut folded_counts = FoldedCounts::zero();

        let entries: Vec<ContextEntryOutput> = strand
            .log
            .iter()
            .enumerate()
            .filter_map(|(idx, e)| {
                // ── [friction] handling ────────────────────────────────
                if e.content.starts_with("[friction]") {
                    if exclude_friction {
                        return None;
                    }
                    if !strand_is_live {
                        // Closed strand: fold to count.
                        friction_folded += 1;
                        return None;
                    }
                    // Live strand: check if paired
                    if pairing.paired_friction.contains(&idx) {
                        // Emit scar entry instead of full text
                        let scar = pairing.scar_content.get(&idx).cloned()
                            .unwrap_or_else(|| "→ fixed".to_string());
                        return Some(ContextEntryOutput {
                            marker: "[friction]".to_string(),
                            content: scar,
                            offset: e.offset,
                            ts: e.ts.clone(),
                        });
                    }
                    // Unpaired friction: expose full text
                    let (marker, content) = extract_marker(&e.content);
                    return Some(ContextEntryOutput {
                        marker: marker.to_string(),
                        content: content.to_string(),
                        offset: e.offset,
                        ts: e.ts.clone(),
                    });
                }

                // ── [fixed] handling ───────────────────────────────────
                // Paired [fixed] entries are already represented in the scar line
                // on their friction counterpart; do not emit them separately.
                if e.content.starts_with("[fixed]") {
                    if pairing.paired_fixed.contains(&idx) {
                        return None;
                    }
                    // Unpaired [fixed]: expose as normal entry
                    let (marker, content) = extract_marker(&e.content);
                    return Some(ContextEntryOutput {
                        marker: marker.to_string(),
                        content: content.to_string(),
                        offset: e.offset,
                        ts: e.ts.clone(),
                    });
                }

                // ── [covers] ───────────────────────────────────────────
                // Exclude from body (already in header)
                if e.content.starts_with("[covers]") {
                    return None;
                }

                // ── observation-class tail-folding ─────────────────────
                if !include_observations {
                    for &om in &OBS_MARKERS {
                        if e.content.starts_with(om) {
                            let tail_idx = last_obs_idx.get(om).copied().unwrap_or(idx);
                            if idx != tail_idx {
                                // Not the tail: fold it
                                match om {
                                    "[progress]" => folded_counts.progress += 1,
                                    "[observed]" => folded_counts.observed += 1,
                                    "[check]"    => folded_counts.check    += 1,
                                    _ => {}
                                }
                                return None;
                            }
                            // This IS the tail: fall through to normal emit
                            break;
                        }
                    }
                }

                // Normal entry
                let (marker, content) = extract_marker(&e.content);
                Some(ContextEntryOutput {
                    marker: marker.to_string(),
                    content: content.to_string(),
                    offset: e.offset,
                    ts: e.ts.clone(),
                })
            })
            .collect();

        // Skip strand if it has no entries after filtering
        if entries.is_empty() {
            continue;
        }

        // Deduplicate covers
        let mut unique_covers: Vec<String> = Vec::new();
        for c in &covers_list {
            if !unique_covers.contains(c) {
                unique_covers.push(c.clone());
            }
        }

        output_strands.push(ContextStrandOutput {
            id: strand.id.clone(),
            covers: unique_covers,
            entries,
            friction_folded,
            friction_paired,
            folded_counts,
        });
    }

    // Sort output strands by last_entry_ts descending (most recent first)
    output_strands.sort_by(|a, b| {
        let ts_a = a.entries.last().map(|e| e.ts.as_str()).unwrap_or("");
        let ts_b = b.entries.last().map(|e| e.ts.as_str()).unwrap_or("");
        ts_b.cmp(ts_a)
    });
    output_strands
}

pub(crate) fn cmd_context(
    context_type: Option<&str>,
    covers: &[String],
    since_offset: Option<usize>,
    format_json: Option<&str>,
    exclude_friction: bool,
    include_hidden: bool,
    include_observations: bool,
) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, include_hidden);

    let target_type = context_type.unwrap_or("prompt-strand");
    let is_json = format_json == Some("json");

    let output_strands =
        build_context_strands(&strands, target_type, covers, since_offset, exclude_friction, include_observations);

    if is_json {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "strands": output_strands,
        })).map_err(|e| format!("serialize error: {}", e))?);
    } else {
        println!("# Strand Context\n");
        let strand_count = output_strands.len();
        for (i, strand) in output_strands.iter().enumerate() {
            let covers_str = if strand.covers.is_empty() {
                String::new()
            } else {
                format!(" [covers: {}]", strand.covers.join(", "))
            };
            println!("## prompt-strand:{} <id:{}>", covers_str, shorten(&strand.id));
            for entry in &strand.entries {
                if entry.marker.is_empty() {
                    println!("  {}", entry.content);
                } else {
                    println!("  {} {}", entry.marker, entry.content);
                }
            }
            if strand.friction_folded > 0 {
                println!(
                    "  friction: ×{} (folded — strand closed; tasktree show {})",
                    strand.friction_folded,
                    shorten(&strand.id)
                );
            }
            if strand.folded_counts.any_folded() {
                let fc = &strand.folded_counts;
                println!(
                    "  folded: progress ×{} | observed ×{} | check ×{}  (tasktree show {})",
                    fc.progress, fc.observed, fc.check,
                    shorten(&strand.id)
                );
            }
            if i + 1 < strand_count {
                println!();
            }
        }
    }

    Ok(())
}

/// Extract bracket-prefix marker from content.
/// Returns ("[guide]", "remaining text") or ("", "full text") if no marker.
pub(crate) fn extract_marker(content: &str) -> (&str, &str) {
    if let Some(rest) = content.strip_prefix("[guide]") {
        ("[guide]", rest.trim())
    } else if let Some(rest) = content.strip_prefix("[observed]") {
        ("[observed]", rest.trim())
    } else if let Some(rest) = content.strip_prefix("[constraint]") {
        ("[constraint]", rest.trim())
    } else if let Some(rest) = content.strip_prefix("[decision]") {
        ("[decision]", rest.trim())
    } else if let Some(rest) = content.strip_prefix("[friction]") {
        ("[friction]", rest.trim())
    } else if let Some(rest) = content.strip_prefix("[covers]") {
        ("[covers]", rest.trim())
    } else if content.starts_with('[') {
        if let Some(bracket_end) = content.find(']') {
            let marker = &content[..=bracket_end];
            let rest = content[bracket_end + 1..].trim();
            (marker, rest)
        } else {
            ("", content)
        }
    } else {
        ("", content)
    }
}

/// Folded observation-class entry counts. Always serialised (including zeros)
/// so the output contract is stable — consumers can rely on the field being present.
#[derive(Debug, serde::Serialize, Clone)]
pub(crate) struct FoldedCounts {
    /// [progress] entries folded (tail-1 count; does NOT include the retained tail entry)
    pub(crate) progress: usize,
    /// [observed] entries folded (tail-1 count)
    pub(crate) observed: usize,
    /// [check] entries folded (tail-1 count)
    pub(crate) check: usize,
}

impl FoldedCounts {
    pub(crate) fn zero() -> Self { FoldedCounts { progress: 0, observed: 0, check: 0 } }
    pub(crate) fn any_folded(&self) -> bool { self.progress > 0 || self.observed > 0 || self.check > 0 }
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ContextStrandOutput {
    pub(crate) id: String,
    pub(crate) covers: Vec<String>,
    pub(crate) entries: Vec<ContextEntryOutput>,
    /// [friction] entries folded away because the strand is closed
    /// (exposure axis: a scar, not a disappearance).
    pub(crate) friction_folded: usize,
    /// Live-strand friction/fixed pairs that were folded into scar entries.
    /// Closed strands always have 0 here — their friction folds via friction_folded.
    pub(crate) friction_paired: usize,
    /// Observation-class entries folded by default (tail kept, rest counted).
    /// Three keys are always present even when 0.
    pub(crate) folded_counts: FoldedCounts,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ContextEntryOutput {
    pub(crate) marker: String,
    pub(crate) content: String,
    pub(crate) offset: usize,
    pub(crate) ts: String,
}
