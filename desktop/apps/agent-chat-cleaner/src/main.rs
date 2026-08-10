#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use agent_chat_cleaner_core::{
    AgentKind, CursorSessionStorage, Fingerprint, SafetyState, SessionRecord, SessionStorage,
};
use agent_chat_cleaner_quarantine::{
    BatchPhase, BatchProgress, DeleteMode, Manifest, ManifestState,
};
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText, Vec2};
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const COMMIT: &str = match option_env!("ACC_GIT_COMMIT") {
    Some(v) => v,
    None => "development",
};

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("Agent Chat Cleaner {VERSION} ({COMMIT})");
        return Ok(());
    }
    let safe_mode = args.iter().any(|arg| arg == "--safe-mode");
    let offscreen_preview = args.iter().any(|arg| arg == "--ui-preview-offscreen");
    let force_onboarding = args.iter().any(|arg| arg == "--first-run-preview");
    let about_preview = args.iter().any(|arg| arg == "--about-preview");
    let settings_preview = args.iter().any(|arg| arg == "--settings-preview");
    let progress_preview = args.iter().any(|arg| arg == "--progress-preview");
    let cursor_preview = args.iter().any(|arg| arg == "--cursor-preview");
    let screenshot_path = value_after(&args, "--ui-screenshot").map(PathBuf::from);
    let override_base = value_after(&args, "--data-dir").map(PathBuf::from);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Agent Chat Cleaner")
        .with_inner_size([1280.0, 760.0])
        .with_min_inner_size([980.0, 600.0]);
    if offscreen_preview {
        viewport = viewport.with_position([-3000.0, -3000.0]);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Agent Chat Cleaner",
        options,
        Box::new(move |cc| {
            let mut app = CleanerApp::new(
                cc,
                override_base,
                safe_mode,
                screenshot_path,
                force_onboarding,
            );
            app.about_open = about_preview;
            app.settings_open = settings_preview;
            if progress_preview {
                app.quarantine_running = true;
                app.delete_progress = Some(BatchProgress {
                    completed: 173,
                    total: 618,
                    phase: BatchPhase::BackingUp,
                    current_title: "Chat management desktop app".to_owned(),
                });
            }
            if cursor_preview {
                app.agent_filter = AgentKind::Cursor;
                app.start_scan(AgentKind::Cursor, false);
            }
            Ok(Box::new(app))
        }),
    )
}

