//! Runtime diagnostic emitters.

/// One emitted diagnostic: (code, one-line detail). The code resolves via
/// `tasktree explain <code>`.
pub(crate) type EmittedDiag = (&'static str, String);

/// Extract comparison tokens for W062 keyword matching: ASCII words of
/// length >= 5 (lowercased) plus contiguous CJK runs of length >= 3.
/// Conservative on purpose — shared full runs, not n-grams.
pub(crate) fn w062_tokens(text: &str) -> std::collections::HashSet<String> {
    let mut tokens = std::collections::HashSet::new();
    let mut ascii_word = String::new();
    let mut cjk_run = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            ascii_word.push(c.to_ascii_lowercase());
        } else {
            if ascii_word.len() >= 5 {
                tokens.insert(ascii_word.clone());
            }
            ascii_word.clear();
        }
        let is_cjk = ('\u{4e00}'..='\u{9fff}').contains(&c);
        if is_cjk {
            cjk_run.push(c);
        } else {
            if cjk_run.chars().count() >= 3 {
                tokens.insert(cjk_run.clone());
            }
            cjk_run.clear();
        }
    }
    if ascii_word.len() >= 5 {
        tokens.insert(ascii_word);
    }
    if cjk_run.chars().count() >= 3 {
        tokens.insert(cjk_run);
    }
    tokens
}

/// Run the W062/W068/W069 emitters over the journal events.
/// Pure: `now` is a parameter, nothing is written.
pub(crate) fn run_journal_diagnostics(
    events: &[crate::event::Event],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<EmittedDiag> {
    use crate::event::{EntryEffect, Event};
    use std::collections::{HashMap, HashSet};
    let mut diags: Vec<EmittedDiag> = Vec::new();

    // Group LogAppended per strand, keeping ts + provenance
    struct EntryRef<'a> {
        ts: &'a str,
        content: &'a str,
        producer: Option<&'a str>,
    }
    let mut per_strand: HashMap<&str, Vec<EntryRef>> = HashMap::new();
    for event in events {
        if let Event::LogAppended {
            id,
            ts,
            content,
            provenance,
            ..
        } = event
        {
            per_strand.entry(id.as_str()).or_default().push(EntryRef {
                ts: ts.as_str(),
                content: content.as_str(),
                producer: provenance
                    .as_ref()
                    .and_then(|p| p.get("producer"))
                    .and_then(|v| v.as_str()),
            });
        }
    }

    // Build closed-strand set from legacy lifecycle events and v2 close/reopen effects.
    // Legacy CLOSING markers in log content are no longer authoritative.
    let mut closed_strands: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut reopened_strands: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for event in events {
        match event {
            Event::StrandClosed { id, .. } => {
                closed_strands.insert(id.as_str());
                reopened_strands.remove(id.as_str());
            }
            Event::StrandReopened { id, .. } => {
                reopened_strands.insert(id.as_str());
                closed_strands.remove(id.as_str());
            }
            Event::LogAppended {
                id,
                effect: Some(EntryEffect::Close { .. }),
                ..
            } => {
                closed_strands.insert(id.as_str());
                reopened_strands.remove(id.as_str());
            }
            Event::LogAppended {
                id,
                effect: Some(EntryEffect::Reopen),
                ..
            } => {
                reopened_strands.insert(id.as_str());
                closed_strands.remove(id.as_str());
            }
            _ => {}
        }
    }

    // ── W068: deadline overdue ──
    for (id, entries) in &per_strand {
        let closed = closed_strands.contains(id);
        if closed {
            continue;
        }
        for e in entries {
            if !e.content.starts_with("[deadline]") {
                continue;
            }
            if let Some(by) = crate::util::parse_deadline_by(e.content) {
                if now > by {
                    diags.push((
                        "W068",
                        format!(
                            "strand {} deadline passed ({})",
                            crate::util::shorten(id),
                            by.to_rfc3339()
                        ),
                    ));
                }
            }
        }
    }

    // ── W069: concurrent marker write ──
    // Same lifecycle marker on the same strand from >= 2 distinct
    // provenance producers. Entries without provenance can't be
    // attributed and are ignored (no guessing).
    // Annotation markers (closing + state) are checked for concurrent writes.
    const ANNOTATION_MARKERS: &[&str] = &[
        "[verified]",
        "[done]",
        "[cancelled]",
        "[failed]",
        "[merged]",
        "[ended]",
        "[dispatched]",
        "[registered]",
    ];
    for (id, entries) in &per_strand {
        let mut writers: HashMap<&str, HashSet<&str>> = HashMap::new();
        for e in entries {
            if let Some(producer) = e.producer {
                if let Some(marker) = ANNOTATION_MARKERS
                    .iter()
                    .find(|m| e.content.starts_with(*m))
                {
                    writers.entry(marker).or_default().insert(producer);
                }
            }
        }
        for (marker, producers) in writers {
            if producers.len() >= 2 {
                let mut who: Vec<&str> = producers.into_iter().collect();
                who.sort();
                diags.push((
                    "W069",
                    format!(
                        "strand {} marker {} written by: {}",
                        crate::util::shorten(id),
                        marker,
                        who.join(", ")
                    ),
                ));
            }
        }
    }

    // ── W062: contradictory decision/constraint ──
    // [decision] and [constraint] sharing a keyword, written within 10
    // minutes, from different strands.
    struct Governed<'a> {
        strand: &'a str,
        ts: chrono::DateTime<chrono::Utc>,
        tokens: std::collections::HashSet<String>,
    }
    let mut decisions: Vec<Governed> = Vec::new();
    let mut constraints: Vec<Governed> = Vec::new();
    for (id, entries) in &per_strand {
        for e in entries {
            let bucket = if e.content.starts_with("[decision]") {
                &mut decisions
            } else if e.content.starts_with("[constraint]") {
                &mut constraints
            } else {
                continue;
            };
            if let Some(ts) = crate::util::parse_event_ts(e.ts) {
                bucket.push(Governed {
                    strand: id,
                    ts,
                    tokens: w062_tokens(e.content),
                });
            }
        }
    }
    let mut seen_pairs: HashSet<(String, String, String)> = HashSet::new();
    for d in &decisions {
        for c in &constraints {
            if d.strand == c.strand {
                continue;
            }
            if (d.ts - c.ts).num_seconds().abs() > 600 {
                continue;
            }
            if let Some(shared) = d.tokens.intersection(&c.tokens).next() {
                let key = (
                    crate::util::shorten(d.strand),
                    crate::util::shorten(c.strand),
                    shared.clone(),
                );
                if seen_pairs.insert(key) {
                    diags.push((
                        "W062",
                        format!(
                            "decision in {} vs constraint in {} share keyword \"{}\" within 10min",
                            crate::util::shorten(d.strand),
                            crate::util::shorten(c.strand),
                            shared
                        ),
                    ));
                }
            }
        }
    }

    diags
}

