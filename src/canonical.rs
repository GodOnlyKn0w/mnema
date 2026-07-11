use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const ENTRY_HASH_DOMAIN: &[u8] = b"mnema.entry.v3\0";
const ANCHOR_HASH_DOMAIN: &[u8] = b"mnema.anchor.v3\0";
const V2_IMPORT_SEED_DOMAIN: &[u8] = b"mnema.import.v2\0";
const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StrandKeyV3 {
    Genesis {
        seed: String,
        slug: Option<String>,
        strand_type: Option<String>,
    },
    Existing {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum EdgeTypeV3 {
    BelongsTo,
    DependsOn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CloseDispositionV3 {
    Done,
    Failed,
    Cancelled,
    Merged,
    Verified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EffectPayloadV3 {
    Close {
        disposition: CloseDispositionV3,
    },
    Reopen {},
    Link {
        edge_type: EdgeTypeV3,
        target_strand_id: String,
    },
    Unlink {
        edge_type: EdgeTypeV3,
        target_strand_id: String,
        link_entry_id: String,
    },
    Hide {},
    Unhide {},
}

impl EffectPayloadV3 {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Link {
                target_strand_id, ..
            } => validate_full_hex("effect target_strand_id", target_strand_id),
            Self::Unlink {
                target_strand_id,
                link_entry_id,
                ..
            } => {
                validate_full_hex("effect target_strand_id", target_strand_id)?;
                validate_full_hex("effect link_entry_id", link_entry_id)
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn into_value(self) -> Value {
        serde_json::to_value(self).expect("EffectPayloadV3 is serializable")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckpointPayloadV3 {
    pub(crate) observed: String,
    pub(crate) action: String,
}

impl CheckpointPayloadV3 {
    pub(crate) fn into_value(self) -> Value {
        serde_json::to_value(self).expect("CheckpointPayloadV3 is serializable")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubjectBindingPayloadV3 {
    pub(crate) subject_type: String,
    pub(crate) subject_id: String,
}

impl SubjectBindingPayloadV3 {
    pub(crate) fn into_value(self) -> Value {
        serde_json::to_value(self).expect("SubjectBindingPayloadV3 is serializable")
    }
}

/// The exact logical value hashed for a v3 entry.
///
/// Every field is present in canonical JSON. Missing legacy values are encoded
/// as JSON null rather than omitted, so identity does not depend on serde
/// defaults or producer-specific field elision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
            StrandKeyV3::Genesis {
                seed,
                slug,
                strand_type,
            } => {
                validate_full_hex("genesis seed", seed)?;
                if self.prev.is_some() {
                    return Err("genesis entry must have prev=null".to_string());
                }
                if slug.as_ref().is_some_and(|value| value.trim().is_empty()) {
                    return Err("genesis slug cannot be empty".to_string());
                }
                if strand_type
                    .as_ref()
                    .is_some_and(|value| value.trim().is_empty())
                {
                    return Err("genesis strand_type cannot be empty".to_string());
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
        match self.kind.as_str() {
            "effect" => {
                let effect: EffectPayloadV3 = serde_json::from_value(self.payload.clone())
                    .map_err(|error| format!("invalid effect payload: {error}"))?;
                effect.validate()?;
            }
            "checkpoint" => {
                let checkpoint: CheckpointPayloadV3 = serde_json::from_value(self.payload.clone())
                    .map_err(|error| format!("invalid checkpoint payload: {error}"))?;
                if checkpoint.observed.trim().is_empty() || checkpoint.action.trim().is_empty() {
                    return Err(
                        "checkpoint payload requires non-empty observed and action".to_string()
                    );
                }
            }
            "subject_binding" => {
                let binding: SubjectBindingPayloadV3 = serde_json::from_value(self.payload.clone())
                    .map_err(|error| format!("invalid subject_binding payload: {error}"))?;
                if binding.subject_type.trim().is_empty() || binding.subject_id.trim().is_empty() {
                    return Err(
                        "subject_binding payload requires non-empty subject_type and subject_id"
                            .to_string(),
                    );
                }
            }
            _ => {}
        }
        validate_canonical_timestamp("created_at", &self.created_at)?;
        validate_ijson_value("payload", &self.payload)?;
        validate_ijson_value("provenance", &self.provenance)?;
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub(crate) struct AnchorHeadV3 {
    pub(crate) strand_id: String,
    pub(crate) entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnchorRecordV3 {
    pub(crate) created_at: String,
    pub(crate) covered_record_count: u64,
    pub(crate) previous_anchor: Option<String>,
    pub(crate) covered_records_digest: String,
    pub(crate) heads: Vec<AnchorHeadV3>,
    pub(crate) digest: String,
}

impl AnchorRecordV3 {
    pub(crate) fn new(
        created_at: impl Into<String>,
        covered_record_count: u64,
        previous_anchor: Option<String>,
        covered_records_digest: String,
        mut heads: Vec<AnchorHeadV3>,
    ) -> Result<Self, String> {
        let created_at = created_at.into();
        validate_canonical_timestamp("anchor created_at", &created_at)?;
        validate_safe_count(covered_record_count)?;
        if let Some(previous) = &previous_anchor {
            validate_full_hex("anchor previous_anchor", previous)?;
        }
        validate_full_hex("anchor covered_records_digest", &covered_records_digest)?;
        heads.sort_by(|left, right| {
            left.strand_id
                .cmp(&right.strand_id)
                .then_with(|| left.entry_id.cmp(&right.entry_id))
        });
        validate_anchor_heads(&heads)?;
        let digest = anchor_digest(
            &created_at,
            covered_record_count,
            previous_anchor.as_deref(),
            &covered_records_digest,
            &heads,
        )?;
        Ok(Self {
            created_at,
            covered_record_count,
            previous_anchor,
            covered_records_digest,
            heads,
            digest,
        })
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_canonical_timestamp("anchor created_at", &self.created_at)?;
        validate_safe_count(self.covered_record_count)?;
        if let Some(previous) = &self.previous_anchor {
            validate_full_hex("anchor previous_anchor", previous)?;
        }
        validate_full_hex(
            "anchor covered_records_digest",
            &self.covered_records_digest,
        )?;
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
        let computed = anchor_digest(
            &self.created_at,
            self.covered_record_count,
            self.previous_anchor.as_deref(),
            &self.covered_records_digest,
            &self.heads,
        )?;
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
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
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

fn anchor_digest(
    created_at: &str,
    covered_record_count: u64,
    previous_anchor: Option<&str>,
    covered_records_digest: &str,
    heads: &[AnchorHeadV3],
) -> Result<String, String> {
    let value = serde_json::json!({
        "created_at": created_at,
        "covered_record_count": covered_record_count,
        "previous_anchor": previous_anchor,
        "covered_records_digest": covered_records_digest,
        "heads": heads,
    });
    let canonical = serde_jcs::to_vec(&value)
        .map_err(|error| format!("canonicalize anchor commitment: {error}"))?;
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

pub(crate) fn validate_full_hex(field: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{field} must be 64 lowercase hex characters"));
    }
    Ok(())
}

pub(crate) fn canonicalize_timestamp(field: &str, value: &str) -> Result<String, String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|error| format!("{field} must be RFC3339: {error}"))?;
    let mut canonical = parsed
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    if let Some(dot) = canonical.rfind('.') {
        let z = canonical.len() - 1;
        let trimmed = canonical[dot + 1..z].trim_end_matches('0').len();
        if trimmed == 0 {
            canonical.replace_range(dot..z, "");
        } else {
            canonical.replace_range(dot + 1 + trimmed..z, "");
        }
    }
    Ok(canonical)
}

fn validate_canonical_timestamp(field: &str, value: &str) -> Result<(), String> {
    let canonical = canonicalize_timestamp(field, value)?;
    if value != canonical {
        return Err(format!(
            "{field} must use canonical UTC form {canonical}, found {value}"
        ));
    }
    Ok(())
}

fn validate_safe_count(value: u64) -> Result<(), String> {
    if value > MAX_SAFE_JSON_INTEGER {
        return Err(format!(
            "anchor covered_record_count exceeds I-JSON safe integer {MAX_SAFE_JSON_INTEGER}"
        ));
    }
    Ok(())
}

fn validate_ijson_value(field: &str, value: &Value) -> Result<(), String> {
    match value {
        Value::Number(number) => {
            let safe = if let Some(value) = number.as_i64() {
                value.unsigned_abs() <= MAX_SAFE_JSON_INTEGER
            } else if let Some(value) = number.as_u64() {
                value <= MAX_SAFE_JSON_INTEGER
            } else {
                number.as_f64().is_some_and(f64::is_finite)
            };
            if !safe {
                return Err(format!(
                    "{field} contains a number outside the I-JSON/IEEE-754 safe domain"
                ));
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                validate_ijson_value(&format!("{field}[{index}]"), value)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                validate_ijson_value(&format!("{field}.{key}"), value)?;
            }
        }
        _ => {}
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

pub(crate) fn random_genesis_seed() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| format!("generate genesis seed: {error}"))?;
    Ok(hex::encode(bytes))
}

pub(crate) fn author_from_provenance(provenance: &Value) -> Option<String> {
    provenance
        .as_object()
        .and_then(|object| object.get("producer"))
        .and_then(Value::as_str)
        .filter(|producer| !producer.trim().is_empty())
        .map(str::to_string)
}

pub(crate) fn kind_from_body(body: &str) -> String {
    match crate::markers::leading_marker(body) {
        // These kinds have mandatory typed payloads. A bracket marker in
        // human-authored prose is an annotation, not permission to forge a
        // structural envelope with payload=null.
        Some("effect" | "checkpoint" | "subject_binding") | None => "note".to_string(),
        Some(marker) => marker.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn genesis(seed: &str, refs: Vec<RefV3>) -> EntryHashViewV3 {
        EntryHashViewV3::new(
            StrandKeyV3::Genesis {
                seed: seed.to_string(),
                slug: None,
                strand_type: None,
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
                slug: None,
                strand_type: None,
            },
            None,
            "note",
            "object ordering",
            Vec::new(),
            Some("agent".to_string()),
            "2026-07-11T00:00:00Z",
            json!({"z": 1, "a": 2}),
            json!({"worker": {"z": true, "a": false}}),
        );
        let second = EntryHashViewV3::new(
            StrandKeyV3::Genesis {
                seed: "11".repeat(32),
                slug: None,
                strand_type: None,
            },
            None,
            "note",
            "object ordering",
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
    fn random_genesis_seed_has_the_canonical_shape() {
        let seed = random_genesis_seed().unwrap();
        assert_eq!(seed.len(), 64);
        assert!(seed.bytes().all(|byte| byte.is_ascii_hexdigit()));
        genesis(&seed, Vec::new()).validate().unwrap();
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
            r#"{"author":null,"body":"hello","created_at":"2026-07-11T00:00:00Z","kind":"note","payload":null,"prev":null,"provenance":null,"refs":[],"schema":"mnema.entry.v3","strand":{"kind":"genesis","seed":"5555555555555555555555555555555555555555555555555555555555555555","slug":null,"strand_type":null}}"#
        );
        assert_eq!(
            entry.entry_id().unwrap(),
            "25520922cec2d45c8619b9ef4f0166c37834fed9eac6152079bcbe8d3263daaf"
        );
    }

    #[test]
    fn genesis_metadata_is_typed_and_participates_in_identity() {
        let plain = genesis(&"56".repeat(32), Vec::new());
        let mut named = plain.clone();
        named.strand = StrandKeyV3::Genesis {
            seed: "56".repeat(32),
            slug: Some("migration-worker".to_string()),
            strand_type: Some("task".to_string()),
        };
        assert_ne!(plain.entry_id().unwrap(), named.entry_id().unwrap());

        let mut invalid = named;
        invalid.strand = StrandKeyV3::Genesis {
            seed: "56".repeat(32),
            slug: Some(" ".to_string()),
            strand_type: Some("task".to_string()),
        };
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("slug cannot be empty")
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
            2,
            None,
            "99".repeat(32),
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
            2,
            None,
            "99".repeat(32),
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

    #[test]
    fn structural_kinds_require_typed_payloads() {
        let effect = EntryHashViewV3::new(
            StrandKeyV3::Genesis {
                seed: "99".repeat(32),
                slug: None,
                strand_type: None,
            },
            None,
            "effect",
            "link belongs-to",
            Vec::new(),
            None,
            "2026-07-11T00:00:00Z",
            EffectPayloadV3::Link {
                edge_type: EdgeTypeV3::BelongsTo,
                target_strand_id: "aa".repeat(32),
            }
            .into_value(),
            Value::Null,
        );
        effect.validate().unwrap();

        let mut invalid = effect;
        invalid.payload = serde_json::json!({"type": "link", "edge_type": "belongs-to"});
        assert!(invalid.validate().unwrap_err().contains("effect payload"));
    }

    #[test]
    fn checkpoint_and_binding_payloads_round_trip() {
        let checkpoint = CheckpointPayloadV3 {
            observed: "tests fail".to_string(),
            action: "inspect logs".to_string(),
        };
        assert_eq!(
            serde_json::from_value::<CheckpointPayloadV3>(checkpoint.clone().into_value()).unwrap(),
            checkpoint
        );
        let binding = SubjectBindingPayloadV3 {
            subject_type: "issue".to_string(),
            subject_id: "MNEMA-42".to_string(),
        };
        assert_eq!(
            serde_json::from_value::<SubjectBindingPayloadV3>(binding.clone().into_value())
                .unwrap(),
            binding
        );
    }

    #[test]
    fn unsafe_json_integers_are_rejected_before_hashing() {
        let mut unsafe_entry = genesis(&"aa".repeat(32), Vec::new());
        unsafe_entry.payload = json!({"value": 9_007_199_254_740_993_u64});
        assert!(
            unsafe_entry
                .entry_id()
                .unwrap_err()
                .contains("I-JSON/IEEE-754 safe domain")
        );

        let mut safe_entry = genesis(&"aa".repeat(32), Vec::new());
        safe_entry.payload = json!({"value": 9_007_199_254_740_991_u64});
        safe_entry.entry_id().unwrap();
    }

    #[test]
    fn timestamps_and_hex_have_one_canonical_spelling() {
        let mut offset_time = genesis(&"bb".repeat(32), Vec::new());
        offset_time.created_at = "2026-07-11T08:00:00+08:00".to_string();
        assert!(
            offset_time
                .validate()
                .unwrap_err()
                .contains("canonical UTC")
        );
        assert_eq!(
            canonicalize_timestamp("created_at", &offset_time.created_at).unwrap(),
            "2026-07-11T00:00:00Z"
        );
        assert_eq!(
            canonicalize_timestamp("created_at", "2026-07-11T00:00:00.100000+00:00").unwrap(),
            "2026-07-11T00:00:00.1Z"
        );

        let uppercase = genesis(&"CC".repeat(32), Vec::new());
        assert!(uppercase.validate().unwrap_err().contains("lowercase hex"));
    }

    #[test]
    fn unknown_fields_are_rejected_at_nested_schema_boundaries() {
        let value = serde_json::to_value(genesis(&"dd".repeat(32), Vec::new())).unwrap();
        let mut object = value.as_object().unwrap().clone();
        object.insert("future_field".to_string(), json!(true));
        let error = serde_json::from_value::<EntryHashViewV3>(Value::Object(object)).unwrap_err();
        assert!(error.to_string().contains("unknown field"));

        let effect = json!({
            "type": "hide",
            "future_field": true
        });
        assert!(serde_json::from_value::<EffectPayloadV3>(effect).is_err());
    }

    #[test]
    fn anchor_timestamp_is_part_of_its_digest() {
        let mut anchor =
            AnchorRecordV3::new("2026-07-11T00:00:00Z", 0, None, "99".repeat(32), Vec::new())
                .unwrap();
        anchor.created_at = "2026-07-11T00:00:01Z".to_string();
        assert!(anchor.validate().unwrap_err().contains("digest mismatch"));
    }

    #[test]
    fn author_is_mechanically_projected_from_provenance_producer() {
        assert_eq!(
            author_from_provenance(&json!({"producer": "grok-4.5", "attempt": 2})),
            Some("grok-4.5".to_string())
        );
        assert_eq!(author_from_provenance(&json!({"producer": 7})), None);
        assert_eq!(author_from_provenance(&json!({"producer": "  "})), None);
        assert_eq!(author_from_provenance(&Value::Null), None);
    }

    #[test]
    fn ordinary_kind_is_mechanically_projected_from_body() {
        assert_eq!(kind_from_body("[decision] choose A"), "decision");
        assert_eq!(kind_from_body("  [friction] blocked"), "friction");
        assert_eq!(kind_from_body("plain note"), "note");
        assert_eq!(kind_from_body("[checkpoint] legacy annotation"), "note");
        assert_eq!(kind_from_body("[effect] prose only"), "note");
    }
}
