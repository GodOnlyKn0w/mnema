//! Strict v2 → v3 journal cutover.
//!
//! Dry-run plans a deterministic conversion. Apply prepares immutable artifacts
//! under `.mnema/history/` and `.mnema/journals/` (copy, never pre-commit move of
//! source), then activates via `activate_initial_v3` under the exclusive journal
//! lock. Failure never installs the active manifest. Legacy `journal.jsonl` is
//! left as a shadow after activation.

use crate::activation::{
    ACTIVE_MANIFEST_SCHEMA, ActivationOriginV3, ActivationOutcome, ActiveJournalManifestV3,
    HistoricalJournalV3, JournalArtifactV3, activate_initial_v3, load_active_manifest,
};
use crate::canonical::{
    CheckpointPayloadV3, CloseDispositionV3, EdgeTypeV3, EffectPayloadV3, EntryHashViewV3,
    JournalRecordV3, RefV3, StrandKeyV3, SubjectBindingPayloadV3, author_from_provenance,
    canonicalize_timestamp, deterministic_v2_import_seed, kind_from_body,
};
use crate::event::{EntryEffect, Event};
use crate::journal::{self, sha256_bytes};
use crate::journal_v3::{self, make_anchor, write_records_prepared};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(crate) const MAP_SCHEMA: &str = "mnema.migration-v2-to-v3.map.v1";
pub(crate) const CERTIFICATE_SCHEMA: &str = "mnema.migration-v2-to-v3.certificate.v1";
pub(crate) const HISTORY_V2_REL: &str = "history/journal.v2.jsonl";
pub(crate) const TARGET_V3_REL: &str = "journals/journal.v3.jsonl";
pub(crate) const MAP_REL: &str = "history/migration-v2-to-v3.json";
pub(crate) const CERTIFICATE_REL: &str = "history/migration-v2-to-v3.certificate.json";
const MIGRATION_ID_DOMAIN: &[u8] = b"mnema.migration.v2-to-v3\0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CutoverV3SourceRecord {
    pub(crate) offset: usize,
    pub(crate) variant: String,
    pub(crate) disposition: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) new_entry_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) new_strand_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CutoverV3EntryMap {
    pub(crate) old_offset: usize,
    pub(crate) old_strand_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) old_entry_id: Option<String>,
    pub(crate) new_strand_id: String,
    pub(crate) new_entry_id: String,
    pub(crate) kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CutoverV3RefMap {
    pub(crate) source: String,
    pub(crate) target: RefV3,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CutoverV3Map {
    pub(crate) schema: String,
    pub(crate) migration_id: String,
    pub(crate) source_journal_id: String,
    pub(crate) source_event_count: usize,
    pub(crate) source_sha256: String,
    pub(crate) target_journal_id: String,
    pub(crate) target_record_count: usize,
    pub(crate) strands: BTreeMap<String, String>,
    pub(crate) entries: Vec<CutoverV3EntryMap>,
    pub(crate) source_records: Vec<CutoverV3SourceRecord>,
    pub(crate) refs: Vec<CutoverV3RefMap>,
    pub(crate) unresolved_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CutoverV3Certificate {
    pub(crate) schema: String,
    pub(crate) created_at: String,
    pub(crate) tool_version: String,
    pub(crate) tool_commit: String,
    pub(crate) migration_id: String,
    pub(crate) source_journal_id: String,
    pub(crate) source_journal: String,
    pub(crate) history_journal: String,
    pub(crate) target_journal: String,
    pub(crate) map_path: String,
    pub(crate) source_event_count: usize,
    pub(crate) source_sha256: String,
    pub(crate) target_record_count: usize,
    pub(crate) target_sha256: String,
    pub(crate) map_sha256: String,
    pub(crate) history_sha256: String,
    pub(crate) unresolved_ref_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct CutoverV3Plan {
    pub(crate) records: Vec<JournalRecordV3>,
    pub(crate) map: CutoverV3Map,
    pub(crate) equivalence: ProjectionEquivalence,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ProjectionEquivalence {
    pub(crate) ok: bool,
    pub(crate) strand_count_v2: usize,
    pub(crate) strand_count_v3: usize,
    pub(crate) entry_count_v2: usize,
    pub(crate) entry_count_v3: usize,
    pub(crate) edge_count_v2: usize,
    pub(crate) edge_count_v3: usize,
    pub(crate) closed_count_v2: usize,
    pub(crate) closed_count_v3: usize,
    pub(crate) hidden_count_v2: usize,
    pub(crate) hidden_count_v3: usize,
    pub(crate) mismatches: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CutoverV3ApplyOutcome {
    Applied,
    AppliedDurabilityUncertain,
    AlreadyActive,
}

struct StrandMeta {
    strand_type: Option<String>,
    slug: Option<String>,
}

/// Live (target_old, edge_type) → ordered link new_entry_ids on a source strand.
type LiveLinks = BTreeMap<(String, String), Vec<String>>;

struct ConvertState {
    source_journal_id: String,
    strand_meta: HashMap<String, StrandMeta>,
    strand_map: BTreeMap<String, String>,
    entry_map: BTreeMap<String, String>,
    heads: HashMap<String, String>,
    live_links: HashMap<String, LiveLinks>,
    records: Vec<JournalRecordV3>,
    entries: Vec<CutoverV3EntryMap>,
    source_records: Vec<CutoverV3SourceRecord>,
    refs: Vec<CutoverV3RefMap>,
    unresolved_refs: Vec<String>,
    max_created_at: Option<String>,
}

pub(crate) fn build_cutover_v3_plan(
    source_journal_id: &str,
    source_sha256: &str,
    source: &[(usize, Event)],
) -> Result<CutoverV3Plan, String> {
    if source_journal_id.len() != 64
        || !source_journal_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!(
            "source journal_id must be 64 lowercase hex, found {source_journal_id}"
        ));
    }
    if source_sha256.len() != 64
        || !source_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!(
            "source sha256 must be 64 lowercase hex, found {source_sha256}"
        ));
    }

    let mut strand_meta = HashMap::new();
    for (_, event) in source {
        if let Event::StrandCreated {
            id,
            strand_type,
            slug,
            ..
        } = event
        {
            strand_meta.insert(
                id.clone(),
                StrandMeta {
                    strand_type: strand_type.clone(),
                    slug: slug.clone(),
                },
            );
        }
    }

    let mut state = ConvertState {
        source_journal_id: source_journal_id.to_string(),
        strand_meta,
        strand_map: BTreeMap::new(),
        entry_map: BTreeMap::new(),
        heads: HashMap::new(),
        live_links: HashMap::new(),
        records: Vec::new(),
        entries: Vec::new(),
        source_records: Vec::new(),
        refs: Vec::new(),
        unresolved_refs: Vec::new(),
        max_created_at: None,
    };

    for (offset, event) in source {
        convert_event(&mut state, *offset, event)?;
    }

    let anchor_ts = state
        .max_created_at
        .clone()
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());
    let anchor = make_anchor(&state.source_journal_id, &state.records, anchor_ts)?;
    state.records.push(anchor);

    journal_v3::validate_records(&state.source_journal_id, &state.records)?;

    let migration_id = migration_id_for(source_journal_id, source_sha256);
    let map = CutoverV3Map {
        schema: MAP_SCHEMA.to_string(),
        migration_id,
        source_journal_id: source_journal_id.to_string(),
        source_event_count: source.len(),
        source_sha256: source_sha256.to_string(),
        target_journal_id: source_journal_id.to_string(),
        target_record_count: state.records.len(),
        strands: state.strand_map.clone(),
        entries: state.entries.clone(),
        source_records: state.source_records.clone(),
        refs: state.refs.clone(),
        unresolved_refs: state.unresolved_refs.clone(),
    };

    let equivalence = check_projection_equivalence(source, &state.records, &map);
    if !equivalence.ok {
        return Err(format!(
            "projection equivalence failed: {}",
            equivalence.mismatches.join("; ")
        ));
    }

    Ok(CutoverV3Plan {
        records: state.records,
        map,
        equivalence,
    })
}

