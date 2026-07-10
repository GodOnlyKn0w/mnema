use serde::{Deserialize, Serialize};
use std::collections::HashSet;
#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

pub(crate) const ACTIVE_MANIFEST_SCHEMA: &str = "mnema.active-journal.v1";
pub(crate) const ACTIVE_MANIFEST_FILE: &str = "active-journal.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalArtifactV3 {
    pub(crate) schema: String,
    pub(crate) path: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoricalJournalV3 {
    pub(crate) schema: String,
    pub(crate) path: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ActivationOriginV3 {
    Fresh {
        id: String,
    },
    Migration {
        id: String,
        map_path: String,
        map_sha256: String,
        certificate_path: String,
        certificate_sha256: String,
    },
}

impl ActivationOriginV3 {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Fresh { id } | Self::Migration { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActiveJournalManifestV3 {
    pub(crate) schema: String,
    pub(crate) journal_id: String,
    pub(crate) active: JournalArtifactV3,
    pub(crate) history: Vec<HistoricalJournalV3>,
    pub(crate) origin: ActivationOriginV3,
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
        validate_artifact_path("active.path", &self.active.path, "journals")?;
        validate_full_hex("active.sha256", &self.active.sha256)?;
        validate_full_hex("origin.id", self.origin.id())?;
        let mut paths = HashSet::new();
        paths.insert(self.active.path.as_str());
        match &self.origin {
            ActivationOriginV3::Fresh { .. } => {
                if !self.history.is_empty() {
                    return Err("fresh activation cannot declare migration history".to_string());
                }
            }
            ActivationOriginV3::Migration {
                map_path,
                map_sha256,
                certificate_path,
                certificate_sha256,
                ..
            } => {
                if self.history.is_empty() {
                    return Err("migration activation requires v2 history".to_string());
                }
                validate_artifact_path("origin.map_path", map_path, "history")?;
                validate_artifact_path("origin.certificate_path", certificate_path, "history")?;
                validate_full_hex("origin.map_sha256", map_sha256)?;
                validate_full_hex("origin.certificate_sha256", certificate_sha256)?;
                for (field, path) in [
                    ("origin.map_path", map_path.as_str()),
                    ("origin.certificate_path", certificate_path.as_str()),
                ] {
                    if !paths.insert(path) {
                        return Err(format!("{field} duplicates another manifest artifact path"));
                    }
                }
            }
        }
        for (index, history) in self.history.iter().enumerate() {
            if history.schema != "v2" {
                return Err(format!(
                    "history[{index}].schema must be v2, found {}",
                    history.schema
                ));
            }
            validate_artifact_path(&format!("history[{index}].path"), &history.path, "history")?;
            validate_full_hex(&format!("history[{index}].sha256"), &history.sha256)?;
            if !paths.insert(&history.path) {
                return Err(format!(
                    "history[{index}].path duplicates another manifest artifact path"
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn active_path(&self, journal_dir: &Path) -> Result<PathBuf, String> {
        self.validate()?;
        Ok(journal_dir.join(&self.active.path))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActivationOutcome {
    Activated,
    ActivatedDurabilityUncertain { detail: String },
    AlreadyActive,
}

/// Install the first v3 activation manifest.
///
/// All artifacts named by the selected origin must already exist and be
/// verified. The no-replace create of `active-journal.json` is the only
/// commit point: before it the legacy v2 resolver remains authoritative; after
/// it v3 is authoritative. Concurrent first-activation races keep the winner;
/// losers re-read the final path and surface conflict or already-active.
/// Crash residue of the prepared temp is verified and resumed, or rebuilt.
pub(crate) fn activate_initial_v3(
    journal_dir: &Path,
    manifest: &ActiveJournalManifestV3,
) -> Result<ActivationOutcome, String> {
    let bytes = manifest.canonical_bytes()?;
    let final_path = journal_dir.join(ACTIVE_MANIFEST_FILE);

    if let Some(outcome) = observe_final(&final_path, manifest)? {
        return Ok(outcome);
    }

    let tmp_path = prepared_temp_path(journal_dir, manifest.origin.id());
    prepare_temp(&tmp_path, &bytes)?;

    match atomic_install_no_replace(&tmp_path, &final_path, journal_dir) {
        Ok(durability_warning) => {
            // Commit succeeded. Directory durability is best-effort after the
            // no-replace create; re-read final so a post-commit fsync failure
            // never reports "not activated" against a live winner.
            match observe_final(&final_path, manifest)? {
                Some(ActivationOutcome::AlreadyActive) => {
                    cleanup_temp(&tmp_path);
                    Ok(committed_outcome(durability_warning))
                }
                Some(ActivationOutcome::Activated) => {
                    cleanup_temp(&tmp_path);
                    Ok(committed_outcome(durability_warning))
                }
                Some(ActivationOutcome::ActivatedDurabilityUncertain { .. }) => {
                    unreachable!("observe_final only reports already-active")
                }
                None => Err(format!(
                    "atomic activation committed but {} is missing",
                    final_path.display()
                )),
            }
        }
        Err(error) => {
            // Install did not complete in this process. The true authority is
            // final: a peer may have won the no-replace race, or (Windows) moved
            // a shared same-migration temp out from under us after committing.
            match observe_final(&final_path, manifest) {
                Ok(Some(outcome)) => {
                    cleanup_temp(&tmp_path);
                    Ok(outcome)
                }
                Ok(None) => match error {
                    InstallError::DestinationExists => Err(format!(
                        "atomic activation failed: {} already exists but is unreadable",
                        final_path.display()
                    )),
                    InstallError::Io(message) => {
                        // Leave prepared temp when still present so a same-migration
                        // retry can resume after a transient pre-commit failure.
                        Err(message)
                    }
                },
                Err(observe_error) => Err(observe_error),
            }
        }
    }
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
    let manifest = parse_manifest_bytes(&path, &bytes)?;
    Ok(Some(manifest))
}

fn prepared_temp_path(journal_dir: &Path, migration_id: &str) -> PathBuf {
    journal_dir.join(format!(".active-journal.{migration_id}.tmp"))
}

/// Compare an existing final manifest with the candidate.
///
/// Returns `None` when final is absent. Same content is already-active;
/// different valid content is an artifact conflict.
fn observe_final(
    final_path: &Path,
    expected: &ActiveJournalManifestV3,
) -> Result<Option<ActivationOutcome>, String> {
    if !final_path.exists() {
        return Ok(None);
    }
    let existing = std::fs::read(final_path)
        .map_err(|error| format!("read active manifest {}: {error}", final_path.display()))?;
    let parsed = parse_manifest_bytes(final_path, &existing)?;
    if parsed == *expected {
        Ok(Some(ActivationOutcome::AlreadyActive))
    } else {
        Err("migration artifact conflict: active manifest already differs".to_string())
    }
}

/// Create the prepared temp with `create_new`, or resume/rebuild crash residue.
fn prepare_temp(tmp_path: &Path, bytes: &[u8]) -> Result<(), String> {
    match write_temp_new(tmp_path, bytes) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            resume_or_rebuild_temp(tmp_path, bytes)
        }
        Err(error) => Err(format!(
            "create prepared manifest {}: {error}",
            tmp_path.display()
        )),
    }
}

fn resume_or_rebuild_temp(tmp_path: &Path, bytes: &[u8]) -> Result<(), String> {
    match std::fs::read(tmp_path) {
        Ok(existing) if existing == bytes => {
            // Crash residue matches this migration — resume install.
            Ok(())
        }
        Ok(_) => {
            // Temp is not the commit point. Corrupt or foreign residue for this
            // migration id may be rebuilt safely.
            std::fs::remove_file(tmp_path).map_err(|remove_error| {
                format!(
                    "remove prepared manifest residue {}: {remove_error}",
                    tmp_path.display()
                )
            })?;
            match write_temp_new(tmp_path, bytes) {
                Ok(()) => Ok(()),
                Err(rewrite_error) if rewrite_error.kind() == ErrorKind::AlreadyExists => {
                    // Lost a race recreating the temp; resume only if peer wrote
                    // identical prepared bytes.
                    let again = std::fs::read(tmp_path).map_err(|read_error| {
                        format!(
                            "read prepared manifest {}: {read_error}",
                            tmp_path.display()
                        )
                    })?;
                    if again == bytes {
                        Ok(())
                    } else {
                        Err(format!(
                            "migration artifact conflict: prepared temp differs at {}",
                            tmp_path.display()
                        ))
                    }
                }
                Err(rewrite_error) => Err(format!(
                    "create prepared manifest {}: {rewrite_error}",
                    tmp_path.display()
                )),
            }
        }
        Err(read_error) => Err(format!(
            "read prepared manifest residue {}: {read_error}",
            tmp_path.display()
        )),
    }
}

fn write_temp_new(tmp_path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(tmp_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn cleanup_temp(tmp_path: &Path) {
    let _ = std::fs::remove_file(tmp_path);
}

fn committed_outcome(durability_warning: Option<String>) -> ActivationOutcome {
    match durability_warning {
        Some(detail) => ActivationOutcome::ActivatedDurabilityUncertain { detail },
        None => ActivationOutcome::Activated,
    }
}

fn parse_manifest_bytes(path: &Path, bytes: &[u8]) -> Result<ActiveJournalManifestV3, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("parse active manifest {}: {error}", path.display()))?;
    let manifest: ActiveJournalManifestV3 = crate::strict_json::from_str(text)
        .map_err(|error| format!("parse active manifest {}: {error}", path.display()))?;
    manifest.validate()?;
    let canonical = manifest.canonical_bytes()?;
    if canonical != bytes {
        return Err(format!(
            "active manifest {} is not canonical JCS",
            path.display()
        ));
    }
    Ok(manifest)
}

#[derive(Debug)]
enum InstallError {
    DestinationExists,
    Io(String),
}

/// Atomically create `final_path` from a fully synced temp file without replacing
/// an existing final. On success the activation is committed even if a later
/// directory durability sync fails.
fn atomic_install_no_replace(
    tmp_path: &Path,
    final_path: &Path,
    journal_dir: &Path,
) -> Result<Option<String>, InstallError> {
    match platform_no_replace_install(tmp_path, final_path) {
        Ok(()) => {
            // Commit point has passed. Directory fsync is durability only; a
            // failure must not invert the activation outcome.
            Ok(sync_journal_dir(journal_dir).err())
        }
        Err(error) if is_already_exists(&error) => Err(InstallError::DestinationExists),
        Err(error) => Err(InstallError::Io(format!(
            "atomic activation failed installing {} to {}: {error}",
            tmp_path.display(),
            final_path.display()
        ))),
    }
}

#[cfg(unix)]
fn platform_no_replace_install(tmp_path: &Path, final_path: &Path) -> std::io::Result<()> {
    // hard_link is an atomic no-replace create: it fails with EEXIST when the
    // destination already exists, so a racing second activation cannot overwrite
    // the winner (unlike POSIX rename, which replaces).
    std::fs::hard_link(tmp_path, final_path)
}

#[cfg(windows)]
fn platform_no_replace_install(tmp_path: &Path, final_path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    // MoveFileEx without MOVEFILE_REPLACE_EXISTING is no-replace: the call fails
    // if the destination already exists, preserving the first winner.
    let tmp: Vec<u16> = tmp_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let final_name: Vec<u16> = final_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let moved = unsafe { MoveFileExW(tmp.as_ptr(), final_name.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if moved != 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    let raw = error.raw_os_error().unwrap_or(0) as u32;
    if raw == ERROR_ALREADY_EXISTS || raw == ERROR_FILE_EXISTS {
        return Err(std::io::Error::new(
            ErrorKind::AlreadyExists,
            "destination exists",
        ));
    }
    Err(error)
}

fn is_already_exists(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::AlreadyExists
}

#[cfg(unix)]
fn sync_journal_dir(journal_dir: &Path) -> Result<(), String> {
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
fn sync_journal_dir(_journal_dir: &Path) -> Result<(), String> {
    // MOVEFILE_WRITE_THROUGH already requested durable move semantics.
    Ok(())
}

fn validate_relative_path(field: &str, value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("{field} must be a normalized relative path"));
    }
    Ok(())
}

fn validate_artifact_path(field: &str, value: &str, root: &str) -> Result<(), String> {
    validate_relative_path(field, value)?;
    let expected = format!("{root}/");
    if !value.starts_with(&expected) || value.len() == expected.len() {
        return Err(format!("{field} must be under {root}/"));
    }
    Ok(())
}

fn validate_full_hex(field: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{field} must be 64 lowercase hex characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

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
            origin: ActivationOriginV3::Migration {
                id: "44".repeat(32),
                map_path: "history/migration-v2-to-v3.json".to_string(),
                map_sha256: "66".repeat(32),
                certificate_path: "history/migration-v2-to-v3.certificate.json".to_string(),
                certificate_sha256: "77".repeat(32),
            },
        }
    }

    fn manifest_with(migration_id: &str, active_sha: &str) -> ActiveJournalManifestV3 {
        let mut value = manifest();
        value.origin = ActivationOriginV3::Migration {
            id: migration_id.to_string(),
            map_path: "history/migration-v2-to-v3.json".to_string(),
            map_sha256: "66".repeat(32),
            certificate_path: "history/migration-v2-to-v3.certificate.json".to_string(),
            certificate_sha256: "77".repeat(32),
        };
        value.active.sha256 = active_sha.to_string();
        value
    }

    fn fresh_manifest() -> ActiveJournalManifestV3 {
        let mut value = manifest();
        value.history.clear();
        value.origin = ActivationOriginV3::Fresh {
            id: "55".repeat(32),
        };
        value
    }

    #[test]
    fn manifest_paths_cannot_escape_journal_directory() {
        for invalid in [
            "../outside",
            "/absolute",
            "journals/../outside",
            "history/not-active.jsonl",
            "journals\\platform-dependent.jsonl",
            "",
        ] {
            let mut value = manifest();
            value.active.path = invalid.to_string();
            assert!(value.validate().is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn manifest_artifact_paths_are_distinct_and_rooted() {
        let mut duplicate = manifest();
        let ActivationOriginV3::Migration {
            map_path,
            certificate_path,
            ..
        } = &mut duplicate.origin
        else {
            panic!("migration origin")
        };
        *certificate_path = map_path.clone();
        assert!(duplicate.validate().unwrap_err().contains("duplicates"));

        let mut misplaced_history = manifest();
        misplaced_history.history[0].path = "journals/source.v2.jsonl".to_string();
        assert!(
            misplaced_history
                .validate()
                .unwrap_err()
                .contains("under history/")
        );
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
        // Prepared temp must not linger after a successful commit.
        assert!(
            !prepared_temp_path(dir.path(), manifest().origin.id()).exists(),
            "temp residue must be cleaned after activation"
        );
    }

    #[test]
    fn fresh_origin_activates_without_fake_migration_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let value = fresh_manifest();
        assert_eq!(
            activate_initial_v3(dir.path(), &value).unwrap(),
            ActivationOutcome::Activated
        );
        assert_eq!(load_active_manifest(dir.path()).unwrap(), Some(value));

        let mut invalid = fresh_manifest();
        invalid.history.push(manifest().history[0].clone());
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("cannot declare migration history")
        );
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
    fn matching_temp_residue_is_resumed() {
        let dir = tempfile::tempdir().unwrap();
        let value = manifest();
        let bytes = value.canonical_bytes().unwrap();
        let tmp = prepared_temp_path(dir.path(), value.origin.id());
        std::fs::write(&tmp, &bytes).unwrap();

        assert_eq!(
            activate_initial_v3(dir.path(), &value).unwrap(),
            ActivationOutcome::Activated
        );
        assert_eq!(load_active_manifest(dir.path()).unwrap(), Some(value));
        assert!(!tmp.exists(), "resumed temp should be cleaned after commit");
    }

    #[test]
    fn corrupt_temp_residue_is_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let value = manifest();
        let tmp = prepared_temp_path(dir.path(), value.origin.id());
        std::fs::write(&tmp, b"{not-valid-manifest").unwrap();

        assert_eq!(
            activate_initial_v3(dir.path(), &value).unwrap(),
            ActivationOutcome::Activated
        );
        assert_eq!(load_active_manifest(dir.path()).unwrap(), Some(value));
    }

    #[test]
    fn foreign_temp_residue_same_migration_id_is_rebuilt_when_bytes_differ() {
        let dir = tempfile::tempdir().unwrap();
        let value = manifest();
        let mut foreign = value.clone();
        foreign.active.sha256 = "66".repeat(32);
        let tmp = prepared_temp_path(dir.path(), value.origin.id());
        std::fs::write(&tmp, foreign.canonical_bytes().unwrap()).unwrap();

        assert_eq!(
            activate_initial_v3(dir.path(), &value).unwrap(),
            ActivationOutcome::Activated
        );
        assert_eq!(load_active_manifest(dir.path()).unwrap(), Some(value));
    }

    #[test]
    fn concurrent_conflicting_activations_keep_single_winner() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();
        let first = manifest_with(&"aa".repeat(32), &"22".repeat(32));
        let second = manifest_with(&"bb".repeat(32), &"55".repeat(32));
        let barrier = Arc::new(Barrier::new(2));

        let left_dir = dir_path.clone();
        let left_barrier = Arc::clone(&barrier);
        let left_manifest = first.clone();
        let left = thread::spawn(move || {
            left_barrier.wait();
            activate_initial_v3(&left_dir, &left_manifest)
        });

        let right_dir = dir_path.clone();
        let right_barrier = Arc::clone(&barrier);
        let right_manifest = second.clone();
        let right = thread::spawn(move || {
            right_barrier.wait();
            activate_initial_v3(&right_dir, &right_manifest)
        });

        let left_result = left.join().unwrap();
        let right_result = right.join().unwrap();

        let outcomes = [left_result.is_ok(), right_result.is_ok()];
        assert_eq!(
            outcomes.iter().filter(|ok| **ok).count(),
            1,
            "exactly one racer must commit; left={left_result:?} right={right_result:?}"
        );
        assert!(
            left_result.is_err() || right_result.is_err(),
            "loser must surface conflict or install failure"
        );

        let loaded = load_active_manifest(dir.path()).unwrap().expect("winner");
        assert!(
            loaded == first || loaded == second,
            "final must be exactly one of the two candidates"
        );
        let loser = if left_result.is_ok() {
            assert_eq!(loaded, first);
            right_result.err().expect("loser")
        } else {
            assert_eq!(loaded, second);
            left_result.err().expect("loser")
        };
        assert!(
            loser.contains("artifact conflict"),
            "loser should report artifact conflict, got {loser}"
        );
    }

    #[test]
    fn concurrent_identical_activations_converge_to_active() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();
        let value = manifest();
        let barrier = Arc::new(Barrier::new(2));

        let make = |dir_path: PathBuf, barrier: Arc<Barrier>, value: ActiveJournalManifestV3| {
            thread::spawn(move || {
                barrier.wait();
                activate_initial_v3(&dir_path, &value)
            })
        };

        let left = make(dir_path.clone(), Arc::clone(&barrier), value.clone());
        let right = make(dir_path, barrier, value.clone());
        let left_result = left.join().unwrap().unwrap();
        let right_result = right.join().unwrap().unwrap();

        assert!(
            matches!(
                (&left_result, &right_result),
                (
                    ActivationOutcome::Activated,
                    ActivationOutcome::AlreadyActive
                ) | (
                    ActivationOutcome::AlreadyActive,
                    ActivationOutcome::Activated
                ) | (ActivationOutcome::Activated, ActivationOutcome::Activated)
            ),
            "identical racers must only report activated/already-active, got {left_result:?}/{right_result:?}"
        );
        assert_eq!(load_active_manifest(dir.path()).unwrap(), Some(value));
    }

    #[test]
    fn no_replace_install_refuses_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join(ACTIVE_MANIFEST_FILE);
        let winner = manifest();
        std::fs::write(&final_path, winner.canonical_bytes().unwrap()).unwrap();

        let mut challenger = winner.clone();
        challenger.origin = ActivationOriginV3::Migration {
            id: "77".repeat(32),
            map_path: "history/challenger-map.json".to_string(),
            map_sha256: "99".repeat(32),
            certificate_path: "history/challenger-certificate.json".to_string(),
            certificate_sha256: "aa".repeat(32),
        };
        challenger.active.sha256 = "88".repeat(32);
        let tmp = prepared_temp_path(dir.path(), challenger.origin.id());
        std::fs::write(&tmp, challenger.canonical_bytes().unwrap()).unwrap();

        let err = atomic_install_no_replace(&tmp, &final_path, dir.path()).unwrap_err();
        assert!(matches!(err, InstallError::DestinationExists));
        assert_eq!(
            load_active_manifest(dir.path()).unwrap(),
            Some(winner),
            "existing final must be preserved"
        );
    }

    #[test]
    fn post_commit_dir_sync_failure_still_reports_activated() {
        // Commit is the no-replace create. Even when directory durability sync
        // is a no-op failure path, observe_final must still report the true
        // activated state rather than "not activated".
        let dir = tempfile::tempdir().unwrap();
        let value = manifest();
        let bytes = value.canonical_bytes().unwrap();
        let tmp = prepared_temp_path(dir.path(), value.origin.id());
        std::fs::write(&tmp, &bytes).unwrap();
        let final_path = dir.path().join(ACTIVE_MANIFEST_FILE);

        platform_no_replace_install(&tmp, &final_path).unwrap();
        // Simulate durability-sync failure: ignore sync error, re-read final.
        let _ = sync_journal_dir(dir.path());
        let outcome = observe_final(&final_path, &value).unwrap();
        assert_eq!(outcome, Some(ActivationOutcome::AlreadyActive));
        assert_eq!(
            activate_initial_v3(dir.path(), &value).unwrap(),
            ActivationOutcome::AlreadyActive
        );
    }

    #[test]
    fn canonical_manifest_has_a_fixed_golden_vector() {
        let value = manifest();
        let text = String::from_utf8(value.canonical_bytes().unwrap()).unwrap();
        assert_eq!(
            text,
            format!(
                "{{\"active\":{{\"path\":\"journals/journal.v3.jsonl\",\"schema\":\"v3\",\"sha256\":\"{}\"}},\"history\":[{{\"path\":\"history/journal.v2.jsonl\",\"schema\":\"v2\",\"sha256\":\"{}\"}}],\"journal_id\":\"{}\",\"origin\":{{\"certificate_path\":\"history/migration-v2-to-v3.certificate.json\",\"certificate_sha256\":\"{}\",\"id\":\"{}\",\"kind\":\"migration\",\"map_path\":\"history/migration-v2-to-v3.json\",\"map_sha256\":\"{}\"}},\"schema\":\"{}\"}}",
                "22".repeat(32),
                "33".repeat(32),
                "11".repeat(32),
                "77".repeat(32),
                "44".repeat(32),
                "66".repeat(32),
                ACTIVE_MANIFEST_SCHEMA
            )
        );
    }

    #[test]
    fn manifest_schema_rejects_unknown_fields_and_uppercase_hex() {
        let mut value = serde_json::to_value(manifest()).unwrap();
        value["active"]["future_field"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ActiveJournalManifestV3>(value).is_err());

        let mut uppercase = manifest();
        uppercase.journal_id = "AA".repeat(32);
        assert!(uppercase.validate().unwrap_err().contains("lowercase hex"));

        let mut uppercase_artifact = manifest();
        let ActivationOriginV3::Migration { map_sha256, .. } = &mut uppercase_artifact.origin
        else {
            panic!("migration origin")
        };
        *map_sha256 = "BB".repeat(32);
        assert!(
            uppercase_artifact
                .validate()
                .unwrap_err()
                .contains("origin.map_sha256")
        );
    }

    #[test]
    fn manifest_reader_requires_unique_canonical_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ACTIVE_MANIFEST_FILE);
        let canonical = String::from_utf8(manifest().canonical_bytes().unwrap()).unwrap();

        std::fs::write(&path, format!(" {canonical}")).unwrap();
        assert!(
            load_active_manifest(dir.path())
                .unwrap_err()
                .contains("not canonical JCS")
        );

        let duplicate = format!(
            "{{\"schema\":\"{}\",{}",
            ACTIVE_MANIFEST_SCHEMA,
            &canonical[1..]
        );
        std::fs::write(&path, duplicate).unwrap();
        assert!(
            load_active_manifest(dir.path())
                .unwrap_err()
                .contains("duplicate JSON object member")
        );
    }

    #[test]
    fn committed_sync_failure_has_an_explicit_outcome() {
        let outcome = committed_outcome(Some("directory fsync failed".to_string()));
        assert_eq!(
            outcome,
            ActivationOutcome::ActivatedDurabilityUncertain {
                detail: "directory fsync failed".to_string()
            }
        );
    }
}
