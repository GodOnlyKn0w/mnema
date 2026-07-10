use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const ENTRY_HASH_DOMAIN: &[u8] = b"mnema.entry.v3\0";
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
        serde_jcs::to_vec(self).map_err(|error| format!("canonicalize v3 entry: {error}"))
    }

    pub(crate) fn entry_id(&self) -> Result<String, String> {
        let bytes = self.canonical_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(ENTRY_HASH_DOMAIN);
        hasher.update(bytes);
        Ok(hex::encode(hasher.finalize()))
    }
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
}
