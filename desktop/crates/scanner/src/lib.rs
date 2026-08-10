use agent_chat_cleaner_core::{
    AgentKind, CursorSessionStorage, Fingerprint, SafetyState, SessionRecord, SessionStorage,
    cursor_process_running,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::{DirEntry, WalkDir};

const ACTIVE_WINDOW: Duration = Duration::from_secs(10 * 60);
const MAX_METADATA_LINE: u64 = 1024 * 1024;
const MAX_TITLE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CODEX_GLOBAL_STATE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ScanRoot {
    pub agent: AgentKind,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ScanSummary {
    pub sessions: Vec<SessionRecord>,
    /// All regular files below the configured roots. This is deliberately
    /// separate from per-session bytes because agents such as Cursor keep
    /// project-level assets and indexes that cannot be assigned safely to one row.
    pub storage_bytes: u64,
}

/// Agent-specific discovery lives behind this adapter boundary. Search, selection,
/// quarantine, restore and purge continue to operate on the shared `SessionRecord`.
pub trait SessionAdapter: Sync {
    fn kind(&self) -> AgentKind;
    fn default_paths(&self, home: &Path) -> Vec<PathBuf>;
    fn identify(&self, root: &Path, path: &Path, first: &Value) -> Option<AdapterIdentity>;
    fn user_content<'a>(&self, value: &'a Value) -> Option<&'a Value>;
}

pub struct AdapterIdentity {
    pub id: String,
    pub id_matches_file: bool,
    pub project_label: String,
    pub project_path: Option<PathBuf>,
    pub exclusive_file: bool,
}

struct CodexAdapter;
struct ClaudeCodeAdapter;
struct CursorAdapter;

static CODEX_ADAPTER: CodexAdapter = CodexAdapter;
static CLAUDE_ADAPTER: ClaudeCodeAdapter = ClaudeCodeAdapter;
static CURSOR_ADAPTER: CursorAdapter = CursorAdapter;

fn adapter_for(agent: AgentKind) -> &'static dyn SessionAdapter {
    match agent {
        AgentKind::Codex => &CODEX_ADAPTER,
        AgentKind::ClaudeCode => &CLAUDE_ADAPTER,
        AgentKind::Cursor => &CURSOR_ADAPTER,
    }
}

impl SessionAdapter for CodexAdapter {
    fn kind(&self) -> AgentKind {
        AgentKind::Codex
    }

    fn default_paths(&self, home: &Path) -> Vec<PathBuf> {
        vec![
            home.join(".codex").join("sessions"),
            home.join(".codex").join("archived_sessions"),
        ]
    }

    fn identify(&self, _root: &Path, path: &Path, first: &Value) -> Option<AdapterIdentity> {
        if first.get("type")?.as_str()? != "session_meta" {
            return None;
        }
        let id = first.pointer("/payload/id")?.as_str()?.to_owned();
        let file_stem = path.file_stem()?.to_string_lossy();
        let project_path = first
            .pointer("/payload/cwd")
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.trim().is_empty())
            .map(PathBuf::from);
        let project_label = project_path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "default".to_owned());
        Some(AdapterIdentity {
            id_matches_file: file_stem.ends_with(&id),
            id,
            project_label,
            project_path,
            exclusive_file: true,
        })
    }

    fn user_content<'a>(&self, value: &'a Value) -> Option<&'a Value> {
        (value.get("type").and_then(Value::as_str) == Some("response_item")
            && value.pointer("/payload/type").and_then(Value::as_str) == Some("message")
            && value.pointer("/payload/role").and_then(Value::as_str) == Some("user"))
        .then(|| value.pointer("/payload/content"))
        .flatten()
    }
}

impl SessionAdapter for ClaudeCodeAdapter {
    fn kind(&self) -> AgentKind {
        AgentKind::ClaudeCode
    }

    fn default_paths(&self, home: &Path) -> Vec<PathBuf> {
        vec![home.join(".claude").join("projects")]
    }

    fn identify(&self, root: &Path, path: &Path, first: &Value) -> Option<AdapterIdentity> {
        let id = first.get("sessionId")?.as_str()?.to_owned();
        let file_stem = path.file_stem()?.to_string_lossy();
        Some(AdapterIdentity {
            id_matches_file: file_stem == id,
            id,
            project_label: project_from_path(root, path),
            project_path: None,
            exclusive_file: true,
        })
    }

    fn user_content<'a>(&self, value: &'a Value) -> Option<&'a Value> {
        let is_user = value.get("type").and_then(Value::as_str) == Some("user")
            || value.get("role").and_then(Value::as_str) == Some("user")
            || value.pointer("/message/role").and_then(Value::as_str) == Some("user");
        is_user
            .then(|| {
                value
                    .pointer("/message/content")
                    .or_else(|| value.get("content"))
            })
            .flatten()
    }
}

impl SessionAdapter for CursorAdapter {
    fn kind(&self) -> AgentKind {
        AgentKind::Cursor
    }

    fn default_paths(&self, home: &Path) -> Vec<PathBuf> {
        vec![home.join(".cursor").join("projects")]
    }

