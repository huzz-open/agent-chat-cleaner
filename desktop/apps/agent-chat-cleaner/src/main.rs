use agent_chat_cleaner_core::{SafetyState, SessionRecord};
use agent_chat_cleaner_quarantine::{Manifest, ManifestState};
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText, Vec2};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, SystemTime};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const COMMIT: &str = match option_env!("ACC_GIT_COMMIT") {
    Some(value) => value,
    None => "development",
};

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("Agent Chat Cleaner {VERSION} ({COMMIT})");
        return Ok(());
    }
    let safe_mode = args.iter().any(|arg| arg == "--safe-mode");
    let data_dir = value_after(&args, "--data-dir")
        .map(PathBuf::from)
        .unwrap_or_else(agent_chat_cleaner_quarantine::default_data_dir);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Agent Chat Cleaner")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([850.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Agent Chat Cleaner",
        options,
        Box::new(move |cc| Ok(Box::new(CleanerApp::new(cc, data_dir, safe_mode)))),
    )
}

fn value_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Page {
    Sessions,
    Quarantine,
    About,
}

struct CleanerApp {
    page: Page,
    sessions: Vec<SessionRecord>,
    selected: BTreeSet<String>,
    manifests: Vec<Manifest>,
    scan_rx: Option<Receiver<Vec<SessionRecord>>>,
    scanning: bool,
    query: String,
    confirm_open: bool,
    confirmation: String,
    safe_mode: bool,
    data_dir: PathBuf,
    status: String,
}

impl CleanerApp {
    fn new(cc: &eframe::CreationContext<'_>, data_dir: PathBuf, safe_mode: bool) -> Self {
        install_chinese_font(&cc.egui_ctx);
        let mut app = Self {
            page: Page::Sessions,
            sessions: Vec::new(),
            selected: BTreeSet::new(),
            manifests: agent_chat_cleaner_quarantine::list_manifests(&data_dir),
            scan_rx: None,
            scanning: false,
            query: String::new(),
            confirm_open: false,
            confirmation: String::new(),
            safe_mode,
            data_dir,
            status: if safe_mode {
                "安全模式：所有写入操作均已禁用"
            } else {
                "仅读取元数据；不会自动清理"
            }
            .to_owned(),
        };
        app.start_scan();
        app
    }

    fn start_scan(&mut self) {
        if self.scanning {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.scanning = true;
        self.status = "正在只读扫描会话元数据…".to_owned();
        thread::spawn(move || {
            let _ = tx.send(agent_chat_cleaner_scanner::scan_default_roots());
        });
    }

    fn receive_scan(&mut self) {
        let result = self
            .scan_rx
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok());
        if let Some(sessions) = result {
            let count = sessions.len();
            self.sessions = sessions;
            self.selected.retain(|id| {
                self.sessions
                    .iter()
                    .any(|item| &item.id == id && item.is_mutable())
            });
            self.scanning = false;
            self.scan_rx = None;
            self.status = format!("扫描完成：识别到 {count} 个会话；活动或未知格式已自动保护");
        }
    }

    fn selected_records(&self) -> Vec<SessionRecord> {
        self.sessions
            .iter()
            .filter(|record| self.selected.contains(&record.id) && record.is_mutable())
            .cloned()
            .collect()
    }

    fn run_quarantine(&mut self) {
        if self.safe_mode {
            self.status = "安全模式禁止隔离操作".to_owned();
            return;
        }
        let records = self.selected_records();
        let mut succeeded = 0usize;
        let mut errors = Vec::new();
        for record in records {
            match agent_chat_cleaner_quarantine::quarantine(&record, &self.data_dir) {
                Ok(_) => {
                    succeeded += 1;
                    self.selected.remove(&record.id);
                }
                Err(error) => errors.push(format!("{}: {error}", record.short_id())),
            }
        }
        self.manifests = agent_chat_cleaner_quarantine::list_manifests(&self.data_dir);
        self.confirm_open = false;
        self.confirmation.clear();
        self.status = if errors.is_empty() {
            format!("已将 {succeeded} 个所选会话移入可恢复隔离区")
        } else {
            format!(
                "已隔离 {succeeded} 个；{} 个被安全阻止：{}",
                errors.len(),
                errors.join("；")
            )
        };
        self.start_scan();
    }

