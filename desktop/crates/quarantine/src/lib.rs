use agent_chat_cleaner_core::{
    AgentKind, CursorSessionStorage, Fingerprint, SessionRecord, SessionStorage,
    cursor_process_running,
};
use chrono::{DateTime, Utc};
use rusqlite::types::Value as SqlValue;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::UNIX_EPOCH;
use thiserror::Error;
use uuid::Uuid;
use walkdir::WalkDir;

const MAX_CODEX_GLOBAL_STATE_BYTES: u64 = 16 * 1024 * 1024;

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
    #[error("隔离内容已经永久删除，无法恢复")]
    PayloadPurged,
    #[error("请先完全退出 Cursor 后再操作")]
    CursorRunning,
    #[error("无法确认 Cursor 是否已退出，已安全停止")]
    CursorProcessUnknown,
    #[error("Cursor 数据库结构不受当前版本支持")]
    UnsupportedCursorSchema,
    #[error("Cursor 会话已经置顶，已自动锁定")]
    CursorPinned,
    #[error("ChatGPT / Codex 会话已经置顶，已自动锁定")]
    CodexPinned,
    #[error("Cursor 会话关联记录不完整或已变化")]
    CursorRelationshipChanged,
    #[error("Cursor 会话恢复目标已经存在")]
    CursorRestoreConflict,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
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
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub project_label: String,
    #[serde(default)]
    pub project_path: Option<PathBuf>,
    #[serde(default)]
    pub pinned: bool,
    pub source: PathBuf,
    pub allowed_root: PathBuf,
    #[serde(default)]
    pub source_modified_millis: Option<u64>,
    #[serde(default)]
    pub session_modified_millis: Option<u64>,
    pub bytes: u64,
    pub blake3: String,
    pub payload: String,
    #[serde(default)]
    pub cursor: Option<CursorManifestData>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CursorManifestData {
    pub database: PathBuf,
    pub global_storage_root: PathBuf,
    pub projects_root: PathBuf,
    pub transcript_dir: Option<PathBuf>,
    #[serde(default)]
    pub staged_transcript_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestState {
    Preparing,
    Quarantined,
    Purging,
    Purged,
    Restored,
}

impl Manifest {
    pub fn display_id(&self) -> &str {
        self.session_id.get(..8).unwrap_or(&self.session_id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteMode {
    Recoverable,
    Permanent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchPhase {
    Checking,
    BackingUp,
    Deleting,
    Finished,
}

#[derive(Clone, Debug)]
pub struct BatchProgress {
    pub completed: usize,
    pub total: usize,
    pub phase: BatchPhase,
    pub current_title: String,
}

pub fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("AgentChatCleaner")
}

pub fn quarantine(record: &SessionRecord, data_dir: &Path) -> Result<Manifest, QuarantineError> {
    if let SessionStorage::CursorSqlite(storage) = &record.storage {
        return quarantine_cursor(record, storage, data_dir, true);
    }
    quarantine_exclusive_file(record, data_dir)
}

pub fn quarantine_batch(
    records: &[SessionRecord],
    data_dir: &Path,
) -> Vec<(String, Result<Manifest, QuarantineError>)> {
    quarantine_batch_impl(records, data_dir, true)
}

pub fn delete_batch<F>(
    records: &[SessionRecord],
    data_dir: &Path,
    mode: DeleteMode,
    enforce_process_guard: bool,
    mut progress: F,
) -> Vec<(String, Result<(), QuarantineError>)>
where
    F: FnMut(BatchProgress),
{
    let total = records.len();
    if records
        .iter()
        .all(|record| matches!(&record.storage, SessionStorage::ExclusiveFile))
    {
        return delete_exclusive_batch(records, data_dir, mode, &mut progress);
    }
    let mut cursor_snapshot: Option<(PathBuf, Fingerprint, Option<Fingerprint>)> = None;
    let mut cursor_pins: Option<HashSet<String>> = None;
    let mut cursor_preflight_error: Option<String> = None;
    if let Some(storage) = records.iter().find_map(|record| match &record.storage {
        SessionStorage::CursorSqlite(storage) => Some(storage),
        SessionStorage::ExclusiveFile => None,
    }) {
        progress(BatchProgress {
            completed: 0,
            total,
            phase: BatchPhase::Checking,
            current_title: String::new(),
        });
        if let Err(error) = ensure_cursor_stopped(enforce_process_guard) {
            cursor_preflight_error = Some(error.to_string());
        } else {
            match cursor_pinned_session_ids(storage) {
                Ok(ids) => cursor_pins = Some(ids),
                Err(error) => cursor_preflight_error = Some(error.to_string()),
            }
        }
    }

    let mut outcomes = Vec::with_capacity(total);
    for (index, record) in records.iter().enumerate() {
        progress(BatchProgress {
            completed: index,
            total,
            phase: if mode == DeleteMode::Recoverable {
                BatchPhase::BackingUp
            } else {
                BatchPhase::Deleting
            },
            current_title: record.title.clone(),
        });
        let mut adjusted = record.clone();
        if let SessionStorage::CursorSqlite(storage) = &mut adjusted.storage
            && let Some((database, fingerprint, wal_fingerprint)) = &cursor_snapshot
            && database == &storage.database
        {
            adjusted.fingerprint = fingerprint.clone();
            storage.wal_fingerprint = wal_fingerprint.clone();
        }
        if matches!(&adjusted.storage, SessionStorage::CursorSqlite(_))
            && enforce_process_guard
            && index > 0
            && index % 32 == 0
            && let Err(error) = ensure_cursor_stopped(true)
        {
            cursor_preflight_error = Some(error.to_string());
        }
        let guard_error = if matches!(&adjusted.storage, SessionStorage::CursorSqlite(_)) {
            cursor_preflight_error.clone()
        } else {
            None
        };
        let result = if let Some(error) = guard_error {
            Err(io::Error::other(error).into())
        } else {
            match (&adjusted.storage, mode) {
                (SessionStorage::CursorSqlite(storage), DeleteMode::Recoverable) => {
                    quarantine_cursor_with_pins(
                        &adjusted,
                        storage,
                        data_dir,
                        false,
                        cursor_pins.as_ref(),
                    )
                    .map(|_| ())
                }
                (SessionStorage::CursorSqlite(storage), DeleteMode::Permanent) => {
                    quarantine_cursor_with_pins(
                        &adjusted,
                        storage,
                        data_dir,
                        false,
                        cursor_pins.as_ref(),
                    )
                    .and_then(|manifest| purge(&manifest, data_dir))
                    .map(|_| ())
                }
                (SessionStorage::ExclusiveFile, DeleteMode::Recoverable) => {
                    quarantine_exclusive_file(&adjusted, data_dir).map(|_| ())
                }
                (SessionStorage::ExclusiveFile, DeleteMode::Permanent) => {
                    delete_exclusive_file_permanently(&adjusted)
                }
            }
        };
        if result.is_ok()
            && let SessionStorage::CursorSqlite(storage) = &adjusted.storage
            && let Ok(fingerprint) = file_fingerprint(&storage.database)
        {
            let wal = storage.global_storage_root.join("state.vscdb-wal");
            let wal_fingerprint = wal
                .exists()
                .then(|| file_fingerprint(&wal))
                .transpose()
                .ok()
                .flatten();
            cursor_snapshot = Some((storage.database.clone(), fingerprint, wal_fingerprint));
        }
        outcomes.push((record.id.clone(), result));
        progress(BatchProgress {
            completed: index + 1,
            total,
            phase: if index + 1 == total {
                BatchPhase::Finished
            } else {
                BatchPhase::Deleting
            },
            current_title: record.title.clone(),
        });
    }
    outcomes
}

fn delete_exclusive_batch<F>(
    records: &[SessionRecord],
    data_dir: &Path,
    mode: DeleteMode,
    progress: &mut F,
) -> Vec<(String, Result<(), QuarantineError>)>
where
    F: FnMut(BatchProgress),
{
    let total = records.len();
    if total == 0 {
        return Vec::new();
    }
    let worker_count = total.min(if mode == DeleteMode::Recoverable {
        2
    } else {
        4
    });
    progress(BatchProgress {
        completed: 0,
        total,
        phase: if mode == DeleteMode::Recoverable {
            BatchPhase::BackingUp
        } else {
            BatchPhase::Deleting
        },
        current_title: String::new(),
    });
    let queue = Arc::new(Mutex::new(VecDeque::from_iter(
        records.iter().cloned().enumerate(),
    )));
    let (tx, rx) = mpsc::channel();
    let mut completed = 0usize;
    let mut ordered: Vec<Option<(String, Result<(), QuarantineError>)>> =
        (0..total).map(|_| None).collect();
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            scope.spawn(move || {
                loop {
                    let next = queue.lock().ok().and_then(|mut queue| queue.pop_front());
                    let Some((index, record)) = next else {
                        break;
                    };
                    let result = match mode {
                        DeleteMode::Recoverable => {
                            quarantine_exclusive_file(&record, data_dir).map(|_| ())
                        }
                        DeleteMode::Permanent => delete_exclusive_file_permanently(&record),
                    };
                    if tx.send((index, record.id, record.title, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        while let Ok((index, id, title, result)) = rx.recv() {
            completed += 1;
            ordered[index] = Some((id, result));
            progress(BatchProgress {
                completed,
                total,
                phase: if completed == total {
                    BatchPhase::Finished
                } else if mode == DeleteMode::Recoverable {
                    BatchPhase::BackingUp
                } else {
                    BatchPhase::Deleting
                },
                current_title: title,
            });
        }
    });
    ordered
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            item.unwrap_or_else(|| {
                (
                    records[index].id.clone(),
                    Err(io::Error::other("batch worker stopped unexpectedly").into()),
                )
            })
        })
        .collect()
}

fn quarantine_batch_impl(
    records: &[SessionRecord],
    data_dir: &Path,
    enforce_process_guard: bool,
) -> Vec<(String, Result<Manifest, QuarantineError>)> {
    let mut cursor_snapshot: Option<(PathBuf, Fingerprint, Option<Fingerprint>)> = None;
    records
        .iter()
        .map(|record| {
            let mut adjusted = record.clone();
            if let SessionStorage::CursorSqlite(storage) = &mut adjusted.storage
                && let Some((database, fingerprint, wal_fingerprint)) = &cursor_snapshot
                && database == &storage.database
            {
                adjusted.fingerprint = fingerprint.clone();
                storage.wal_fingerprint = wal_fingerprint.clone();
            }
            let result = match &adjusted.storage {
                SessionStorage::CursorSqlite(storage) => {
                    quarantine_cursor(&adjusted, storage, data_dir, enforce_process_guard)
                }
                SessionStorage::ExclusiveFile => quarantine_exclusive_file(&adjusted, data_dir),
            };
            if result.is_ok()
                && let SessionStorage::CursorSqlite(storage) = &adjusted.storage
                && let Ok(fingerprint) = file_fingerprint(&storage.database)
            {
                let wal = storage.global_storage_root.join("state.vscdb-wal");
                let wal_fingerprint = wal
                    .exists()
                    .then(|| file_fingerprint(&wal))
                    .transpose()
                    .ok()
                    .flatten();
                cursor_snapshot = Some((storage.database.clone(), fingerprint, wal_fingerprint));
            }
            (record.id.clone(), result)
        })
        .collect()
}

fn quarantine_exclusive_file(
    record: &SessionRecord,
    data_dir: &Path,
) -> Result<Manifest, QuarantineError> {
    if !record.is_mutable() {
        return Err(QuarantineError::NotMutable);
    }
    ensure_exclusive_session_not_pinned(record)?;
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
        title: record.title.clone(),
        project_label: record.project_label.clone(),
        project_path: record.project_path.clone(),
        pinned: record.pinned,
        source: record.source.clone(),
        allowed_root: record.allowed_root.clone(),
        source_modified_millis: record.fingerprint.modified_millis.try_into().ok(),
        session_modified_millis: record
            .modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| duration.as_millis().try_into().ok()),
        bytes: record.bytes,
        blake3: source_hash,
        payload: "payload.jsonl".to_owned(),
        cursor: None,
    };
    write_manifest_atomic(&operation_dir, &manifest)?;

    // The exact source file is removed only after the recoverable copy and manifest are durable.
    ensure_exclusive_session_not_pinned(record)?;
    revalidate(record)?;
    fs::remove_file(&record.source)?;
    if record.source.exists() {
        return Err(io::Error::other("source still exists after quarantine").into());
    }
    manifest.state = ManifestState::Quarantined;
    write_manifest_atomic(&operation_dir, &manifest)?;
    Ok(manifest)
}

fn ensure_exclusive_session_not_pinned(record: &SessionRecord) -> Result<(), QuarantineError> {
    if record.agent == AgentKind::Codex && codex_session_is_pinned(record)? {
        return Err(QuarantineError::CodexPinned);
    }
    Ok(())
}

fn codex_session_is_pinned(record: &SessionRecord) -> Result<bool, QuarantineError> {
    let Some(codex_root) = record.allowed_root.parent() else {
        return Ok(false);
    };
    if codex_desktop_state_pins_session(codex_root, &record.id)? {
        return Ok(true);
    }
    let Some(database) = latest_codex_state_database(codex_root) else {
        return Ok(false);
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(database, flags)?;
    if !table_has_columns(&connection, "threads", &["id", "is_pinned"]) {
        return Ok(false);
    }
    let pinned = connection
        .query_row(
            "SELECT coalesce(is_pinned, 0) FROM threads WHERE id = ?1",
            [&record.id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(pinned.is_some_and(|value| value != 0))
}

fn codex_desktop_state_pins_session(
    codex_root: &Path,
    session_id: &str,
) -> Result<bool, QuarantineError> {
    let path = codex_root.join(".codex-global-state.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(QuarantineError::LinkedPath);
    }
    if metadata.len() > MAX_CODEX_GLOBAL_STATE_BYTES {
        return Err(io::Error::other("Codex global state exceeds the safety limit").into());
    }
    let file = File::open(path)?;
    let value: serde_json::Value =
        serde_json::from_reader(BufReader::new(file).take(MAX_CODEX_GLOBAL_STATE_BYTES))?;
    Ok(value
        .get("pinned-thread-ids")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|ids| {
            ids.iter()
                .filter_map(serde_json::Value::as_str)
                .any(|id| id == session_id)
        }))
}

fn latest_codex_state_database(codex_root: &Path) -> Option<PathBuf> {
    fs::read_dir(codex_root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let version = name
                .strip_prefix("state_")?
                .strip_suffix(".sqlite")?
                .parse::<u32>()
                .ok()?;
            entry
                .file_type()
                .ok()?
                .is_file()
                .then_some((version, entry.path()))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path)
}

const CURSOR_PAYLOAD: &str = "cursor-session.sqlite";
const CURSOR_PREFIX_FAMILIES: [&str; 4] = [
    "bubbleId",
    "checkpointId",
    "codeBlockPartialInlineDiffFates",
    "ofsContent",
];

#[derive(Clone, Debug, PartialEq)]
struct CursorHeaderRow {
    composer_id: String,
    workspace_id: Option<String>,
    created_at: Option<i64>,
    last_updated_at: Option<i64>,
    is_archived: Option<i64>,
    is_subagent: Option<i64>,
    recency: Option<i64>,
    checkpoint_at: Option<i64>,
    value: Option<String>,
}

fn quarantine_cursor(
    record: &SessionRecord,
    storage: &CursorSessionStorage,
    data_dir: &Path,
    enforce_process_guard: bool,
) -> Result<Manifest, QuarantineError> {
    quarantine_cursor_with_pins(record, storage, data_dir, enforce_process_guard, None)
}

fn quarantine_cursor_with_pins(
    record: &SessionRecord,
    storage: &CursorSessionStorage,
    data_dir: &Path,
    enforce_process_guard: bool,
    pinned_ids: Option<&HashSet<String>>,
) -> Result<Manifest, QuarantineError> {
    ensure_cursor_stopped(enforce_process_guard)?;
    if !record.is_mutable() {
        return Err(QuarantineError::NotMutable);
    }
    if pinned_ids.map_or_else(
        || cursor_session_is_pinned(storage, &record.id),
        |ids| Ok(ids.contains(&record.id)),
    )? {
        return Err(QuarantineError::CursorPinned);
    }
    revalidate_cursor(record, storage)?;

    let operation_id = Uuid::new_v4();
    let operation_dir = data_dir.join("quarantine").join(operation_id.to_string());
    fs::create_dir_all(&operation_dir)?;
    let payload_path = operation_dir.join(CURSOR_PAYLOAD);
    if let Err(error) = create_cursor_payload(record, storage, &payload_path) {
        let _ = fs::remove_dir_all(&operation_dir);
        return Err(error);
    }
    let payload_hash = match hash_file(&payload_path) {
        Ok(hash) => hash,
        Err(error) => {
            let _ = fs::remove_dir_all(&operation_dir);
            return Err(error);
        }
    };

    let mut manifest = Manifest {
        schema_version: 2,
        operation_id,
        created_at: Utc::now(),
        restored_at: None,
        state: ManifestState::Preparing,
        agent: AgentKind::Cursor,
        session_id: record.id.clone(),
        title: record.title.clone(),
        project_label: record.project_label.clone(),
        project_path: record.project_path.clone(),
        pinned: record.pinned,
        source: storage.database.clone(),
        allowed_root: storage.global_storage_root.clone(),
        source_modified_millis: None,
        session_modified_millis: record
            .modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| duration.as_millis().try_into().ok()),
        bytes: fs::metadata(&payload_path)?.len(),
        blake3: payload_hash,
        payload: CURSOR_PAYLOAD.to_owned(),
        cursor: Some(CursorManifestData {
            database: storage.database.clone(),
            global_storage_root: storage.global_storage_root.clone(),
            projects_root: storage.projects_root.clone(),
            transcript_dir: storage.transcript_dir.clone(),
            staged_transcript_dir: None,
        }),
    };
    write_manifest_atomic(&operation_dir, &manifest)?;

    ensure_cursor_stopped(enforce_process_guard)?;
    revalidate_cursor(record, storage)?;
    let staged = stage_cursor_transcript(record, storage, operation_id)?;
    if let Some(cursor) = &mut manifest.cursor {
        cursor.staged_transcript_dir = staged.clone();
    }
    write_manifest_atomic(&operation_dir, &manifest)?;

    if let Err(error) = delete_cursor_database_rows(record, storage, &payload_path) {
        if let (Some(staged), Some(original)) = (staged.as_ref(), storage.transcript_dir.as_ref()) {
            let _ = fs::rename(staged, original);
        }
        return Err(error);
    }

    if let Some(staged) = &staged
        && fs::remove_dir_all(staged).is_ok()
        && let Some(cursor) = &mut manifest.cursor
    {
        cursor.staged_transcript_dir = None;
    }
    manifest.state = ManifestState::Quarantined;
    write_manifest_atomic(&operation_dir, &manifest)?;
    Ok(manifest)
}

fn delete_exclusive_file_permanently(record: &SessionRecord) -> Result<(), QuarantineError> {
    if !record.is_mutable() {
        return Err(QuarantineError::NotMutable);
    }
    ensure_exclusive_session_not_pinned(record)?;
    revalidate(record)?;
    fs::remove_file(&record.source)?;
    if record.source.exists() {
        return Err(io::Error::other("source still exists after permanent deletion").into());
    }
    Ok(())
}

fn create_cursor_payload(
    record: &SessionRecord,
    storage: &CursorSessionStorage,
    payload_path: &Path,
) -> Result<(), QuarantineError> {
    let source = open_cursor_database(&storage.database, false)?;
    ensure_cursor_schema(&source)?;
    let mut payload = Connection::open(payload_path)?;
    payload.execute_batch(
        "PRAGMA journal_mode=DELETE;
         PRAGMA synchronous=FULL;
         CREATE TABLE payload_meta (session_id TEXT PRIMARY KEY);
         CREATE TABLE kv_rows (key TEXT PRIMARY KEY, value_type INTEGER NOT NULL, value BLOB NOT NULL);
         CREATE TABLE header_rows (
           composerId TEXT PRIMARY KEY,
           workspaceId TEXT,
           createdAt INTEGER,
           lastUpdatedAt INTEGER,
           isArchived INTEGER,
           isSubagent INTEGER,
           recency INTEGER,
           checkpointAt INTEGER,
           value TEXT
         );
         CREATE TABLE transcript_files (
           relative_path TEXT PRIMARY KEY,
           contents BLOB NOT NULL,
           blake3 TEXT NOT NULL
         );",
    )?;
    let tx = payload.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO payload_meta(session_id) VALUES (?1)",
        params![record.id],
    )?;

    let keys = cursor_session_keys(&source, &record.id)?;
    for key in &keys {
        let value: SqlValue = source.query_row(
            "SELECT value FROM cursorDiskKV WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )?;
        let (value_type, bytes) = encode_sql_value(&value);
        tx.execute(
            "INSERT INTO kv_rows(key, value_type, value) VALUES (?1, ?2, ?3)",
            params![key, value_type, bytes],
        )?;
    }

    if let Some(header) = read_cursor_header(&source, &record.id)? {
        insert_payload_header(&tx, &header)?;
    }
    if keys.is_empty()
        && tx.query_row("SELECT count(*) FROM header_rows", [], |row| {
            row.get::<_, i64>(0)
        })? == 0
        && storage.transcript_dir.is_none()
    {
        return Err(QuarantineError::CursorRelationshipChanged);
    }
    if let Some(transcript_dir) = &storage.transcript_dir {
        copy_transcript_into_payload(&tx, transcript_dir, storage, &record.id)?;
    }
    tx.commit()?;
    let check: String = payload.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(QuarantineError::HashMismatch);
    }
    payload.close().map_err(|(_, error)| error)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(payload_path)?
        .sync_all()?;
    Ok(())
}

fn delete_cursor_database_rows(
    record: &SessionRecord,
    storage: &CursorSessionStorage,
    payload_path: &Path,
) -> Result<(), QuarantineError> {
    let payload = open_payload(payload_path)?;
    let payload_keys = payload_keys(&payload)?;
    let mut database = open_cursor_database(&storage.database, true)?;
    database.busy_timeout(std::time::Duration::from_secs(2))?;
    ensure_cursor_schema(&database)?;
    let tx = database.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if cursor_session_keys(&tx, &record.id)? != payload_keys {
        return Err(QuarantineError::CursorRelationshipChanged);
    }
    for key in &payload_keys {
        let expected = payload_value(&payload, key)?;
        let current: Option<SqlValue> = tx
            .query_row(
                "SELECT value FROM cursorDiskKV WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        if current.as_ref() != Some(&expected) {
            return Err(QuarantineError::CursorRelationshipChanged);
        }
        if tx.execute("DELETE FROM cursorDiskKV WHERE key = ?1", params![key])? != 1 {
            return Err(QuarantineError::CursorRelationshipChanged);
        }
    }

    let expected_header = read_payload_header(&payload)?;
    let current_header = read_cursor_header(&tx, &record.id)?;
    if current_header != expected_header {
        return Err(QuarantineError::CursorRelationshipChanged);
    }
    if expected_header.is_some()
        && tx.execute(
            "DELETE FROM composerHeaders WHERE composerId = ?1",
            params![record.id],
        )? != 1
    {
        return Err(QuarantineError::CursorRelationshipChanged);
    }
    tx.commit()?;
    Ok(())
}

fn stage_cursor_transcript(
    record: &SessionRecord,
    storage: &CursorSessionStorage,
    operation_id: Uuid,
) -> Result<Option<PathBuf>, QuarantineError> {
    let Some(source) = &storage.transcript_dir else {
        return Ok(None);
    };
    validate_cursor_transcript_dir(source, storage, &record.id)?;
    let parent = source.parent().ok_or(QuarantineError::OutsideRoot)?;
    let staged = parent.join(format!(".acc-quarantine-{operation_id}"));
    if staged.exists() {
        return Err(QuarantineError::RestoreConflict);
    }
    fs::rename(source, &staged)?;
    Ok(Some(staged))
}

fn restore_cursor(
    manifest: &Manifest,
    data_dir: &Path,
    enforce_process_guard: bool,
) -> Result<Manifest, QuarantineError> {
    ensure_cursor_stopped(enforce_process_guard)?;
    if manifest.state == ManifestState::Restored {
        return Ok(manifest.clone());
    }
    if manifest.state == ManifestState::Purged {
        return Err(QuarantineError::PayloadPurged);
    }
    let cursor = manifest
        .cursor
        .as_ref()
        .ok_or(QuarantineError::UnsupportedCursorSchema)?;
    let operation_dir = data_dir
        .join("quarantine")
        .join(manifest.operation_id.to_string());
    let payload_path = operation_dir.join(&manifest.payload);
    if hash_file(&payload_path)? != manifest.blake3 {
        return Err(QuarantineError::HashMismatch);
    }
    let payload = open_payload(&payload_path)?;
    let mut database = open_cursor_database(&cursor.database, true)?;
    database.busy_timeout(std::time::Duration::from_secs(2))?;
    ensure_cursor_schema(&database)?;

    let target = cursor.transcript_dir.as_ref();
    let staged_restore = if let Some(target) = target {
        if target.exists() {
            return Err(QuarantineError::CursorRestoreConflict);
        }
        let parent = target.parent().ok_or(QuarantineError::OutsideRoot)?;
        ensure_restore_parent_is_safe(target, &cursor.projects_root)?;
        let temporary = parent.join(format!(".acc-restore-{}", manifest.operation_id));
        if temporary.exists() {
            return Err(QuarantineError::CursorRestoreConflict);
        }
        if let Some(staged) = cursor
            .staged_transcript_dir
            .as_ref()
            .filter(|path| path.exists())
        {
            validate_staged_cursor_dir(staged, cursor)?;
            fs::rename(staged, &temporary)?;
        } else {
            extract_transcript_from_payload(&payload, &temporary)?;
        }
        Some(temporary)
    } else {
        None
    };

    let tx = database.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let keys = payload_keys(&payload)?;
    for key in &keys {
        if tx
            .query_row(
                "SELECT 1 FROM cursorDiskKV WHERE key = ?1",
                params![key],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            cleanup_staged_restore(staged_restore.as_deref());
            return Err(QuarantineError::CursorRestoreConflict);
        }
    }
    let header = read_payload_header(&payload)?;
    if header.is_some() && read_cursor_header(&tx, &manifest.session_id)?.is_some() {
        cleanup_staged_restore(staged_restore.as_deref());
        return Err(QuarantineError::CursorRestoreConflict);
    }
    for key in &keys {
        let value = payload_value(&payload, key)?;
        tx.execute(
            "INSERT INTO cursorDiskKV(key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
    }
    if let Some(header) = &header {
        insert_cursor_header(&tx, header)?;
    }
    if let (Some(temporary), Some(target)) = (&staged_restore, target) {
        fs::rename(temporary, target)?;
    }
    if let Err(error) = tx.commit() {
        if let (Some(target), Some(temporary)) = (
            target.filter(|path| path.exists()),
            staged_restore.as_deref(),
        ) {
            let _ = fs::rename(target, temporary);
            cleanup_staged_restore(Some(temporary));
        }
        return Err(error.into());
    }

    let mut updated = manifest.clone();
    if let Some(cursor) = &mut updated.cursor {
        cursor.staged_transcript_dir = None;
    }
    updated.state = ManifestState::Restored;
    updated.restored_at = Some(Utc::now());
    write_manifest_atomic(&operation_dir, &updated)?;
    Ok(updated)
}

fn ensure_cursor_stopped(enforce: bool) -> Result<(), QuarantineError> {
    if !enforce {
        return Ok(());
    }
    match cursor_process_running() {
        Some(false) => Ok(()),
        Some(true) => Err(QuarantineError::CursorRunning),
        None => Err(QuarantineError::CursorProcessUnknown),
    }
}

fn open_cursor_database(path: &Path, writable: bool) -> Result<Connection, QuarantineError> {
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    Ok(Connection::open_with_flags(path, flags)?)
}

fn open_payload(path: &Path) -> Result<Connection, QuarantineError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let session_count: i64 =
        connection.query_row("SELECT count(*) FROM payload_meta", [], |row| row.get(0))?;
    if session_count != 1 {
        return Err(QuarantineError::HashMismatch);
    }
    Ok(connection)
}

fn ensure_cursor_schema(connection: &Connection) -> Result<(), QuarantineError> {
    if table_has_columns(
        connection,
        "composerHeaders",
        &[
            "composerId",
            "workspaceId",
            "createdAt",
            "lastUpdatedAt",
            "isArchived",
            "isSubagent",
            "recency",
            "checkpointAt",
            "value",
        ],
    ) && table_has_columns(connection, "cursorDiskKV", &["key", "value"])
    {
        Ok(())
    } else {
        Err(QuarantineError::UnsupportedCursorSchema)
    }
}

fn table_has_columns(connection: &Connection, table: &str, required: &[&str]) -> bool {
    let Ok(mut statement) = connection.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(1)) else {
        return false;
    };
    let columns: std::collections::HashSet<String> = rows.filter_map(Result::ok).collect();
    required.iter().all(|column| columns.contains(*column))
}

fn cursor_session_is_pinned(
    storage: &CursorSessionStorage,
    session_id: &str,
) -> Result<bool, QuarantineError> {
    Ok(cursor_pinned_session_ids(storage)?.contains(session_id))
}

fn cursor_pinned_session_ids(
    storage: &CursorSessionStorage,
) -> Result<HashSet<String>, QuarantineError> {
    let mut pinned = cursor_database_pinned_ids(&storage.database)?;
    let workspace_storage_root = storage
        .global_storage_root
        .parent()
        .map(|user_root| user_root.join("workspaceStorage"));
    let Some(workspace_storage_root) = workspace_storage_root else {
        return Ok(pinned);
    };
    let Ok(entries) = fs::read_dir(workspace_storage_root) else {
        return Ok(pinned);
    };
    for entry in entries.filter_map(Result::ok) {
        if !entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            continue;
        }
        let database = entry.path().join("state.vscdb");
        let Ok(metadata) = fs::symlink_metadata(&database) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        pinned.extend(cursor_database_pinned_ids(&database)?);
    }
    Ok(pinned)
}

fn cursor_database_pinned_ids(database: &Path) -> Result<HashSet<String>, QuarantineError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(database, flags)?;
    if !table_has_columns(&connection, "ItemTable", &["key", "value"]) {
        return Ok(HashSet::new());
    }
    let value = connection
        .query_row(
            "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = 'cursor/pinnedComposers'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(value) = value else {
        return Ok(HashSet::new());
    };
    let ids: Vec<String> = serde_json::from_str(&value)?;
    Ok(ids.into_iter().collect())
}