    fn identify(&self, root: &Path, path: &Path, _first: &Value) -> Option<AdapterIdentity> {
        let file_stem = path.file_stem()?.to_string_lossy();
        let parent = path.parent()?.file_name()?.to_string_lossy();
        if parent != file_stem
            || !path
                .components()
                .any(|part| part.as_os_str() == "agent-transcripts")
        {
            return None;
        }
        Some(AdapterIdentity {
            id: file_stem.to_string(),
            id_matches_file: true,
            project_label: project_from_path(root, path),
            project_path: None,
            // Cursor also maintains a shared SQLite index; transcript-only writes stay disabled.
            exclusive_file: false,
        })
    }

    fn user_content<'a>(&self, value: &'a Value) -> Option<&'a Value> {
        (value.get("role").and_then(Value::as_str) == Some("user"))
            .then(|| value.pointer("/message/content"))
            .flatten()
    }
}

fn project_from_path(root: &Path, path: &Path) -> String {
    path.parent()
        .and_then(|parent| parent.strip_prefix(root).ok())
        .and_then(|relative| relative.components().next())
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| "default".to_owned())
}

pub fn default_roots() -> Vec<ScanRoot> {
    AgentKind::ALL
        .into_iter()
        .flat_map(default_roots_for)
        .collect()
}

pub fn default_roots_for(agent: AgentKind) -> Vec<ScanRoot> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let adapter = adapter_for(agent);
    debug_assert_eq!(adapter.kind(), agent);
    adapter
        .default_paths(&home)
        .into_iter()
        .map(|path| ScanRoot { agent, path })
        .filter(|root| root.path.is_dir())
        .collect()
}

pub fn scan_default_roots() -> Vec<SessionRecord> {
    scan_roots(&default_roots())
}

pub fn scan_default_agent(agent: AgentKind) -> Vec<SessionRecord> {
    scan_default_agent_summary(agent).sessions
}

pub fn scan_default_agent_summary(agent: AgentKind) -> ScanSummary {
    match agent {
        AgentKind::Cursor => scan_default_cursor(),
        AgentKind::Codex => scan_default_codex(),
        AgentKind::ClaudeCode => scan_default_claude(),
    }
}

fn scan_default_codex() -> ScanSummary {
    let Some(home) = dirs::home_dir() else {
        return ScanSummary {
            sessions: Vec::new(),
            storage_bytes: 0,
        };
    };
    scan_codex_root(&home.join(".codex"))
}

fn scan_codex_root(codex_root: &Path) -> ScanSummary {
    let roots = ["sessions", "archived_sessions"]
        .into_iter()
        .map(|folder| ScanRoot {
            agent: AgentKind::Codex,
            path: codex_root.join(folder),
        })
        .filter(|root| root.path.is_dir())
        .collect::<Vec<_>>();
    let mut summary = scan_roots_with_summary(&roots);
    let mut pinned = read_codex_pinned_ids(codex_root);
    pinned.extend(read_codex_desktop_pinned_ids(codex_root));
    let titles = read_codex_session_titles(codex_root);
    for record in &mut summary.sessions {
        record.pinned = pinned.contains(&record.id);
        if let Some(title) = titles.get(&record.id) {
            record.title.clone_from(title);
        }
    }
    summary
}

fn scan_default_claude() -> ScanSummary {
    scan_roots_with_summary(&default_roots_for(AgentKind::ClaudeCode))
}

pub fn scan_roots_with_summary(roots: &[ScanRoot]) -> ScanSummary {
    ScanSummary {
        sessions: scan_roots(roots),
        storage_bytes: roots.iter().map(root_storage_bytes).sum(),
    }
}

pub fn scan_roots(roots: &[ScanRoot]) -> Vec<SessionRecord> {
    let mut found = Vec::new();
    for root in roots {
        let Ok(canonical_root) = root.path.canonicalize() else {
            continue;
        };
        for entry in WalkDir::new(&canonical_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(not_subagent_dir)
            .filter_map(Result::ok)
        {
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            if entry.path().extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(record) = inspect_file(root.agent, &canonical_root, entry.path()) {
                found.push(record);
            }
        }
    }
    found.sort_by_key(|item| std::cmp::Reverse(item.modified));
    found
}

fn not_subagent_dir(entry: &DirEntry) -> bool {
    entry.file_name().to_string_lossy().to_ascii_lowercase() != "subagents"
}

fn inspect_file(agent: AgentKind, root: &Path, path: &Path) -> Option<SessionRecord> {
    let canonical = path.canonicalize().ok()?;
    if !canonical.starts_with(root) {
        return None;
    }
    let metadata = fs::symlink_metadata(&canonical).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
    let first = read_first_line(&canonical).ok()?;
    let json: Value = serde_json::from_str(&first).ok()?;
    let adapter = adapter_for(agent);
    let mut identity = adapter.identify(root, &canonical, &json)?;
    if agent == AgentKind::ClaudeCode
        && let Some(project_path) = read_claude_project_path(&canonical)
    {
        if let Some(label) = project_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|label| !label.is_empty())
        {
            identity.project_label = label;
        }
        identity.project_path = Some(project_path);
    }
    let id = identity.id;
    let id_matches = identity.id_matches_file;
    let looks_like_uuid = id.len() == 36 && id.chars().filter(|c| *c == '-').count() == 4;
    let recent = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default()
        < ACTIVE_WINDOW;
    let safety = if !id_matches || !looks_like_uuid {
        SafetyState::UnknownFormat
    } else if recent {
        SafetyState::Active
    } else {
        SafetyState::SupportedIdle
    };
    let title_metadata = read_title_metadata(&canonical, adapter);
    let title = title_metadata
        .as_ref()
        .map(|metadata| metadata.title.clone())
        .unwrap_or_else(|| format!("未命名会话 {}", id.get(..8).unwrap_or(&id)));
    let bytes = if agent == AgentKind::Cursor {
        canonical
            .parent()
            .map(directory_storage_bytes)
            .unwrap_or(metadata.len())
    } else {
        metadata.len()
    };
    Some(SessionRecord {
        id,
        title,
        agent,
        project_label: identity.project_label,
        project_path: identity.project_path,
        // Claude Code does not expose a normal-session pin flag. A custom title is
        // created by the user's explicit rename action, so treat it as the
        // equivalent keep signal and protect the transcript from deletion.
        pinned: agent == AgentKind::ClaudeCode
            && title_metadata.is_some_and(|metadata| metadata.custom),
        source: canonical,
        allowed_root: root.to_owned(),
        bytes,
        modified,
        fingerprint: fingerprint(&metadata, modified),
        safety,
        exclusive_file: identity.exclusive_file && id_matches && looks_like_uuid,
        storage: SessionStorage::ExclusiveFile,
    })
}