fn migration_id_for(source_journal_id: &str, source_sha256: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(MIGRATION_ID_DOMAIN);
    hasher.update(source_journal_id.as_bytes());
    hasher.update([0]);
    hasher.update(source_sha256.as_bytes());
    hex::encode(hasher.finalize())
}

fn convert_event(state: &mut ConvertState, offset: usize, event: &Event) -> Result<(), String> {
    match event {
        Event::StrandCreated { id, .. } => {
            state.source_records.push(CutoverV3SourceRecord {
                offset,
                variant: "strand_created".to_string(),
                disposition: "absorbed_into_genesis".to_string(),
                new_entry_ids: Vec::new(),
                new_strand_id: None,
            });
            let _ = id;
            Ok(())
        }
        Event::JournalAnchored { .. } => {
            state.source_records.push(CutoverV3SourceRecord {
                offset,
                variant: "journal_anchored".to_string(),
                disposition: "regenerated_anchor".to_string(),
                new_entry_ids: Vec::new(),
                new_strand_id: None,
            });
            Ok(())
        }
        Event::LogAppended {
            id,
            ts,
            content,
            effect,
            refs,
            provenance,
            entry_id,
            ..
        } => match effect {
            Some(EntryEffect::Unlink {
                target,
                edge_type,
                link_entry_id: None,
            }) => convert_key_tombstone_unlink(
                state,
                offset,
                id,
                ts,
                content,
                target,
                edge_type,
                provenance.as_ref(),
                "log_appended",
            ),
            other => convert_single_entry(
                state,
                offset,
                id,
                ts,
                content,
                other.as_ref(),
                refs,
                provenance.as_ref(),
                entry_id.as_deref(),
                "log_appended",
            ),
        },
        Event::EdgeLinked {
            id,
            ts,
            to,
            edge_type,
            provenance,
        } => {
            let etype = edge_type.as_deref().unwrap_or("depends-on");
            // Reject unsupported edge types early (migration-source-invalid).
            parse_edge_type(etype).map_err(|e| {
                format!("migration-source-invalid at offset {offset}: {e} (event edge_linked)")
            })?;
            let (body, _) = crate::event::link_entry_parts(to, etype);
            convert_single_entry(
                state,
                offset,
                id,
                ts,
                &body,
                Some(&EntryEffect::Link {
                    target: to.clone(),
                    edge_type: etype.to_string(),
                }),
                &[],
                provenance.as_ref(),
                None,
                "edge_linked",
            )
        }
        Event::EdgeUnlinked {
            id,
            ts,
            to,
            edge_type,
            provenance,
        } => {
            let etype = edge_type.as_deref().unwrap_or("depends-on");
            parse_edge_type(etype).map_err(|e| {
                format!("migration-source-invalid at offset {offset}: {e} (event edge_unlinked)")
            })?;
            let (body, _) = crate::event::unlink_entry_parts(to, etype, None);
            convert_key_tombstone_unlink(
                state,
                offset,
                id,
                ts,
                &body,
                to,
                etype,
                provenance.as_ref(),
                "edge_unlinked",
            )
        }
        Event::StrandClosed {
            id,
            ts,
            disposition,
            provenance,
        } => {
            let (body, effect) = crate::event::close_entry_parts(disposition, None);
            convert_single_entry(
                state,
                offset,
                id,
                ts,
                &body,
                Some(&effect),
                &[],
                provenance.as_ref(),
                None,
                "strand_closed",
            )
        }
        Event::StrandReopened { id, ts, provenance } => {
            let (body, effect) = crate::event::reopen_entry_parts(None);
            convert_single_entry(
                state,
                offset,
                id,
                ts,
                &body,
                Some(&effect),
                &[],
                provenance.as_ref(),
                None,
                "strand_reopened",
            )
        }
        Event::StrandHidden { id, ts } => {
            let (body, effect) = crate::event::hide_entry_parts(None);
            convert_single_entry(
                state,
                offset,
                id,
                ts,
                &body,
                Some(&effect),
                &[],
                None,
                None,
                "strand_hidden",
            )
        }
        Event::StrandUnhidden { id, ts } => {
            let (body, effect) = crate::event::unhide_entry_parts();
            convert_single_entry(
                state,
                offset,
                id,
                ts,
                &body,
                Some(&effect),
                &[],
                None,
                None,
                "strand_unhidden",
            )
        }
        Event::CheckpointCreated {
            id,
            ts,
            observed,
            action,
            provenance,
            ..
        } => convert_checkpoint(state, offset, id, ts, observed, action, provenance.as_ref()),
        Event::SubjectBound {
            id,
            ts,
            subject_type,
            subject_id,
            strand_id,
            provenance,
        } => convert_subject_binding(
            state,
            offset,
            id,
            ts,
            subject_type,
            subject_id,
            strand_id,
            provenance.as_ref(),
        ),
    }
}