fn value_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ViewMode {
    Existing,
    Deleted,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AppSettings {
    codex: PathBuf,
    claude: PathBuf,
    cursor: PathBuf,
    #[serde(default)]
    root_confirmed: bool,
    #[serde(default = "default_true")]
    keep_recovery_copies: bool,
}

fn default_true() -> bool {
    true
}

impl AppSettings {
    fn defaults(base: Option<PathBuf>) -> Self {
        let base = base.unwrap_or_else(default_storage_base);
        Self {
            codex: base.join(AgentKind::Codex.slug()),
            claude: base.join(AgentKind::ClaudeCode.slug()),
            cursor: base.join(AgentKind::Cursor.slug()),
            root_confirmed: false,
            keep_recovery_copies: true,
        }
    }

    fn from_root(root: &Path) -> Self {
        let mut settings = Self::defaults(Some(root.to_owned()));
        settings.root_confirmed = true;
        settings
    }

    fn dir(&self, agent: AgentKind) -> &Path {
        match agent {
            AgentKind::Codex => &self.codex,
            AgentKind::ClaudeCode => &self.claude,
            AgentKind::Cursor => &self.cursor,
        }
    }

    fn set_dir(&mut self, agent: AgentKind, value: PathBuf) {
        match agent {
            AgentKind::Codex => self.codex = value,
            AgentKind::ClaudeCode => self.claude = value,
            AgentKind::Cursor => self.cursor = value,
        }
    }
}

#[derive(Clone)]
struct StoredManifest {
    manifest: Manifest,
    data_dir: PathBuf,
}

struct AgentScanResult {
    agent: AgentKind,
    sessions: Vec<SessionRecord>,
    storage_bytes: u64,
}

struct QuarantineBatchResult {
    agent: AgentKind,
    recoverable: bool,
    entries: Vec<(String, Result<(), String>)>,
}

struct CleanerApp {
    view: ViewMode,
    agent_filter: AgentKind,
    sessions: Vec<SessionRecord>,
    selected: BTreeSet<String>,
    manifests: Vec<StoredManifest>,
    scan_tx: Sender<AgentScanResult>,
    scan_rx: Receiver<AgentScanResult>,
    quarantine_tx: Sender<QuarantineBatchResult>,
    quarantine_rx: Receiver<QuarantineBatchResult>,
    progress_tx: Sender<BatchProgress>,
    progress_rx: Receiver<BatchProgress>,
    delete_progress: Option<BatchProgress>,
    quarantine_running: bool,
    scanning_agents: HashSet<AgentKind>,
    scanned_agents: HashSet<AgentKind>,
    agent_storage_bytes: HashMap<AgentKind, u64>,
    scan_started_at: Instant,
    query: String,
    confirm_open: bool,
    purge_index: Option<usize>,
    safe_mode: bool,
    settings: AppSettings,
    settings_draft: AppSettings,
    settings_open: bool,
    onboarding_open: bool,
    onboarding_root: String,
    onboarding_chosen: bool,
    about_open: bool,
    status: String,
    screenshot_path: Option<PathBuf>,
    screenshot_requested: bool,
    delete_preview: bool,
}

impl CleanerApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        override_base: Option<PathBuf>,
        safe_mode: bool,
        screenshot_path: Option<PathBuf>,
        force_onboarding: bool,
    ) -> Self {
        install_chinese_font(&cc.egui_ctx);
        let saved = load_settings();
        let settings = override_base
            .clone()
            .map(|base| AppSettings::from_root(&base))
            .or(saved)
            .unwrap_or_else(|| AppSettings::defaults(None));
        let onboarding_open =
            force_onboarding || (override_base.is_none() && !settings.root_confirmed);
        let onboarding_root = common_storage_root(&settings).display().to_string();
        let manifests = load_manifests(&settings);
        let (scan_tx, scan_rx) = mpsc::channel();
        let (quarantine_tx, quarantine_rx) = mpsc::channel();
        let (progress_tx, progress_rx) = mpsc::channel();
        let mut app = Self {
            view: ViewMode::Existing,
            agent_filter: AgentKind::Codex,
            sessions: Vec::new(),
            selected: BTreeSet::new(),
            manifests,
            scan_tx,
            scan_rx,
            quarantine_tx,
            quarantine_rx,
            progress_tx,
            progress_rx,
            delete_progress: None,
            quarantine_running: false,
            scanning_agents: HashSet::new(),
            scanned_agents: HashSet::new(),
            agent_storage_bytes: HashMap::new(),
            scan_started_at: Instant::now(),
            query: String::new(),
            confirm_open: false,
            purge_index: None,
            safe_mode,
            settings_draft: settings.clone(),
            settings,
            settings_open: false,
            onboarding_open,
            onboarding_root,
            onboarding_chosen: false,
            about_open: false,
            status: if safe_mode {
                "安全模式：所有写入操作均已禁用"
            } else {
                "仅读取元数据；不会自动清理"
            }
            .to_owned(),
            screenshot_path,
            screenshot_requested: false,
            delete_preview: std::env::args().any(|arg| arg == "--delete-preview"),
        };
        app.start_scan(AgentKind::Codex, false);
        app
    }

    fn start_scan(&mut self, agent: AgentKind, force: bool) {
        if self.scanning_agents.contains(&agent) || (!force && self.scanned_agents.contains(&agent))
        {
            return;
        }
        self.scanning_agents.insert(agent);
        if force {
            self.scanned_agents.remove(&agent);
        }
        if agent == self.agent_filter {
            self.scan_started_at = Instant::now();
        }
        self.status = format!("正在只读扫描 {} 会话…", agent.label());
        let tx = self.scan_tx.clone();
        thread::spawn(move || {
            let summary = agent_chat_cleaner_scanner::scan_default_agent_summary(agent);
            let _ = tx.send(AgentScanResult {
                agent,
                sessions: summary.sessions,
                storage_bytes: summary.storage_bytes,
            });
        });
    }

    fn receive_scan(&mut self) {
        while let Ok(result) = self.scan_rx.try_recv() {
            let count = result.sessions.len();
            self.sessions.retain(|record| record.agent != result.agent);
            self.sessions.extend(result.sessions);
            self.sessions
                .sort_by_key(|item| std::cmp::Reverse(item.modified));
            self.selected
                .retain(|id| self.sessions.iter().any(|r| &r.id == id && r.is_mutable()));
            self.scanning_agents.remove(&result.agent);
            self.scanned_agents.insert(result.agent);
            self.agent_storage_bytes
                .insert(result.agent, result.storage_bytes);
            if self.delete_preview && result.agent == self.agent_filter {
                if let Some(record) = self
                    .sessions
                    .iter()
                    .find(|record| record.agent == result.agent && record.is_mutable())
                {
                    self.selected.insert(record.id.clone());
                    self.confirm_open = true;
                    self.scan_started_at = Instant::now();
                }
                self.delete_preview = false;
            }
            self.status = if result.agent == AgentKind::Cursor {
                format!(
                    "Cursor 扫描完成：识别到 {count} 个逻辑会话；运行中或关系未验证的记录已自动保护"
                )
            } else {
                format!(
                    "{} 扫描完成：识别到 {count} 个会话；活动或未知格式会话已自动保护",
                    result.agent.label()
                )
            };
        }
    }

    fn current_agent_scanning(&self) -> bool {
        self.scanning_agents.contains(&self.agent_filter)
    }

    fn ensure_current_agent_scanned(&mut self) {
        if self.view == ViewMode::Existing {
            self.start_scan(self.agent_filter, false);
        }
    }

    fn selected_records(&self) -> Vec<SessionRecord> {
        self.sessions
            .iter()
            .filter(|r| {
                r.agent == self.agent_filter && self.selected.contains(&r.id) && r.is_mutable()
            })
            .cloned()
            .collect()
    }

    fn refresh_manifests(&mut self) {
        self.manifests = load_manifests(&self.settings);
    }

    fn run_quarantine(&mut self) {
        if !self.settings.root_confirmed {
            self.onboarding_open = true;
            self.status = "请先确认隔离根目录".to_owned();
            return;
        }
        if self.safe_mode {
            self.status = "安全模式禁止删除操作".to_owned();
            return;
        }
        if self.quarantine_running {
            return;
        }
        let records = self.selected_records();
        if records.is_empty() {
            self.confirm_open = false;
            return;
        }
        let data_dir = self.settings.dir(self.agent_filter).to_owned();
        let agent = self.agent_filter;
        let tx = self.quarantine_tx.clone();
        let progress_tx = self.progress_tx.clone();
        let recoverable = self.settings.keep_recovery_copies;
        self.quarantine_running = true;
        self.delete_progress = Some(BatchProgress {
            completed: 0,
            total: records.len(),
            phase: BatchPhase::Checking,
            current_title: String::new(),
        });
        self.confirm_open = false;
        self.status = format!("正在处理 {} 个会话…", records.len());
        thread::spawn(move || {
            let mode = if recoverable {
                DeleteMode::Recoverable
            } else {
                DeleteMode::Permanent
            };
            let entries = agent_chat_cleaner_quarantine::delete_batch(
                &records,
                &data_dir,
                mode,
                true,
                |progress| {
                    let _ = progress_tx.send(progress);
                },
            )
            .into_iter()
            .map(|(id, result)| (id, result.map_err(|error| error.to_string())))
            .collect();
            let _ = tx.send(QuarantineBatchResult {
                agent,
                recoverable,
                entries,
            });
        });
    }

    fn receive_quarantine(&mut self) {
        while let Ok(progress) = self.progress_rx.try_recv() {
            self.delete_progress = Some(progress);
        }
        let Ok(result) = self.quarantine_rx.try_recv() else {
            return;
        };
        self.quarantine_running = false;
        self.delete_progress = None;
        let mut succeeded = 0usize;
        let mut succeeded_ids = HashSet::new();
        let mut errors = Vec::new();
        for (id, outcome) in result.entries {
            match outcome {
                Ok(()) => {
                    succeeded += 1;
                    self.selected.remove(&id);
                    succeeded_ids.insert(id);
                }
                Err(error) => errors.push(format!("{}: {error}", id.get(..8).unwrap_or(&id))),
            }
        }
        let released_exclusive_bytes =
            remove_quarantined_sessions(&mut self.sessions, result.agent, &succeeded_ids);
        if let Some(total) = self.agent_storage_bytes.get_mut(&result.agent) {
            *total = total.saturating_sub(released_exclusive_bytes);
        }
        if result.agent == AgentKind::Cursor && !succeeded_ids.is_empty() {
            self.refresh_cursor_fingerprints();
        }
        if result.recoverable {
            self.refresh_manifests();
        }
        let error_detail = if errors.len() > 3 {
            format!(
                "{}；另有 {} 项，请修正原因后重新扫描",
                errors
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("；"),
                errors.len() - 3
            )
        } else {
            errors.join("；")
        };
        self.status =
            if errors.is_empty() && result.agent == AgentKind::Cursor && result.recoverable {
                format!(
                    "已删除 {succeeded} 个会话，列表已更新；Cursor 占用统计可在需要时点击“扫描”校准"
                )
            } else if errors.is_empty() && result.agent == AgentKind::Cursor {
                format!("已永久删除 {succeeded} 个会话；Cursor 占用统计可在需要时点击“扫描”校准")
            } else if errors.is_empty() && !result.recoverable {
                format!("已永久删除 {succeeded} 个会话，列表已更新")
            } else if errors.is_empty() {
                format!("已删除 {succeeded} 个会话，列表已更新；可从“已删除”视图恢复")
            } else {
                let action = if result.recoverable {
                    "已删除"
                } else {
                    "已永久删除"
                };
                format!(
                    "{action} {succeeded} 个；{} 个被安全阻止：{}",
                    errors.len(),
                    error_detail
                )
            };
    }

    fn restore_at(&mut self, index: usize) {
        if self.quarantine_running {
            return;
        }
        if !self.settings.root_confirmed {
            self.onboarding_open = true;
            self.status = "请先确认隔离根目录".to_owned();
            return;
        }
        if self.safe_mode {
            self.status = "安全模式禁止恢复操作".to_owned();
            return;
        }
        let Some(stored) = self.manifests.get(index).cloned() else {
            return;
        };
        match agent_chat_cleaner_quarantine::restore(&stored.manifest, &stored.data_dir) {
            Ok(updated) => {
                self.manifests[index].manifest = updated.clone();
                if updated.agent == AgentKind::Cursor {
                    self.refresh_cursor_fingerprints();
                }
                if self.upsert_restored_session(&updated) {
                    self.status = if updated.agent == AgentKind::Cursor {
                        format!(
                            "会话 {} 已恢复，列表已更新；Cursor 占用统计可在需要时点击“扫描”校准",
                            stored.manifest.display_id()
                        )
                    } else {
                        format!(
                            "会话 {} 已恢复，列表已立即更新",
                            stored.manifest.display_id()
                        )
                    };
                } else {
                    self.status =
                        format!("会话 {} 已恢复；正在校验列表", stored.manifest.display_id());
                    self.start_scan(stored.manifest.agent, true);
                }
            }
            Err(error) => self.status = format!("恢复已停止：{error}"),
        }
    }

    fn refresh_cursor_fingerprints(&mut self) {
        let mut database_fingerprints: HashMap<PathBuf, Option<Fingerprint>> = HashMap::new();
        let mut wal_fingerprints: HashMap<PathBuf, Option<Fingerprint>> = HashMap::new();
        for record in &mut self.sessions {
            let SessionStorage::CursorSqlite(storage) = &mut record.storage else {
                continue;
            };
            let database_fingerprint = database_fingerprints
                .entry(storage.database.clone())
                .or_insert_with(|| file_fingerprint(&storage.database));
            if let Some(fingerprint) = database_fingerprint {
                record.fingerprint = fingerprint.clone();
            }
            let wal = storage.global_storage_root.join("state.vscdb-wal");
            storage.wal_fingerprint = wal_fingerprints
                .entry(wal.clone())
                .or_insert_with(|| wal.exists().then(|| file_fingerprint(&wal)).flatten())
                .clone();
        }
    }

    fn upsert_restored_session(&mut self, manifest: &Manifest) -> bool {
        let (source, allowed_root, storage, exclusive_file, bytes) =
            if let Some(cursor) = &manifest.cursor {
                let wal = cursor.global_storage_root.join("state.vscdb-wal");
                (
                    cursor.database.clone(),
                    cursor.global_storage_root.clone(),
                    SessionStorage::CursorSqlite(CursorSessionStorage {
                        database: cursor.database.clone(),
                        global_storage_root: cursor.global_storage_root.clone(),
                        projects_root: cursor.projects_root.clone(),
                        transcript_dir: cursor.transcript_dir.clone(),
                        wal_fingerprint: wal.exists().then(|| file_fingerprint(&wal)).flatten(),
                    }),
                    false,
                    manifest.bytes,
                )
            } else {
                (
                    manifest.source.clone(),
                    manifest.allowed_root.clone(),
                    SessionStorage::ExclusiveFile,
                    true,
                    manifest.bytes,
                )
            };
        let Some(fingerprint) = file_fingerprint(&source) else {
            return false;
        };
        let modified = manifest
            .session_modified_millis
            .and_then(|millis| UNIX_EPOCH.checked_add(Duration::from_millis(millis)))
            .or_else(|| {
                fs::metadata(&source)
                    .and_then(|metadata| metadata.modified())
                    .ok()
            })
            .unwrap_or(UNIX_EPOCH);
        let actual_bytes = if exclusive_file {
            fingerprint.len
        } else {
            bytes
        };
        let record = SessionRecord {
            id: manifest.session_id.clone(),
            title: manifest.title.clone(),
            agent: manifest.agent,
            project_label: manifest.project_label.clone(),
            project_path: manifest.project_path.clone(),
            pinned: manifest.pinned,
            source,
            allowed_root,
            bytes: actual_bytes,
            modified,
            fingerprint,
            safety: SafetyState::SupportedIdle,
            exclusive_file,
            storage,
        };
        let already_present = self
            .sessions
            .iter()
            .any(|existing| existing.agent == record.agent && existing.id == record.id);
        self.sessions
            .retain(|existing| existing.agent != record.agent || existing.id != record.id);
        if !already_present
            && matches!(&record.storage, SessionStorage::ExclusiveFile)
            && let Some(total) = self.agent_storage_bytes.get_mut(&record.agent)
        {
            *total = total.saturating_add(record.bytes);
        }
        self.sessions.push(record);
        self.sessions
            .sort_by_key(|item| std::cmp::Reverse(item.modified));
        true
    }

    fn purge_at(&mut self, index: usize) {
        if self.quarantine_running {
            return;
        }
        if !self.settings.root_confirmed {
            self.onboarding_open = true;
            self.status = "请先确认隔离根目录".to_owned();
            return;
        }
        if self.safe_mode {
            self.status = "安全模式禁止永久删除".to_owned();
            return;
        }
        let Some(stored) = self.manifests.get(index).cloned() else {
            return;
        };
        match agent_chat_cleaner_quarantine::purge(&stored.manifest, &stored.data_dir) {
            Ok(updated) => {
                self.manifests[index].manifest = updated;
                self.status = format!("会话 {} 的隔离内容已永久删除", stored.manifest.display_id());
            }
            Err(error) => self.status = format!("永久删除已停止：{error}"),
        }
        self.purge_index = None;
    }

    fn top_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("top_menu").show(root, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("文件", |ui| {
                    if ui
                        .add_enabled(
                            !self.current_agent_scanning(),
                            egui::Button::new("重新扫描当前来源").shortcut_text("F5"),
                        )
                        .clicked()
                    {
                        self.start_scan(self.agent_filter, true);
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add(egui::Button::new("退出").shortcut_text("Alt+F4"))
                        .clicked()
                    {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("选项", |ui| {
                    if ui.button("删除与存储设置…").clicked() {
                        self.settings_draft = self.settings.clone();
                        self.settings_open = true;
                        ui.close();
                    }
                });
                ui.menu_button("帮助", |ui| {
                    if ui.button("关于 Agent Chat Cleaner").clicked() {
                        self.about_open = true;
                        ui.close();
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("Agent Chat Cleaner  {VERSION}")).strong());
                });
            });
            ui.separator();
            egui::Frame::new()
                .fill(Color32::from_rgb(246, 248, 251))
                .inner_margin(egui::Margin::symmetric(8, 7))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().interact_size.y = 28.0;
                        ui.allocate_ui_with_layout(
                            egui::vec2(70.0, 28.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.label("会话来源：");
                            },
                        );
                        let old_agent = self.agent_filter;
                        egui::ComboBox::from_id_salt("agent_selector")
                            .selected_text(self.agent_filter.label())
                            .width(170.0)
                            .show_ui(ui, |ui| {
                                for agent in AgentKind::ALL {
                                    let count = self
                                        .sessions
                                        .iter()
                                        .filter(|item| item.agent == agent)
                                        .count();
                                    let suffix = if self.scanned_agents.contains(&agent) {
                                        format!("{count}")
                                    } else if self.scanning_agents.contains(&agent) {
                                        "扫描中".to_owned()
                                    } else {
                                        "未扫描".to_owned()
                                    };
                                    ui.selectable_value(
                                        &mut self.agent_filter,
                                        agent,
                                        format!("{}  ({suffix})", agent.label()),
                                    );
                                }
                            });
                        if old_agent != self.agent_filter {
                            self.selected.clear();
                            self.scan_started_at = Instant::now();
                            self.ensure_current_agent_scanned();
                        }
                        let scanning = self.current_agent_scanning();
                        if ui
                            .add_enabled(
                                !scanning,
                                egui::Button::new(if scanning { "扫描中…" } else { "扫描" })
                                    .min_size(egui::vec2(68.0, 28.0)),
                            )
                            .clicked()
                        {
                            self.start_scan(self.agent_filter, true);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button("删除设置…")
                                .on_hover_text("设置恢复副本及每个 Agent 的存储目录")
                                .clicked()
                            {
                                self.settings_draft = self.settings.clone();
                                self.settings_open = true;
                            }
                        });
                    })
                });
        });
    }

    fn filters(
        &mut self,
        ui: &mut egui::Ui,
        count: usize,
        bytes: u64,
        selectable_ids: &[String],
    ) -> bool {
        let mut action_clicked = false;
        let existing_count = self
            .sessions
            .iter()
            .filter(|item| item.agent == self.agent_filter)
            .count();
        let deleted_count = self
            .manifests
            .iter()
            .filter(|item| {
                item.manifest.agent == self.agent_filter
                    && matches!(
                        item.manifest.state,
                        ManifestState::Preparing
                            | ManifestState::Quarantined
                            | ManifestState::Purging
                    )
            })
            .count();
        egui::Frame::new()
            .fill(Color32::WHITE)
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(218, 222, 228)))
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let old_view = self.view;
                    ui.selectable_value(
                        &mut self.view,
                        ViewMode::Existing,
                        format!("现有会话  {existing_count}"),
                    );
                    ui.selectable_value(
                        &mut self.view,
                        ViewMode::Deleted,
                        format!("已删除  {deleted_count}"),
                    );
                    if old_view != self.view {
                        self.selected.clear();
                        if self.view == ViewMode::Deleted {
                            self.refresh_manifests();
                        } else {
                            self.ensure_current_agent_scanned();
                        }
                    }
                    ui.separator();
                    if ui
                        .add_enabled(!selectable_ids.is_empty(), egui::Button::new("全选"))
                        .clicked()
                    {
                        select_all(&mut self.selected, selectable_ids);
                    }
                    if ui
                        .add_enabled(!selectable_ids.is_empty(), egui::Button::new("反选"))
                        .clicked()
                    {
                        invert_selection(&mut self.selected, selectable_ids);
                    }
                    ui.separator();
                    let selected_count = selectable_ids
                        .iter()
                        .filter(|id| self.selected.contains(*id))
                        .count();
                    let action_label = match self.view {
                        ViewMode::Existing if self.settings.keep_recovery_copies => {
                            format!("删除所选 ({selected_count})")
                        }
                        ViewMode::Existing => format!("永久删除 ({selected_count})"),
                        ViewMode::Deleted => format!("恢复所选 ({selected_count})"),
                    };
                    let action =
                        egui::Button::new(RichText::new(action_label).color(Color32::WHITE))
                            .fill(Color32::from_rgb(36, 104, 180));
                    if ui
                        .add_enabled(
                            selected_count > 0
                                && !self.safe_mode
                                && !self.quarantine_running
                                && self.settings.root_confirmed,
                            action,
                        )
                        .on_hover_text(match self.view {
                            ViewMode::Existing if self.settings.keep_recovery_copies => {
                                "将所选会话移入可恢复存储"
                            }
                            ViewMode::Existing => "永久删除所选会话，不保留恢复副本",
                            ViewMode::Deleted => "将所选会话恢复到原位置",
                        })
                        .clicked()
                    {
                        action_clicked = true;
                    }
                    if selected_count > 0 && ui.button("取消选择").clicked() {
                        self.selected.clear();
                    }
                    ui.separator();
                    ui.add_sized(
                        [300.0, 28.0],
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("搜索任务标题、项目或会话 ID")
                            .vertical_align(egui::Align::Center)
                            .margin(egui::Margin::symmetric(8, 4)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("{count} 项   {}", bytes_label(bytes)));
                    });
                })
            });
        action_clicked
    }

    fn existing_page(&mut self, ui: &mut egui::Ui) {
        let q = self.query.to_ascii_lowercase();
        let visible: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.agent == self.agent_filter
                    && (q.is_empty()
                        || r.id.to_ascii_lowercase().contains(&q)
                        || r.title.to_ascii_lowercase().contains(&q)
                        || r.project_label.to_ascii_lowercase().contains(&q)
                        || r.project_path.as_ref().is_some_and(|path| {
                            path.to_string_lossy().to_ascii_lowercase().contains(&q)
                        }))
            })
            .map(|(i, _)| i)
            .collect();
        let visible_total = visible.iter().map(|i| self.sessions[*i].bytes).sum();
        let total = if q.is_empty() {
            self.agent_storage_bytes
                .get(&self.agent_filter)
                .copied()
                .unwrap_or(visible_total)
        } else {
            visible_total
        };
        let selectable_ids: Vec<String> = visible
            .iter()
            .filter_map(|index| {
                let record = &self.sessions[*index];
                record.is_mutable().then(|| record.id.clone())
            })
            .collect();
        if self.filters(ui, visible.len(), total, &selectable_ids) {
            self.confirm_open = true;
        }
        if visible.is_empty() && self.current_agent_scanning() {
            self.loading_page(ui);
            return;
        }
        if visible.is_empty() {
            empty_state(
                ui,
                "没有匹配的现有会话",
                "尝试切换会话来源、清除搜索条件或重新扫描。",
            );
            return;
        }
        ui.add_space(8.0);
        let mut delete_one: Option<String> = None;
        let mut all_selected = !selectable_ids.is_empty()
            && selectable_ids.iter().all(|id| self.selected.contains(id));
        ui.scope(|ui| {
            ui.visuals_mut().widgets.noninteractive.bg_stroke =
                egui::Stroke::new(1.0, Color32::TRANSPARENT);
            TableBuilder::new(ui)
                .id_salt("existing_sessions_table")
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(44.0).resizable(false))
                .column(Column::initial(330.0).at_least(160.0).clip(true))
                .column(Column::initial(180.0).at_least(90.0).clip(true))
                .column(Column::initial(110.0).at_least(85.0))
                .column(Column::initial(100.0).at_least(80.0))
                .column(Column::initial(110.0).at_least(95.0))
                .column(Column::remainder().at_least(230.0).resizable(false))
                .header(32.0, |mut h| {
                    h.col(|ui| {
                        if ui
                            .checkbox(&mut all_selected, "")
                            .on_hover_text("选择或取消选择当前筛选结果中的全部可操作会话")
                            .changed()
                        {
                            if all_selected {
                                self.selected.extend(selectable_ids.iter().cloned());
                            } else {
                                for id in &selectable_ids {
                                    self.selected.remove(id);
                                }
                            }
                        }
                    });
                    for (label, right) in [
                        ("任务标题", false),
                        ("项目", false),
                        ("最后活动", false),
                        ("预估大小", true),
                        ("状态", false),
                        ("操作", false),
                    ] {
                        h.col(|ui| {
                            table_header(ui, label, right);
                        });
                    }
                })
                // `rows` virtualizes the body: only the visible scroll range is rendered.
                .body(|body| {
                    body.rows(40.0, visible.len(), |mut row| {
                        let record = &self.sessions[visible[row.index()]];
                        let selectable = record.is_mutable();
                        let mutable = record.is_mutable()
                            && !self.safe_mode
                            && !self.quarantine_running
                            && self.settings.root_confirmed;
                        row.col(|ui| {
                            let mut checked = self.selected.contains(&record.id);
                            if ui
                                .add_enabled(selectable, egui::Checkbox::without_text(&mut checked))
                                .on_hover_text(if selectable {
                                    "选择此会话"
                                } else {
                                    mutation_reason(
                                        record,
                                        self.safe_mode,
                                        self.settings.root_confirmed,
                                    )
                                })
                                .changed()
                            {
                                if checked {
                                    self.selected.insert(record.id.clone());
                                } else {
                                    self.selected.remove(&record.id);
                                }
                            }
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(&record.title).truncate());
                        });
                        row.col(|ui| {
                            let response =
                                ui.add(egui::Label::new(&record.project_label).truncate());
                            if let Some(path) = &record.project_path {
                                response.on_hover_text(path.display().to_string());
                            } else {
                                response.on_hover_text(&record.project_label);
                            }
                        });
                        row.col(|ui| {
                            ui.label(age_label(record.modified));
                        });
                        row.col(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(bytes_label(record.bytes))
                                        .on_hover_text(if record.agent == AgentKind::Cursor {
                                            "当前可直接归属的 transcript 与会话元数据；共享内容块、附件和备份计入顶部总占用"
                                        } else {
                                            "根据该会话独立文件大小估算"
                                        });
                                },
                            );
                        });
                        row.col(|ui| {
                            session_status_icons(ui, record);
                        });
                        row.col(|ui| {
                            ui.horizontal(|ui| {
                                ui.add_enabled(
                                    false,
                                    egui::Button::new("恢复").small().frame(false),
                                )
                                .on_hover_text("会话仍存在，无需恢复");
                                if ui
                                    .add_enabled(
                                        mutable,
                                        egui::Button::new(
                                            RichText::new("删除")
                                                .color(Color32::from_rgb(38, 102, 174)),
                                        )
                                        .small()
                                        .frame(false),
                                    )
                                    .on_hover_text(mutation_reason(
                                        record,
                                        self.safe_mode,
                                        self.settings.root_confirmed,
                                    ))
                                    .clicked()
                                {
                                    delete_one = Some(record.id.clone());
                                }
                                ui.add_enabled(
                                    false,
                                    egui::Button::new("彻底删除").small().frame(false),
                                )
                                .on_hover_text("请先删除到可恢复存储后再彻底删除");
                            });
                        });
                    });
                });
        });
        if let Some(id) = delete_one {
            self.selected.clear();
            self.selected.insert(id);
            self.confirm_open = true;
        }
    }

    fn deleted_page(&mut self, ui: &mut egui::Ui) {
        let q = self.query.to_ascii_lowercase();
        let visible: Vec<usize> = self
            .manifests
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                let m = &s.manifest;
                m.agent == self.agent_filter
                    && matches!(
                        m.state,
                        ManifestState::Preparing
                            | ManifestState::Quarantined
                            | ManifestState::Purging
                    )
                    && (q.is_empty()
                        || m.session_id.to_ascii_lowercase().contains(&q)
                        || m.title.to_ascii_lowercase().contains(&q)
                        || m.project_label.to_ascii_lowercase().contains(&q)
                        || m.project_path.as_ref().is_some_and(|path| {
                            path.to_string_lossy().to_ascii_lowercase().contains(&q)
                        }))
            })
            .map(|(i, _)| i)
            .collect();
        let total = visible
            .iter()
            .map(|i| self.manifests[*i].manifest.bytes)
            .sum();
        let selectable_ids: Vec<String> = visible
            .iter()
            .map(|index| self.manifests[*index].manifest.operation_id.to_string())
            .collect();
        let restore_selected = self.filters(ui, visible.len(), total, &selectable_ids);
        if visible.is_empty() {
            empty_state(
                ui,
                "没有可恢复的会话",
                "已删除的会话会显示在这里，直到恢复或彻底删除。",
            );
            return;
        }
        ui.add_space(8.0);
        let mut restore_one = None;
        let mut purge_one = None;
        let mut all_selected = !selectable_ids.is_empty()
            && selectable_ids.iter().all(|id| self.selected.contains(id));
        ui.scope(|ui| {
            ui.visuals_mut().widgets.noninteractive.bg_stroke =
                egui::Stroke::new(1.0, Color32::TRANSPARENT);
            TableBuilder::new(ui)
                .id_salt("deleted_sessions_table")
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(44.0).resizable(false))
                .column(Column::initial(330.0).at_least(160.0).clip(true))
                .column(Column::initial(180.0).at_least(90.0).clip(true))
                .column(Column::initial(125.0).at_least(105.0))
                .column(Column::initial(100.0).at_least(80.0))
                .column(Column::initial(95.0).at_least(85.0))
                .column(Column::remainder().at_least(230.0).resizable(false))
                .header(32.0, |mut h| {
                    h.col(|ui| {
                        if ui
                            .checkbox(&mut all_selected, "")
                            .on_hover_text("选择或取消选择当前筛选结果中的全部已删除会话")
                            .changed()
                        {
                            if all_selected {
                                self.selected.extend(selectable_ids.iter().cloned());
                            } else {
                                for id in &selectable_ids {
                                    self.selected.remove(id);
                                }
                            }
                        }
                    });
                    for (label, right) in [
                        ("任务标题", false),
                        ("项目", false),
                        ("删除时间", false),
                        ("预估大小", true),
                        ("状态", false),
                        ("操作", false),
                    ] {
                        h.col(|ui| {
                            table_header(ui, label, right);
                        });
                    }
                })
                .body(|body| {
                    body.rows(40.0, visible.len(), |mut row| {
                        let index = visible[row.index()];
                        let m = &self.manifests[index].manifest;
                        let can_write = !self.safe_mode
                            && !self.quarantine_running
                            && self.settings.root_confirmed;
                        row.col(|ui| {
                            let key = m.operation_id.to_string();
                            let mut checked = self.selected.contains(&key);
                            if ui
                                .checkbox(&mut checked, "")
                                .on_hover_text("选择此已删除会话")
                                .changed()
                            {
                                if checked {
                                    self.selected.insert(key);
                                } else {
                                    self.selected.remove(&key);
                                }
                            }
                        });
                        row.col(|ui| {
                            let title = if m.title.is_empty() {
                                format!("未命名会话 {}", m.display_id())
                            } else {
                                m.title.clone()
                            };
                            ui.add(egui::Label::new(title).truncate());
                        });
                        row.col(|ui| {
                            let response = ui.add(
                                egui::Label::new(if m.project_label.is_empty() {
                                    "未知项目"
                                } else {
                                    &m.project_label
                                })
                                .truncate(),
                            );
                            if let Some(path) = &m.project_path {
                                response.on_hover_text(path.display().to_string());
                            }
                        });
                        row.col(|ui| {
                            ui.label(m.created_at.format("%Y-%m-%d %H:%M").to_string());
                        });
                        row.col(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(bytes_label(m.bytes));
                                },
                            );
                        });
                        row.col(|ui| {
                            deleted_status_icons(ui, m);
                        });
                        row.col(|ui| {
                            ui.horizontal(|ui| {
                                let can_purge = can_write && m.state != ManifestState::Preparing;
                                if ui
                                    .add_enabled(
                                        can_purge,
                                        egui::Button::new(
                                            RichText::new("恢复")
                                                .color(Color32::from_rgb(38, 102, 174)),
                                        )
                                        .small()
                                        .frame(false),
                                    )
                                    .on_hover_text(if m.state == ManifestState::Preparing {
                                        "上次操作未完成，请先尝试恢复"
                                    } else if can_write {
                                        "恢复到原位置；已存在同名文件时会安全停止"
                                    } else if !self.settings.root_confirmed {
                                        "请先确认隔离根目录"
                                    } else {
                                        "安全模式已禁用"
                                    })
                                    .clicked()
                                {
                                    restore_one = Some(index);
                                }
                                ui.add_enabled(
                                    false,
                                    egui::Button::new("删除").small().frame(false),
                                )
                                .on_hover_text("该会话已经删除");
                                if ui
                                    .add_enabled(
                                        can_write,
                                        egui::Button::new(
                                            RichText::new("彻底删除")
                                                .color(Color32::from_rgb(177, 65, 55)),
                                        )
                                        .small()
                                        .frame(false),
                                    )
                                    .on_hover_text(if can_write {
                                        "只永久删除这一个会话的隔离副本，无法恢复"
                                    } else if !self.settings.root_confirmed {
                                        "请先确认隔离根目录"
                                    } else {
                                        "安全模式已禁用"
                                    })
                                    .clicked()
                                {
                                    purge_one = Some(index);
                                }
                            });
                        });
                    });
                });
        });
        if let Some(index) = restore_one {
            self.restore_at(index);
        }
        if let Some(index) = purge_one {
            self.purge_index = Some(index);
        }
        let selected_deleted: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|index| {
                self.selected
                    .contains(&self.manifests[*index].manifest.operation_id.to_string())
            })
            .collect();
        if restore_selected {
            for index in selected_deleted {
                self.restore_at(index);
            }
            self.selected.clear();
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        egui::Window::new("删除与存储设置")
            .open(&mut open)
            .resizable(false)
            .default_width(720.0)
            .show(ctx, |ui| {
                ui.heading("可恢复数据存储");
                ui.label("为每个会话来源指定独立文件夹。应用只在执行删除时创建和写入这些目录。");
                ui.add_space(10.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(247, 248, 250))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 224, 229)))
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.checkbox(
                            &mut self.settings_draft.keep_recovery_copies,
                            "删除时保留可恢复副本（推荐）",
                        );
                        ui.small(if self.settings_draft.keep_recovery_copies {
                            "删除的会话会进入“已删除”，之后可以恢复。"
                        } else {
                            "关闭后，新删除的会话将永久删除，不进入“已删除”。"
                        });
                        ui.small("此设置同时适用于 ChatGPT / Codex、Claude Code 和 Cursor。");
                    });
                ui.add_space(12.0);
                if ui.button("统一选择新的根目录…").clicked()
                    && let Some(root) = rfd::FileDialog::new()
                        .set_title("选择隔离根目录")
                        .pick_folder()
                {
                    let keep_recovery_copies = self.settings_draft.keep_recovery_copies;
                    self.settings_draft = AppSettings::from_root(&root);
                    self.settings_draft.keep_recovery_copies = keep_recovery_copies;
                }
                ui.add_space(8.0);
                for agent in AgentKind::ALL {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(247, 248, 250))
                        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 224, 229)))
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(570.0);
                                    ui.label(RichText::new(agent.label()).strong());
                                    let full_path =
                                        self.settings_draft.dir(agent).display().to_string();
                                    ui.label(
                                        RichText::new(ellipsize(&full_path, 72))
                                            .color(Color32::from_rgb(85, 91, 100)),
                                    )
                                    .on_hover_text(full_path);
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("更改…").clicked()
                                            && let Some(path) = rfd::FileDialog::new()
                                                .set_title(format!(
                                                    "选择 {} 隔离目录",
                                                    agent.label()
                                                ))
                                                .set_directory(self.settings_draft.dir(agent))
                                                .pick_folder()
                                        {
                                            self.settings_draft.set_dir(agent, path);
                                        }
                                    },
                                );
                            })
                        });
                    ui.add_space(5.0);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("恢复推荐设置").clicked() {
                        self.settings_draft = AppSettings::from_root(&default_storage_base());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("保存").clicked() {
                            if settings_valid(&self.settings_draft) {
                                self.settings_draft.root_confirmed = true;
                                match save_settings(&self.settings_draft) {
                                    Ok(()) => {
                                        self.settings = self.settings_draft.clone();
                                        self.refresh_manifests();
                                        self.status = "隔离存储位置已保存".to_owned();
                                        self.settings_open = false;
                                    }
                                    Err(e) => self.status = format!("设置保存失败：{e}"),
                                }
                            } else {
                                self.status = "设置未保存：三个目录必须是不同的绝对路径".to_owned();
                            }
                        }
                        if ui.button("取消").clicked() {
                            self.settings_draft = self.settings.clone();
                            self.settings_open = false;
                        }
                    });
                });
            });
        self.settings_open &= open;
    }

    fn onboarding_window(&mut self, ctx: &egui::Context) {
        if !self.onboarding_open {
            return;
        }
        egui::Window::new("设置隔离根目录")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(620.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("▣")
                            .size(34.0)
                            .color(Color32::from_rgb(45, 114, 190)),
                    );
                    ui.vertical(|ui| {
                        ui.heading("选择可恢复数据的存储位置");
                        ui.label("删除的会话会先安全复制到这里，并按会话来源自动分开。");
                    });
                });
                ui.add_space(14.0);
                let root = PathBuf::from(self.onboarding_root.trim());
                let valid = self.onboarding_chosen && root.is_absolute();
                egui::Frame::new()
                    .fill(Color32::from_rgb(247, 249, 252))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(211, 217, 224)))
                    .corner_radius(5.0)
                    .inner_margin(egui::Margin::symmetric(14, 11))
                    .show(ui, |ui| {
                        ui.set_min_width(560.0);
                        ui.label(
                            RichText::new(if self.onboarding_chosen {
                                "已选择"
                            } else {
                                "推荐位置"
                            })
                            .strong(),
                        );
                        ui.label(root.display().to_string());
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("使用推荐位置").clicked() {
                        self.onboarding_root = default_storage_base().display().to_string();
                        self.onboarding_chosen = true;
                    }
                    if ui.button("选择其他文件夹…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title("选择 Agent Chat Cleaner 隔离根目录")
                            .set_directory(&root)
                            .pick_folder()
                    {
                        self.onboarding_root = path.display().to_string();
                        self.onboarding_chosen = true;
                    }
                });
                ui.add_space(14.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("所有数据仅保存在本机").color(Color32::from_rgb(42, 112, 80)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                valid,
                                egui::Button::new(RichText::new("继续").color(Color32::WHITE))
                                    .fill(Color32::from_rgb(36, 104, 180)),
                            )
                            .clicked()
                        {
                            let settings = AppSettings::from_root(&root);
                            match save_settings(&settings) {
                                Ok(()) => {
                                    self.settings = settings.clone();
                                    self.settings_draft = settings;
                                    self.refresh_manifests();
                                    self.onboarding_open = false;
                                    self.status =
                                        "隔离根目录已设置；目录将在首次删除时创建".to_owned();
                                }
                                Err(error) => self.status = format!("根目录设置保存失败：{error}"),
                            }
                        }
                    });
                });
            });
    }

    fn loading_page(&self, ui: &mut egui::Ui) {
        let elapsed = self.scan_started_at.elapsed().as_secs();
        let phase = match elapsed {
            0..=1 => "正在发现本机 Agent 数据目录…",
            2..=4 => "正在读取会话标题、大小和最后活动时间…",
            _ => "会话较多，正在整理扫描结果…",
        };
        ui.add_space(90.0);
        ui.vertical_centered(|ui| {
            ui.add(egui::Spinner::new().size(28.0));
            ui.add_space(14.0);
            ui.heading(format!("正在扫描 {}", self.agent_filter.label()));
            ui.label(phase);
            ui.add_space(12.0);
            ui.small("扫描为只读操作；不会在后台自动删除或移动会话。");
        });
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = self.about_open;
        let mut close_requested = false;
        egui::Window::new("关于 Agent Chat Cleaner")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size([480.0, 245.0])
            .show(ctx, |ui| {
                ui.set_min_width(460.0);
                ui.horizontal(|ui| {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(36, 104, 180))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin::symmetric(13, 10))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("AC")
                                    .size(21.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                        });
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Agent Chat Cleaner")
                                .size(20.0)
                                .strong()
                                .color(Color32::from_rgb(42, 48, 58)),
                        );
                        ui.label(
                            RichText::new("AI 会话管理工具")
                                .color(Color32::from_rgb(105, 112, 122)),
                        );
                    });
                });
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                egui::Grid::new("about_metadata")
                    .num_columns(2)
                    .spacing([28.0, 7.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("版本").color(Color32::from_rgb(110, 117, 127)));
                        ui.label(VERSION);
                        ui.end_row();
                        ui.label(RichText::new("构建").color(Color32::from_rgb(110, 117, 127)));
                        ui.label(COMMIT);
                        ui.end_row();
                        ui.label(RichText::new("平台").color(Color32::from_rgb(110, 117, 127)));
                        ui.label("Windows x64 · Rust / egui");
                        ui.end_row();
                    });
                ui.add_space(18.0);
                ui.label(
                    RichText::new("© 2026 Agent Chat Cleaner")
                        .size(11.0)
                        .color(Color32::from_rgb(125, 131, 140)),
                );
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("关闭").clicked() {
                        close_requested = true;
                    }
                });
            });
        self.about_open = open && !close_requested;
    }

    fn confirmation_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_open {
            return;
        }
        let count = self.selected_records().len();
        let recoverable = self.settings.keep_recovery_copies;
        egui::Window::new(if recoverable {
            format!("删除 {count} 个会话？")
        } else {
            format!("永久删除 {count} 个会话？")
        })
        .collapsible(false)
        .resizable(false)
        .fixed_size([420.0, 145.0])
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(if recoverable {
                        "删除后可在“已删除”中恢复。"
                    } else {
                        "这些会话不会保留恢复副本。"
                    })
                    .size(14.0)
                    .color(Color32::from_rgb(75, 82, 92)),
                );
            });
            ui.add_space(22.0);
            ui.with_layout(
                egui::Layout::left_to_right(egui::Align::Center)
                    .with_main_align(egui::Align::Center),
                |ui| {
                    if ui
                        .add(egui::Button::new("取消").min_size(Vec2::new(88.0, 30.0)))
                        .clicked()
                    {
                        self.confirm_open = false;
                    }
                    ui.add_space(8.0);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(if recoverable {
                                    "删除"
                                } else {
                                    "永久删除"
                                })
                                .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(190, 63, 54))
                            .min_size(Vec2::new(88.0, 30.0)),
                        )
                        .clicked()
                    {
                        self.run_quarantine();
                    }
                },
            );
        });
    }

    fn purge_confirmation_dialog(&mut self, ctx: &egui::Context) {
        let Some(index) = self.purge_index else {
            return;
        };
        let Some(stored) = self.manifests.get(index) else {
            self.purge_index = None;
            return;
        };
        let title = if stored.manifest.title.is_empty() {
            format!("会话 {}", stored.manifest.display_id())
        } else {
            stored.manifest.title.clone()
        };
        egui::Window::new("彻底删除此会话？")
            .collapsible(false)
            .resizable(false)
            .fixed_size([420.0, 175.0])
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.add(egui::Label::new(title).truncate());
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("此操作无法恢复。")
                            .strong()
                            .color(Color32::from_rgb(177, 65, 55)),
                    );
                });
                ui.add_space(20.0);
                ui.with_layout(
                    egui::Layout::left_to_right(egui::Align::Center)
                        .with_main_align(egui::Align::Center),
                    |ui| {
                        if ui
                            .add(egui::Button::new("取消").min_size(Vec2::new(88.0, 30.0)))
                            .clicked()
                        {
                            self.purge_index = None;
                        }
                        ui.add_space(8.0);
                        if ui
                            .add(
                                egui::Button::new(RichText::new("彻底删除").color(Color32::WHITE))
                                    .fill(Color32::from_rgb(190, 63, 54))
                                    .min_size(Vec2::new(88.0, 30.0)),
                            )
                            .clicked()
                        {
                            self.purge_at(index);
                        }
                    },
                );
            });
    }
}