fn revalidate_cursor(
    record: &SessionRecord,
    storage: &CursorSessionStorage,
) -> Result<(), QuarantineError> {
    let root = storage
        .global_storage_root
        .canonicalize()
        .map_err(|_| QuarantineError::OutsideRoot)?;
    let metadata = fs::symlink_metadata(&storage.database)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(QuarantineError::LinkedPath);
    }
    let database = storage.database.canonicalize()?;
    if !database.starts_with(&root) {
        return Err(QuarantineError::OutsideRoot);
    }
    if file_fingerprint(&storage.database)? != record.fingerprint {
        return Err(QuarantineError::Changed);
    }
    let wal = storage.global_storage_root.join("state.vscdb-wal");
    let current_wal = if wal.exists() {
        Some(file_fingerprint(&wal)?)
    } else {
        None
    };
    if current_wal != storage.wal_fingerprint {
        return Err(QuarantineError::Changed);
    }
    Ok(())
}

fn cursor_session_keys(
    connection: &Connection,
    session_id: &str,
) -> Result<Vec<String>, QuarantineError> {
    let mut keys = Vec::new();
    for exact in [
        format!("composerData:{session_id}"),
        format!("composerVirtualRowHeights:{session_id}"),
    ] {
        if connection
            .query_row(
                "SELECT 1 FROM cursorDiskKV WHERE key = ?1",
                params![exact],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            keys.push(exact);
        }
    }
    for family in CURSOR_PREFIX_FAMILIES {
        let prefix = format!("{family}:{session_id}:");
        let upper = format!("{prefix}\u{10ffff}");
        let mut statement = connection
            .prepare("SELECT key FROM cursorDiskKV WHERE key >= ?1 AND key < ?2 ORDER BY key")?;
        let rows = statement.query_map(params![prefix, upper], |row| row.get::<_, String>(0))?;
        keys.extend(rows.filter_map(Result::ok));
    }
    keys.sort();
    keys.dedup();
    Ok(keys)
}

