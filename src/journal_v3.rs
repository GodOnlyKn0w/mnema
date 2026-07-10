use crate::canonical::{
    AnchorHeadV3, AnchorRecordV3, JournalRecordV3, RefV3, StrandKeyV3, validate_full_hex,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

const RECORD_SEQUENCE_DOMAIN: &[u8] = b"mnema.journal-sequence.v3\0";

pub(crate) fn read_records_strict(
    path: &Path,
    journal_id: &str,
) -> Result<Vec<JournalRecordV3>, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("open v3 journal {}: {error}", path.display()))?;
    let mut records = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line_number = index + 1;
        let line = line.map_err(|error| format!("read v3 journal line {line_number}: {error}"))?;
        if line.is_empty() {
            return Err(format!("v3 journal line {line_number} is empty"));
        }
        let record: JournalRecordV3 = serde_json::from_str(&line)
            .map_err(|error| format!("parse v3 journal line {line_number}: {error}"))?;
        record
            .validate()
            .map_err(|error| format!("invalid v3 journal line {line_number}: {error}"))?;
        records.push(record);
    }
    validate_records(journal_id, &records)?;
    Ok(records)
}

pub(crate) fn write_records_prepared(
    path: &Path,
    journal_id: &str,
    records: &[JournalRecordV3],
) -> Result<String, String> {
    validate_records(journal_id, records)?;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("create v3 journal {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    for record in records {
        let bytes = serde_jcs::to_vec(record)
            .map_err(|error| format!("canonicalize v3 journal record: {error}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| format!("write v3 journal {}: {error}", path.display()))?;
        hasher.update(&bytes);
        hasher.update(b"\n");
    }
    file.sync_all()
        .map_err(|error| format!("sync v3 journal {}: {error}", path.display()))?;
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn validate_records(
    journal_id: &str,
    records: &[JournalRecordV3],
) -> Result<(), String> {
    replay_records(journal_id, records, true).map(|_| ())
}

pub(crate) fn make_anchor(
    journal_id: &str,
    records: &[JournalRecordV3],
    created_at: impl Into<String>,
) -> Result<JournalRecordV3, String> {
    let state = replay_records(journal_id, records, false)?;
    let covered_record_count = u64::try_from(records.len())
        .map_err(|_| "v3 journal record count does not fit u64".to_string())?;
    let heads = state
        .heads
        .into_iter()
        .map(|(strand_id, entry_id)| AnchorHeadV3 {
            strand_id,
            entry_id,
        })
        .collect();
    Ok(JournalRecordV3::Anchor(AnchorRecordV3::new(
        created_at,
        covered_record_count,
        state.previous_anchor,
        sequence_digest(&state.sequence_hasher),
        heads,
    )?))
}

struct ReplayState {
    heads: BTreeMap<String, String>,
    previous_anchor: Option<String>,
    sequence_hasher: Sha256,
}

fn replay_records(
    journal_id: &str,
    records: &[JournalRecordV3],
    require_final_anchor: bool,
) -> Result<ReplayState, String> {
    validate_full_hex("journal_id", journal_id)?;
    let mut heads: BTreeMap<String, String> = BTreeMap::new();
    let mut entries: HashSet<String> = HashSet::new();
    let mut previous_anchor: Option<String> = None;
    let mut sequence_hasher = Sha256::new();
    sequence_hasher.update(RECORD_SEQUENCE_DOMAIN);

    for (index, record) in records.iter().enumerate() {
        record
            .validate()
            .map_err(|error| format!("record {index}: {error}"))?;
        match record {
            JournalRecordV3::Entry(record) => {
                if entries.contains(&record.entry_id) {
                    return Err(format!(
                        "record {index}: duplicate entry id {}",
                        record.entry_id
                    ));
                }
                validate_local_refs(index, journal_id, &record.entry.refs, &entries, &heads)?;
                match &record.entry.strand {
                    StrandKeyV3::Genesis { .. } => {
                        if heads
                            .insert(record.strand_id.clone(), record.entry_id.clone())
                            .is_some()
                        {
                            return Err(format!(
                                "record {index}: duplicate genesis strand {}",
                                record.strand_id
                            ));
                        }
                    }
                    StrandKeyV3::Existing { id } => {
                        let expected = heads.get(id).ok_or_else(|| {
                            format!("record {index}: existing strand {id} has no genesis")
                        })?;
                        let actual_prev = record.entry.prev.as_deref().ok_or_else(|| {
                            format!("record {index}: existing strand {id} has no prev")
                        })?;
                        if actual_prev != expected {
                            return Err(format!(
                                "record {index}: prev mismatch for strand {id}: expected {expected}, found {actual_prev}"
                            ));
                        }
                        heads.insert(id.clone(), record.entry_id.clone());
                    }
                }
                entries.insert(record.entry_id.clone());
            }
            JournalRecordV3::Anchor(anchor) => {
                let expected: Vec<AnchorHeadV3> = heads
                    .iter()
                    .map(|(strand_id, entry_id)| AnchorHeadV3 {
                        strand_id: strand_id.clone(),
                        entry_id: entry_id.clone(),
                    })
                    .collect();
                if anchor.heads != expected {
                    return Err(format!(
                        "record {index}: anchor heads do not match replayed strand heads"
                    ));
                }
                if anchor.covered_record_count != index as u64 {
                    return Err(format!(
                        "record {index}: anchor covers {} records, expected {index}",
                        anchor.covered_record_count
                    ));
                }
                if anchor.previous_anchor != previous_anchor {
                    return Err(format!(
                        "record {index}: anchor previous digest does not match prior anchor"
                    ));
                }
                let expected_sequence = sequence_digest(&sequence_hasher);
                if anchor.covered_records_digest != expected_sequence {
                    return Err(format!(
                        "record {index}: anchor covered-record digest mismatch"
                    ));
                }
                previous_anchor = Some(anchor.digest.clone());
            }
        }
        update_sequence_digest(&mut sequence_hasher, record)
            .map_err(|error| format!("record {index}: {error}"))?;
    }
    if require_final_anchor && !matches!(records.last(), Some(JournalRecordV3::Anchor(_))) {
        return Err(
            "v3 journal is missing its final anchor (unanchored tail or truncation)".to_string(),
        );
    }
    Ok(ReplayState {
        heads,
        previous_anchor,
        sequence_hasher,
    })
}

fn update_sequence_digest(hasher: &mut Sha256, record: &JournalRecordV3) -> Result<(), String> {
    let bytes = serde_jcs::to_vec(record)
        .map_err(|error| format!("canonicalize record for sequence digest: {error}"))?;
    let length = u64::try_from(bytes.len())
        .map_err(|_| "canonical record length does not fit u64".to_string())?;
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
    Ok(())
}

fn sequence_digest(hasher: &Sha256) -> String {
    hex::encode(hasher.clone().finalize())
}

fn validate_local_refs(
    index: usize,
    journal_id: &str,
    refs: &[RefV3],
    entries: &HashSet<String>,
    heads: &BTreeMap<String, String>,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for reference in refs {
        let key = serde_jcs::to_vec(reference)
            .map_err(|error| format!("record {index}: canonicalize ref: {error}"))?;
        if !seen.insert(key) {
            return Err(format!("record {index}: duplicate ref"));
        }
        match reference {
            RefV3::Entry {
                journal_id: target_journal,
                entry_id,
            } if target_journal == journal_id && !entries.contains(entry_id) => {
                return Err(format!(
                    "record {index}: local entry ref {entry_id} is not an earlier entry"
                ));
            }
            RefV3::Strand {
                journal_id: target_journal,
                strand_id,
            } if target_journal == journal_id && !heads.contains_key(strand_id) => {
                return Err(format!(
                    "record {index}: local strand ref {strand_id} is not an existing strand"
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::EntryHashViewV3;
    use serde_json::Value;

    const CREATED_AT: &str = "2026-07-11T00:00:00Z";

    fn genesis(seed_byte: &str) -> JournalRecordV3 {
        EntryHashViewV3::new(
            StrandKeyV3::Genesis {
                seed: seed_byte.repeat(32),
            },
            None,
            "note",
            "genesis",
            Vec::new(),
            None,
            CREATED_AT,
            Value::Null,
            Value::Null,
        )
        .into_record()
        .unwrap()
    }

    fn append(strand_id: &str, prev: &str, refs: Vec<RefV3>) -> JournalRecordV3 {
        EntryHashViewV3::new(
            StrandKeyV3::Existing {
                id: strand_id.to_string(),
            },
            Some(prev.to_string()),
            "note",
            "next",
            refs,
            None,
            "2026-07-11T00:00:01Z",
            Value::Null,
            Value::Null,
        )
        .into_record()
        .unwrap()
    }

    fn ids(record: &JournalRecordV3) -> (&str, &str) {
        let JournalRecordV3::Entry(entry) = record else {
            panic!("expected entry");
        };
        (&entry.strand_id, &entry.entry_id)
    }

    #[test]
    fn strict_codec_round_trips_a_chain_and_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.v3.jsonl");
        let journal_id = "aa".repeat(32);
        let first = genesis("11");
        let (strand_id, first_id) = ids(&first);
        let second = append(
            strand_id,
            first_id,
            vec![RefV3::Entry {
                journal_id: journal_id.clone(),
                entry_id: first_id.to_string(),
            }],
        );
        let mut records = vec![first, second];
        let anchor = make_anchor(&journal_id, &records, "2026-07-11T00:00:02Z").unwrap();
        records.push(anchor);
        let digest = write_records_prepared(&path, &journal_id, &records).unwrap();
        assert_eq!(digest.len(), 64);
        assert_eq!(read_records_strict(&path, &journal_id).unwrap(), records);
    }

    #[test]
    fn broken_prev_is_rejected() {
        let first = genesis("22");
        let (strand_id, _) = ids(&first);
        let broken = append(strand_id, &"ff".repeat(32), Vec::new());
        let error = validate_records(&"aa".repeat(32), &[first, broken]).unwrap_err();
        assert!(error.contains("prev mismatch"));
    }

    #[test]
    fn anchor_must_equal_replayed_heads() {
        let first = genesis("33");
        let anchor = make_anchor(&"aa".repeat(32), &[], CREATED_AT).unwrap();
        let error = validate_records(&"aa".repeat(32), &[first, anchor]).unwrap_err();
        assert!(error.contains("anchor heads"));
    }

    #[test]
    fn forward_local_ref_is_rejected() {
        let first = genesis("44");
        let (strand_id, first_id) = ids(&first);
        let second = append(
            strand_id,
            first_id,
            vec![RefV3::Entry {
                journal_id: "aa".repeat(32),
                entry_id: "55".repeat(32),
            }],
        );
        let error = validate_records(&"aa".repeat(32), &[first, second]).unwrap_err();
        assert!(error.contains("not an earlier entry"));
    }

    #[test]
    fn v2_event_line_is_not_a_v3_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.v3.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"strand_created","id":"legacy","ts":"2026-01-01T00:00:00Z"}
"#,
        )
        .unwrap();
        assert!(
            read_records_strict(&path, &"aa".repeat(32))
                .unwrap_err()
                .contains("parse v3 journal line 1")
        );
    }

    #[test]
    fn final_anchor_is_required() {
        let error = validate_records(&"aa".repeat(32), &[genesis("66")]).unwrap_err();
        assert!(error.contains("missing its final anchor"));
    }

    #[test]
    fn anchor_commits_cross_strand_record_order() {
        let journal_id = "aa".repeat(32);
        let first = genesis("77");
        let second = genesis("88");
        let mut records = vec![first, second];
        records.push(make_anchor(&journal_id, &records, CREATED_AT).unwrap());
        validate_records(&journal_id, &records).unwrap();

        records.swap(0, 1);
        let error = validate_records(&journal_id, &records).unwrap_err();
        assert!(error.contains("covered-record digest mismatch"));
    }

    #[test]
    fn anchor_chain_detects_deleted_anchor() {
        let journal_id = "aa".repeat(32);
        let mut records = vec![genesis("99")];
        records.push(make_anchor(&journal_id, &records, CREATED_AT).unwrap());
        records.push(make_anchor(&journal_id, &records, "2026-07-11T00:00:01Z").unwrap());
        validate_records(&journal_id, &records).unwrap();

        records.remove(1);
        assert!(validate_records(&journal_id, &records).is_err());
    }

    #[test]
    fn strict_reader_rejects_unknown_nested_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.v3.jsonl");
        let mut value = serde_json::to_value(genesis("aa")).unwrap();
        value["entry"]["entry"]["future_field"] = serde_json::json!(true);
        std::fs::write(&path, format!("{}\n", value)).unwrap();
        let error = read_records_strict(&path, &"aa".repeat(32)).unwrap_err();
        assert!(error.contains("unknown field"), "{error}");
    }
}