impl eframe::App for CleanerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.quarantine_running && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "安全事务完成前暂不能退出".to_owned();
        }
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let (Some(path), Some(image)) = (&self.screenshot_path, screenshot) {
            let _ = save_bmp(path, &image);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.receive_scan();
        self.receive_quarantine();
        if !self.scanning_agents.is_empty() || self.quarantine_running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.top_bar(ui);
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.small("●");
                ui.small(&self.status);
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(12.0);
            match self.view {
                ViewMode::Existing => self.existing_page(ui),
                ViewMode::Deleted => self.deleted_page(ui),
            }
        });
        self.onboarding_window(&ctx);
        self.settings_window(&ctx);
        self.about_window(&ctx);
        self.confirmation_dialog(&ctx);
        self.purge_confirmation_dialog(&ctx);
        if self.quarantine_running {
            let progress = self.delete_progress.clone().unwrap_or(BatchProgress {
                completed: 0,
                total: 1,
                phase: BatchPhase::Checking,
                current_title: String::new(),
            });
            let fraction = if progress.total == 0 {
                0.0
            } else {
                progress.completed as f32 / progress.total as f32
            };
            let phase = match progress.phase {
                BatchPhase::Checking => "正在检查运行状态和锁定会话…",
                BatchPhase::BackingUp => "正在创建恢复数据…",
                BatchPhase::Deleting => "正在提交删除事务…",
                BatchPhase::Finished => "正在更新列表…",
            };
            egui::Window::new("正在删除会话")
                .collapsible(false)
                .resizable(false)
                .fixed_size([440.0, 180.0])
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(&ctx, |ui| {
                    ui.add_space(8.0);
                    ui.vertical_centered(|ui| {
                        ui.label(phase);
                        ui.add_space(9.0);
                        ui.add(egui::ProgressBar::new(fraction).desired_width(380.0));
                        ui.small(format!(
                            "已完成 {} / {}  ({:.0}%)",
                            progress.completed,
                            progress.total,
                            fraction * 100.0
                        ));
                        if !progress.current_title.is_empty() {
                            ui.add_space(7.0);
                            ui.label(ellipsize(&progress.current_title, 48))
                                .on_hover_text(&progress.current_title);
                        }
                        ui.add_space(8.0);
                        ui.small("请勿在操作完成前启动对应 Agent 或关闭本程序。");
                    });
                });
        }
        let dialog_preview_ready =
            !self.confirm_open || self.scan_started_at.elapsed() >= Duration::from_millis(500);
        if self.screenshot_path.is_some()
            && !self.screenshot_requested
            && dialog_preview_ready
            && ((self.onboarding_open && self.scan_started_at.elapsed() >= Duration::from_secs(2))
                || (!self.onboarding_open && !self.current_agent_scanning()))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.screenshot_requested = true;
            ctx.request_repaint();
        } else if self.screenshot_path.is_some() && self.confirm_open && !dialog_preview_ready {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}