fn read_codex_pinned_ids(codex_root: &Path) -> HashSet<String> {
    let Some(database) = latest_codex_state_database(codex_root) else {
        return HashSet::new();
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(connection) = Connection::open_with_flags(database, flags) else {
        return HashSet::new();
    };
    if !table_has_columns(&connection, "threads", &["id", "is_pinned"]) {
        return HashSet::new();
    }
    let Ok(mut statement) =
        connection.prepare("SELECT id FROM threads WHERE coalesce(is_pinned, 0) != 0")
    else {
        return HashSet::new();
    };
    let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) else {
        return HashSet::new();
    };
    rows.filter_map(Result::ok).collect()
}

fn read_codex_desktop_pinned_ids(codex_root: &Path) -> HashSet<String> {
    let path = codex_root.join(".codex-global-state.json");
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return HashSet::new();
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_CODEX_GLOBAL_STATE_BYTES
    {
        return HashSet::new();
    }
    let Ok(file) = File::open(path) else {
        return HashSet::new();
    };
    let Ok(value) = serde_json::from_reader::<_, Value>(
        BufReader::new(file).take(MAX_CODEX_GLOBAL_STATE_BYTES),
    ) else {
        return HashSet::new();
    };
    value
        .get("pinned-thread-ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn read_codex_session_titles(codex_root: &Path) -> HashMap<String, String> {
    let Ok(file) = File::open(codex_root.join("session_index.jsonl")) else {
        return HashMap::new();
    };
    let mut latest: HashMap<String, (String, String)> = HashMap::new();
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        let Ok(read) = reader.read_line(&mut line) else {
            break;
        };
        if read == 0 {
            break;
        }
        if read as u64 > MAX_METADATA_LINE {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(title) = value
            .get("thread_name")
            .and_then(Value::as_str)
            .and_then(clean_title)
        else {
            continue;
        };
        let updated_at = value
            .get("updated_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let should_replace = latest
            .get(id)
            .is_none_or(|(current, _)| updated_at >= *current);
        if should_replace {
            latest.insert(id.to_owned(), (updated_at, title));
        }
    }
    latest
        .into_iter()
        .map(|(id, (_, title))| (id, title))
        .collect()
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

#[derive(Debug)]
struct CursorHeader {
    id: String,
    title: String,
    project_label: String,
    project_path: Option<PathBuf>,
    created: SystemTime,
    modified: SystemTime,
    metadata_bytes: u64,
}

fn scan_default_cursor() -> ScanSummary {
    let Some(home) = dirs::home_dir() else {
        return ScanSummary {
            sessions: Vec::new(),
            storage_bytes: 0,
        };
    };
    let projects_root = home.join(".cursor").join("projects");
    let config_root = dirs::config_dir()
        .unwrap_or_else(|| home.join("AppData").join("Roaming"))
        .join("Cursor")
        .join("User");
    let global_storage_root = config_root.join("globalStorage");
    let workspace_storage_root = config_root.join("workspaceStorage");
    scan_cursor_locations(
        projects_root,
        global_storage_root,
        workspace_storage_root,
        cursor_process_running(),
    )
}

fn scan_cursor_locations(
    projects_root: PathBuf,
    global_storage_root: PathBuf,
    workspace_storage_root: PathBuf,
    cursor_running: Option<bool>,
) -> ScanSummary {
    let database = global_storage_root.join("state.vscdb");
    let roots = projects_root
        .is_dir()
        .then(|| ScanRoot {
            agent: AgentKind::Cursor,
            path: projects_root.clone(),
        })
        .into_iter()
        .collect::<Vec<_>>();
    let mut transcripts = scan_roots(&roots);
    let storage_bytes = [
        &projects_root,
        &global_storage_root,
        &workspace_storage_root,
    ]
    .into_iter()
    .map(|path| directory_storage_bytes(path))
    .sum();

    let Ok(database_metadata) = fs::symlink_metadata(&database) else {
        for record in &mut transcripts {
            record.safety = SafetyState::UnknownFormat;
        }
        return ScanSummary {
            sessions: transcripts,
            storage_bytes,
        };
    };
    if database_metadata.file_type().is_symlink() || !database_metadata.is_file() {
        for record in &mut transcripts {
            record.safety = SafetyState::UnknownFormat;
        }
        return ScanSummary {
            sessions: transcripts,
            storage_bytes,
        };
    }
    let db_modified = database_metadata.modified().unwrap_or(UNIX_EPOCH);
    let db_fingerprint = fingerprint(&database_metadata, db_modified);
    let wal_fingerprint = fingerprint_from_path(&global_storage_root.join("state.vscdb-wal"));
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(connection) = Connection::open_with_flags(&database, flags) else {
        for record in &mut transcripts {
            record.safety = SafetyState::UnknownFormat;
        }
        return ScanSummary {
            sessions: transcripts,
            storage_bytes,
        };
    };
    if !cursor_schema_supported(&connection) {
        for record in &mut transcripts {
            record.safety = SafetyState::UnknownFormat;
        }
        return ScanSummary {
            sessions: transcripts,
            storage_bytes,
        };
    }

    let mut pinned_ids = read_cursor_pinned_ids(&connection);
    pinned_ids.extend(read_cursor_workspace_pinned_ids(&workspace_storage_root));
    let mut headers = read_cursor_headers(&connection);
    for header in &mut headers {
        header.metadata_bytes = header.metadata_bytes.saturating_add(cursor_value_bytes(
            &connection,
            &format!("composerData:{}", header.id),
        ));
    }
    let mut headers_by_id: HashMap<String, CursorHeader> = headers
        .into_iter()
        .map(|header| (header.id.clone(), header))
        .collect();
    let now = SystemTime::now();
    let mut sessions = Vec::with_capacity(transcripts.len() + headers_by_id.len());
    let mut seen = HashSet::new();

    for mut record in transcripts {
        let transcript_dir = record.source.parent().map(Path::to_owned);
        let header = headers_by_id.remove(&record.id);
        if let Some(header) = &header {
            if !header.title.is_empty() {
                record.title = header.title.clone();
            }
            if header.project_path.is_some() && !header.project_label.is_empty() {
                record.project_label = header.project_label.clone();
                record.project_path = header.project_path.clone();
            }
            record.modified = record.modified.max(header.modified);
            record.bytes = record.bytes.saturating_add(header.metadata_bytes);
        } else {
            record.bytes = record.bytes.saturating_add(cursor_value_bytes(
                &connection,
                &format!("composerData:{}", record.id),
            ));
        }
        record.source = database.clone();
        record.allowed_root = global_storage_root.clone();
        record.fingerprint = db_fingerprint.clone();
        record.exclusive_file = false;
        record.pinned = pinned_ids.contains(&record.id);
        record.safety = cursor_safety(cursor_running, record.modified, now);
        record.storage = SessionStorage::CursorSqlite(CursorSessionStorage {
            database: database.clone(),
            global_storage_root: global_storage_root.clone(),
            projects_root: projects_root.clone(),
            transcript_dir,
            wal_fingerprint: wal_fingerprint.clone(),
        });
        seen.insert(record.id.clone());
        sessions.push(record);
    }

    for header in headers_by_id.into_values() {
        if !seen.insert(header.id.clone()) {
            continue;
        }
        let modified = header.modified.max(header.created);
        sessions.push(SessionRecord {
            id: header.id.clone(),
            title: if header.title.is_empty() {
                format!("未命名会话 {}", header.id.get(..8).unwrap_or(&header.id))
            } else {
                header.title
            },
            agent: AgentKind::Cursor,
            project_label: if header.project_label.is_empty() {
                "Cursor 全局".to_owned()
            } else {
                header.project_label
            },
            project_path: header.project_path,
            pinned: pinned_ids.contains(&header.id),
            source: database.clone(),
            allowed_root: global_storage_root.clone(),
            bytes: header.metadata_bytes,
            modified,
            fingerprint: db_fingerprint.clone(),
            safety: cursor_safety(cursor_running, modified, now),
            exclusive_file: false,
            storage: SessionStorage::CursorSqlite(CursorSessionStorage {
                database: database.clone(),
                global_storage_root: global_storage_root.clone(),
                projects_root: projects_root.clone(),
                transcript_dir: None,
                wal_fingerprint: wal_fingerprint.clone(),
            }),
        });
    }
    sessions.sort_by_key(|item| std::cmp::Reverse(item.modified));
    ScanSummary {
        sessions,
        storage_bytes,
    }
}

fn cursor_value_bytes(connection: &Connection, key: &str) -> u64 {
    connection
        .query_row(
            "SELECT length(value) FROM cursorDiskKV WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .ok()
        .flatten()
        .flatten()
        .unwrap_or_default()
        .max(0) as u64
}

fn cursor_safety(running: Option<bool>, _modified: SystemTime, _now: SystemTime) -> SafetyState {
    match running {
        Some(true) => SafetyState::Active,
        None => SafetyState::UnknownFormat,
        Some(false) => SafetyState::SupportedIdle,
    }
}

fn cursor_schema_supported(connection: &Connection) -> bool {
    table_has_columns(
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
}

fn read_cursor_pinned_ids(connection: &Connection) -> HashSet<String> {
    if !table_has_columns(connection, "ItemTable", &["key", "value"]) {
        return HashSet::new();
    }
    let value = connection
        .query_row(
            "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = 'cursor/pinnedComposers'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten();
    value
        .as_deref()
        .and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn read_cursor_workspace_pinned_ids(workspace_storage_root: &Path) -> HashSet<String> {
    let Ok(entries) = fs::read_dir(workspace_storage_root) else {
        return HashSet::new();
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        })
        .filter_map(|entry| {
            let database = entry.path().join("state.vscdb");
            let metadata = fs::symlink_metadata(&database).ok()?;
            (!metadata.file_type().is_symlink() && metadata.is_file()).then_some(database)
        })
        .filter_map(|database| Connection::open_with_flags(database, flags).ok())
        .flat_map(|connection| read_cursor_pinned_ids(&connection))
        .collect()
}

fn table_has_columns(connection: &Connection, table: &str, required: &[&str]) -> bool {
    let sql = format!("PRAGMA table_info({table})");
    let Ok(mut statement) = connection.prepare(&sql) else {
        return false;
    };
    let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(1)) else {
        return false;
    };
    let columns: HashSet<String> = rows.filter_map(Result::ok).collect();
    required.iter().all(|column| columns.contains(*column))
}

fn read_cursor_headers(connection: &Connection) -> Vec<CursorHeader> {
    let Ok(mut statement) = connection.prepare(
        "SELECT composerId, workspaceId, createdAt, lastUpdatedAt, value, length(value) \
         FROM composerHeaders WHERE coalesce(isSubagent, 0) = 0",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        let id: String = row.get(0)?;
        let _workspace_id: Option<String> = row.get(1)?;
        let created: Option<i64> = row.get(2)?;
        let modified: Option<i64> = row.get(3)?;
        let value: Option<String> = row.get(4)?;
        let value_bytes: Option<i64> = row.get(5)?;
        let json = value
            .as_deref()
            .and_then(|text| serde_json::from_str::<Value>(text).ok());
        let title = json
            .as_ref()
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .and_then(clean_title)
            .unwrap_or_default();
        let (project_label, project_path) = json
            .as_ref()
            .and_then(cursor_header_project)
            .unwrap_or_else(|| ("Cursor 全局".to_owned(), None));
        Ok(CursorHeader {
            id,
            title,
            project_label,
            project_path,
            created: system_time_from_millis(created.unwrap_or_default()),
            modified: system_time_from_millis(modified.unwrap_or_default()),
            metadata_bytes: value_bytes.unwrap_or_default().max(0) as u64,
        })
    }) else {
        return Vec::new();
    };
    rows.filter_map(Result::ok).collect()
}

fn cursor_header_project(value: &Value) -> Option<(String, Option<PathBuf>)> {
    let workspace = value.get("workspaceIdentifier")?;
    for field in ["uri", "configPath"] {
        let Some(path) = workspace
            .get(field)
            .and_then(|uri| uri.get("fsPath"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let project_path = PathBuf::from(path);
        let label = if field == "configPath" {
            project_path.file_stem()
        } else {
            project_path.file_name()
        }
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|label| !label.is_empty());
        if let Some(label) = label {
            return Some((label, Some(project_path)));
        }
    }
    None
}

fn system_time_from_millis(value: i64) -> SystemTime {
    if value <= 0 {
        return UNIX_EPOCH;
    }
    UNIX_EPOCH
        .checked_add(Duration::from_millis(value as u64))
        .unwrap_or(UNIX_EPOCH)
}

fn fingerprint_from_path(path: &Path) -> Option<Fingerprint> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
    Some(fingerprint(&metadata, modified))
}

fn root_storage_bytes(root: &ScanRoot) -> u64 {
    let Ok(canonical_root) = root.path.canonicalize() else {
        return 0;
    };
    directory_storage_bytes(&canonical_root)
}

fn directory_storage_bytes(path: &Path) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && !entry.file_type().is_symlink())
        .filter_map(|entry| fs::symlink_metadata(entry.path()).ok())
        .fold(0_u64, |total, metadata| {
            total.saturating_add(metadata.len())
        })
}

struct TitleMetadata {
    title: String,
    custom: bool,
}

fn read_title_metadata(path: &Path, adapter: &dyn SessionAdapter) -> Option<TitleMetadata> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut first_user_title = None;
    let mut custom_title = None;
    let mut ai_title = None;
    let mut bytes_read = 0_u64;
    let mut lines_read = 0_usize;
    let scan_entire_file_for_custom_title = adapter.kind() == AgentKind::ClaudeCode;
    loop {
        line.clear();
        let read = reader.read_line(&mut line).ok()?;
        if read == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(read as u64);
        lines_read += 1;
        let within_prompt_budget = bytes_read <= MAX_TITLE_BYTES && lines_read <= 256;
        if !within_prompt_budget && !scan_entire_file_for_custom_title {
            break;
        }
        let might_have_session_title =
            line.contains("\"customTitle\"") || line.contains("\"aiTitle\"");
        if read as u64 > MAX_METADATA_LINE || (!within_prompt_budget && !might_have_session_title) {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(custom) = value.get("customTitle").and_then(Value::as_str)
            && let Some(title) = clean_title(custom)
        {
            custom_title = Some(title);
        }
        if let Some(generated) = value.get("aiTitle").and_then(Value::as_str)
            && let Some(title) = clean_title(generated)
        {
            ai_title = Some(title);
        }
        if within_prompt_budget && first_user_title.is_none() {
            first_user_title = adapter
                .user_content(&value)
                .and_then(first_text)
                .and_then(clean_title);
        }
    }
    custom_title
        .map(|title| TitleMetadata {
            title,
            custom: true,
        })
        .or_else(|| {
            ai_title.map(|title| TitleMetadata {
                title,
                custom: false,
            })
        })
        .or_else(|| {
            first_user_title.map(|title| TitleMetadata {
                title,
                custom: false,
            })
        })
}

fn read_claude_project_path(path: &Path) -> Option<PathBuf> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file).take(MAX_TITLE_BYTES);
    let mut line = String::new();
    for _ in 0..256 {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(cwd) = value
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.trim().is_empty())
        {
            return Some(PathBuf::from(cwd));
        }
    }
    None
}

