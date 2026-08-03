use agent_chat_cleaner_core::{AgentKind, Fingerprint, SessionRecord};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum QuarantineError {
    #[error("该会话不是可安全处理的独占文件")]
    NotMutable,
    #[error("源文件超出允许的数据目录")]
    OutsideRoot,
    #[error("源文件已经变化，请重新扫描")]
    Changed,
    #[error("路径包含符号链接或重解析目标")]
    LinkedPath,
    #[error("恢复目标已经存在，未覆盖任何文件")]
    RestoreConflict,
    #[error("隔离内容校验失败")]
    HashMismatch,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub operation_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub restored_at: Option<DateTime<Utc>>,
    pub state: ManifestState,
    pub agent: AgentKind,
    pub session_id: String,
    pub source: PathBuf,
    pub allowed_root: PathBuf,
    pub bytes: u64,
    pub blake3: String,
    pub payload: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestState {
    Preparing,
    Quarantined,
    Restored,
}

impl Manifest {
    pub fn display_id(&self) -> &str {
        self.session_id.get(..8).unwrap_or(&self.session_id)
    }
}

pub fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("AgentChatCleaner")
}

pub fn quarantine(record: &SessionRecord, data_dir: &Path) -> Result<Manifest, QuarantineError> {
    if !record.is_mutable() {
        return Err(QuarantineError::NotMutable);
    }
    revalidate(record)?;

    let operation_id = Uuid::new_v4();
    let operation_dir = data_dir.join("quarantine").join(operation_id.to_string());
    fs::create_dir_all(&operation_dir)?;
    let payload_path = operation_dir.join("payload.jsonl");
    let source_hash = hash_file(&record.source)?;
    copy_and_sync(&record.source, &payload_path)?;
    if hash_file(&payload_path)? != source_hash {
        return Err(QuarantineError::HashMismatch);
    }

    let mut manifest = Manifest {
        schema_version: 1,
        operation_id,
        created_at: Utc::now(),
        restored_at: None,
        state: ManifestState::Preparing,
        agent: record.agent,
        session_id: record.id.clone(),
        source: record.source.clone(),
        allowed_root: record.allowed_root.clone(),
        bytes: record.bytes,
        blake3: source_hash,
        payload: "payload.jsonl".to_owned(),
    };
    write_manifest_atomic(&operation_dir, &manifest)?;

    // The exact source file is removed only after the recoverable copy and manifest are durable.
    revalidate(record)?;
    fs::remove_file(&record.source)?;
    if record.source.exists() {
        return Err(io::Error::other("source still exists after quarantine").into());
    }
    manifest.state = ManifestState::Quarantined;
    write_manifest_atomic(&operation_dir, &manifest)?;
    Ok(manifest)
}

pub fn restore(manifest: &Manifest, data_dir: &Path) -> Result<Manifest, QuarantineError> {
    if manifest.state == ManifestState::Restored {
        return Ok(manifest.clone());
    }
    let operation_dir = data_dir
        .join("quarantine")
        .join(manifest.operation_id.to_string());
    let payload = operation_dir.join(&manifest.payload);
    if hash_file(&payload)? != manifest.blake3 {
        return Err(QuarantineError::HashMismatch);
    }
    if manifest.source.exists() {
        return Err(QuarantineError::RestoreConflict);
    }
    ensure_restore_parent_is_safe(&manifest.source, &manifest.allowed_root)?;
    let parent = manifest
        .source
        .parent()
        .ok_or(QuarantineError::OutsideRoot)?;
    fs::create_dir_all(parent)?;
    ensure_restore_parent_is_safe(&manifest.source, &manifest.allowed_root)?;
    let temp = parent.join(format!(".acc-restore-{}.tmp", manifest.operation_id));
    if temp.exists() {
        return Err(QuarantineError::RestoreConflict);
    }
    copy_and_sync(&payload, &temp)?;
    if hash_file(&temp)? != manifest.blake3 {
        let _ = fs::remove_file(&temp);
        return Err(QuarantineError::HashMismatch);
    }
    if manifest.source.exists() {
        let _ = fs::remove_file(&temp);
        return Err(QuarantineError::RestoreConflict);
    }
    fs::rename(&temp, &manifest.source)?;
    let mut updated = manifest.clone();
    updated.state = ManifestState::Restored;
    updated.restored_at = Some(Utc::now());
    write_manifest_atomic(&operation_dir, &updated)?;
    Ok(updated)
}

pub fn list_manifests(data_dir: &Path) -> Vec<Manifest> {
    let root = data_dir.join("quarantine");
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut manifests: Vec<Manifest> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let final_path = entry.path().join("manifest.json");
            let temporary_path = entry.path().join("manifest.json.tmp");
            File::open(&final_path)
                .or_else(|_| File::open(&temporary_path))
                .ok()
        })
        .filter_map(|file| serde_json::from_reader::<_, Manifest>(BufReader::new(file)).ok())
        .collect();
    manifests.sort_by_key(|item| std::cmp::Reverse(item.created_at));
    manifests
}