fn save_bmp(path: &Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let width = image.size[0] as u32;
    let height = image.size[1] as u32;
    let pixel_bytes = width
        .checked_mul(height)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| std::io::Error::other("screenshot is too large"))?;
    let file_bytes = 54_u32
        .checked_add(pixel_bytes)
        .ok_or_else(|| std::io::Error::other("screenshot is too large"))?;
    let mut file = std::fs::File::create(path)?;
    file.write_all(b"BM")?;
    file.write_all(&file_bytes.to_le_bytes())?;
    file.write_all(&[0_u8; 4])?;
    file.write_all(&54_u32.to_le_bytes())?;
    file.write_all(&40_u32.to_le_bytes())?;
    file.write_all(&(width as i32).to_le_bytes())?;
    file.write_all(&(height as i32).to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&32_u16.to_le_bytes())?;
    file.write_all(&0_u32.to_le_bytes())?;
    file.write_all(&pixel_bytes.to_le_bytes())?;
    file.write_all(&[0_u8; 16])?;
    for y in (0..image.size[1]).rev() {
        for x in 0..image.size[0] {
            let [red, green, blue, _alpha] = image.pixels[y * image.size[0] + x].to_array();
            file.write_all(&[blue, green, red, 255])?;
        }
    }
    file.flush()
}

