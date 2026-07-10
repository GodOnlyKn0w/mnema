use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub(crate) const ACTIVE_MANIFEST_SCHEMA: &str = "mnema.active-journal.v1";
pub(crate) const ACTIVE_MANIFEST_FILE: &str = "active-journal.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct JournalArtifactV3 {
    pub(crate) schema: String,
    pub(crate) path: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HistoricalJournalV3 {
    pub(crate) schema: String,
    pub(crate) path: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MigrationActivationV3 {
    pub(crate) id: String,
    pub(crate) map_path: String,
    pub(crate) certificate_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ActiveJournalManifestV3 {
    pub(crate) schema: String,
    pub(crate) journal_id: String,
    pub(crate) active: JournalArtifactV3,
    pub(crate) history: Vec<HistoricalJournalV3>,
    pub(crate) migration: MigrationActivationV3,
}

impl ActiveJournalManifestV3 {
    pub(crate) fn canonical_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_jcs::to_vec(self).map_err(|error| format!("canonicalize active manifest: {error}"))
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema != ACTIVE_MANIFEST_SCHEMA {
            return Err(format!(
                "active schema unsupported: expected {}, found {}",
                ACTIVE_MANIFEST_SCHEMA, self.schema
            ));
        }
        validate_full_hex("journal_id", &self.journal_id)?;
        if self.active.schema != "v3" {
            return Err(format!(
                "active schema unsupported: expected v3, found {}",
                self.active.schema
            ));
        }
        validate_relative_path("active.path", &self.active.path)?;
        validate_full_hex("active.sha256", &self.active.sha256)?;
        validate_full_hex("migration.id", &self.migration.id)?;
        validate_relative_path("migration.map_path", &self.migration.map_path)?;
        validate_relative_path(
            "migration.certificate_path",
            &self.migration.certificate_path,
        )?;
        for (index, history) in self.history.iter().enumerate() {
            if history.schema != "v2" {
                return Err(format!(
                    "history[{index}].schema must be v2, found {}",
                    history.schema
                ));
            }
            validate_relative_path(&format!("history[{index}].path"), &history.path)?;
            validate_full_hex(&format!("history[{index}].sha256"), &history.sha256)?;
        }
        Ok(())
    }

    pub(crate) fn active_path(&self, journal_dir: &Path) -> Result<PathBuf, String> {
        self.validate()?;
        Ok(journal_dir.join(&self.active.path))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivationOutcome {
    Activated,
    AlreadyActive,
}

pub(crate) fn load_active_manifest(
    journal_dir: &Path,
) -> Result<Option<ActiveJournalManifestV3>, String> {
    let path = journal_dir.join(ACTIVE_MANIFEST_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("read active manifest {}: {error}", path.display()))?;
    let manifest: ActiveJournalManifestV3 = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse active manifest {}: {error}", path.display()))?;
    manifest.validate()?;
    Ok(Some(manifest))
}

/// Install the first v3 activation manifest.
///
/// All target, history, map and certificate artifacts must already exist and
/// be verified. The manifest rename is the only commit point: before it the
/// legacy v2 resolver remains authoritative; after it v3 is authoritative.
pub(crate) fn activate_initial_v3(
    journal_dir: &Path,
    manifest: &ActiveJournalManifestV3,
) -> Result<ActivationOutcome, String> {
    let bytes = manifest.canonical_bytes()?;
    let final_path = journal_dir.join(ACTIVE_MANIFEST_FILE);
    if final_path.exists() {
        let existing = std::fs::read(&final_path)
            .map_err(|error| format!("read active manifest {}: {error}", final_path.display()))?;
        let parsed: ActiveJournalManifestV3 = serde_json::from_slice(&existing)
            .map_err(|error| format!("parse active manifest {}: {error}", final_path.display()))?;
        parsed.validate()?;
        if parsed == *manifest {
            return Ok(ActivationOutcome::AlreadyActive);
        }
        return Err("migration artifact conflict: active manifest already differs".to_string());
    }

    let tmp_path = journal_dir.join(format!(".active-journal.{}.tmp", manifest.migration.id));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp_path)
        .map_err(|error| format!("create prepared manifest {}: {error}", tmp_path.display()))?;
    file.write_all(&bytes)
        .map_err(|error| format!("write prepared manifest {}: {error}", tmp_path.display()))?;
    file.sync_all()
        .map_err(|error| format!("sync prepared manifest {}: {error}", tmp_path.display()))?;
    drop(file);

    if let Err(error) = atomic_install(&tmp_path, &final_path, journal_dir) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error);
    }
    Ok(ActivationOutcome::Activated)
}

