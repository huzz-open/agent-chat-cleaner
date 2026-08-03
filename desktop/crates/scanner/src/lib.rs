use agent_chat_cleaner_core::{AgentKind, Fingerprint, SafetyState, SessionRecord};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::{DirEntry, WalkDir};

const ACTIVE_WINDOW: Duration = Duration::from_secs(10 * 60);
const MAX_METADATA_LINE: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ScanRoot {
    pub agent: AgentKind,
    pub path: PathBuf,
}

pub fn default_roots() -> Vec<ScanRoot> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [
        ScanRoot {
            agent: AgentKind::Codex,
            path: home.join(".codex").join("sessions"),
        },
        ScanRoot {
            agent: AgentKind::Codex,
            path: home.join(".codex").join("archived_sessions"),
        },
        ScanRoot {
            agent: AgentKind::ClaudeCode,
            path: home.join(".claude").join("projects"),
        },
    ]
    .into_iter()
    .filter(|root| root.path.is_dir())
    .collect()
}

pub fn scan_default_roots() -> Vec<SessionRecord> {
    scan_roots(&default_roots())
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
    let file_stem = canonical.file_stem()?.to_string_lossy();
    let id = match agent {
        AgentKind::Codex => {
            if json.get("type")?.as_str()? != "session_meta" {
                return None;
            }
            json.pointer("/payload/id")?.as_str()?.to_owned()
        }
        AgentKind::ClaudeCode => json.get("sessionId")?.as_str()?.to_owned(),
    };
    let id_matches = match agent {
        AgentKind::Codex => file_stem.ends_with(&id),
        AgentKind::ClaudeCode => file_stem == id,
    };
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
    let project_label = canonical
        .parent()
        .and_then(|parent| parent.strip_prefix(root).ok())
        .and_then(|relative| relative.components().next())
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| "default".to_owned());
    Some(SessionRecord {
        id,
        agent,
        project_label,
        source: canonical,
        allowed_root: root.to_owned(),
        bytes: metadata.len(),
        modified,
        fingerprint: fingerprint(&metadata, modified),
        safety,
        exclusive_file: id_matches && looks_like_uuid,
    })
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
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\"}}}}"
        )
        .unwrap();
        let result = scan_roots(&[ScanRoot {
            agent: AgentKind::Codex,
            path: root,
        }]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, id);
        assert_eq!(result[0].safety, SafetyState::Active);
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
}