fn config_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("AgentChatCleaner")
        .join("settings.json")
}

fn default_storage_base() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let system = std::env::var("SystemDrive")
            .unwrap_or_else(|_| "C:".to_owned())
            .to_ascii_uppercase();
        for letter in b'D'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            if !drive.to_ascii_uppercase().starts_with(&system) && Path::new(&drive).is_dir() {
                return PathBuf::from(drive).join("AgentChatCleanerData");
            }
        }
    }
    agent_chat_cleaner_quarantine::default_data_dir().join("data")
}

fn common_storage_root(settings: &AppSettings) -> PathBuf {
    let parents: Vec<&Path> = AgentKind::ALL
        .iter()
        .filter_map(|agent| settings.dir(*agent).parent())
        .collect();
    if parents.len() == AgentKind::ALL.len() && parents.iter().all(|path| *path == parents[0]) {
        parents[0].to_owned()
    } else {
        default_storage_base()
    }
}

fn load_settings() -> Option<AppSettings> {
    serde_json::from_reader(std::fs::File::open(config_path()).ok()?).ok()
}

fn save_settings(settings: &AppSettings) -> std::io::Result<()> {
    let path = config_path();
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("invalid settings path"))?;
    std::fs::create_dir_all(parent)?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
    std::fs::write(&temp, bytes)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    std::fs::rename(temp, path)
}