    fn restore_at(&mut self, index: usize) {
        if self.safe_mode {
            self.status = "安全模式禁止恢复操作".to_owned();
            return;
        }
        let Some(manifest) = self.manifests.get(index).cloned() else {
            return;
        };
        match agent_chat_cleaner_quarantine::restore(&manifest, &self.data_dir) {
            Ok(updated) => {
                self.manifests[index] = updated;
                self.status = format!("会话 {} 已恢复到原位置", manifest.display_id());
                self.start_scan();
            }
            Err(error) => self.status = format!("恢复已停止：{error}"),
        }
    }

    fn sidebar(&mut self, root_ui: &mut egui::Ui) {
        egui::Panel::left("sidebar")
            .exact_size(205.0)
            .show(root_ui, |ui| {
                ui.add_space(20.0);
                ui.heading(RichText::new("Agent Chat Cleaner").size(18.0));
                ui.label(
                    RichText::new("LOCAL · REVERSIBLE")
                        .color(Color32::from_rgb(54, 199, 138))
                        .size(10.0),
                );
                ui.add_space(28.0);
                ui.selectable_value(&mut self.page, Page::Sessions, "▦  会话");
                ui.add_space(8.0);
                ui.selectable_value(&mut self.page, Page::Quarantine, "▣  隔离区");
                ui.add_space(8.0);
                ui.selectable_value(&mut self.page, Page::About, "ⓘ  关于");
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(16.0);
                    let label = if self.safe_mode {
                        "● 安全模式"
                    } else {
                        "● 本地模式"
                    };
                    ui.label(RichText::new(label).color(Color32::from_rgb(54, 199, 138)));
                    ui.small("聊天正文不会上传");
                });
            });
    }

    fn sessions_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("本地会话");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.scanning,
                        egui::Button::new(if self.scanning {
                            "扫描中…"
                        } else {
                            "重新扫描"
                        }),
                    )
                    .clicked()
                {
                    self.start_scan();
                }
            });
        });
        ui.add_space(6.0);
        ui.label("仅展示可验证的 Codex / Claude Code JSONL 会话；运行中、格式未知或共享容器不会进入操作列表。");
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label("搜索");
            ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Agent、项目或会话 ID")
                    .desired_width(360.0),
            );
            let total: u64 = self.sessions.iter().map(|item| item.bytes).sum();
            ui.label(format!(
                "共 {} · {}",
                self.sessions.len(),
                bytes_label(total)
            ));
        });
        ui.separator();

        let query = self.query.to_ascii_lowercase();
        let visible: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, record)| {
                query.is_empty()
                    || record.id.to_ascii_lowercase().contains(&query)
                    || record.agent.label().to_ascii_lowercase().contains(&query)
                    || record.project_label.to_ascii_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        egui::Grid::new("sessions_header")
            .num_columns(6)
            .spacing([14.0, 10.0])
            .show(ui, |ui| {
                ui.strong("选择");
                ui.strong("Agent");
                ui.strong("会话");
                ui.strong("项目");
                ui.strong("最后活动");
                ui.strong("大小 / 状态");
                ui.end_row();
            });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 30.0, visible.len(), |ui, row_range| {
                egui::Grid::new("sessions_grid")
                    .num_columns(6)
                    .striped(true)
                    .spacing([14.0, 10.0])
                    .show(ui, |ui| {
                        for row in row_range {
                            let index = visible[row];
                            let record = &self.sessions[index];
                            let mutable = record.is_mutable() && !self.safe_mode;
                            let mut checked = self.selected.contains(&record.id);
                            if ui
                                .add_enabled(mutable, egui::Checkbox::without_text(&mut checked))
                                .changed()
                            {
                                if checked {
                                    self.selected.insert(record.id.clone());
                                } else {
                                    self.selected.remove(&record.id);
                                }
                            }
                            ui.label(record.agent.label());
                            ui.label(record.short_id())
                                .on_hover_text("界面默认隐藏完整路径和聊天正文");
                            ui.label(ellipsize(&record.project_label, 24));
                            ui.label(age_label(record.modified));
                            let state = match record.safety {
                                SafetyState::SupportedIdle => bytes_label(record.bytes),
                                SafetyState::Active => "活动中 · 已保护".to_owned(),
                                SafetyState::UnknownFormat => "未知格式 · 只读".to_owned(),
                                SafetyState::OutsideAllowedRoot => "越界 · 已阻止".to_owned(),
                            };
                            ui.label(state);
                            ui.end_row();
                        }
                    });
            });
        ui.separator();
        ui.horizontal(|ui| {
            let count = self.selected_records().len();
            let bytes: u64 = self.selected_records().iter().map(|item| item.bytes).sum();
            ui.strong(format!("已选择 {count} 个会话 · {}", bytes_label(bytes)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = count > 0 && !self.safe_mode && !self.scanning;
                if ui
                    .add_enabled(enabled, egui::Button::new("移入隔离区…"))
                    .clicked()
                {
                    self.confirm_open = true;
                    self.confirmation.clear();
                }
            });
        });
    }

    fn quarantine_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("可恢复隔离区");
        ui.label("这里没有“永久删除”。恢复时若原位置已有文件，会停止并保留两份数据。");
        ui.add_space(12.0);
        if self.manifests.is_empty() {
            ui.group(|ui| ui.label("隔离区为空。应用不会后台自动清理。"));
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut restore_index = None;
            for (index, manifest) in self.manifests.iter().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.strong(format!(
                                "{} · {}",
                                manifest.agent.label(),
                                manifest.display_id()
                            ));
                            ui.small(format!(
                                "{} · {}",
                                manifest.created_at.format("%Y-%m-%d %H:%M"),
                                bytes_label(manifest.bytes)
                            ));
                            ui.small(match manifest.state {
                                ManifestState::Preparing => "状态：操作中断，可核对或恢复",
                                ManifestState::Quarantined => "状态：已隔离，可恢复",
                                ManifestState::Restored => "状态：已恢复",
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    manifest.state != ManifestState::Restored && !self.safe_mode,
                                    egui::Button::new("恢复"),
                                )
                                .clicked()
                            {
                                restore_index = Some(index);
                            }
                        });
                    });
                });
                ui.add_space(6.0);
            }
            if let Some(index) = restore_index {
                self.restore_at(index);
            }
        });
    }

    fn about_page(&self, ui: &mut egui::Ui) {
        ui.heading("关于");
        ui.add_space(12.0);
        ui.label(format!("Agent Chat Cleaner {VERSION}"));
        ui.label(format!("构建：{COMMIT}"));
        ui.label("纯 Rust / egui Windows x64 桌面应用");
        ui.add_space(18.0);
        ui.strong("当前安全边界");
        ui.label("• 只处理经元数据验证且一个文件只对应一个会话的 JSONL。\n• 共享 JSONL、SQLite、子代理目录、活动会话和未知格式保持只读。\n• 变更前建立可校验隔离副本；当前版本不提供永久删除。\n• 测试只使用临时 fixture，不调用真实用户目录的变更接口。");
        ui.add_space(18.0);
        ui.strong("应用数据目录");
        ui.monospace(self.data_dir.display().to_string());
    }

    fn confirmation_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_open {
            return;
        }
        let count = self.selected_records().len();
        egui::Window::new("确认移入隔离区")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "只会处理当前选中的 {count} 个独占会话文件，不会删除它们的父目录或其他会话。"
                ));
                ui.label("执行前将再次校验路径、大小和修改时间，并创建通过哈希验证的恢复副本。");
                ui.add_space(8.0);
                ui.label("输入 隔离 或 QUARANTINE 继续：");
                ui.text_edit_singleline(&mut self.confirmation);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        self.confirm_open = false;
                        self.confirmation.clear();
                    }
                    let confirmed = self.confirmation.trim() == "隔离"
                        || self.confirmation.trim() == "QUARANTINE";
                    if ui
                        .add_enabled(confirmed, egui::Button::new("确认隔离"))
                        .clicked()
                    {
                        self.run_quarantine();
                    }
                });
            });
    }
}

impl eframe::App for CleanerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive_scan();
        if self.scanning {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.sidebar(ui);
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.small("●");
                ui.small(&self.status);
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(14.0);
            match self.page {
                Page::Sessions => self.sessions_page(ui),
                Page::Quarantine => self.quarantine_page(ui),
                Page::About => self.about_page(ui),
            }
        });
        self.confirmation_dialog(&ctx);
    }
}

fn install_chinese_font(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];
    for candidate in candidates {
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
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(18, 23, 29);
    visuals.window_fill = Color32::from_rgb(23, 29, 36);
    visuals.selection.bg_fill = Color32::from_rgb(28, 126, 91);
    ctx.set_visuals(visuals);
}

fn bytes_label(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;
    const GB: f64 = 1024.0 * MB;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.2} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

fn age_label(modified: SystemTime) -> String {
    let elapsed = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    let minutes = elapsed.as_secs() / 60;
    if minutes < 60 {
        format!("{minutes} 分钟前")
    } else if minutes < 24 * 60 {
        format!("{} 小时前", minutes / 60)
    } else {
        format!("{} 天前", minutes / (24 * 60))
    }
}

fn ellipsize(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let prefix: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{prefix}…")
}

#[allow(dead_code)]
fn _path_is_under(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}