fn revalidate(record: &SessionRecord) -> Result<(), QuarantineError> {
    let root = record
        .allowed_root
        .canonicalize()
        .map_err(|_| QuarantineError::OutsideRoot)?;
    let metadata = fs::symlink_metadata(&record.source)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(QuarantineError::LinkedPath);
    }
    let source = record.source.canonicalize()?;
    if !source.starts_with(&root) {
        return Err(QuarantineError::OutsideRoot);
    }
    let current = Fingerprint {
        len: metadata.len(),
        modified_millis: metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    };
    if current != record.fingerprint {
        return Err(QuarantineError::Changed);
    }
    Ok(())
}

fn ensure_restore_parent_is_safe(
    source: &Path,
    allowed_root: &Path,
) -> Result<(), QuarantineError> {
    let root = allowed_root
        .canonicalize()
        .map_err(|_| QuarantineError::OutsideRoot)?;
    let parent = source.parent().ok_or(QuarantineError::OutsideRoot)?;
    let existing_parent = nearest_existing_parent(parent).ok_or(QuarantineError::OutsideRoot)?;
    let canonical_parent = existing_parent.canonicalize()?;
    if !canonical_parent.starts_with(&root) {
        return Err(QuarantineError::OutsideRoot);
    }
    Ok(())
}

fn nearest_existing_parent(path: &Path) -> Option<&Path> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return Some(candidate);
        }
        current = candidate.parent();
    }
    None
}

fn hash_file(path: &Path) -> Result<String, QuarantineError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn copy_and_sync(source: &Path, destination: &Path) -> Result<(), QuarantineError> {
    let input = File::open(source)?;
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut reader = BufReader::new(input);
    let mut writer = BufWriter::new(output);
    io::copy(&mut reader, &mut writer)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

fn write_manifest_atomic(operation_dir: &Path, manifest: &Manifest) -> Result<(), QuarantineError> {
    let final_path = operation_dir.join("manifest.json");
    let temp_path = operation_dir.join("manifest.json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp_path)?;
    serde_json::to_writer_pretty(&mut file, manifest)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    if final_path.exists() {
        fs::remove_file(&final_path)?;
    }
    fs::rename(temp_path, final_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_chat_cleaner_core::SafetyState;
    use std::time::{Duration, SystemTime};

    fn fixture_record(root: &Path) -> SessionRecord {
        fs::create_dir_all(root.join("project")).unwrap();
        let source = root.join("project/session.jsonl");
        fs::write(&source, b"fixture only\n").unwrap();
        let metadata = fs::metadata(&source).unwrap();
        let modified = metadata.modified().unwrap();
        SessionRecord {
            id: "11111111-2222-4333-8444-555555555555".to_owned(),
            agent: AgentKind::Codex,
            project_label: "fixture".to_owned(),
            source,
            allowed_root: root.canonicalize().unwrap(),
            bytes: metadata.len(),
            modified: SystemTime::now() - Duration::from_secs(3600),
            fingerprint: Fingerprint {
                len: metadata.len(),
                modified_millis: modified.duration_since(UNIX_EPOCH).unwrap().as_millis(),
            },
            safety: SafetyState::SupportedIdle,
            exclusive_file: true,
        }
    }

    #[test]
    fn quarantine_and_restore_touch_only_selected_fixture() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let data = temp.path().join("data");
        let record = fixture_record(&root);
        let neighbor = root.join("project/neighbor.jsonl");
        fs::write(&neighbor, b"must remain\n").unwrap();
        let manifest = quarantine(&record, &data).unwrap();
        assert!(!record.source.exists());
        assert_eq!(fs::read(&neighbor).unwrap(), b"must remain\n");
        let restored = restore(&manifest, &data).unwrap();
        assert_eq!(restored.state, ManifestState::Restored);
        assert_eq!(fs::read(&record.source).unwrap(), b"fixture only\n");
        assert_eq!(fs::read(&neighbor).unwrap(), b"must remain\n");
    }

    #[test]
    fn changed_source_is_never_removed() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let data = temp.path().join("data");
        let record = fixture_record(&root);
        fs::write(&record.source, b"changed after scan\n").unwrap();
        assert!(matches!(
            quarantine(&record, &data),
            Err(QuarantineError::Changed)
        ));
        assert!(record.source.exists());
    }

    #[test]
    fn restore_never_overwrites_a_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let data = temp.path().join("data");
        let record = fixture_record(&root);
        let manifest = quarantine(&record, &data).unwrap();
        fs::write(&record.source, b"new content\n").unwrap();
        assert!(matches!(
            restore(&manifest, &data),
            Err(QuarantineError::RestoreConflict)
        ));
        assert_eq!(fs::read(&record.source).unwrap(), b"new content\n");
    }
}