fn settings_valid(settings: &AppSettings) -> bool {
    let paths = AgentKind::ALL.map(|a| settings.dir(a));
    paths.iter().all(|p| p.is_absolute())
        && paths[0] != paths[1]
        && paths[0] != paths[2]
        && paths[1] != paths[2]
}

fn load_manifests(settings: &AppSettings) -> Vec<StoredManifest> {
    let mut dirs: Vec<PathBuf> = AgentKind::ALL
        .iter()
        .map(|a| settings.dir(*a).to_owned())
        .collect();
    let legacy = agent_chat_cleaner_quarantine::default_data_dir();
    if !dirs.contains(&legacy) {
        dirs.push(legacy);
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for data_dir in dirs {
        for manifest in agent_chat_cleaner_quarantine::list_manifests(&data_dir) {
            if seen.insert(manifest.operation_id) {
                result.push(StoredManifest {
                    manifest,
                    data_dir: data_dir.clone(),
                });
            }
        }
    }
    result.sort_by_key(|s| std::cmp::Reverse(s.manifest.created_at));
    result
}

fn mutation_reason(record: &SessionRecord, safe_mode: bool, root_confirmed: bool) -> &'static str {
    if record.pinned {
        "该会话已在来源中标记为保留，已自动锁定"
    } else if !root_confirmed {
        "请先确认隔离根目录"
    } else if safe_mode {
        "安全模式已禁用写操作"
    } else {
        match record.safety {
            SafetyState::SupportedIdle if record.agent == AgentKind::Cursor => {
                "删除关联数据库记录和独占 transcript；可从“已删除”恢复"
            }
            SafetyState::SupportedIdle => "删除后可从“已删除”视图恢复",
            SafetyState::Active if record.agent == AgentKind::Cursor => {
                "请先完全退出 Cursor；运行中的数据库已自动保护"
            }
            SafetyState::Active => "最近仍在活动，已自动保护",
            SafetyState::UnknownFormat => "格式未通过安全校验，已禁用写操作",
            SafetyState::OutsideAllowedRoot => "路径越界，已阻止操作",
        }
    }
}