/// Check W070: checkpoint's provenance.producer differs from the last
/// LogAppended entry's provenance.producer on the target strand.
///
/// Both producers must be non-empty strings for this check to fire;
/// if either is absent the function returns None (no guessing).
///
/// Returns `Some((code, detail))` when the check fires, `None` otherwise.
pub(crate) fn check_w070_strand_moved(
    events: &[(usize, crate::event::Event)],
    strand_id: &str,
    checkpoint_producer: Option<&str>,
) -> Option<EmittedDiag> {
    use crate::event::Event;
    let cp_producer = checkpoint_producer?;
    if cp_producer.is_empty() {
        return None;
    }
    // Find the last LogAppended event for this strand.
    let last_entry_producer: Option<&str> = events
        .iter()
        .filter_map(|(_, e)| {
            if let Event::LogAppended { id, provenance, .. } = e {
                if id == strand_id {
                    Some(
                        provenance
                            .as_ref()
                            .and_then(|p| p.get("producer"))
                            .and_then(|v| v.as_str()),
                    )
                } else {
                    None
                }
            } else {
                None
            }
        })
        .last()
        .flatten();
    let last_producer = last_entry_producer?;
    if last_producer.is_empty() {
        return None;
    }
    if last_producer != cp_producer {
        Some((
            "W070",
            format!(
                "strand moved under you: last entry by \"{}\", you are \"{}\"",
                last_producer, cp_producer
            ),
        ))
    } else {
        None
    }
}

/// Check W071: checkpoint target strand state is not "registered" (already closed).
///
/// Returns `Some((code, detail))` when the check fires, `None` otherwise.
pub(crate) fn check_w071_closed_strand(
    strand: &crate::projection::ProjectedStrand,
) -> Option<EmittedDiag> {
    if strand.state() != "registered" {
        Some((
            "W071",
            format!("checkpoint on closed strand: state is {}", strand.state()),
        ))
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeenOffsetWarning {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
    pub(crate) seen_offset: usize,
    pub(crate) strand_last_offset: usize,
    pub(crate) seen_gap: usize,
    pub(crate) catch_up: String,
}

/// Check W076: caller-declared --seen-offset is behind the target strand's
/// pre-write last_offset. Missing or future offsets are best-effort ignored.
pub(crate) fn check_w076_seen_offset(
    strand_id: &str,
    seen_offset: Option<usize>,
    strand_last_offset: usize,
) -> Option<SeenOffsetWarning> {
    let seen = seen_offset?;
    if seen >= strand_last_offset {
        return None;
    }
    let gap = strand_last_offset - seen;
    let catch_up = format!(
        "tasktree timeline --since-offset {} --links {}",
        seen,
        crate::util::shorten(strand_id)
    );
    Some(SeenOffsetWarning {
        code: "W076",
        detail: format!(
            "seen offset {} is {} entries behind strand last offset {}; catch-up: {}",
            seen, gap, strand_last_offset, catch_up
        ),
        seen_offset: seen,
        strand_last_offset,
        seen_gap: gap,
        catch_up,
    })
}
