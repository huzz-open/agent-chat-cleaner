use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AgentKind {
    Codex,
    ClaudeCode,
}

impl AgentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
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
    pub agent: AgentKind,
    pub project_label: String,
    pub source: PathBuf,
    pub allowed_root: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
    pub fingerprint: Fingerprint,
    pub safety: SafetyState,
    pub exclusive_file: bool,
}

impl SessionRecord {
    pub fn is_mutable(&self) -> bool {
        self.safety == SafetyState::SupportedIdle && self.exclusive_file
    }

    pub fn short_id(&self) -> &str {
        self.id.get(..8).unwrap_or(&self.id)
    }
}