fn read_cursor_header(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<CursorHeaderRow>, QuarantineError> {
    connection
        .query_row(
            "SELECT composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent, \
                    recency, checkpointAt, value \
             FROM composerHeaders WHERE composerId = ?1",
            params![session_id],
            |row| {
                Ok(CursorHeaderRow {
                    composer_id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    created_at: row.get(2)?,
                    last_updated_at: row.get(3)?,
                    is_archived: row.get(4)?,
                    is_subagent: row.get(5)?,
                    recency: row.get(6)?,
                    checkpoint_at: row.get(7)?,
                    value: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn insert_payload_header(
    transaction: &Transaction<'_>,
    header: &CursorHeaderRow,
) -> Result<(), QuarantineError> {
    transaction.execute(
        "INSERT INTO header_rows(
           composerId, workspaceId, createdAt, lastUpdatedAt, isArchived,
           isSubagent, recency, checkpointAt, value
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            header.composer_id,
            header.workspace_id,
            header.created_at,
            header.last_updated_at,
            header.is_archived,
            header.is_subagent,
            header.recency,
            header.checkpoint_at,
            header.value,
        ],
    )?;
    Ok(())
}

fn read_payload_header(
    connection: &Connection,
) -> Result<Option<CursorHeaderRow>, QuarantineError> {
    connection
        .query_row(
            "SELECT composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent, \
                    recency, checkpointAt, value FROM header_rows LIMIT 1",
            [],
            |row| {
                Ok(CursorHeaderRow {
                    composer_id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    created_at: row.get(2)?,
                    last_updated_at: row.get(3)?,
                    is_archived: row.get(4)?,
                    is_subagent: row.get(5)?,
                    recency: row.get(6)?,
                    checkpoint_at: row.get(7)?,
                    value: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn insert_cursor_header(
    transaction: &Transaction<'_>,
    header: &CursorHeaderRow,
) -> Result<(), QuarantineError> {
    transaction.execute(
        "INSERT INTO composerHeaders(
           composerId, workspaceId, createdAt, lastUpdatedAt, isArchived,
           isSubagent, recency, checkpointAt, value
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            header.composer_id,
            header.workspace_id,
            header.created_at,
            header.last_updated_at,
            header.is_archived,
            header.is_subagent,
            header.recency,
            header.checkpoint_at,
            header.value,
        ],
    )?;
    Ok(())
}

fn encode_sql_value(value: &SqlValue) -> (i64, Vec<u8>) {
    match value {
        SqlValue::Null => (0, Vec::new()),
        SqlValue::Integer(value) => (1, value.to_le_bytes().to_vec()),
        SqlValue::Real(value) => (2, value.to_le_bytes().to_vec()),
        SqlValue::Text(value) => (3, value.as_bytes().to_vec()),
        SqlValue::Blob(value) => (4, value.clone()),
    }
}

fn decode_sql_value(value_type: i64, bytes: Vec<u8>) -> Result<SqlValue, QuarantineError> {
    match value_type {
        0 => Ok(SqlValue::Null),
        1 if bytes.len() == 8 => Ok(SqlValue::Integer(i64::from_le_bytes(
            bytes.try_into().expect("length checked"),
        ))),
        2 if bytes.len() == 8 => Ok(SqlValue::Real(f64::from_le_bytes(
            bytes.try_into().expect("length checked"),
        ))),
        3 => Ok(SqlValue::Text(
            String::from_utf8(bytes).map_err(|_| QuarantineError::HashMismatch)?,
        )),
        4 => Ok(SqlValue::Blob(bytes)),
        _ => Err(QuarantineError::HashMismatch),
    }
}

fn payload_keys(connection: &Connection) -> Result<Vec<String>, QuarantineError> {
    let mut statement = connection.prepare("SELECT key FROM kv_rows ORDER BY key")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn payload_value(connection: &Connection, key: &str) -> Result<SqlValue, QuarantineError> {
    let (value_type, bytes): (i64, Vec<u8>) = connection.query_row(
        "SELECT value_type, value FROM kv_rows WHERE key = ?1",
        params![key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    decode_sql_value(value_type, bytes)
}

fn copy_transcript_into_payload(
    transaction: &Transaction<'_>,
    transcript_dir: &Path,
    storage: &CursorSessionStorage,
    session_id: &str,
) -> Result<(), QuarantineError> {
    validate_cursor_transcript_dir(transcript_dir, storage, session_id)?;
    for entry in WalkDir::new(transcript_dir).follow_links(false) {
        let entry = entry.map_err(|error| io::Error::other(error.to_string()))?;
        if entry.file_type().is_symlink() {
            return Err(QuarantineError::LinkedPath);
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(transcript_dir)
            .map_err(|_| QuarantineError::OutsideRoot)?;
        let relative = relative_path_string(relative)?;
        let contents = fs::read(entry.path())?;
        let hash = blake3::hash(&contents).to_hex().to_string();
        transaction.execute(
            "INSERT INTO transcript_files(relative_path, contents, blake3) VALUES (?1, ?2, ?3)",
            params![relative, contents, hash],
        )?;
    }
    Ok(())
}

fn extract_transcript_from_payload(
    payload: &Connection,
    destination: &Path,
) -> Result<(), QuarantineError> {
    fs::create_dir(destination)?;
    let mut statement = payload.prepare(
        "SELECT relative_path, contents, blake3 FROM transcript_files ORDER BY relative_path",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let relative: String = row.get(0)?;
        let contents: Vec<u8> = row.get(1)?;
        let expected: String = row.get(2)?;
        if blake3::hash(&contents).to_hex().to_string() != expected {
            cleanup_staged_restore(Some(destination));
            return Err(QuarantineError::HashMismatch);
        }
        let relative = safe_relative_path(&relative)?;
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        file.write_all(&contents)?;
        file.sync_all()?;
    }
    Ok(())
}

fn validate_cursor_transcript_dir(
    transcript_dir: &Path,
    storage: &CursorSessionStorage,
    session_id: &str,
) -> Result<(), QuarantineError> {
    let projects_root = storage
        .projects_root
        .canonicalize()
        .map_err(|_| QuarantineError::OutsideRoot)?;
    let metadata = fs::symlink_metadata(transcript_dir)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(QuarantineError::LinkedPath);
    }
    let canonical = transcript_dir.canonicalize()?;
    if !canonical.starts_with(&projects_root)
        || canonical.file_name().and_then(|name| name.to_str()) != Some(session_id)
        || canonical
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("agent-transcripts")
    {
        return Err(QuarantineError::OutsideRoot);
    }
    Ok(())
}

fn validate_staged_cursor_dir(
    staged: &Path,
    cursor: &CursorManifestData,
) -> Result<(), QuarantineError> {
    let root = cursor
        .projects_root
        .canonicalize()
        .map_err(|_| QuarantineError::OutsideRoot)?;
    let metadata = fs::symlink_metadata(staged)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(QuarantineError::LinkedPath);
    }
    let canonical = staged.canonicalize()?;
    let valid_name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".acc-quarantine-"));
    if !canonical.starts_with(root) || !valid_name {
        return Err(QuarantineError::OutsideRoot);
    }
    Ok(())
}

fn relative_path_string(path: &Path) -> Result<String, QuarantineError> {
    use std::path::Component;
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(QuarantineError::OutsideRoot);
    }
    let value = path.to_string_lossy().replace('\\', "/");
    if value.is_empty() {
        return Err(QuarantineError::OutsideRoot);
    }
    Ok(value)
}

fn safe_relative_path(value: &str) -> Result<PathBuf, QuarantineError> {
    let path = PathBuf::from(value.replace('/', std::path::MAIN_SEPARATOR_STR));
    relative_path_string(&path)?;
    Ok(path)
}

fn cleanup_staged_restore(path: Option<&Path>) {
    if let Some(path) = path
        && path.exists()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".acc-restore-"))
    {
        let _ = fs::remove_dir_all(path);
    }
}

fn file_fingerprint(path: &Path) -> Result<Fingerprint, QuarantineError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(QuarantineError::LinkedPath);
    }
    Ok(Fingerprint {
        len: metadata.len(),
        modified_millis: metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    })
}

pub fn restore(manifest: &Manifest, data_dir: &Path) -> Result<Manifest, QuarantineError> {
    if manifest.cursor.is_some() {
        return restore_cursor(manifest, data_dir, true);
    }
    if manifest.state == ManifestState::Restored {
        return Ok(manifest.clone());
    }
    if manifest.state == ManifestState::Purged {
        return Err(QuarantineError::PayloadPurged);
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
    if let Some(modified_millis) = manifest.source_modified_millis
        && let Some(modified) =
            UNIX_EPOCH.checked_add(std::time::Duration::from_millis(modified_millis))
    {
        OpenOptions::new()
            .write(true)
            .open(&manifest.source)?
            .set_times(FileTimes::new().set_modified(modified))?;
    }
    let mut updated = manifest.clone();
    updated.state = ManifestState::Restored;
    updated.restored_at = Some(Utc::now());
    write_manifest_atomic(&operation_dir, &updated)?;
    Ok(updated)
}

pub fn purge(manifest: &Manifest, data_dir: &Path) -> Result<Manifest, QuarantineError> {
    if !matches!(
        manifest.state,
        ManifestState::Quarantined | ManifestState::Purging
    ) {
        return Err(QuarantineError::NotMutable);
    }
    if manifest.payload != "payload.jsonl" && manifest.payload != "cursor-session.sqlite" {
        return Err(QuarantineError::OutsideRoot);
    }
    let quarantine_root = data_dir.join("quarantine").canonicalize()?;
    let operation_dir = quarantine_root.join(manifest.operation_id.to_string());
    let canonical_operation = operation_dir.canonicalize()?;
    if !canonical_operation.starts_with(&quarantine_root) {
        return Err(QuarantineError::OutsideRoot);
    }
    let payload = canonical_operation.join(&manifest.payload);

    let mut updated = manifest.clone();
    updated.state = ManifestState::Purging;
    write_manifest_atomic(&canonical_operation, &updated)?;

    if let Some(cursor) = &manifest.cursor
        && let Some(staged) = cursor
            .staged_transcript_dir
            .as_ref()
            .filter(|path| path.exists())
    {
        validate_staged_cursor_dir(staged, cursor)?;
        fs::remove_dir_all(staged)?;
        if let Some(cursor) = &mut updated.cursor {
            cursor.staged_transcript_dir = None;
        }
    }

    if payload.exists() {
        let metadata = fs::symlink_metadata(&payload)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(QuarantineError::LinkedPath);
        }
        if hash_file(&payload)? != manifest.blake3 {
            return Err(QuarantineError::HashMismatch);
        }
        fs::remove_file(&payload)?;
    }
    updated.state = ManifestState::Purged;
    write_manifest_atomic(&canonical_operation, &updated)?;
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
    use agent_chat_cleaner_core::{SafetyState, SessionStorage};
    use std::time::{Duration, SystemTime};

    fn fixture_record(root: &Path) -> SessionRecord {
        fs::create_dir_all(root.join("project")).unwrap();
        let source = root.join("project/session.jsonl");
        fs::write(&source, b"fixture only\n").unwrap();
        File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(3600)))
            .unwrap();
        let metadata = fs::metadata(&source).unwrap();
        let modified = metadata.modified().unwrap();
        SessionRecord {
            id: "11111111-2222-4333-8444-555555555555".to_owned(),
            title: "fixture title".to_owned(),
            agent: AgentKind::Codex,
            project_label: "fixture".to_owned(),
            project_path: Some(root.join("project")),
            pinned: false,
            source,
            allowed_root: root.canonicalize().unwrap(),
            bytes: metadata.len(),
            modified,
            fingerprint: Fingerprint {
                len: metadata.len(),
                modified_millis: modified.duration_since(UNIX_EPOCH).unwrap().as_millis(),
            },
            safety: SafetyState::SupportedIdle,
            exclusive_file: true,
            storage: SessionStorage::ExclusiveFile,
        }
    }

    fn cursor_fixture(temp: &Path) -> (SessionRecord, PathBuf, PathBuf) {
        let projects = temp.join("projects");
        let global = temp.join("globalStorage");
        fs::create_dir_all(&global).unwrap();
        let id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        let neighbor_id = "11111111-2222-4333-8444-555555555555";
        let transcript_dir = projects.join("demo/agent-transcripts").join(id);
        fs::create_dir_all(transcript_dir.join("subagents")).unwrap();
        fs::write(
            transcript_dir.join(format!("{id}.jsonl")),
            b"selected transcript\n",
        )
        .unwrap();
        fs::write(
            transcript_dir.join("subagents/helper.jsonl"),
            b"selected helper\n",
        )
        .unwrap();
        let database = global.join("state.vscdb");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=DELETE;
                 CREATE TABLE composerHeaders (
                   composerId TEXT PRIMARY KEY, workspaceId TEXT, createdAt INTEGER,
                   lastUpdatedAt INTEGER, isArchived INTEGER, isSubagent INTEGER,
                   recency INTEGER, checkpointAt INTEGER, value TEXT
                 );
                 CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
            )
            .unwrap();
        for session in [id, neighbor_id] {
            connection
                .execute(
                    "INSERT INTO composerHeaders VALUES (?1, 'workspace', 1, 2, 0, 0, 3, 4, ?2)",
                    params![session, format!("{{\"name\":\"{session}\"}}")],
                )
                .unwrap();
            for (key, value) in [
                (
                    format!("composerData:{session}"),
                    SqlValue::Text(format!("{{\"composerId\":\"{session}\"}}")),
                ),
                (
                    format!("bubbleId:{session}:bubble"),
                    SqlValue::Text(format!("{{\"session\":\"{session}\"}}")),
                ),
                (
                    format!("checkpointId:{session}:checkpoint"),
                    SqlValue::Blob(vec![1, 2, 3, 4]),
                ),
            ] {
                connection
                    .execute(
                        "INSERT INTO cursorDiskKV(key, value) VALUES (?1, ?2)",
                        params![key, value],
                    )
                    .unwrap();
            }
        }
        connection
            .execute(
                "INSERT INTO cursorDiskKV(key, value) VALUES ('agentKv:blob:shared', x'010203')",
                [],
            )
            .unwrap();
        drop(connection);
        let fingerprint = file_fingerprint(&database).unwrap();
        let record = SessionRecord {
            id: id.to_owned(),
            title: "Cursor fixture".to_owned(),
            agent: AgentKind::Cursor,
            project_label: "demo".to_owned(),
            project_path: Some(PathBuf::from("C:/work/demo")),
            pinned: false,
            source: database.clone(),
            allowed_root: global.clone(),
            bytes: 42,
            modified: SystemTime::now() - Duration::from_secs(3600),
            fingerprint,
            safety: SafetyState::SupportedIdle,
            exclusive_file: false,
            storage: SessionStorage::CursorSqlite(CursorSessionStorage {
                database: database.clone(),
                global_storage_root: global,
                projects_root: projects,
                transcript_dir: Some(transcript_dir),
                wal_fingerprint: None,
            }),
        };
        (record, database, temp.join("data"))
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
        assert_eq!(
            file_fingerprint(&record.source).unwrap().modified_millis,
            record.fingerprint.modified_millis
        );
        assert_eq!(fs::read(&neighbor).unwrap(), b"must remain\n");
    }

    #[test]
    fn restored_exclusive_session_can_be_deleted_again_immediately() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let data = temp.path().join("data");
        let record = fixture_record(&root);
        let original_modified = record.modified;
        let first = quarantine(&record, &data).unwrap();
        restore(&first, &data).unwrap();

        let mut restored_record = record.clone();
        restored_record.fingerprint = file_fingerprint(&record.source).unwrap();
        restored_record.modified = fs::metadata(&record.source).unwrap().modified().unwrap();
        assert_eq!(
            restored_record
                .modified
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            original_modified
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );
        let second = quarantine(&restored_record, &data).unwrap();
        assert_eq!(second.state, ManifestState::Quarantined);
        assert!(!record.source.exists());
    }

    #[test]
    fn codex_pin_added_after_scan_blocks_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let codex_root = temp.path().join(".codex");
        let sessions = codex_root.join("sessions");
        let data = temp.path().join("data");
        let record = fixture_record(&sessions);
        let state = Connection::open(codex_root.join("state_5.sqlite")).unwrap();
        state
            .execute_batch("CREATE TABLE threads (id TEXT, is_pinned INTEGER);")
            .unwrap();
        state
            .execute("INSERT INTO threads VALUES (?1, 1)", [&record.id])
            .unwrap();
        drop(state);

        assert!(matches!(
            quarantine(&record, &data),
            Err(QuarantineError::CodexPinned)
        ));
        assert!(record.source.exists());
        assert!(!data.join("quarantine").exists());
    }

    #[test]
    fn codex_desktop_pin_added_after_scan_blocks_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let codex_root = temp.path().join(".codex");
        let sessions = codex_root.join("sessions");
        let data = temp.path().join("data");
        let record = fixture_record(&sessions);
        fs::write(
            codex_root.join(".codex-global-state.json"),
            format!(r#"{{"pinned-thread-ids":["{}"]}}"#, record.id),
        )
        .unwrap();

        assert!(matches!(
            quarantine(&record, &data),
            Err(QuarantineError::CodexPinned)
        ));
        assert!(record.source.exists());
        assert!(!data.join("quarantine").exists());
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

    #[test]
    fn permanent_purge_removes_only_committed_fixture_payload() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let data = temp.path().join("data");
        let record = fixture_record(&root);
        let neighbor = root.join("project/neighbor.jsonl");
        fs::write(&neighbor, b"must remain\n").unwrap();
        let manifest = quarantine(&record, &data).unwrap();
        let payload = data
            .join("quarantine")
            .join(manifest.operation_id.to_string())
            .join("payload.jsonl");
        let purged = purge(&manifest, &data).unwrap();
        assert_eq!(purged.state, ManifestState::Purged);
        assert!(!payload.exists());
        assert_eq!(fs::read(&neighbor).unwrap(), b"must remain\n");
        assert!(matches!(
            restore(&purged, &data),
            Err(QuarantineError::PayloadPurged)
        ));
    }

    #[test]
    fn permanent_batch_delete_skips_recovery_for_exclusive_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let data = temp.path().join("data");
        let record = fixture_record(&root);
        let outcomes = delete_batch(
            std::slice::from_ref(&record),
            &data,
            DeleteMode::Permanent,
            false,
            |_| {},
        );
        assert!(outcomes[0].1.is_ok(), "{outcomes:?}");
        assert!(!record.source.exists());
        assert!(!data.join("quarantine").exists());
    }

    #[test]
    fn cursor_quarantine_and_restore_are_session_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let (record, database, data) = cursor_fixture(temp.path());
        let storage = match &record.storage {
            SessionStorage::CursorSqlite(storage) => storage.clone(),
            SessionStorage::ExclusiveFile => unreachable!(),
        };

        let manifest = quarantine_cursor(&record, &storage, &data, false).unwrap();
        assert_eq!(manifest.state, ManifestState::Quarantined);
        assert!(!storage.transcript_dir.as_ref().unwrap().exists());
        let connection = Connection::open(&database).unwrap();
        assert_eq!(
            cursor_session_keys(&connection, &record.id).unwrap(),
            Vec::<String>::new()
        );
        assert!(
            read_cursor_header(&connection, &record.id)
                .unwrap()
                .is_none()
        );
        assert!(
            read_cursor_header(&connection, "11111111-2222-4333-8444-555555555555")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM cursorDiskKV WHERE key = 'agentKv:blob:shared'",
                    [],
                    |row| row.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
            vec![1, 2, 3]
        );
        drop(connection);

        let restored = restore_cursor(&manifest, &data, false).unwrap();
        assert_eq!(restored.state, ManifestState::Restored);
        assert_eq!(
            fs::read(
                storage
                    .transcript_dir
                    .as_ref()
                    .unwrap()
                    .join(format!("{}.jsonl", record.id))
            )
            .unwrap(),
            b"selected transcript\n"
        );
        let connection = Connection::open(database).unwrap();
        assert_eq!(
            cursor_session_keys(&connection, &record.id).unwrap().len(),
            3
        );
        assert!(
            read_cursor_header(&connection, &record.id)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn cursor_purge_removes_only_recoverable_payload() {
        let temp = tempfile::tempdir().unwrap();
        let (record, database, data) = cursor_fixture(temp.path());
        let storage = match &record.storage {
            SessionStorage::CursorSqlite(storage) => storage.clone(),
            SessionStorage::ExclusiveFile => unreachable!(),
        };
        let manifest = quarantine_cursor(&record, &storage, &data, false).unwrap();
        let payload = data
            .join("quarantine")
            .join(manifest.operation_id.to_string())
            .join(CURSOR_PAYLOAD);
        assert!(payload.exists());
        let purged = purge(&manifest, &data).unwrap();
        assert_eq!(purged.state, ManifestState::Purged);
        assert!(!payload.exists());
        let connection = Connection::open(database).unwrap();
        assert!(
            read_cursor_header(&connection, "11111111-2222-4333-8444-555555555555")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn cursor_batch_refreshes_only_its_own_database_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let (record, database, data) = cursor_fixture(temp.path());
        let mut neighbor = record.clone();
        neighbor.id = "11111111-2222-4333-8444-555555555555".to_owned();
        neighbor.title = "Neighbor".to_owned();
        if let SessionStorage::CursorSqlite(storage) = &mut neighbor.storage {
            storage.transcript_dir = None;
        }

        let mut progress = Vec::new();
        let outcomes = delete_batch(
            &[record, neighbor],
            &data,
            DeleteMode::Recoverable,
            false,
            |update| progress.push(update),
        );
        assert_eq!(outcomes.len(), 2);
        assert!(
            outcomes.iter().all(|(_, result)| result.is_ok()),
            "{outcomes:?}"
        );
        let connection = Connection::open(database).unwrap();
        let remaining_headers: i64 = connection
            .query_row("SELECT count(*) FROM composerHeaders", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining_headers, 0);
        let shared: Vec<u8> = connection
            .query_row(
                "SELECT value FROM cursorDiskKV WHERE key = 'agentKv:blob:shared'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(shared, vec![1, 2, 3]);
        let finished = progress.last().unwrap();
        assert_eq!(finished.completed, 2);
        assert_eq!(finished.total, 2);
        assert_eq!(finished.phase, BatchPhase::Finished);
    }

    #[test]
    fn cursor_permanent_batch_keeps_other_sessions_and_discards_payload() {
        let temp = tempfile::tempdir().unwrap();
        let (record, database, data) = cursor_fixture(temp.path());
        let outcomes = delete_batch(
            std::slice::from_ref(&record),
            &data,
            DeleteMode::Permanent,
            false,
            |_| {},
        );
        assert!(outcomes[0].1.is_ok(), "{outcomes:?}");
        let manifests = list_manifests(&data);
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].state, ManifestState::Purged);
        let operation_dir = data
            .join("quarantine")
            .join(manifests[0].operation_id.to_string());
        assert!(!operation_dir.join(CURSOR_PAYLOAD).exists());
        let connection = Connection::open(database).unwrap();
        assert!(
            read_cursor_header(&connection, &record.id)
                .unwrap()
                .is_none()
        );
        assert!(
            read_cursor_header(&connection, "11111111-2222-4333-8444-555555555555")
                .unwrap()
                .is_some()
        );
        let shared: Vec<u8> = connection
            .query_row(
                "SELECT value FROM cursorDiskKV WHERE key = 'agentKv:blob:shared'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(shared, vec![1, 2, 3]);
    }

    #[test]
    fn cursor_pin_added_after_scan_blocks_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let (record, database, data) = cursor_fixture(temp.path());
        let storage = match &record.storage {
            SessionStorage::CursorSqlite(storage) => storage.clone(),
            SessionStorage::ExclusiveFile => unreachable!(),
        };
        let pin_database = storage
            .global_storage_root
            .parent()
            .unwrap()
            .join("workspaceStorage/workspace/state.vscdb");
        fs::create_dir_all(pin_database.parent().unwrap()).unwrap();
        let connection = Connection::open(pin_database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ItemTable(key, value) VALUES ('cursor/pinnedComposers', ?1)",
                [format!("[\"{}\"]", record.id)],
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            quarantine_cursor(&record, &storage, &data, false),
            Err(QuarantineError::CursorPinned)
        ));
        let connection = Connection::open(database).unwrap();
        assert!(
            read_cursor_header(&connection, &record.id)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn cursor_database_changed_after_scan_is_never_touched() {
        let temp = tempfile::tempdir().unwrap();
        let (record, database, data) = cursor_fixture(temp.path());
        let storage = match &record.storage {
            SessionStorage::CursorSqlite(storage) => storage.clone(),
            SessionStorage::ExclusiveFile => unreachable!(),
        };
        let connection = Connection::open(&database).unwrap();
        connection
            .execute(
                "UPDATE cursorDiskKV SET value = 'changed' WHERE key = ?1",
                params![format!("composerData:{}", record.id)],
            )
            .unwrap();
        drop(connection);
        assert!(matches!(
            quarantine_cursor(&record, &storage, &data, false),
            Err(QuarantineError::Changed)
        ));
        let connection = Connection::open(database).unwrap();
        assert!(
            read_cursor_header(&connection, &record.id)
                .unwrap()
                .is_some()
        );
    }
}