/// Expand a legacy key-tombstone unlink into 0..N typed Unlink entries.
#[allow(clippy::too_many_arguments)]
fn convert_key_tombstone_unlink(
    state: &mut ConvertState,
    offset: usize,
    old_strand_id: &str,
    ts: &str,
    body: &str,
    target_old: &str,
    edge_type: &str,
    provenance: Option<&serde_json::Value>,
    variant: &str,
) -> Result<(), String> {
    parse_edge_type(edge_type).map_err(|e| {
        format!("migration-source-invalid at offset {offset}: {e} (variant {variant})")
    })?;
    let live = state
        .live_links
        .get(old_strand_id)
        .and_then(|m| m.get(&(target_old.to_string(), edge_type.to_string())))
        .cloned()
        .unwrap_or_default();

    if live.is_empty() {
        state.source_records.push(CutoverV3SourceRecord {
            offset,
            variant: variant.to_string(),
            disposition: "unlink_noop_no_live_links".to_string(),
            new_entry_ids: Vec::new(),
            new_strand_id: state.strand_map.get(old_strand_id).cloned(),
        });
        return Ok(());
    }

    let mut new_ids = Vec::new();
    for link_entry_id in &live {
        let effect = EntryEffect::Unlink {
            target: target_old.to_string(),
            edge_type: edge_type.to_string(),
            link_entry_id: Some(link_entry_id.clone()),
        };
        let new_id = emit_entry(
            state,
            offset,
            old_strand_id,
            ts,
            body,
            Some(&effect),
            &[],
            provenance,
            None,
            /* force_kind */ Some("effect"),
        )?;
        new_ids.push(new_id);
    }
    // Clear live set for this key after full tombstone.
    if let Some(map) = state.live_links.get_mut(old_strand_id) {
        map.remove(&(target_old.to_string(), edge_type.to_string()));
    }
    let new_strand_id = state.strand_map.get(old_strand_id).cloned();
    state.source_records.push(CutoverV3SourceRecord {
        offset,
        variant: variant.to_string(),
        disposition: format!("converted_to_{}_unlinks", new_ids.len()),
        new_entry_ids: new_ids,
        new_strand_id,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn convert_single_entry(
    state: &mut ConvertState,
    offset: usize,
    old_strand_id: &str,
    ts: &str,
    content: &str,
    effect: Option<&EntryEffect>,
    refs: &[String],
    provenance: Option<&serde_json::Value>,
    old_entry_id: Option<&str>,
    variant: &str,
) -> Result<(), String> {
    let new_id = emit_entry(
        state,
        offset,
        old_strand_id,
        ts,
        content,
        effect,
        refs,
        provenance,
        old_entry_id,
        None,
    )?;
    state.source_records.push(CutoverV3SourceRecord {
        offset,
        variant: variant.to_string(),
        disposition: "converted_to_entry".to_string(),
        new_entry_ids: vec![new_id],
        new_strand_id: state.strand_map.get(old_strand_id).cloned(),
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_entry(
    state: &mut ConvertState,
    offset: usize,
    old_strand_id: &str,
    ts: &str,
    content: &str,
    effect: Option<&EntryEffect>,
    refs: &[String],
    provenance: Option<&serde_json::Value>,
    old_entry_id: Option<&str>,
    force_kind: Option<&str>,
) -> Result<String, String> {
    let created_at = canonicalize_timestamp("created_at", ts)?;
    note_max_ts(state, &created_at);

    let mapped_refs = translate_refs(state, refs)?;
    let (kind, payload) = match effect {
        Some(effect) => (
            "effect".to_string(),
            effect_to_payload(state, effect)?.into_value(),
        ),
        None if force_kind == Some("effect") => {
            return Err("internal: force_kind effect without payload".to_string());
        }
        None => (
            force_kind
                .map(str::to_string)
                .unwrap_or_else(|| kind_from_body(content)),
            serde_json::Value::Null,
        ),
    };

    // Provenance is copied as-is (null when absent). Never merge git/append_id.
    let provenance_value = provenance.cloned().unwrap_or(serde_json::Value::Null);
    let author = author_from_provenance(&provenance_value);

    let is_genesis = !state.heads.contains_key(old_strand_id);
    let strand_key = if is_genesis {
        let meta = state.strand_meta.get(old_strand_id);
        StrandKeyV3::Genesis {
            seed: deterministic_v2_import_seed(&state.source_journal_id, old_strand_id),
            slug: meta.and_then(|m| m.slug.clone()),
            strand_type: meta.and_then(|m| m.strand_type.clone()),
        }
    } else {
        let new_id = state
            .strand_map
            .get(old_strand_id)
            .cloned()
            .ok_or_else(|| format!("offset {offset}: strand {old_strand_id} missing genesis"))?;
        StrandKeyV3::Existing { id: new_id }
    };
    let prev = state.heads.get(old_strand_id).cloned();

    let view = EntryHashViewV3::new(
        strand_key,
        prev,
        kind.clone(),
        content.to_string(),
        mapped_refs,
        author,
        created_at,
        payload,
        provenance_value,
    );
    let record = view.into_record()?;
    let (new_strand_id, new_entry_id) = match &record {
        JournalRecordV3::Entry(entry) => (entry.strand_id.clone(), entry.entry_id.clone()),
        JournalRecordV3::Anchor(_) => unreachable!("entry conversion produced anchor"),
    };

    if is_genesis {
        state
            .strand_map
            .insert(old_strand_id.to_string(), new_strand_id.clone());
    }
    if let Some(old) = old_entry_id {
        state
            .entry_map
            .insert(old.to_string(), new_entry_id.clone());
    }
    // Also map new entry under itself for link_entry_id that already points at
    // v3 ids after expansion (key-tombstone path passes mapped ids).
    state
        .entry_map
        .entry(new_entry_id.clone())
        .or_insert_with(|| new_entry_id.clone());

    state
        .heads
        .insert(old_strand_id.to_string(), new_entry_id.clone());

    // Maintain live link index using old target keys for tombstone matching.
    if let Some(EntryEffect::Link { target, edge_type }) = effect {
        state
            .live_links
            .entry(old_strand_id.to_string())
            .or_default()
            .entry((target.clone(), edge_type.clone()))
            .or_default()
            .push(new_entry_id.clone());
    }
    if let Some(EntryEffect::Unlink {
        target,
        edge_type,
        link_entry_id: Some(link_id),
    }) = effect
    {
        let mapped_link_id = state
            .entry_map
            .get(link_id)
            .cloned()
            .unwrap_or_else(|| link_id.clone());
        if let Some(map) = state.live_links.get_mut(old_strand_id) {
            if let Some(list) = map.get_mut(&(target.clone(), edge_type.clone())) {
                list.retain(|id| id != &mapped_link_id);
                if list.is_empty() {
                    map.remove(&(target.clone(), edge_type.clone()));
                }
            }
        }
    }

    state.entries.push(CutoverV3EntryMap {
        old_offset: offset,
        old_strand_id: old_strand_id.to_string(),
        old_entry_id: old_entry_id.map(str::to_string),
        new_strand_id,
        new_entry_id: new_entry_id.clone(),
        kind,
    });
    state.records.push(record);
    Ok(new_entry_id)
}

fn convert_checkpoint(
    state: &mut ConvertState,
    offset: usize,
    old_strand_id: &str,
    ts: &str,
    observed: &str,
    action: &str,
    provenance: Option<&serde_json::Value>,
) -> Result<(), String> {
    let created_at = canonicalize_timestamp("created_at", ts)?;
    note_max_ts(state, &created_at);
    let payload = CheckpointPayloadV3 {
        observed: observed.to_string(),
        action: action.to_string(),
    }
    .into_value();
    let provenance_value = provenance.cloned().unwrap_or(serde_json::Value::Null);
    let author = author_from_provenance(&provenance_value);
    let is_genesis = !state.heads.contains_key(old_strand_id);
    let strand_key = if is_genesis {
        let meta = state.strand_meta.get(old_strand_id);
        StrandKeyV3::Genesis {
            seed: deterministic_v2_import_seed(&state.source_journal_id, old_strand_id),
            slug: meta.and_then(|m| m.slug.clone()),
            strand_type: meta.and_then(|m| m.strand_type.clone()),
        }
    } else {
        StrandKeyV3::Existing {
            id: state.strand_map[old_strand_id].clone(),
        }
    };
    let prev = state.heads.get(old_strand_id).cloned();
    let body = format!("[checkpoint] observed=\"{observed}\" action=\"{action}\"");
    let view = EntryHashViewV3::new(
        strand_key,
        prev,
        "checkpoint",
        body,
        Vec::new(),
        author,
        created_at,
        payload,
        provenance_value,
    );
    let record = view.into_record()?;
    let (new_strand_id, new_entry_id) = match &record {
        JournalRecordV3::Entry(entry) => (entry.strand_id.clone(), entry.entry_id.clone()),
        _ => unreachable!(),
    };
    if is_genesis {
        state
            .strand_map
            .insert(old_strand_id.to_string(), new_strand_id.clone());
    }
    state
        .heads
        .insert(old_strand_id.to_string(), new_entry_id.clone());
    state.entries.push(CutoverV3EntryMap {
        old_offset: offset,
        old_strand_id: old_strand_id.to_string(),
        old_entry_id: None,
        new_strand_id: new_strand_id.clone(),
        new_entry_id: new_entry_id.clone(),
        kind: "checkpoint".to_string(),
    });
    state.source_records.push(CutoverV3SourceRecord {
        offset,
        variant: "checkpoint".to_string(),
        disposition: "converted_to_entry".to_string(),
        new_entry_ids: vec![new_entry_id],
        new_strand_id: Some(new_strand_id),
    });
    state.records.push(record);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn convert_subject_binding(
    state: &mut ConvertState,
    offset: usize,
    binding_id: &str,
    ts: &str,
    subject_type: &str,
    subject_id: &str,
    strand_id: &str,
    provenance: Option<&serde_json::Value>,
) -> Result<(), String> {
    if !state.heads.contains_key(strand_id) {
        return Err(format!(
            "offset {offset}: subject_bound targets unknown strand {strand_id}"
        ));
    }
    let created_at = canonicalize_timestamp("created_at", ts)?;
    note_max_ts(state, &created_at);
    let payload = SubjectBindingPayloadV3 {
        subject_type: subject_type.to_string(),
        subject_id: subject_id.to_string(),
    }
    .into_value();
    let provenance_value = provenance.cloned().unwrap_or(serde_json::Value::Null);
    let author = author_from_provenance(&provenance_value);
    let new_strand = state.strand_map[strand_id].clone();
    let prev = state.heads.get(strand_id).cloned();
    let view = EntryHashViewV3::new(
        StrandKeyV3::Existing { id: new_strand },
        prev,
        "subject_binding",
        format!("subject_bound {subject_type}:{subject_id}"),
        Vec::new(),
        author,
        created_at,
        payload,
        provenance_value,
    );
    let record = view.into_record()?;
    let (new_strand_id, new_entry_id) = match &record {
        JournalRecordV3::Entry(entry) => (entry.strand_id.clone(), entry.entry_id.clone()),
        _ => unreachable!(),
    };
    state
        .heads
        .insert(strand_id.to_string(), new_entry_id.clone());
    state
        .entry_map
        .insert(binding_id.to_string(), new_entry_id.clone());
    state.entries.push(CutoverV3EntryMap {
        old_offset: offset,
        old_strand_id: strand_id.to_string(),
        old_entry_id: Some(binding_id.to_string()),
        new_strand_id: new_strand_id.clone(),
        new_entry_id: new_entry_id.clone(),
        kind: "subject_binding".to_string(),
    });
    state.source_records.push(CutoverV3SourceRecord {
        offset,
        variant: "subject_bound".to_string(),
        disposition: "converted_to_entry".to_string(),
        new_entry_ids: vec![new_entry_id],
        new_strand_id: Some(new_strand_id),
    });
    state.records.push(record);
    Ok(())
}

fn note_max_ts(state: &mut ConvertState, ts: &str) {
    match &state.max_created_at {
        Some(current) if current.as_str() >= ts => {}
        _ => state.max_created_at = Some(ts.to_string()),
    }
}

fn effect_to_payload(
    state: &ConvertState,
    effect: &EntryEffect,
) -> Result<EffectPayloadV3, String> {
    match effect {
        EntryEffect::Close { disposition } => Ok(EffectPayloadV3::Close {
            disposition: parse_close_disposition(disposition)?,
        }),
        EntryEffect::Reopen => Ok(EffectPayloadV3::Reopen {}),
        EntryEffect::Hide => Ok(EffectPayloadV3::Hide {}),
        EntryEffect::Unhide => Ok(EffectPayloadV3::Unhide {}),
        EntryEffect::Link { target, edge_type } => {
            let target_strand_id = state
                .strand_map
                .get(target)
                .cloned()
                .ok_or_else(|| format!("link target strand {target} not yet converted"))?;
            Ok(EffectPayloadV3::Link {
                edge_type: parse_edge_type(edge_type)?,
                target_strand_id,
            })
        }
        EntryEffect::Unlink {
            target,
            edge_type,
            link_entry_id,
        } => {
            let target_strand_id = state
                .strand_map
                .get(target)
                .cloned()
                .ok_or_else(|| format!("unlink target strand {target} not yet converted"))?;
            let link_entry_id = match link_entry_id {
                Some(id) => {
                    // id may already be a v3 entry id (tombstone expansion) or old.
                    state
                        .entry_map
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| id.clone())
                }
                None => {
                    return Err(
                        "internal: key-tombstone unlink must be expanded before effect_to_payload"
                            .to_string(),
                    );
                }
            };
            Ok(EffectPayloadV3::Unlink {
                edge_type: parse_edge_type(edge_type)?,
                target_strand_id,
                link_entry_id,
            })
        }
    }
}

fn parse_edge_type(value: &str) -> Result<EdgeTypeV3, String> {
    match value {
        "belongs-to" => Ok(EdgeTypeV3::BelongsTo),
        "depends-on" => Ok(EdgeTypeV3::DependsOn),
        other => Err(format!("unsupported edge type for v3 cutover: {other}")),
    }
}

fn parse_close_disposition(value: &str) -> Result<CloseDispositionV3, String> {
    match value {
        "done" => Ok(CloseDispositionV3::Done),
        "failed" => Ok(CloseDispositionV3::Failed),
        "cancelled" => Ok(CloseDispositionV3::Cancelled),
        "merged" => Ok(CloseDispositionV3::Merged),
        "verified" => Ok(CloseDispositionV3::Verified),
        other => Err(format!(
            "unsupported close disposition for v3 cutover: {other}"
        )),
    }
}

fn translate_refs(state: &mut ConvertState, refs: &[String]) -> Result<Vec<RefV3>, String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for r in refs {
        let mapped = resolve_one_ref(state, r);
        let key = serde_jcs::to_vec(&mapped).map_err(|e| format!("canonicalize ref: {e}"))?;
        if seen.insert(key) {
            state.refs.push(CutoverV3RefMap {
                source: r.clone(),
                target: mapped.clone(),
            });
            out.push(mapped);
        }
    }
    Ok(out)
}

fn resolve_one_ref(state: &mut ConvertState, raw: &str) -> RefV3 {
    if let Some(entry_id) = state.entry_map.get(raw) {
        return RefV3::Entry {
            journal_id: state.source_journal_id.clone(),
            entry_id: entry_id.clone(),
        };
    }
    if let Some(strand_id) = state.strand_map.get(raw) {
        return RefV3::Strand {
            journal_id: state.source_journal_id.clone(),
            strand_id: strand_id.clone(),
        };
    }
    // Unresolved: never pretend it is a local v3 id.
    state.unresolved_refs.push(raw.to_string());
    RefV3::External {
        scheme: "mnema-v2-entry".to_string(),
        locator: format!("{}:{}", state.source_journal_id, raw),
    }
}

fn check_projection_equivalence(
    source: &[(usize, Event)],
    records: &[JournalRecordV3],
    map: &CutoverV3Map,
) -> ProjectionEquivalence {
    let mut mismatches = Vec::new();

    let mut v2_strands: HashSet<String> = HashSet::new();
    let mut v2_entry_events = 0usize;
    let mut v2_edges = 0usize;
    let mut v2_closed: HashSet<String> = HashSet::new();
    let mut v2_hidden: HashMap<String, i32> = HashMap::new();
    // Replay live links to estimate net edges after tombstones (for comparison
    // with v3 which expands key-tombstones into N unlinks).
    let mut live: HashMap<String, BTreeMap<(String, String), usize>> = HashMap::new();

    for (_, event) in source {
        match event {
            Event::StrandCreated { id, .. } => {
                v2_strands.insert(id.clone());
            }
            Event::LogAppended {
                id,
                effect,
                content,
                ..
            } => {
                v2_strands.insert(id.clone());
                v2_entry_events += 1;
                match effect {
                    Some(EntryEffect::Link { target, edge_type }) => {
                        *live
                            .entry(id.clone())
                            .or_default()
                            .entry((target.clone(), edge_type.clone()))
                            .or_default() += 1;
                        v2_edges += 1;
                    }
                    Some(EntryEffect::Unlink {
                        target,
                        edge_type,
                        link_entry_id: None,
                    }) => {
                        if let Some(n) = live
                            .get_mut(id)
                            .and_then(|m| m.remove(&(target.clone(), edge_type.clone())))
                        {
                            v2_edges = v2_edges.saturating_sub(n);
                        }
                    }
                    Some(EntryEffect::Unlink {
                        target,
                        edge_type,
                        link_entry_id: Some(_),
                    }) => {
                        if let Some(m) = live.get_mut(id) {
                            if let Some(n) = m.get_mut(&(target.clone(), edge_type.clone())) {
                                *n = n.saturating_sub(1);
                                v2_edges = v2_edges.saturating_sub(1);
                                if *n == 0 {
                                    m.remove(&(target.clone(), edge_type.clone()));
                                }
                            }
                        }
                    }
                    Some(EntryEffect::Close { .. }) => {
                        v2_closed.insert(id.clone());
                    }
                    Some(EntryEffect::Reopen) => {
                        v2_closed.remove(id);
                    }
                    Some(EntryEffect::Hide) => {
                        *v2_hidden.entry(id.clone()).or_default() += 1;
                    }
                    Some(EntryEffect::Unhide) => {
                        *v2_hidden.entry(id.clone()).or_default() -= 1;
                    }
                    None => {
                        let _ = content;
                    }
                }
            }
            Event::EdgeLinked {
                id, to, edge_type, ..
            } => {
                v2_strands.insert(id.clone());
                v2_entry_events += 1;
                let et = edge_type
                    .clone()
                    .unwrap_or_else(|| "depends-on".to_string());
                *live
                    .entry(id.clone())
                    .or_default()
                    .entry((to.clone(), et))
                    .or_default() += 1;
                v2_edges += 1;
            }
            Event::EdgeUnlinked {
                id, to, edge_type, ..
            } => {
                v2_strands.insert(id.clone());
                v2_entry_events += 1;
                let et = edge_type
                    .clone()
                    .unwrap_or_else(|| "depends-on".to_string());
                if let Some(n) = live.get_mut(id).and_then(|m| m.remove(&(to.clone(), et))) {
                    v2_edges = v2_edges.saturating_sub(n);
                }
            }
            Event::StrandClosed { id, .. } => {
                v2_strands.insert(id.clone());
                v2_entry_events += 1;
                v2_closed.insert(id.clone());
            }
            Event::StrandReopened { id, .. } => {
                v2_strands.insert(id.clone());
                v2_entry_events += 1;
                v2_closed.remove(id);
            }
            Event::StrandHidden { id, .. } => {
                v2_strands.insert(id.clone());
                v2_entry_events += 1;
                *v2_hidden.entry(id.clone()).or_default() += 1;
            }
            Event::StrandUnhidden { id, .. } => {
                v2_strands.insert(id.clone());
                v2_entry_events += 1;
                *v2_hidden.entry(id.clone()).or_default() -= 1;
            }
            Event::CheckpointCreated { id, .. } | Event::SubjectBound { strand_id: id, .. } => {
                v2_strands.insert(id.clone());
                v2_entry_events += 1;
            }
            Event::JournalAnchored { .. } => {}
        }
    }

    let mut v3_strands: HashSet<String> = HashSet::new();
    let mut v3_entries = 0usize;
    let mut v3_edges = 0usize;
    let mut v3_closed: HashSet<String> = HashSet::new();
    let mut v3_hidden: HashMap<String, i32> = HashMap::new();

    for record in records {
        if let JournalRecordV3::Entry(entry) = record {
            v3_strands.insert(entry.strand_id.clone());
            v3_entries += 1;
            if entry.entry.kind == "effect" {
                if let Ok(effect) =
                    serde_json::from_value::<EffectPayloadV3>(entry.entry.payload.clone())
                {
                    match effect {
                        EffectPayloadV3::Link { .. } => v3_edges += 1,
                        EffectPayloadV3::Unlink { .. } => v3_edges = v3_edges.saturating_sub(1),
                        EffectPayloadV3::Close { .. } => {
                            v3_closed.insert(entry.strand_id.clone());
                        }
                        EffectPayloadV3::Reopen {} => {
                            v3_closed.remove(&entry.strand_id);
                        }
                        EffectPayloadV3::Hide {} => {
                            *v3_hidden.entry(entry.strand_id.clone()).or_default() += 1;
                        }
                        EffectPayloadV3::Unhide {} => {
                            *v3_hidden.entry(entry.strand_id.clone()).or_default() -= 1;
                        }
                    }
                }
            }
        }
    }

    if map.strands.len() != v2_strands.len() {
        mismatches.push(format!(
            "strand map size {} != v2 strand count {}",
            map.strands.len(),
            v2_strands.len()
        ));
    }
    for old in &v2_strands {
        if !map.strands.contains_key(old) {
            mismatches.push(format!("missing strand map for {old}"));
        }
    }
    if v3_strands.len() != map.strands.len() {
        mismatches.push(format!(
            "v3 strand count {} != mapped {}",
            v3_strands.len(),
            map.strands.len()
        ));
    }
    // Entry *event* counts may expand on key-tombstone unlinks (1→N). Compare
    // net edge/lifecycle state instead of raw entry counts.
    if v2_edges != v3_edges {
        mismatches.push(format!("edge count v2={v2_edges} v3={v3_edges}"));
    }
    let closed_v2 = v2_closed.len();
    let closed_v3 = v3_closed.len();
    if closed_v2 != closed_v3 {
        mismatches.push(format!("closed count v2={closed_v2} v3={closed_v3}"));
    }
    let hidden_v2 = v2_hidden.values().filter(|n| **n > 0).count();
    let hidden_v3 = v3_hidden.values().filter(|n| **n > 0).count();
    if hidden_v2 != hidden_v3 {
        mismatches.push(format!("hidden count v2={hidden_v2} v3={hidden_v3}"));
    }

    // Compare each strand's durable projection, not only aggregate counts.
    // Entry counts may legitimately expand for legacy key tombstones, but
    // identity metadata, lifecycle, visibility, and live typed edges must be
    // equivalent after applying the migration map.
    let v2_projected = crate::projection::project_strands(source, true);
    let v3_events: Vec<(usize, Event)> = records_to_v2_events(records)
        .into_iter()
        .enumerate()
        .map(|(index, event)| (index + 1, event))
        .collect();
    let v3_projected = crate::projection::project_strands(&v3_events, true);
    for old in &v2_projected {
        let Some(new_id) = map.strands.get(&old.id) else {
            continue;
        };
        let Some(new) = v3_projected.iter().find(|strand| &strand.id == new_id) else {
            mismatches.push(format!(
                "mapped strand {} missing from v3 projection",
                old.id
            ));
            continue;
        };
        if old.slug != new.slug {
            mismatches.push(format!("strand {} slug changed", old.id));
        }
        if old.strand_type != new.strand_type {
            mismatches.push(format!("strand {} type changed", old.id));
        }
        if old.hidden != new.hidden {
            mismatches.push(format!("strand {} visibility changed", old.id));
        }
        let old_state = crate::projection::compute_state_from_events(source, &old.id).0;
        let new_state = crate::projection::compute_state_from_events(&v3_events, new_id).0;
        if old_state != new_state {
            mismatches.push(format!(
                "strand {} lifecycle changed: {} -> {}",
                old.id, old_state, new_state
            ));
        }
        let map_targets = |targets: &[String]| -> Vec<String> {
            let mut mapped: Vec<String> = targets
                .iter()
                .map(|target| {
                    map.strands
                        .get(target)
                        .cloned()
                        .unwrap_or_else(|| format!("unmapped:{target}"))
                })
                .collect();
            mapped.sort();
            mapped
        };
        let mut new_belongs = new.belongs_to_edges.clone();
        new_belongs.sort();
        if map_targets(&old.belongs_to_edges) != new_belongs {
            mismatches.push(format!("strand {} belongs-to projection changed", old.id));
        }
        let mut new_depends = new.depends_on_edges.clone();
        new_depends.sort();
        if map_targets(&old.depends_on_edges) != new_depends {
            mismatches.push(format!("strand {} depends-on projection changed", old.id));
        }
    }
    let _ = v2_entry_events;

    ProjectionEquivalence {
        ok: mismatches.is_empty(),
        strand_count_v2: v2_strands.len(),
        strand_count_v3: v3_strands.len(),
        entry_count_v2: v2_entry_events,
        entry_count_v3: v3_entries,
        edge_count_v2: v2_edges,
        edge_count_v3: v3_edges,
        closed_count_v2: closed_v2,
        closed_count_v3: closed_v3,
        hidden_count_v2: hidden_v2,
        hidden_count_v3: hidden_v3,
        mismatches,
    }
}

pub(crate) fn default_history_path(journal_dir: &Path) -> PathBuf {
    journal_dir.join(HISTORY_V2_REL)
}

pub(crate) fn default_target_path(journal_dir: &Path) -> PathBuf {
    journal_dir.join(TARGET_V3_REL)
}

pub(crate) fn default_map_path(journal_dir: &Path) -> PathBuf {
    journal_dir.join(MAP_REL)
}

pub(crate) fn default_certificate_path(journal_dir: &Path) -> PathBuf {
    journal_dir.join(CERTIFICATE_REL)
}

/// Apply a prepared v2→v3 cutover under the exclusive journal lock.
/// Source is copied (not moved) into history before activation so the pre-commit
/// resolver still sees a complete `journal.jsonl`.
pub(crate) fn apply_cutover_v3(
    journal_dir: &Path,
    source_journal: &Path,
    plan: &CutoverV3Plan,
) -> Result<CutoverV3ApplyOutcome, String> {
    let lock_path = journal_dir.join("journal.lock");
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("cannot open journal.lock: {e}"))?;
    fs2::FileExt::lock_exclusive(&lock_file)
        .map_err(|e| format!("cannot acquire journal lock: {e}"))?;

    let result = (|| {
        // Final source recheck under the same exclusive lock as v2 writers.
        let source_bytes = std::fs::read(source_journal)
            .map_err(|e| format!("read source journal {}: {e}", source_journal.display()))?;
        let source_sha = sha256_bytes(&source_bytes);
        if source_sha != plan.map.source_sha256 {
            return Err(format!(
                "migration-source-changed: expected sha256 {}, found {}; rerun cutover-v3",
                plan.map.source_sha256, source_sha
            ));
        }

        let history_path = default_history_path(journal_dir);
        let target_path = default_target_path(journal_dir);
        let map_path = default_map_path(journal_dir);
        let certificate_path = default_certificate_path(journal_dir);

        std::fs::create_dir_all(journal_dir.join("history"))
            .map_err(|e| format!("create history dir: {e}"))?;
        std::fs::create_dir_all(journal_dir.join("journals"))
            .map_err(|e| format!("create journals dir: {e}"))?;

        // Prepare history as a verified copy (never pre-commit move/remove source).
        install_bytes_idempotent(&history_path, &source_bytes, "history v2 archive")?;

        let target_sha = if target_path.exists() {
            let existing = std::fs::read(&target_path)
                .map_err(|e| format!("read existing target {}: {e}", target_path.display()))?;
            let expected = encode_records_bytes(&plan.records)?;
            if existing != expected {
                return Err(format!(
                    "migration artifact conflict: {} already differs from this plan",
                    target_path.display()
                ));
            }
            sha256_bytes(&existing)
        } else {
            write_records_prepared(&target_path, &plan.map.target_journal_id, &plan.records)?
        };

        let map_json =
            serde_jcs::to_vec(&plan.map).map_err(|e| format!("canonicalize migration map: {e}"))?;
        install_bytes_idempotent(&map_path, &map_json, "migration map")?;
        let map_sha = sha256_bytes(&map_json);

        let certificate = CutoverV3Certificate {
            schema: CERTIFICATE_SCHEMA.to_string(),
            created_at: canonicalize_timestamp(
                "certificate created_at",
                &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            )?,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            tool_commit: env!("MNEMA_COMMIT").to_string(),
            migration_id: plan.map.migration_id.clone(),
            source_journal_id: plan.map.source_journal_id.clone(),
            source_journal: source_journal.display().to_string(),
            history_journal: history_path.display().to_string(),
            target_journal: target_path.display().to_string(),
            map_path: map_path.display().to_string(),
            source_event_count: plan.map.source_event_count,
            source_sha256: plan.map.source_sha256.clone(),
            target_record_count: plan.map.target_record_count,
            target_sha256: target_sha.clone(),
            map_sha256: map_sha,
            history_sha256: source_sha,
            unresolved_ref_count: plan.map.unresolved_refs.len(),
        };
        let cert_json = serde_jcs::to_vec(&certificate)
            .map_err(|e| format!("canonicalize certificate: {e}"))?;
        if certificate_path.exists() {
            let existing = std::fs::read(&certificate_path).map_err(|e| {
                format!(
                    "read existing certificate {}: {e}",
                    certificate_path.display()
                )
            })?;
            let text = std::str::from_utf8(&existing).map_err(|e| {
                format!(
                    "parse existing certificate {} as UTF-8: {e}",
                    certificate_path.display()
                )
            })?;
            let parsed: CutoverV3Certificate = crate::strict_json::from_str(text).map_err(|e| {
                format!(
                    "parse existing certificate {}: {e}",
                    certificate_path.display()
                )
            })?;
            let canonical = serde_jcs::to_vec(&parsed)
                .map_err(|e| format!("canonicalize existing certificate: {e}"))?;
            if existing != canonical {
                return Err(format!(
                    "migration artifact conflict: {} is not canonical JCS",
                    certificate_path.display()
                ));
            }
            if parsed.schema != CERTIFICATE_SCHEMA
                || parsed.migration_id != certificate.migration_id
                || parsed.source_sha256 != certificate.source_sha256
                || parsed.target_sha256 != certificate.target_sha256
                || parsed.map_sha256 != certificate.map_sha256
            {
                return Err(format!(
                    "migration artifact conflict: {}",
                    certificate_path.display()
                ));
            }
        } else {
            install_bytes_idempotent(&certificate_path, &cert_json, "migration certificate")?;
        }

        // Hash the exact on-disk map/certificate bytes for the origin commitment.
        let map_bytes_on_disk = std::fs::read(&map_path)
            .map_err(|e| format!("re-read map {}: {e}", map_path.display()))?;
        let map_sha_on_disk = sha256_bytes(&map_bytes_on_disk);
        let cert_bytes_on_disk = std::fs::read(&certificate_path)
            .map_err(|e| format!("re-read certificate {}: {e}", certificate_path.display()))?;
        let cert_sha_on_disk = sha256_bytes(&cert_bytes_on_disk);

        // Re-verify history hash equals prepared source before commit.
        let history_bytes = std::fs::read(&history_path)
            .map_err(|e| format!("re-read history {}: {e}", history_path.display()))?;
        if sha256_bytes(&history_bytes) != plan.map.source_sha256 {
            return Err("history artifact hash diverged before activation".to_string());
        }

        let manifest = ActiveJournalManifestV3 {
            schema: ACTIVE_MANIFEST_SCHEMA.to_string(),
            journal_id: plan.map.target_journal_id.clone(),
            active: JournalArtifactV3 {
                schema: "v3".to_string(),
                path: TARGET_V3_REL.to_string(),
                sha256: target_sha,
            },
            history: vec![HistoricalJournalV3 {
                schema: "v2".to_string(),
                path: HISTORY_V2_REL.to_string(),
                sha256: plan.map.source_sha256.clone(),
            }],
            origin: ActivationOriginV3::Migration {
                id: plan.map.migration_id.clone(),
                map_path: MAP_REL.to_string(),
                map_sha256: map_sha_on_disk,
                certificate_path: CERTIFICATE_REL.to_string(),
                certificate_sha256: cert_sha_on_disk,
            },
        };

        match activate_initial_v3(journal_dir, &manifest)? {
            ActivationOutcome::Activated => Ok(CutoverV3ApplyOutcome::Applied),
            ActivationOutcome::ActivatedDurabilityUncertain { .. } => {
                Ok(CutoverV3ApplyOutcome::AppliedDurabilityUncertain)
            }
            ActivationOutcome::AlreadyActive => Ok(CutoverV3ApplyOutcome::AlreadyActive),
        }
    })();

    let _ = fs2::FileExt::unlock(&lock_file);
    result
}

fn install_bytes_idempotent(path: &Path, bytes: &[u8], label: &str) -> Result<(), String> {
    if path.exists() {
        let existing = std::fs::read(path)
            .map_err(|e| format!("read existing {label} {}: {e}", path.display()))?;
        if existing != bytes {
            return Err(format!(
                "migration artifact conflict: {label} {} already differs",
                path.display()
            ));
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create parent for {}: {e}", path.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("create {label} {}: {e}", path.display()))?;
    use std::io::Write;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("persist {label} {}: {e}", path.display()))?;
    Ok(())
}

fn encode_records_bytes(records: &[JournalRecordV3]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for record in records {
        let bytes = serde_jcs::to_vec(record)
            .map_err(|e| format!("canonicalize record for compare: {e}"))?;
        out.extend_from_slice(&bytes);
        out.push(b'\n');
    }
    Ok(out)
}

/// Project v3 journal records into the legacy Event stream so existing
/// projections and CLI paths can read a v3-active journal.
pub(crate) fn records_to_v2_events(records: &[JournalRecordV3]) -> Vec<Event> {
    let mut events = Vec::new();
    let mut genesis_emitted: HashSet<String> = HashSet::new();

    for record in records {
        match record {
            JournalRecordV3::Entry(entry) => {
                if !genesis_emitted.contains(&entry.strand_id) {
                    let (slug, strand_type) = match &entry.entry.strand {
                        StrandKeyV3::Genesis {
                            slug, strand_type, ..
                        } => (slug.clone(), strand_type.clone()),
                        StrandKeyV3::Existing { .. } => (None, None),
                    };
                    events.push(Event::StrandCreated {
                        id: entry.strand_id.clone(),
                        ts: entry.entry.created_at.clone(),
                        strand_type,
                        slug,
                    });
                    genesis_emitted.insert(entry.strand_id.clone());
                }
                let effect = if entry.entry.kind == "effect" {
                    serde_json::from_value::<EffectPayloadV3>(entry.entry.payload.clone())
                        .ok()
                        .map(effect_payload_to_v2)
                } else {
                    None
                };
                let refs: Vec<String> = entry
                    .entry
                    .refs
                    .iter()
                    .map(|r| match r {
                        RefV3::Entry { entry_id, .. } => entry_id.clone(),
                        RefV3::Strand { strand_id, .. } => strand_id.clone(),
                        RefV3::External { scheme, locator } => format!("{scheme}:{locator}"),
                    })
                    .collect();
                if entry.entry.kind == "checkpoint" {
                    if let Ok(cp) =
                        serde_json::from_value::<CheckpointPayloadV3>(entry.entry.payload.clone())
                    {
                        events.push(Event::CheckpointCreated {
                            id: entry.strand_id.clone(),
                            ts: entry.entry.created_at.clone(),
                            observed: cp.observed,
                            action: cp.action,
                            append_id: None,
                            provenance: Some(entry.entry.provenance.clone()),
                        });
                        continue;
                    }
                }
                if entry.entry.kind == "subject_binding" {
                    if let Ok(sb) = serde_json::from_value::<SubjectBindingPayloadV3>(
                        entry.entry.payload.clone(),
                    ) {
                        events.push(Event::SubjectBound {
                            id: entry.entry_id.clone(),
                            ts: entry.entry.created_at.clone(),
                            subject_type: sb.subject_type,
                            subject_id: sb.subject_id,
                            strand_id: entry.strand_id.clone(),
                            provenance: Some(entry.entry.provenance.clone()),
                        });
                        continue;
                    }
                }
                events.push(Event::LogAppended {
                    id: entry.strand_id.clone(),
                    ts: entry.entry.created_at.clone(),
                    content: entry.entry.body.clone(),
                    effect,
                    prev_entry_id: entry.entry.prev.clone(),
                    entry_id: Some(entry.entry_id.clone()),
                    refs,
                    ref_: None,
                    append_id: None,
                    git: None,
                    provenance: Some(entry.entry.provenance.clone()),
                });
            }
            JournalRecordV3::Anchor(anchor) => {
                events.push(Event::JournalAnchored {
                    ts: anchor.created_at.clone(),
                    covered_event_count: anchor.covered_record_count as usize,
                    heads: anchor
                        .heads
                        .iter()
                        .map(|h| crate::event::JournalAnchorHead {
                            strand_id: h.strand_id.clone(),
                            entry_id: h.entry_id.clone(),
                        })
                        .collect(),
                    digest: anchor.digest.clone(),
                    previous_anchor: anchor.previous_anchor.clone(),
                });
            }
        }
    }
    events
}

fn effect_payload_to_v2(effect: EffectPayloadV3) -> EntryEffect {
    match effect {
        EffectPayloadV3::Close { disposition } => EntryEffect::Close {
            disposition: match disposition {
                CloseDispositionV3::Done => "done",
                CloseDispositionV3::Failed => "failed",
                CloseDispositionV3::Cancelled => "cancelled",
                CloseDispositionV3::Merged => "merged",
                CloseDispositionV3::Verified => "verified",
            }
            .to_string(),
        },
        EffectPayloadV3::Reopen {} => EntryEffect::Reopen,
        EffectPayloadV3::Link {
            edge_type,
            target_strand_id,
        } => EntryEffect::Link {
            target: target_strand_id,
            edge_type: match edge_type {
                EdgeTypeV3::BelongsTo => "belongs-to",
                EdgeTypeV3::DependsOn => "depends-on",
            }
            .to_string(),
        },
        EffectPayloadV3::Unlink {
            edge_type,
            target_strand_id,
            link_entry_id,
        } => EntryEffect::Unlink {
            target: target_strand_id,
            edge_type: match edge_type {
                EdgeTypeV3::BelongsTo => "belongs-to",
                EdgeTypeV3::DependsOn => "depends-on",
            }
            .to_string(),
            link_entry_id: Some(link_entry_id),
        },
        EffectPayloadV3::Hide {} => EntryEffect::Hide,
        EffectPayloadV3::Unhide {} => EntryEffect::Unhide,
    }
}

pub(crate) fn plan_from_journal_dir(journal_dir: &Path) -> Result<CutoverV3Plan, String> {
    if load_active_manifest(journal_dir)?.is_some() {
        return Err("active-journal.json already present; journal is already on v3".to_string());
    }
    let source_path = journal_dir.join("journal.jsonl");
    if !source_path.exists() {
        return Err(format!("source journal missing: {}", source_path.display()));
    }
    let source_bytes = std::fs::read(&source_path)
        .map_err(|e| format!("read source journal {}: {e}", source_path.display()))?;
    let source_sha = sha256_bytes(&source_bytes);
    let read = journal::read_journal_lossy(&source_path);
    if let Some(error) = read.read_error {
        return Err(error);
    }
    if !read.diagnostics.is_empty() {
        return Err(format!(
            "cannot cut over: journal has {} parse error(s); run doctor first",
            read.diagnostics.len()
        ));
    }
    let journal_id = journal::existing_journal_id_in(journal_dir)?
        .ok_or_else(|| {
            "journal-id.json is missing; run mnema init once before planning cutover-v3".to_string()
        })?
        .to_ascii_lowercase();
    if journal_id.len() != 64 || !journal_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("journal_id sidecar is not 64 hex".to_string());
    }
    build_cutover_v3_plan(&journal_id, &source_sha, &read.events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event;

    fn pure_v2_fixture() -> (String, Vec<(usize, Event)>) {
        let journal_id = "aa".repeat(32);
        let (created, appended) = event::make_strand_created("root task", Some("task"));
        let parent_id = match &created {
            Event::StrandCreated { id, .. } => id.clone(),
            _ => unreachable!(),
        };
        let (child_c, child_a) = event::make_strand_created("child", Some("task"));
        let child_id = match &child_c {
            Event::StrandCreated { id, .. } => id.clone(),
            _ => unreachable!(),
        };
        let link = Event::EdgeLinked {
            id: child_id.clone(),
            ts: "2026-07-11T00:00:10Z".to_string(),
            to: parent_id,
            edge_type: Some("belongs-to".to_string()),
            provenance: None,
        };
        let close = Event::StrandClosed {
            id: child_id,
            ts: "2026-07-11T00:00:11Z".to_string(),
            disposition: "done".to_string(),
            provenance: None,
        };
        let events = vec![created, appended, child_c, child_a, link, close];
        let numbered: Vec<_> = events
            .into_iter()
            .enumerate()
            .map(|(i, e)| (i + 1, e))
            .collect();
        (journal_id, numbered)
    }

    fn event_entry_id(event: &Event) -> String {
        match event {
            Event::LogAppended {
                entry_id: Some(id), ..
            } => id.clone(),
            _ => panic!("expected chained log entry"),
        }
    }

    #[test]
    fn converter_is_deterministic_and_equivalent() {
        let (journal_id, source) = pure_v2_fixture();
        let sha = "bb".repeat(32);
        let plan1 = build_cutover_v3_plan(&journal_id, &sha, &source).unwrap();
        let plan2 = build_cutover_v3_plan(&journal_id, &sha, &source).unwrap();
        assert_eq!(plan1.map.migration_id, plan2.map.migration_id);
        assert_eq!(
            encode_records_bytes(&plan1.records).unwrap(),
            encode_records_bytes(&plan2.records).unwrap()
        );
        assert!(plan1.equivalence.ok);
        assert_eq!(plan1.equivalence.strand_count_v2, 2);
        assert_eq!(plan1.map.strands.len(), 2);
        assert!(
            plan1
                .records
                .iter()
                .any(|r| matches!(r, JournalRecordV3::Anchor(_)))
        );
    }

    #[test]
    fn unsupported_edge_type_is_rejected_before_activation() {
        let journal_id = "aa".repeat(32);
        let (created, appended) = event::make_strand_created("root", Some("task"));
        let id = match &created {
            Event::StrandCreated { id, .. } => id.clone(),
            _ => unreachable!(),
        };
        let bad = Event::EdgeLinked {
            id,
            ts: "2026-07-11T00:00:10Z".to_string(),
            to: "bb".repeat(32),
            edge_type: Some("blocks".to_string()),
            provenance: None,
        };
        let source: Vec<_> = vec![created, appended, bad]
            .into_iter()
            .enumerate()
            .map(|(i, e)| (i + 1, e))
            .collect();
        let err = build_cutover_v3_plan(&journal_id, &"cc".repeat(32), &source).unwrap_err();
        assert!(err.contains("migration-source-invalid"), "{err}");
    }

    #[test]
    fn apply_installs_manifest_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mnema = dir.path().join(".mnema");
        std::fs::create_dir_all(&mnema).unwrap();
        let (journal_id, source) = pure_v2_fixture();
        std::fs::write(
            mnema.join("journal-id.json"),
            serde_json::json!({ "journal_id": journal_id }).to_string(),
        )
        .unwrap();
        let mut lines = Vec::new();
        for (_, event) in &source {
            lines.push(serde_json::to_string(event).unwrap());
        }
        let source_path = mnema.join("journal.jsonl");
        std::fs::write(&source_path, lines.join("\n") + "\n").unwrap();
        let source_bytes = std::fs::read(&source_path).unwrap();
        let plan =
            build_cutover_v3_plan(&journal_id, &sha256_bytes(&source_bytes), &source).unwrap();

        let first = apply_cutover_v3(&mnema, &source_path, &plan).unwrap();
        assert!(matches!(
            first,
            CutoverV3ApplyOutcome::Applied | CutoverV3ApplyOutcome::AppliedDurabilityUncertain
        ));
        assert!(mnema.join("active-journal.json").exists());
        assert!(mnema.join(HISTORY_V2_REL).exists());
        assert!(mnema.join(TARGET_V3_REL).exists());
        // Source remains (copy, not move).
        assert!(source_path.exists());
        let map_bytes = std::fs::read(mnema.join(MAP_REL)).unwrap();
        let parsed_map: CutoverV3Map =
            crate::strict_json::from_str(std::str::from_utf8(&map_bytes).unwrap()).unwrap();
        assert_eq!(map_bytes, serde_jcs::to_vec(&parsed_map).unwrap());
        let certificate_bytes = std::fs::read(mnema.join(CERTIFICATE_REL)).unwrap();
        let parsed_certificate: CutoverV3Certificate =
            crate::strict_json::from_str(std::str::from_utf8(&certificate_bytes).unwrap()).unwrap();
        assert_eq!(
            certificate_bytes,
            serde_jcs::to_vec(&parsed_certificate).unwrap()
        );

        let second = apply_cutover_v3(&mnema, &source_path, &plan).unwrap();
        assert_eq!(second, CutoverV3ApplyOutcome::AlreadyActive);
    }

    #[test]
    fn dry_run_does_not_write_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let mnema = dir.path().join(".mnema");
        std::fs::create_dir_all(&mnema).unwrap();
        let (journal_id, source) = pure_v2_fixture();
        std::fs::write(
            mnema.join("journal-id.json"),
            serde_json::json!({ "journal_id": journal_id }).to_string(),
        )
        .unwrap();
        let mut lines = Vec::new();
        for (_, event) in &source {
            lines.push(serde_json::to_string(event).unwrap());
        }
        std::fs::write(mnema.join("journal.jsonl"), lines.join("\n") + "\n").unwrap();
        let plan = plan_from_journal_dir(&mnema).unwrap();
        assert!(plan.equivalence.ok);
        assert!(!mnema.join("active-journal.json").exists());
        assert!(!mnema.join("history").exists());
        assert!(!mnema.join("journals").exists());
    }

    #[test]
    fn dry_run_does_not_create_missing_journal_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mnema = dir.path().join(".mnema");
        std::fs::create_dir_all(&mnema).unwrap();
        std::fs::write(mnema.join("journal.jsonl"), "").unwrap();
        let error = plan_from_journal_dir(&mnema).unwrap_err();
        assert!(error.contains("journal-id.json is missing"), "{error}");
        assert!(!mnema.join("journal-id.json").exists());
        assert!(!mnema.join("history").exists());
        assert!(!mnema.join("journals").exists());
    }

    #[test]
    fn multi_refs_are_typed_and_fully_mapped() {
        let journal_id = "aa".repeat(32);
        let (first_created, first_entry) = event::make_strand_created("first", Some("task"));
        let first_id = event_entry_id(&first_entry);
        let (second_created, second_entry) = event::make_strand_created("second", Some("task"));
        let second_id = event_entry_id(&second_entry);
        let (consumer_created, consumer_entry) = event::make_strand_created_with_refs_and_slug(
            "consumer",
            Some("task"),
            vec![first_id, second_id],
            None,
            None,
            Some("consumer"),
        );
        let source: Vec<_> = vec![
            first_created,
            first_entry,
            second_created,
            second_entry,
            consumer_created,
            consumer_entry,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, event)| (index + 1, event))
        .collect();
        let plan = build_cutover_v3_plan(&journal_id, &"bb".repeat(32), &source).unwrap();
        let consumer = plan
            .records
            .iter()
            .find_map(|record| match record {
                JournalRecordV3::Entry(entry) if entry.entry.body == "consumer" => Some(entry),
                _ => None,
            })
            .unwrap();
        assert_eq!(2, consumer.entry.refs.len());
        assert!(consumer.entry.refs.iter().all(|reference| matches!(
            reference,
            RefV3::Entry { journal_id: id, .. } if id == &journal_id
        )));
        assert_eq!(2, plan.map.refs.len());
        assert!(plan.map.unresolved_refs.is_empty());
    }

    #[test]
    fn typed_unlink_is_not_reexpanded_by_later_key_tombstone() {
        let journal_id = "aa".repeat(32);
        let (parent_created, parent_entry) = event::make_strand_created("parent", Some("task"));
        let parent_id = parent_created.strand_id().unwrap().to_string();
        let (child_created, child_entry) = event::make_strand_created("child", Some("task"));
        let child_id = child_created.strand_id().unwrap().to_string();
        let child_head = event_entry_id(&child_entry);
        let link = event::make_edge_linked(
            &child_id,
            Some(&child_head),
            &parent_id,
            Some("depends-on"),
            None,
        );
        let link_id = event_entry_id(&link);
        let typed_unlink = event::make_log_appended_entry_with_effect(
            &child_id,
            Some(&link_id),
            "unlink depends-on parent",
            Vec::new(),
            None,
            Some(EntryEffect::Unlink {
                target: parent_id.clone(),
                edge_type: "depends-on".to_string(),
                link_entry_id: Some(link_id.clone()),
            }),
            None,
        );
        let legacy_tombstone = Event::EdgeUnlinked {
            id: child_id,
            ts: "2026-07-11T00:00:12Z".to_string(),
            to: parent_id,
            edge_type: Some("depends-on".to_string()),
            provenance: None,
        };
        let source: Vec<_> = vec![
            parent_created,
            parent_entry,
            child_created,
            child_entry,
            link,
            typed_unlink,
            legacy_tombstone,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, event)| (index + 1, event))
        .collect();
        let plan = build_cutover_v3_plan(&journal_id, &"bb".repeat(32), &source).unwrap();
        let last = plan.map.source_records.last().unwrap();
        assert_eq!("unlink_noop_no_live_links", last.disposition);
        assert!(last.new_entry_ids.is_empty());
        assert_eq!(0, plan.equivalence.edge_count_v3);
    }

    #[test]
    fn migration_artifact_schemas_reject_unknown_fields() {
        let (journal_id, source) = pure_v2_fixture();
        let plan = build_cutover_v3_plan(&journal_id, &"bb".repeat(32), &source).unwrap();
        let mut value = serde_json::to_value(&plan.map).unwrap();
        value["future"] = serde_json::json!(true);
        let text = serde_json::to_string(&value).unwrap();
        assert!(crate::strict_json::from_str::<CutoverV3Map>(&text).is_err());
    }
}