fn table_header(ui: &mut egui::Ui, label: &str, right_aligned: bool) {
    ui.painter()
        .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(239, 242, 246));
    let text = RichText::new(label)
        .size(12.0)
        .strong()
        .color(Color32::from_rgb(71, 78, 88));
    if right_aligned {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(text);
        });
    } else {
        if matches!(label, "状态" | "操作") {
            ui.add_space(8.0);
        }
        ui.label(text);
    }
}

fn empty_state(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.add_space(70.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(title)
                .size(16.0)
                .strong()
                .color(Color32::from_rgb(65, 71, 80)),
        );
        ui.add_space(5.0);
        ui.label(RichText::new(detail).color(Color32::from_rgb(112, 118, 126)));
    });
}

#[derive(Clone, Copy)]
enum StatusGlyph {
    Locked,
    Ready,
    Active,
    Unknown,
    Blocked,
    Recoverable,
}

fn session_status_icons(ui: &mut egui::Ui, record: &SessionRecord) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 3.0;
        if record.pinned {
            status_glyph(ui, StatusGlyph::Locked).on_hover_text(format!(
                "已在 {} 中置顶或命名保留，默认锁定，不会被选择或删除",
                record.agent.label()
            ));
        }
        match record.safety {
            SafetyState::SupportedIdle => {
                let tip = if record.pinned {
                    "会话数据关联已验证"
                } else {
                    "可以删除；删除后可从“已删除”恢复"
                };
                status_glyph(ui, StatusGlyph::Ready).on_hover_text(tip);
            }
            SafetyState::Active if record.agent == AgentKind::Cursor => {
                status_glyph(ui, StatusGlyph::Active)
                    .on_hover_text("Cursor 正在运行，当前会话已保护");
            }
            SafetyState::Active => {
                status_glyph(ui, StatusGlyph::Active).on_hover_text("会话最近仍在使用，暂不可删除");
            }
            SafetyState::UnknownFormat => {
                status_glyph(ui, StatusGlyph::Unknown)
                    .on_hover_text("数据格式或关联关系未验证，已禁止删除");
            }
            SafetyState::OutsideAllowedRoot => {
                status_glyph(ui, StatusGlyph::Blocked)
                    .on_hover_text("会话路径超出允许的数据目录，已阻止操作");
            }
        }
    });
}

fn deleted_status_icons(ui: &mut egui::Ui, manifest: &Manifest) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 3.0;
        if manifest.pinned {
            status_glyph(ui, StatusGlyph::Locked).on_hover_text("该会话删除前处于置顶状态");
        }
        if manifest.state == ManifestState::Preparing {
            status_glyph(ui, StatusGlyph::Active)
                .on_hover_text("上次操作未完成；恢复会重新验证来源，遇到冲突会安全停止");
        } else if manifest.state == ManifestState::Purging {
            status_glyph(ui, StatusGlyph::Active).on_hover_text("正在彻底删除隔离副本");
        } else {
            status_glyph(ui, StatusGlyph::Recoverable)
                .on_hover_text("隔离副本完整，可以恢复到原位置");
        }
    });
}

