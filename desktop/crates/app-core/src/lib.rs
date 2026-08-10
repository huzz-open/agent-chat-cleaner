use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum AgentKind {
    Codex,
    ClaudeCode,
    Cursor,
}

impl AgentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "ChatGPT / Codex",
            Self::ClaudeCode => "Claude Code",
            Self::Cursor => "Cursor",
        }
    }

    pub const ALL: [Self; 3] = [Self::Codex, Self::ClaudeCode, Self::Cursor];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Codex => "chatgpt-codex",
            Self::ClaudeCode => "claude-code",
            Self::Cursor => "cursor",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SafetyState {
    SupportedIdle,
    Active,
    UnknownFormat,
    OutsideAllowedRoot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Fingerprint {
    pub len: u64,
    pub modified_millis: u128,
}

#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub id: String,
    pub title: String,
    pub agent: AgentKind,
    pub project_label: String,
    /// The project/workspace path recorded by the agent. It may no longer exist,
    /// so it is intentionally not canonicalized during discovery.
    pub project_path: Option<PathBuf>,
    /// The source agent marks this conversation as pinned/kept.
    pub pinned: bool,
    pub source: PathBuf,
    pub allowed_root: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
    pub fingerprint: Fingerprint,
    pub safety: SafetyState,
    pub exclusive_file: bool,
    pub storage: SessionStorage,
}

impl SessionRecord {
    pub fn is_mutable(&self) -> bool {
        !self.pinned
            && self.safety == SafetyState::SupportedIdle
            && (self.exclusive_file || matches!(self.storage, SessionStorage::CursorSqlite(_)))
    }

    pub fn short_id(&self) -> &str {
        self.id.get(..8).unwrap_or(&self.id)
    }
}

#[derive(Clone, Debug)]
pub enum SessionStorage {
    ExclusiveFile,
    CursorSqlite(CursorSessionStorage),
}

#[derive(Clone, Debug)]
pub struct CursorSessionStorage {
    pub database: PathBuf,
    pub global_storage_root: PathBuf,
    pub projects_root: PathBuf,
    pub transcript_dir: Option<PathBuf>,
    pub wal_fingerprint: Option<Fingerprint>,
}

/// Returns `None` when the platform process query itself cannot be trusted.
/// Cursor database mutations fail closed in that case.
pub fn cursor_process_running() -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let output = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq Cursor.exe", "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
        Some(text.contains("\"cursor.exe\""))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("pgrep").args(["-x", "Cursor"]).output().ok()?;
        Some(output.status.success())
    }
}