fn validate_relative_path(field: &str, value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("{field} must be a normalized relative path"));
    }
    Ok(())
}

fn validate_full_hex(field: &str, value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{field} must be 64 hex characters"));
    }
    Ok(())
}

#[cfg(unix)]
fn atomic_install(tmp_path: &Path, final_path: &Path, journal_dir: &Path) -> Result<(), String> {
    std::fs::rename(tmp_path, final_path).map_err(|error| {
        format!(
            "atomic activation failed renaming {} to {}: {error}",
            tmp_path.display(),
            final_path.display()
        )
    })?;
    File::open(journal_dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "sync journal directory {} after activation: {error}",
                journal_dir.display()
            )
        })
}

#[cfg(windows)]
fn atomic_install(tmp_path: &Path, final_path: &Path, _journal_dir: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let tmp: Vec<u16> = tmp_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let final_name: Vec<u16> = final_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let moved = unsafe { MoveFileExW(tmp.as_ptr(), final_name.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if moved == 0 {
        return Err(format!(
            "atomic activation failed moving {} to {} with write-through: {}",
            tmp_path.display(),
            final_path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ActiveJournalManifestV3 {
        ActiveJournalManifestV3 {
            schema: ACTIVE_MANIFEST_SCHEMA.to_string(),
            journal_id: "11".repeat(32),
            active: JournalArtifactV3 {
                schema: "v3".to_string(),
                path: "journals/journal.v3.jsonl".to_string(),
                sha256: "22".repeat(32),
            },
            history: vec![HistoricalJournalV3 {
                schema: "v2".to_string(),
                path: "history/journal.v2.jsonl".to_string(),
                sha256: "33".repeat(32),
            }],
            migration: MigrationActivationV3 {
                id: "44".repeat(32),
                map_path: "history/migration-v2-to-v3.json".to_string(),
                certificate_path: "history/migration-v2-to-v3.certificate.json".to_string(),
            },
        }
    }

    #[test]
    fn manifest_paths_cannot_escape_journal_directory() {
        for invalid in ["../outside", "/absolute", "journals/../outside", ""] {
            let mut value = manifest();
            value.active.path = invalid.to_string();
            assert!(value.validate().is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn activation_is_atomic_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let value = manifest();
        assert_eq!(
            activate_initial_v3(dir.path(), &value).unwrap(),
            ActivationOutcome::Activated
        );
        assert_eq!(
            activate_initial_v3(dir.path(), &value).unwrap(),
            ActivationOutcome::AlreadyActive
        );
        assert_eq!(load_active_manifest(dir.path()).unwrap(), Some(value));
    }

    #[test]
    fn conflicting_manifest_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let first = manifest();
        activate_initial_v3(dir.path(), &first).unwrap();
        let mut conflicting = first.clone();
        conflicting.active.sha256 = "55".repeat(32);
        let error = activate_initial_v3(dir.path(), &conflicting).unwrap_err();
        assert!(error.contains("artifact conflict"));
        assert_eq!(load_active_manifest(dir.path()).unwrap(), Some(first));
    }

    #[test]
    fn canonical_manifest_has_a_fixed_golden_vector() {
        let value = manifest();
        let text = String::from_utf8(value.canonical_bytes().unwrap()).unwrap();
        assert_eq!(
            text,
            format!(
                "{{\"active\":{{\"path\":\"journals/journal.v3.jsonl\",\"schema\":\"v3\",\"sha256\":\"{}\"}},\"history\":[{{\"path\":\"history/journal.v2.jsonl\",\"schema\":\"v2\",\"sha256\":\"{}\"}}],\"journal_id\":\"{}\",\"migration\":{{\"certificate_path\":\"history/migration-v2-to-v3.certificate.json\",\"id\":\"{}\",\"map_path\":\"history/migration-v2-to-v3.json\"}},\"schema\":\"{}\"}}",
                "22".repeat(32),
                "33".repeat(32),
                "11".repeat(32),
                "44".repeat(32),
                ACTIVE_MANIFEST_SCHEMA
            )
        );
    }
}