fn status_glyph(ui: &mut egui::Ui, glyph: StatusGlyph) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    let center = rect.center();
    match glyph {
        StatusGlyph::Locked => {
            let color = Color32::from_rgb(45, 102, 174);
            painter.circle_stroke(
                egui::pos2(center.x, center.y - 2.5),
                3.8,
                egui::Stroke::new(1.8, color),
            );
            painter.rect_filled(
                egui::Rect::from_center_size(
                    egui::pos2(center.x, center.y + 2.5),
                    egui::vec2(10.5, 8.5),
                ),
                2.0,
                color,
            );
            painter.circle_filled(egui::pos2(center.x, center.y + 2.0), 1.1, Color32::WHITE);
        }
        StatusGlyph::Ready => {
            painter.circle_filled(center, 7.0, Color32::from_rgb(34, 145, 91));
            let stroke = egui::Stroke::new(1.7, Color32::WHITE);
            painter.line_segment(
                [
                    egui::pos2(center.x - 3.2, center.y),
                    egui::pos2(center.x - 0.8, center.y + 2.4),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 0.8, center.y + 2.4),
                    egui::pos2(center.x + 3.8, center.y - 3.0),
                ],
                stroke,
            );
        }
        StatusGlyph::Active => {
            let color = Color32::from_rgb(202, 132, 35);
            painter.circle_stroke(center, 6.5, egui::Stroke::new(1.7, color));
            painter.line_segment(
                [center, egui::pos2(center.x, center.y - 3.7)],
                egui::Stroke::new(1.5, color),
            );
            painter.line_segment(
                [center, egui::pos2(center.x + 3.0, center.y + 1.5)],
                egui::Stroke::new(1.5, color),
            );
        }
        StatusGlyph::Unknown => {
            let color = Color32::from_rgb(132, 139, 149);
            painter.circle_filled(center, 7.0, color);
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        StatusGlyph::Blocked => {
            painter.circle_filled(center, 7.0, Color32::from_rgb(190, 68, 68));
            let stroke = egui::Stroke::new(1.7, Color32::WHITE);
            painter.line_segment(
                [
                    egui::pos2(center.x - 2.7, center.y - 2.7),
                    egui::pos2(center.x + 2.7, center.y + 2.7),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 2.7, center.y - 2.7),
                    egui::pos2(center.x - 2.7, center.y + 2.7),
                ],
                stroke,
            );
        }
        StatusGlyph::Recoverable => {
            let color = Color32::from_rgb(45, 112, 184);
            painter.rect_filled(
                egui::Rect::from_center_size(
                    egui::pos2(center.x, center.y + 2.0),
                    egui::vec2(12.0, 8.0),
                ),
                2.0,
                color,
            );
            let stroke = egui::Stroke::new(1.6, Color32::WHITE);
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y + 3.0),
                    egui::pos2(center.x, center.y - 4.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y - 4.0),
                    egui::pos2(center.x - 2.6, center.y - 1.2),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y - 4.0),
                    egui::pos2(center.x + 2.6, center.y - 1.2),
                ],
                stroke,
            );
        }
    }
    response
}

fn install_chinese_font(ctx: &egui::Context) {
    for candidate in [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ] {
        if let Ok(bytes) = std::fs::read(candidate) {
            let mut fonts = FontDefinitions::default();
            fonts
                .font_data
                .insert("system-cjk".to_owned(), FontData::from_owned(bytes).into());
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, "system-cjk".to_owned());
            }
            ctx.set_fonts(fonts);
            break;
        }
    }
    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(Color32::from_rgb(35, 39, 45));
    visuals.panel_fill = Color32::from_rgb(250, 251, 252);
    visuals.window_fill = Color32::WHITE;
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.faint_bg_color = Color32::from_rgb(247, 248, 250);
    visuals.selection.bg_fill = Color32::from_rgb(45, 114, 190);
    visuals.selection.stroke.color = Color32::WHITE;
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(245, 247, 249);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(245, 247, 249);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, Color32::from_rgb(204, 209, 216));
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(232, 239, 248);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(232, 239, 248);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, Color32::from_rgb(92, 139, 190));
    visuals.window_stroke = egui::Stroke::new(1.0, Color32::from_rgb(188, 194, 202));
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
    style.spacing.item_spacing = egui::vec2(7.0, 5.0);
    style.spacing.button_padding = egui::vec2(9.0, 4.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    ctx.set_style_of(egui::Theme::Light, style);
}

fn bytes_label(bytes: u64) -> String {
    let value = bytes as f64;
    if value >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", value / (1024.0 * 1024.0 * 1024.0))
    } else if value >= 1024.0 * 1024.0 {
        format!("{:.1} MB", value / (1024.0 * 1024.0))
    } else if value >= 1024.0 {
        format!("{:.1} KB", value / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn file_fingerprint(path: &Path) -> Option<Fingerprint> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
    Some(Fingerprint {
        len: metadata.len(),
        modified_millis: modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    })
}

fn age_label(modified: SystemTime) -> String {
    let minutes = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default()
        .as_secs()
        / 60;
    if minutes < 60 {
        format!("{minutes} 分钟前")
    } else if minutes < 1440 {
        format!("{} 小时前", minutes / 60)
    } else {
        format!("{} 天前", minutes / 1440)
    }
}

fn ellipsize(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_owned()
    } else {
        format!(
            "{}…",
            value
                .chars()
                .take(max.saturating_sub(1))
                .collect::<String>()
        )
    }
}

fn remove_quarantined_sessions(
    sessions: &mut Vec<SessionRecord>,
    agent: AgentKind,
    succeeded_ids: &HashSet<String>,
) -> u64 {
    let released_exclusive_bytes = sessions
        .iter()
        .filter(|record| {
            record.agent == agent
                && succeeded_ids.contains(&record.id)
                && matches!(&record.storage, SessionStorage::ExclusiveFile)
        })
        .map(|record| record.bytes)
        .sum();
    sessions.retain(|record| record.agent != agent || !succeeded_ids.contains(&record.id));
    released_exclusive_bytes
}

fn select_all(selected: &mut BTreeSet<String>, keys: &[String]) {
    selected.extend(keys.iter().cloned());
}

fn invert_selection(selected: &mut BTreeSet<String>, keys: &[String]) {
    for key in keys {
        if !selected.remove(key) {
            selected.insert(key.clone());
        }
    }
}

#[cfg(test)]
mod ui_state_tests {
    use super::*;

    fn record(agent: AgentKind, id: &str, bytes: u64, storage: SessionStorage) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            title: id.to_owned(),
            agent,
            project_label: "fixture".to_owned(),
            project_path: None,
            pinned: false,
            source: PathBuf::from(format!("C:/fixture/{id}")),
            allowed_root: PathBuf::from("C:/fixture"),
            bytes,
            modified: UNIX_EPOCH,
            fingerprint: Fingerprint {
                len: bytes,
                modified_millis: 0,
            },
            safety: SafetyState::SupportedIdle,
            exclusive_file: matches!(&storage, SessionStorage::ExclusiveFile),
            storage,
        }
    }

    #[test]
    fn select_all_and_invert_only_touch_visible_keys() {
        let visible = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let mut selected = BTreeSet::from(["outside".to_owned()]);
        select_all(&mut selected, &visible);
        assert!(visible.iter().all(|key| selected.contains(key)));
        invert_selection(&mut selected, &visible);
        assert!(visible.iter().all(|key| !selected.contains(key)));
        assert!(selected.contains("outside"));
    }

    #[test]
    fn successful_deletion_removes_only_committed_rows_immediately() {
        let mut sessions = vec![
            record(
                AgentKind::Codex,
                "deleted",
                120,
                SessionStorage::ExclusiveFile,
            ),
            record(AgentKind::Codex, "kept", 80, SessionStorage::ExclusiveFile),
            record(
                AgentKind::ClaudeCode,
                "deleted",
                60,
                SessionStorage::ExclusiveFile,
            ),
        ];
        let released = remove_quarantined_sessions(
            &mut sessions,
            AgentKind::Codex,
            &HashSet::from(["deleted".to_owned()]),
        );
        assert_eq!(released, 120);
        assert_eq!(sessions.len(), 2);
        assert!(
            sessions
                .iter()
                .any(|record| record.agent == AgentKind::Codex && record.id == "kept")
        );
        assert!(
            sessions
                .iter()
                .any(|record| record.agent == AgentKind::ClaudeCode && record.id == "deleted")
        );
    }
}