fn first_text(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => Some(text),
        Value::Array(items) => items.iter().find_map(first_text),
        Value::Object(map) => map
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| map.get("content").and_then(first_text)),
        _ => None,
    }
}

fn clean_title(value: &str) -> Option<String> {
    let lower_value = value.to_ascii_lowercase();
    if [
        "<recommended_plugins>",
        "<environment_context>",
        "<codex_internal_context",
        "<in-app-browser-context",
        "<timestamp>",
        "# files mentioned by the user:",
    ]
    .iter()
    .any(|marker| lower_value.contains(marker))
    {
        return None;
    }
    let one_line = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !looks_like_internal_context(line))?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if one_line.is_empty() {
        return None;
    }
    let mut title: String = one_line.chars().take(160).collect();
    if one_line.chars().count() > 160 {
        title.push('…');
    }
    Some(title)
}

fn looks_like_internal_context(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    (lower.starts_with('<') && lower.ends_with('>'))
        || lower.starts_with("# agents.md")
        || lower.starts_with("## skills")
        || lower.starts_with("<timestamp>")
}

fn read_first_line(path: &Path) -> std::io::Result<String> {
    let file = File::open(path)?;
    let mut line = String::new();
    BufReader::new(file)
        .take(MAX_METADATA_LINE)
        .read_line(&mut line)?;
    Ok(line)
}

