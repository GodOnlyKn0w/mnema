use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const ENTRY_HASH_DOMAIN: &[u8] = b"mnema.entry.v3\0";
const ANCHOR_HASH_DOMAIN: &[u8] = b"mnema.anchor.v3\0";
const V2_IMPORT_SEED_DOMAIN: &[u8] = b"mnema.import.v2\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum StrandKeyV3 {
    Genesis { seed: String },
    Existing { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RefV3 {
    Entry {
        journal_id: String,
        entry_id: String,
    },
    Strand {
        journal_id: String,
        strand_id: String,
    },
    External {
        scheme: String,
        locator: String,
    },
}

/// The exact logical value hashed for a v3 entry.
///
/// Every field is present in canonical JSON. Missing legacy values are encoded
/// as JSON null rather than omitted, so identity does not depend on serde
/// defaults or producer-specific field elision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EntryHashViewV3 {
    schema: String,
    pub(crate) strand: StrandKeyV3,
    pub(crate) prev: Option<String>,
    pub(crate) kind: String,
    pub(crate) body: String,
    pub(crate) refs: Vec<RefV3>,
    pub(crate) author: Option<String>,
    pub(crate) created_at: String,
    pub(crate) payload: Value,
    pub(crate) provenance: Value,
}

impl EntryHashViewV3 {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        strand: StrandKeyV3,
        prev: Option<String>,
        kind: impl Into<String>,
        body: impl Into<String>,
        refs: Vec<RefV3>,
        author: Option<String>,
        created_at: impl Into<String>,
        payload: Value,
        provenance: Value,
    ) -> Self {
        Self {
            schema: "mnema.entry.v3".to_string(),
            strand,
            prev,
            kind: kind.into(),
            body: body.into(),
            refs,
            author,
            created_at: created_at.into(),
            payload,
            provenance,
        }
    }

    pub(crate) fn canonical_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_jcs::to_vec(self).map_err(|error| format!("canonicalize v3 entry: {error}"))
    }

    pub(crate) fn entry_id(&self) -> Result<String, String> {
        let bytes = self.canonical_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(ENTRY_HASH_DOMAIN);
        hasher.update(bytes);
        Ok(hex::encode(hasher.finalize()))
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema != "mnema.entry.v3" {
            return Err(format!("unsupported entry schema {}", self.schema));
        }
        match &self.strand {
            StrandKeyV3::Genesis { seed } => {
                validate_full_hex("genesis seed", seed)?;
                if self.prev.is_some() {
                    return Err("genesis entry must have prev=null".to_string());
                }
            }
            StrandKeyV3::Existing { id } => {
                validate_full_hex("existing strand id", id)?;
                if self.prev.is_none() {
                    return Err("existing-strand entry must name prev".to_string());
                }
            }
        }
        if let Some(prev) = &self.prev {
            validate_full_hex("prev entry id", prev)?;
        }
        if self.kind.trim().is_empty() {
            return Err("entry kind cannot be empty".to_string());
        }
        chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|error| format!("created_at must be RFC3339: {error}"))?;
        for reference in &self.refs {
            match reference {
                RefV3::Entry {
                    journal_id,
                    entry_id,
                } => {
                    validate_full_hex("entry ref journal_id", journal_id)?;
                    validate_full_hex("entry ref entry_id", entry_id)?;
                }
                RefV3::Strand {
                    journal_id,
                    strand_id,
                } => {
                    validate_full_hex("strand ref journal_id", journal_id)?;
                    validate_full_hex("strand ref strand_id", strand_id)?;
                }
                RefV3::External { scheme, locator } => {
                    if scheme.trim().is_empty() || locator.trim().is_empty() {
                        return Err(
                            "external ref requires non-empty scheme and locator".to_string()
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn into_record(self) -> Result<JournalRecordV3, String> {
        let entry_id = self.entry_id()?;
        let strand_id = match &self.strand {
            StrandKeyV3::Genesis { .. } => entry_id.clone(),
            StrandKeyV3::Existing { id } => id.clone(),
        };
        Ok(JournalRecordV3::Entry(EntryRecordV3 {
            entry_id,
            strand_id,
            entry: self,
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EntryRecordV3 {
    pub(crate) entry_id: String,
    pub(crate) strand_id: String,
    pub(crate) entry: EntryHashViewV3,
}

impl EntryRecordV3 {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let computed = self.entry.entry_id()?;
        if self.entry_id != computed {
            return Err(format!(
                "entry id mismatch: stored {}, computed {}",
                self.entry_id, computed
            ));
        }
        let expected_strand = match &self.entry.strand {
            StrandKeyV3::Genesis { .. } => &self.entry_id,
            StrandKeyV3::Existing { id } => id,
        };
        if &self.strand_id != expected_strand {
            return Err(format!(
                "strand id mismatch: stored {}, expected {}",
                self.strand_id, expected_strand
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnchorHeadV3 {
    pub(crate) strand_id: String,
    pub(crate) entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnchorRecordV3 {
    pub(crate) created_at: String,
    pub(crate) heads: Vec<AnchorHeadV3>,
    pub(crate) digest: String,
}

impl AnchorRecordV3 {
    pub(crate) fn new(
        created_at: impl Into<String>,
        mut heads: Vec<AnchorHeadV3>,
    ) -> Result<Self, String> {
        let created_at = created_at.into();
        chrono::DateTime::parse_from_rfc3339(&created_at)
            .map_err(|error| format!("anchor created_at must be RFC3339: {error}"))?;
        heads.sort_by(|left, right| {
            left.strand_id
                .cmp(&right.strand_id)
                .then_with(|| left.entry_id.cmp(&right.entry_id))
        });
        validate_anchor_heads(&heads)?;
        let digest = anchor_digest(&heads)?;
        Ok(Self {
            created_at,
            heads,
            digest,
        })
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|error| format!("anchor created_at must be RFC3339: {error}"))?;
        validate_anchor_heads(&self.heads)?;
        let mut sorted = self.heads.clone();
        sorted.sort_by(|left, right| {
            left.strand_id
                .cmp(&right.strand_id)
                .then_with(|| left.entry_id.cmp(&right.entry_id))
        });
        if sorted != self.heads {
            return Err("anchor heads must be in canonical order".to_string());
        }
        let computed = anchor_digest(&self.heads)?;
        if computed != self.digest {
            return Err(format!(
                "anchor digest mismatch: stored {}, computed {}",
                self.digest, computed
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub(crate) enum JournalRecordV3 {
    Entry(EntryRecordV3),
    Anchor(AnchorRecordV3),
}

impl JournalRecordV3 {
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Entry(entry) => entry.validate(),
            Self::Anchor(anchor) => anchor.validate(),
        }
    }
}

fn anchor_digest(heads: &[AnchorHeadV3]) -> Result<String, String> {
    let canonical =
        serde_jcs::to_vec(heads).map_err(|error| format!("canonicalize anchor heads: {error}"))?;
    let mut hasher = Sha256::new();
    hasher.update(ANCHOR_HASH_DOMAIN);
    hasher.update(canonical);
    Ok(hex::encode(hasher.finalize()))
}

fn validate_anchor_heads(heads: &[AnchorHeadV3]) -> Result<(), String> {
    let mut previous: Option<&str> = None;
    for head in heads {
        validate_full_hex("anchor strand_id", &head.strand_id)?;
        validate_full_hex("anchor entry_id", &head.entry_id)?;
        if previous == Some(head.strand_id.as_str()) {
            return Err(format!(
                "anchor contains duplicate strand {}",
                head.strand_id
            ));
        }
        previous = Some(&head.strand_id);
    }
    Ok(())
}

fn validate_full_hex(field: &str, value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{field} must be 64 hex characters"));
    }
    Ok(())
}

pub(crate) fn deterministic_v2_import_seed(source_journal_id: &str, old_strand_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(V2_IMPORT_SEED_DOMAIN);
    hasher.update(source_journal_id.as_bytes());
    hasher.update([0]);
    hasher.update(old_strand_id.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn genesis(seed: &str, refs: Vec<RefV3>) -> EntryHashViewV3 {
        EntryHashViewV3::new(
            StrandKeyV3::Genesis {
                seed: seed.to_string(),
            },
            None,
            "note",
            "hello",
            refs,
            None,
            "2026-07-11T00:00:00Z",
            Value::Null,
            Value::Null,
        )
    }

    #[test]
    fn canonical_json_is_independent_of_object_insertion_order() {
        let first = EntryHashViewV3::new(
            StrandKeyV3::Genesis {
                seed: "11".repeat(32),
            },
            None,
            "effect",
            "link",
            Vec::new(),
            Some("agent".to_string()),
            "2026-07-11T00:00:00Z",
            json!({"z": 1, "a": 2}),
            json!({"worker": {"z": true, "a": false}}),
        );
        let second = EntryHashViewV3::new(
            StrandKeyV3::Genesis {
                seed: "11".repeat(32),
            },
            None,
            "effect",
            "link",
            Vec::new(),
            Some("agent".to_string()),
            "2026-07-11T00:00:00Z",
            json!({"a": 2, "z": 1}),
            json!({"worker": {"a": false, "z": true}}),
        );
        assert_eq!(
            first.canonical_bytes().unwrap(),
            second.canonical_bytes().unwrap()
        );
        assert_eq!(first.entry_id().unwrap(), second.entry_id().unwrap());
    }

    #[test]
    fn genesis_identity_changes_with_seed() {
        assert_ne!(
            genesis(&"11".repeat(32), Vec::new()).entry_id().unwrap(),
            genesis(&"22".repeat(32), Vec::new()).entry_id().unwrap()
        );
    }

    #[test]
    fn authored_ref_order_participates_in_identity() {
        let a = RefV3::Entry {
            journal_id: "aa".repeat(32),
            entry_id: "11".repeat(32),
        };
        let b = RefV3::Strand {
            journal_id: "aa".repeat(32),
            strand_id: "22".repeat(32),
        };
        let seed = "33".repeat(32);
        assert_ne!(
            genesis(&seed, vec![a.clone(), b.clone()])
                .entry_id()
                .unwrap(),
            genesis(&seed, vec![b, a]).entry_id().unwrap()
        );
    }

    #[test]
    fn v2_import_seed_is_deterministic_and_source_scoped() {
        let seed = deterministic_v2_import_seed(&"aa".repeat(32), &"11".repeat(32));
        assert_eq!(
            seed,
            deterministic_v2_import_seed(&"aa".repeat(32), &"11".repeat(32))
        );
        assert_ne!(
            seed,
            deterministic_v2_import_seed(&"bb".repeat(32), &"11".repeat(32))
        );
        assert_ne!(
            seed,
            deterministic_v2_import_seed(&"aa".repeat(32), &"22".repeat(32))
        );
    }

    #[test]
    fn missing_values_are_explicit_nulls_in_canonical_bytes() {
        let text = String::from_utf8(
            genesis(&"44".repeat(32), Vec::new())
                .canonical_bytes()
                .unwrap(),
        )
        .unwrap();
        assert!(text.contains("\"author\":null"));
        assert!(text.contains("\"payload\":null"));
        assert!(text.contains("\"prev\":null"));
        assert!(text.contains("\"provenance\":null"));
    }

    #[test]
    fn golden_v3_entry_vector() {
        let entry = genesis(&"55".repeat(32), Vec::new());
        assert_eq!(
            String::from_utf8(entry.canonical_bytes().unwrap()).unwrap(),
            r#"{"author":null,"body":"hello","created_at":"2026-07-11T00:00:00Z","kind":"note","payload":null,"prev":null,"provenance":null,"refs":[],"schema":"mnema.entry.v3","strand":{"kind":"genesis","seed":"5555555555555555555555555555555555555555555555555555555555555555"}}"#
        );
        assert_eq!(
            entry.entry_id().unwrap(),
            "460534434a692630e97e4fbd2935b43843bed07b32ced25d62bda498b354f22b"
        );
    }

    #[test]
    fn genesis_record_projects_its_entry_id_as_strand_id() {
        let record = genesis(&"66".repeat(32), Vec::new()).into_record().unwrap();
        let JournalRecordV3::Entry(entry) = record else {
            panic!("expected entry record");
        };
        assert_eq!(entry.entry_id, entry.strand_id);
        entry.validate().unwrap();
    }

    #[test]
    fn tampered_public_strand_id_is_rejected() {
        let record = genesis(&"77".repeat(32), Vec::new()).into_record().unwrap();
        let JournalRecordV3::Entry(mut entry) = record else {
            panic!("expected entry record");
        };
        entry.strand_id = "88".repeat(32);
        assert!(entry.validate().unwrap_err().contains("strand id mismatch"));
    }

    #[test]
    fn anchor_heads_are_sorted_and_digest_is_verified() {
        let anchor = AnchorRecordV3::new(
            "2026-07-11T00:00:00Z",
            vec![
                AnchorHeadV3 {
                    strand_id: "bb".repeat(32),
                    entry_id: "22".repeat(32),
                },
                AnchorHeadV3 {
                    strand_id: "aa".repeat(32),
                    entry_id: "11".repeat(32),
                },
            ],
        )
        .unwrap();
        assert_eq!(anchor.heads[0].strand_id, "aa".repeat(32));
        anchor.validate().unwrap();

        let mut tampered = anchor;
        tampered.digest = "00".repeat(32);
        assert!(tampered.validate().unwrap_err().contains("digest mismatch"));
    }

    #[test]
    fn duplicate_anchor_strand_is_rejected() {
        let error = AnchorRecordV3::new(
            "2026-07-11T00:00:00Z",
            vec![
                AnchorHeadV3 {
                    strand_id: "aa".repeat(32),
                    entry_id: "11".repeat(32),
                },
                AnchorHeadV3 {
                    strand_id: "aa".repeat(32),
                    entry_id: "22".repeat(32),
                },
            ],
        )
        .unwrap_err();
        assert!(error.contains("duplicate strand"));
    }
}