fn fingerprint(metadata: &fs::Metadata, modified: SystemTime) -> Fingerprint {
    Fingerprint {
        len: metadata.len(),
        modified_millis: modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn scans_only_valid_fixture_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sessions");
        fs::create_dir_all(root.join("2026/08/03")).unwrap();
        let id = "11111111-2222-4333-8444-555555555555";
        let path = root
            .join("2026/08/03")
            .join(format!("rollout-test-{id}.jsonl"));
        writeln!(
            File::create(&path).unwrap(),
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"C:/work/demo-project\"}}}}"
        )
        .unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"<recommended_plugins>\\nHere is a list of plugins\\n</recommended_plugins>\"}}]}}}}" ).unwrap();
        writeln!(file, "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"修复登录窗口\"}}]}}}}" ).unwrap();
        let result = scan_roots(&[ScanRoot {
            agent: AgentKind::Codex,
            path: root,
        }]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, id);
        assert_eq!(result[0].safety, SafetyState::Active);
        assert_eq!(result[0].title, "修复登录窗口");
        assert_eq!(result[0].project_label, "demo-project");
        assert_eq!(
            result[0].project_path.as_deref(),
            Some(Path::new("C:/work/demo-project"))
        );
    }

    #[test]
    fn claude_uses_recorded_cwd_instead_of_encoded_project_folder() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        let encoded = root.join("D--code-inner-todo-name");
        fs::create_dir_all(&encoded).unwrap();
        let id = "11111111-2222-4333-8444-555555555555";
        let path = encoded.join(format!("{id}.jsonl"));
        let mut file = File::create(path).unwrap();
        writeln!(file, "{{\"sessionId\":\"{id}\",\"type\":\"assistant\"}}").unwrap();
        writeln!(file, "{{\"sessionId\":\"{id}\",\"type\":\"user\",\"cwd\":\"D:/code/inner/todo_name\",\"message\":{{\"role\":\"user\",\"content\":\"真实项目路径\"}}}}").unwrap();

        let result = scan_roots(&[ScanRoot {
            agent: AgentKind::ClaudeCode,
            path: root,
        }]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].project_label, "todo_name");
        assert_eq!(
            result[0].project_path.as_deref(),
            Some(Path::new("D:/code/inner/todo_name"))
        );
    }

    #[test]
    fn claude_custom_title_wins_after_first_prompt_and_locks_session() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects/demo");
        fs::create_dir_all(&root).unwrap();
        let id = "11111111-2222-4333-8444-555555555555";
        let path = root.join(format!("{id}.jsonl"));
        let mut file = File::create(path).unwrap();
        writeln!(file, "{{\"sessionId\":\"{id}\",\"type\":\"assistant\"}}").unwrap();
        writeln!(file, "{{\"sessionId\":\"{id}\",\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"original prompt\"}}}}").unwrap();
        writeln!(
            file,
            "{{\"sessionId\":\"{id}\",\"type\":\"custom-title\",\"customTitle\":\"Old name\"}}"
        )
        .unwrap();
        for _ in 0..260 {
            writeln!(file, "{{\"sessionId\":\"{id}\",\"type\":\"system\"}}").unwrap();
        }
        writeln!(
            file,
            "{{\"sessionId\":\"{id}\",\"type\":\"custom-title\",\"customTitle\":\"Named session\"}}"
        )
        .unwrap();

        let result = scan_roots(&[ScanRoot {
            agent: AgentKind::ClaudeCode,
            path: temp.path().join("projects"),
        }]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Named session");
        assert!(result[0].pinned);
        assert!(!result[0].is_mutable());
    }

    #[test]
    fn claude_ai_title_wins_over_prompt_without_locking_session() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects/demo");
        fs::create_dir_all(&root).unwrap();
        let id = "11111111-2222-4333-8444-555555555555";
        let path = root.join(format!("{id}.jsonl"));
        let mut file = File::create(path).unwrap();
        writeln!(file, "{{\"sessionId\":\"{id}\",\"type\":\"assistant\"}}").unwrap();
        writeln!(file, "{{\"sessionId\":\"{id}\",\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"long original prompt\"}}}}").unwrap();
        writeln!(
            file,
            "{{\"sessionId\":\"{id}\",\"type\":\"ai-title\",\"aiTitle\":\"Generated title\"}}"
        )
        .unwrap();

        let result = scan_roots(&[ScanRoot {
            agent: AgentKind::ClaudeCode,
            path: temp.path().join("projects"),
        }]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Generated title");
        assert!(!result[0].pinned);
    }

    #[test]
    fn never_descends_into_subagents() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        let nested = root.join("demo/subagents");
        fs::create_dir_all(&nested).unwrap();
        let id = "11111111-2222-4333-8444-555555555555";
        writeln!(
            File::create(nested.join(format!("{id}.jsonl"))).unwrap(),
            "{{\"sessionId\":\"{id}\"}}"
        )
        .unwrap();
        assert!(
            scan_roots(&[ScanRoot {
                agent: AgentKind::ClaudeCode,
                path: root
            }])
            .is_empty()
        );
    }

    #[test]
    fn reads_codex_pins_from_latest_state_database() {
        let temp = tempfile::tempdir().unwrap();
        let older = Connection::open(temp.path().join("state_4.sqlite")).unwrap();
        older
            .execute_batch("CREATE TABLE threads (id TEXT, is_pinned INTEGER);")
            .unwrap();
        older
            .execute("INSERT INTO threads VALUES ('older', 1)", [])
            .unwrap();
        drop(older);

        let latest = Connection::open(temp.path().join("state_5.sqlite")).unwrap();
        latest
            .execute_batch("CREATE TABLE threads (id TEXT, is_pinned INTEGER);")
            .unwrap();
        latest
            .execute("INSERT INTO threads VALUES ('kept', 1), ('normal', 0)", [])
            .unwrap();
        drop(latest);

        assert_eq!(
            read_codex_pinned_ids(temp.path()),
            HashSet::from(["kept".to_owned()])
        );
    }

    #[test]
    fn reads_codex_desktop_pins_from_global_state() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(".codex-global-state.json"),
            r#"{"electron-persisted-atom-state":{"sidebar-collapsed-sections-v1":{"pinned":false}},"pinned-thread-ids":["desktop-kept"]}"#,
        )
        .unwrap();

        assert_eq!(
            read_codex_desktop_pinned_ids(temp.path()),
            HashSet::from(["desktop-kept".to_owned()])
        );
    }

    #[test]
    fn codex_uses_latest_sidebar_title_and_pin_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let codex_root = temp.path().join(".codex");
        let sessions = codex_root.join("sessions/2026/08/03");
        fs::create_dir_all(&sessions).unwrap();
        let id = "11111111-2222-4333-8444-555555555555";
        let transcript = sessions.join(format!("rollout-test-{id}.jsonl"));
        let mut file = File::create(transcript).unwrap();
        writeln!(file, "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"D:/code/codex-proxy\"}}}}").unwrap();
        writeln!(file, "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"long original prompt\"}}]}}}}").unwrap();
        let mut index = File::create(codex_root.join("session_index.jsonl")).unwrap();
        writeln!(index, "{{\"id\":\"{id}\",\"thread_name\":\"Old name\",\"updated_at\":\"2026-08-01T00:00:00Z\"}}").unwrap();
        writeln!(index, "{{\"id\":\"{id}\",\"thread_name\":\"修复 API 配置无法提交\",\"updated_at\":\"2026-08-03T00:00:00Z\"}}").unwrap();
        let state = Connection::open(codex_root.join("state_5.sqlite")).unwrap();
        state
            .execute_batch("CREATE TABLE threads (id TEXT, is_pinned INTEGER);")
            .unwrap();
        state
            .execute("INSERT INTO threads VALUES (?1, 0)", [id])
            .unwrap();
        drop(state);
        fs::write(
            codex_root.join(".codex-global-state.json"),
            format!(r#"{{"pinned-thread-ids":["{id}"]}}"#),
        )
        .unwrap();

        let summary = scan_codex_root(&codex_root);
        assert_eq!(summary.sessions.len(), 1);
        assert_eq!(summary.sessions[0].title, "修复 API 配置无法提交");
        assert!(summary.sessions[0].pinned);
        assert!(!summary.sessions[0].is_mutable());
    }

    #[test]
    fn discovers_cursor_primary_transcript_but_keeps_it_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        let id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        let folder = root.join("demo/agent-transcripts").join(id);
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("{id}.jsonl"));
        writeln!(File::create(&path).unwrap(), "{{\"role\":\"user\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"Cursor task title\"}}]}}}}" ).unwrap();
        let subagents = folder.join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        fs::write(subagents.join("helper.jsonl"), vec![b'x'; 2048]).unwrap();
        let result = scan_roots(&[ScanRoot {
            agent: AgentKind::Cursor,
            path: root,
        }]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Cursor task title");
        assert_eq!(result[0].bytes, fs::metadata(path).unwrap().len() + 2048);
        assert!(!result[0].is_mutable());
    }

    #[test]
    fn summary_includes_cursor_project_assets_without_assigning_them_to_a_session() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        let id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        let folder = root.join("demo/agent-transcripts").join(id);
        fs::create_dir_all(&folder).unwrap();
        let transcript = folder.join(format!("{id}.jsonl"));
        fs::write(
            &transcript,
            b"{\"role\":\"user\",\"message\":{\"content\":\"title\"}}\n",
        )
        .unwrap();
        let assets = root.join("demo/assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("image.png"), vec![0_u8; 4096]).unwrap();

        let summary = scan_roots_with_summary(&[ScanRoot {
            agent: AgentKind::Cursor,
            path: root,
        }]);

        assert_eq!(summary.sessions.len(), 1);
        assert_eq!(
            summary.sessions[0].bytes,
            fs::metadata(transcript).unwrap().len()
        );
        assert_eq!(summary.storage_bytes, summary.sessions[0].bytes + 4096);
    }

    #[test]
    fn cursor_sqlite_headers_merge_with_project_transcripts() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("projects");
        let global = temp.path().join("globalStorage");
        let workspace = temp.path().join("workspaceStorage");
        fs::create_dir_all(&global).unwrap();
        let id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        let db_only = "11111111-2222-4333-8444-555555555555";
        let subagent = "99999999-2222-4333-8444-555555555555";
        let transcript_dir = projects.join("demo/agent-transcripts").join(id);
        fs::create_dir_all(&transcript_dir).unwrap();
        fs::write(
            transcript_dir.join(format!("{id}.jsonl")),
            b"{\"role\":\"user\",\"message\":{\"content\":\"Transcript title\"}}\n",
        )
        .unwrap();
        let database = global.join("state.vscdb");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE composerHeaders (
                   composerId TEXT PRIMARY KEY, workspaceId TEXT, createdAt INTEGER,
                   lastUpdatedAt INTEGER, isArchived INTEGER, isSubagent INTEGER,
                   recency INTEGER, checkpointAt INTEGER, value TEXT
                 );
                 CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
                 CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
            )
            .unwrap();
        for (session, is_subagent, name) in [
            (id, 0, "Header title"),
            (db_only, 0, "Database only"),
            (subagent, 1, "Subagent"),
        ] {
            connection
                .execute(
                    "INSERT INTO composerHeaders VALUES (?1, 'workspace', 1, 2, 0, ?2, 3, 4, ?3)",
                    rusqlite::params![
                        session,
                        is_subagent,
                        format!(
                            "{{\"name\":\"{name}\",\"workspaceIdentifier\":{{\"uri\":{{\"fsPath\":\"C:/work/demo-project\"}}}}}}"
                        )
                    ],
                )
                .unwrap();
        }
        drop(connection);
        let workspace_database = workspace.join("fixture-workspace/state.vscdb");
        fs::create_dir_all(workspace_database.parent().unwrap()).unwrap();
        let workspace_connection = Connection::open(workspace_database).unwrap();
        workspace_connection
            .execute_batch(
                "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
            )
            .unwrap();
        workspace_connection
            .execute(
                "INSERT INTO ItemTable(key, value) VALUES ('cursor/pinnedComposers', ?1)",
                [format!("[\"{db_only}\"]")],
            )
            .unwrap();
        drop(workspace_connection);

        let summary = scan_cursor_locations(
            projects.clone(),
            global.clone(),
            workspace.clone(),
            Some(false),
        );
        assert_eq!(summary.sessions.len(), 2);
        let merged = summary
            .sessions
            .iter()
            .find(|record| record.id == id)
            .unwrap();
        assert_eq!(merged.title, "Header title");
        assert_eq!(merged.project_label, "demo-project");
        assert_eq!(
            merged.project_path.as_deref(),
            Some(Path::new("C:/work/demo-project"))
        );
        assert!(merged.is_mutable());
        assert!(matches!(merged.storage, SessionStorage::CursorSqlite(_)));
        let pinned = summary
            .sessions
            .iter()
            .find(|record| record.id == db_only)
            .unwrap();
        assert!(pinned.pinned);
        assert!(!pinned.is_mutable());
        assert!(!summary.sessions.iter().any(|record| record.id == subagent));

        let protected = scan_cursor_locations(projects, global, workspace, Some(true));
        assert!(protected.sessions.iter().all(|record| !record.is_mutable()));
        assert!(
            protected
                .sessions
                .iter()
                .all(|record| record.safety == SafetyState::Active)
        );
    }
}
