//! DupeHunter — main application entry point and egui UI.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod grouper;
mod hasher;
mod perceptual;
mod scanner;

use std::path::PathBuf;
use std::thread;

use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;
use egui::{Color32, FontId, RichText, Ui};

use actions::{ActionKind, export_csv, execute_bulk};
use grouper::{DuplicateGroup, FileMark, MatchKind, ScanStats};
use scanner::{ScanOptions, format_size};

// ── Scan pipeline messages ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum ScanMsg {
    Progress { stage: String, current: usize, total: usize },
    Done { groups: Vec<DuplicateGroup>, files_scanned: usize },
    Error(String),
}

// ── App state ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum View {
    Setup,
    Scanning,
    Results,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FilterKind {
    All,
    Exact,
    Image,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SortMode {
    WastedDesc,
    WastedAsc,
    CopiesDesc,
    NameAsc,
}

struct DupeHunter {
    // ── Setup panel ──
    opts: ScanOptions,

    // ── Scan state ──
    view: View,
    #[allow(dead_code)]
    scan_tx: Option<Sender<()>>,       // reserved for future cancel support
    msg_rx: Option<Receiver<ScanMsg>>,
    progress_stage: String,
    progress_current: usize,
    progress_total: usize,

    // ── Results ──
    groups: Vec<DuplicateGroup>,
    files_scanned: usize,
    filter_kind: FilterKind,
    filter_text: String,
    sort_mode: SortMode,
    show_only_marked: bool,

    // ── Action panel ──
    action_quarantine_path: String,
    action_log: Vec<(bool, String)>,   // (success, message)
    confirm_dialog: Option<ConfirmDialog>,

    // ── Settings ──
    image_threshold: u32,
    video_threshold: u32,
    video_frame_samples: u32,
}

#[derive(Debug)]
struct ConfirmDialog {
    title: String,
    body: String,
    on_confirm: ConfirmAction,
}

#[derive(Debug, Clone)]
enum ConfirmAction {
    DeleteMarked,
    QuarantineMarked,
}

impl Default for DupeHunter {
    fn default() -> Self {
        Self {
            opts: ScanOptions::default(),
            view: View::Setup,
            scan_tx: None,
            msg_rx: None,
            progress_stage: String::new(),
            progress_current: 0,
            progress_total: 1,
            groups: Vec::new(),
            files_scanned: 0,
            filter_kind: FilterKind::All,
            filter_text: String::new(),
            sort_mode: SortMode::WastedDesc,
            show_only_marked: false,
            action_quarantine_path: String::new(),
            action_log: Vec::new(),
            confirm_dialog: None,
            image_threshold: perceptual::IMAGE_THRESHOLD,
            video_threshold: perceptual::VIDEO_THRESHOLD,
            video_frame_samples: perceptual::VIDEO_FRAME_SAMPLES,
        }
    }
}

// ── Scan engine (runs in background thread) ───────────────────────────────

fn run_scan(
    opts: ScanOptions,
    image_threshold: u32,
    video_threshold: u32,
    video_frame_samples: u32,
    tx: Sender<ScanMsg>,
) {
    macro_rules! send {
        ($msg:expr) => { let _ = tx.send($msg); }
    }

    // 1. Collect files
    send!(ScanMsg::Progress { stage: "Collecting files…".into(), current: 0, total: 1 });
    let entries = match scanner::collect_files(&opts, |n, path| {
        let _ = tx.send(ScanMsg::Progress {
            stage: format!("Scanning: {}", path),
            current: n,
            total: n + 1,
        });
    }) {
        Ok(e) => e,
        Err(e) => { send!(ScanMsg::Error(e.to_string())); return; }
    };

    let files_scanned = entries.len();
    send!(ScanMsg::Progress {
        stage: format!("Found {} files — hashing…", files_scanned),
        current: 0,
        total: files_scanned,
    });

    // 2. Exact hash
    let exact_groups = match hasher::find_exact_duplicates(&entries, |done, total| {
        let _ = tx.send(ScanMsg::Progress {
            stage: format!("Hashing file {}/{}…", done, total),
            current: done,
            total: total.max(1),
        });
    }) {
        Ok(g) => g,
        Err(e) => { send!(ScanMsg::Error(e.to_string())); return; }
    };

    // 3. Perceptual image hashing
    let img_entries: Vec<_> = entries.iter()
        .filter(|e| e.kind == scanner::FileKind::Image)
        .collect();

    send!(ScanMsg::Progress {
        stage: format!("Perceptual image scan ({} images)…", img_entries.len()),
        current: 0,
        total: img_entries.len().max(1),
    });

    let img_prints: Vec<perceptual::ImageFingerprint> = img_entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let fp = perceptual::fingerprint_image(&e.path).ok()?;
            let _ = tx.send(ScanMsg::Progress {
                stage: format!("Hashing image {}/{}…", i + 1, img_entries.len()),
                current: i + 1,
                total: img_entries.len(),
            });
            Some(fp)
        })
        .collect();

    let perceptual_image_groups =
        perceptual::group_similar_images(&img_prints, image_threshold);

    // 4. Perceptual video hashing
    let vid_entries: Vec<_> = entries.iter()
        .filter(|e| e.kind == scanner::FileKind::Video)
        .collect();

    send!(ScanMsg::Progress {
        stage: format!("Perceptual video scan ({} videos)…", vid_entries.len()),
        current: 0,
        total: vid_entries.len().max(1),
    });

    let vid_prints: Vec<perceptual::VideoFingerprint> = vid_entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let fp = perceptual::fingerprint_video(&e.path, video_frame_samples).ok()?;
            let _ = tx.send(ScanMsg::Progress {
                stage: format!("Sampling video {}/{}…", i + 1, vid_entries.len()),
                current: i + 1,
                total: vid_entries.len(),
            });
            Some(fp)
        })
        .collect();

    let perceptual_video_groups =
        perceptual::group_similar_videos(&vid_prints, video_threshold);

    // 5. Build unified groups
    send!(ScanMsg::Progress {
        stage: "Building duplicate groups…".into(),
        current: 0, total: 1,
    });

    let groups = grouper::build_groups(
        &entries,
        exact_groups,
        perceptual_image_groups,
        perceptual_video_groups,
    );

    send!(ScanMsg::Done { groups, files_scanned });
}

// ── egui App impl ─────────────────────────────────────────────────────────

impl eframe::App for DupeHunter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll background scan messages — collect first, then mutate self
        let msgs: Vec<ScanMsg> = self.msg_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();

        for msg in msgs {
            match msg {
                ScanMsg::Progress { stage, current, total } => {
                    self.progress_stage = stage;
                    self.progress_current = current;
                    self.progress_total = total;
                    ctx.request_repaint();
                }
                ScanMsg::Done { groups, files_scanned } => {
                    self.groups = groups;
                    self.files_scanned = files_scanned;
                    self.view = View::Results;
                    self.msg_rx = None;
                    ctx.request_repaint();
                }
                ScanMsg::Error(e) => {
                    self.action_log.push((false, format!("Scan error: {}", e)));
                    self.view = View::Setup;
                    self.msg_rx = None;
                    ctx.request_repaint();
                }
            }
        }

        // Confirm dialog (modal)
        if self.confirm_dialog.is_some() {
            self.draw_confirm_dialog(ctx);
            return;
        }

        // Top navigation bar
        egui::TopBottomPanel::top("nav").show(ctx, |ui| {
            self.draw_nav(ui);
        });

        // Bottom log/status bar
        egui::TopBottomPanel::bottom("log").show(ctx, |ui| {
            self.draw_status_bar(ui);
        });

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.view.clone() {
                View::Setup    => self.draw_setup(ui, ctx),
                View::Scanning => self.draw_scanning(ui),
                View::Results  => self.draw_results(ui, ctx),
            }
        });
    }
}

// ── UI sections ────────────────────────────────────────────────────────────

impl DupeHunter {

    // ── Nav bar ──────────────────────────────────────────────────────────

    fn draw_nav(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("🔍 DupeHunter").size(18.0).strong());
            ui.separator();

            // Tab buttons
            let tab = |ui: &mut Ui, label: &str, active: bool| -> bool {
                let text = if active {
                    RichText::new(label).color(Color32::WHITE).strong()
                } else {
                    RichText::new(label).color(Color32::GRAY)
                };
                ui.button(text).clicked()
            };

            if tab(ui, "⚙ Setup", self.view == View::Setup) && self.view != View::Scanning {
                self.view = View::Setup;
            }
            if !self.groups.is_empty() {
                let results_label = format!("📋 Results ({})", self.groups.len());
                if tab(ui, &results_label, self.view == View::Results)
                    && self.view != View::Scanning {
                    self.view = View::Results;
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if self.view == View::Results && !self.groups.is_empty() {
                    let stats = ScanStats::from_groups(self.files_scanned, &self.groups);
                    ui.label(
                        RichText::new(format!(
                            "{} files  |  {} groups  |  {} reclaimable",
                            stats.files_scanned,
                            stats.groups_found,
                            format_size(stats.total_wasted)
                        ))
                        .color(Color32::GRAY)
                        .size(12.0),
                    );
                }
            });
        });
    }

    // ── Status bar ───────────────────────────────────────────────────────

    fn draw_status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            if let Some(last) = self.action_log.last() {
                let color = if last.0 { Color32::GREEN } else { Color32::RED };
                ui.label(RichText::new(&last.1).color(color).size(11.0));
            } else {
                ui.label(RichText::new("Ready").color(Color32::GRAY).size(11.0));
            }
        });
    }

    // ── Setup view ───────────────────────────────────────────────────────

    fn draw_setup(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_max_width(700.0);

            // ── Scan Folders ──
            section(ui, "📁 Scan Locations");

            if self.opts.roots.is_empty() {
                ui.label(
                    RichText::new("No folders added yet. Click 'Add Folder' to start.")
                        .color(Color32::GRAY),
                );
            } else {
                let mut to_remove = Vec::new();
                for (i, root) in self.opts.roots.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("📁").size(16.0));
                        ui.label(
                            RichText::new(root.to_string_lossy())
                                .monospace()
                                .color(Color32::LIGHT_GRAY),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").clicked() {
                                to_remove.push(i);
                            }
                        });
                    });
                }
                for i in to_remove.into_iter().rev() {
                    self.opts.roots.remove(i);
                }
            }

            ui.add_space(6.0);
            if ui.button("➕  Add Folder…").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    if !self.opts.roots.contains(&path) {
                        self.opts.roots.push(path);
                    }
                }
            }

            ui.add_space(16.0);

            // ── Scan Options ──
            section(ui, "⚙ Options");

            egui::Grid::new("opts_grid")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Scan subdirectories:");
                    ui.checkbox(&mut self.opts.recursive, "");
                    ui.end_row();

                    ui.label("Skip hidden files:");
                    ui.checkbox(&mut self.opts.skip_hidden, "");
                    ui.end_row();

                    ui.label("Minimum file size (bytes):");
                    ui.add(egui::DragValue::new(&mut self.opts.min_size_bytes)
                        .speed(1.0).range(0..=u64::MAX));
                    ui.end_row();

                    ui.label("Perceptual image matching:");
                    ui.checkbox(&mut self.opts.compare_images_perceptually, "");
                    ui.end_row();

                    ui.label("Perceptual video matching:");
                    ui.checkbox(&mut self.opts.compare_videos_perceptually, "");
                    ui.end_row();
                });

            ui.add_space(16.0);

            // ── Sensitivity ──
            section(ui, "🎚 Perceptual Sensitivity");

            egui::Grid::new("sens_grid")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Image threshold (0–64 bits):");
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut self.image_threshold, 0..=20));
                        ui.label(
                            RichText::new(match self.image_threshold {
                                0..=2  => "Strict (byte-near-identical)",
                                3..=8  => "Normal (same image, different format/compression)",
                                9..=15 => "Loose (similar images)",
                                _      => "Very loose",
                            }).color(Color32::GRAY).size(11.0),
                        );
                    });
                    ui.end_row();

                    ui.label("Video threshold (0–64 bits):");
                    ui.add(egui::Slider::new(&mut self.video_threshold, 0..=25));
                    ui.end_row();

                    ui.label("Video frame samples:");
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut self.video_frame_samples, 4..=32));
                        ui.label(
                            RichText::new("↑ More = accurate but slower")
                                .color(Color32::GRAY).size(11.0),
                        );
                    });
                    ui.end_row();
                });

            ui.add_space(24.0);

            // ── Scan button ──
            ui.horizontal(|ui| {
                let can_scan = !self.opts.roots.is_empty();
                ui.add_enabled_ui(can_scan, |ui| {
                    if ui.add_sized(
                        [200.0, 36.0],
                        egui::Button::new(RichText::new("🔍  Scan for Duplicates").size(15.0)),
                    ).clicked() {
                        self.start_scan(ctx);
                    }
                });
                if !can_scan {
                    ui.label(
                        RichText::new("Add at least one folder to scan.")
                            .color(Color32::GRAY),
                    );
                }
            });
        });
    }

    // ── Scanning progress view ────────────────────────────────────────────

    fn draw_scanning(&mut self, ui: &mut Ui) {
        ui.add_space(60.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("🔍 Scanning…").size(22.0).strong());
            ui.add_space(16.0);

            let frac = if self.progress_total > 0 {
                self.progress_current as f32 / self.progress_total as f32
            } else {
                0.0
            };

            ui.add(
                egui::ProgressBar::new(frac)
                    .animate(true)
                    .desired_width(500.0),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(&self.progress_stage)
                    .color(Color32::GRAY)
                    .size(12.0),
            );
            ui.add_space(24.0);
            if ui.button("⛔  Cancel").clicked() {
                self.view = View::Setup;
                self.msg_rx = None;
            }
        });
    }

    // ── Results view ─────────────────────────────────────────────────────

    fn draw_results(&mut self, ui: &mut Ui, _ctx: &egui::Context) {
        // ── Top toolbar ──
        egui::TopBottomPanel::top("results_toolbar").show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(4.0);

                // Filter by type
                ui.label("Show:");
                for (label, fk) in [
                    ("All",     FilterKind::All),
                    ("Exact",   FilterKind::Exact),
                    ("Image",   FilterKind::Image),
                    ("Video",   FilterKind::Video),
                ] {
                    let active = self.filter_kind == fk;
                    if ui.selectable_label(active, label).clicked() {
                        self.filter_kind = fk;
                    }
                }

                ui.separator();

                // Sort
                ui.label("Sort:");
                egui::ComboBox::from_id_salt("sort")
                    .selected_text(match self.sort_mode {
                        SortMode::WastedDesc => "Largest waste ↓",
                        SortMode::WastedAsc  => "Smallest waste ↓",
                        SortMode::CopiesDesc => "Most copies ↓",
                        SortMode::NameAsc    => "Name A→Z",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sort_mode, SortMode::WastedDesc, "Largest waste ↓");
                        ui.selectable_value(&mut self.sort_mode, SortMode::WastedAsc, "Smallest waste ↓");
                        ui.selectable_value(&mut self.sort_mode, SortMode::CopiesDesc, "Most copies ↓");
                        ui.selectable_value(&mut self.sort_mode, SortMode::NameAsc, "Name A→Z");
                    });

                ui.separator();

                // Search
                ui.label("Filter:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter_text)
                        .hint_text("filename or path…")
                        .desired_width(180.0),
                );
                if !self.filter_text.is_empty() && ui.small_button("✕").clicked() {
                    self.filter_text.clear();
                }

                ui.separator();
                ui.checkbox(&mut self.show_only_marked, "Marked only");

                ui.separator();

                // Bulk auto-mark buttons
                if ui.button("Keep newest").clicked() {
                    for g in &mut self.groups { g.auto_keep_newest(); }
                }
                if ui.button("Keep oldest").clicked() {
                    for g in &mut self.groups { g.auto_keep_oldest(); }
                }
                if ui.button("Clear marks").clicked() {
                    for g in &mut self.groups { g.clear_marks(); }
                }

                ui.separator();

                // Expand / collapse all
                if ui.small_button("⊞ Expand all").clicked() {
                    for g in &mut self.groups { g.collapsed = false; }
                }
                if ui.small_button("⊟ Collapse all").clicked() {
                    for g in &mut self.groups { g.collapsed = true; }
                }
            });
        });

        // ── Left: group list | Right: action panel ──
        egui::SidePanel::right("action_panel")
            .resizable(true)
            .default_width(260.0)
            .min_width(200.0)
            .max_width(340.0)
            .show_inside(ui, |ui| {
                self.draw_action_panel(ui);
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.draw_group_list(ui);
        });
    }

    // ── Group list ───────────────────────────────────────────────────────

    fn draw_group_list(&mut self, ui: &mut Ui) {
        let filter_text = self.filter_text.to_lowercase();

        // Build sorted index
        let mut indices: Vec<usize> = (0..self.groups.len())
            .filter(|&i| {
                let g = &self.groups[i];
                // Type filter
                let kind_ok = match &self.filter_kind {
                    FilterKind::All   => true,
                    FilterKind::Exact => g.kind == MatchKind::ExactHash,
                    FilterKind::Image => g.kind == MatchKind::PerceptualImage,
                    FilterKind::Video => g.kind == MatchKind::PerceptualVideo,
                };
                // Text filter
                let text_ok = filter_text.is_empty() || g.files.iter().any(|f| {
                    f.path.to_string_lossy().to_lowercase().contains(&filter_text)
                });
                // Marked-only filter
                let mark_ok = !self.show_only_marked || g.files.iter().any(|f| {
                    f.mark != FileMark::None
                });
                kind_ok && text_ok && mark_ok
            })
            .collect();

        indices.sort_by(|&a, &b| {
            let ga = &self.groups[a];
            let gb = &self.groups[b];
            match self.sort_mode {
                SortMode::WastedDesc => gb.wasted_bytes().cmp(&ga.wasted_bytes()),
                SortMode::WastedAsc  => ga.wasted_bytes().cmp(&gb.wasted_bytes()),
                SortMode::CopiesDesc => gb.files.len().cmp(&ga.files.len()),
                SortMode::NameAsc    => {
                    let na = ga.files.first().map(|f| f.path.to_string_lossy().into_owned()).unwrap_or_default();
                    let nb = gb.files.first().map(|f| f.path.to_string_lossy().into_owned()).unwrap_or_default();
                    na.cmp(&nb)
                }
            }
        });

        let total_shown = indices.len();

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if total_shown == 0 {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.label(RichText::new("No groups match your filter.").color(Color32::GRAY).size(14.0));
                    });
                    return;
                }

                for idx in indices {
                    let g = &mut self.groups[idx];
                    draw_group_card(ui, g);
                    ui.add_space(4.0);
                }
            });
    }

    // ── Action panel ─────────────────────────────────────────────────────

    fn draw_action_panel(&mut self, ui: &mut Ui) {
        // Fix width so buttons don't push the panel wider each frame
        ui.set_width(ui.available_width());
        ui.add_space(8.0);
        section(ui, "🗑 Actions");

        // Stats
        let stats = ScanStats::from_groups(self.files_scanned, &self.groups);
        egui::Grid::new("action_stats")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Marked for delete:");
                ui.label(RichText::new(format!(
                    "{} files ({})",
                    stats.marked_delete_count,
                    format_size(stats.marked_delete_bytes)
                )).strong());
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // Delete marked
        let has_marked = stats.marked_delete_count > 0;
        ui.add_enabled_ui(has_marked, |ui| {
            if ui.add_sized(
                [ui.available_width(), 32.0],
                egui::Button::new(RichText::new("🗑  Delete marked files").color(Color32::RED)),
            ).clicked() {
                self.confirm_dialog = Some(ConfirmDialog {
                    title: "Confirm Deletion".into(),
                    body: format!(
                        "Permanently delete {} file(s) totalling {}?\n\nThis cannot be undone.",
                        stats.marked_delete_count,
                        format_size(stats.marked_delete_bytes)
                    ),
                    on_confirm: ConfirmAction::DeleteMarked,
                });
            }
        });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);

        // Quarantine
        ui.label(RichText::new("📦 Move marked to folder:").size(12.0));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.action_quarantine_path)
                .hint_text("Quarantine path…")
                .desired_width(f32::INFINITY));
            if ui.button("📂").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.action_quarantine_path = p.to_string_lossy().to_string();
                }
            }
        });
        ui.add_space(4.0);
        ui.add_enabled_ui(
            has_marked && !self.action_quarantine_path.is_empty(),
            |ui| {
                if ui.add_sized(
                    [ui.available_width(), 28.0],
                    egui::Button::new("📦  Move to quarantine"),
                ).clicked() {
                    self.confirm_dialog = Some(ConfirmDialog {
                        title: "Confirm Quarantine".into(),
                        body: format!(
                            "Move {} file(s) to {}?",
                            stats.marked_delete_count,
                            self.action_quarantine_path
                        ),
                        on_confirm: ConfirmAction::QuarantineMarked,
                    });
                }
            },
        );

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);

        // Export CSV
        if ui.add_sized(
            [ui.available_width(), 28.0],
            egui::Button::new("📄  Export CSV report"),
        ).clicked() {
            if let Some(dest) = rfd::FileDialog::new()
                .set_file_name("dupehunter-report.csv")
                .save_file()
            {
                match export_csv(&self.groups, &dest) {
                    Ok(_) => self.action_log.push((true, format!("Exported to {}", dest.display()))),
                    Err(e) => self.action_log.push((false, format!("Export failed: {e}"))),
                }
            }
        }

        // Re-scan button
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        if ui.add_sized(
            [ui.available_width(), 28.0],
            egui::Button::new("🔄  New scan"),
        ).clicked() {
            self.groups.clear();
            self.view = View::Setup;
        }

        // Action log
        if !self.action_log.is_empty() {
            ui.add_space(12.0);
            ui.separator();
            section(ui, "📋 Log");
            egui::ScrollArea::vertical()
                .id_salt("log_scroll")
                .max_height(200.0)
                .show(ui, |ui| {
                    for (ok, msg) in self.action_log.iter().rev().take(50) {
                        let color = if *ok { Color32::GREEN } else { Color32::RED };
                        ui.label(RichText::new(msg).color(color).size(11.0));
                    }
                });
        }
    }

    // ── Confirm dialog ───────────────────────────────────────────────────

    fn draw_confirm_dialog(&mut self, ctx: &egui::Context) {
        let dialog = self.confirm_dialog.as_ref().unwrap();
        let title = dialog.title.clone();
        let body = dialog.body.clone();
        let action = dialog.on_confirm.clone();

        let mut open = true;
        egui::Window::new(&title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.label(&body);
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("  Cancel  ")).clicked() {
                        self.confirm_dialog = None;
                    }
                    ui.add_space(16.0);
                    let confirm_label = match action {
                        ConfirmAction::DeleteMarked    => "🗑  Delete",
                        ConfirmAction::QuarantineMarked => "📦  Move",
                    };
                    if ui.add(egui::Button::new(
                        RichText::new(confirm_label).color(Color32::RED)
                    )).clicked() {
                        self.confirm_dialog = None;
                        self.execute_action(action);
                    }
                });
            });

        if !open {
            self.confirm_dialog = None;
        }
    }

    // ── Scan launcher ────────────────────────────────────────────────────

    fn start_scan(&mut self, ctx: &egui::Context) {
        let opts = self.opts.clone();
        let img_thresh = self.image_threshold;
        let vid_thresh = self.video_threshold;
        let vid_frames = self.video_frame_samples;

        let (tx, rx) = bounded::<ScanMsg>(128);
        self.msg_rx = Some(rx);
        self.view = View::Scanning;
        self.progress_stage = "Starting…".into();
        self.progress_current = 0;
        self.progress_total = 1;

        let ctx2 = ctx.clone();
        thread::spawn(move || {
            run_scan(opts, img_thresh, vid_thresh, vid_frames, tx);
            ctx2.request_repaint();
        });
    }

    // ── Execute confirmed action ──────────────────────────────────────────

    fn execute_action(&mut self, action: ConfirmAction) {
        let marked_paths: Vec<PathBuf> = self.groups
            .iter()
            .flat_map(|g| &g.files)
            .filter(|f| f.mark == FileMark::Delete)
            .map(|f| f.path.clone())
            .collect();

        let kind = match action {
            ConfirmAction::DeleteMarked => ActionKind::Delete,
            ConfirmAction::QuarantineMarked => {
                ActionKind::Quarantine(PathBuf::from(&self.action_quarantine_path))
            }
        };

        let (ok, fail, errors) = execute_bulk(&marked_paths, &kind);

        // Remove successfully actioned files from groups
        let actioned: std::collections::HashSet<PathBuf> = marked_paths[..ok].iter().cloned().collect();
        for g in &mut self.groups {
            g.files.retain(|f| !actioned.contains(&f.path));
        }
        self.groups.retain(|g| g.files.len() > 1);

        // Log results
        let verb = match kind { ActionKind::Delete => "Deleted", _ => "Moved" };
        self.action_log.push((
            fail == 0,
            format!("{} {} file(s){}", verb, ok,
                if fail > 0 { format!(", {} failed", fail) } else { String::new() })
        ));
        for e in errors {
            self.action_log.push((false, e));
        }
    }
}

// ── Group card widget ──────────────────────────────────────────────────────

fn draw_group_card(ui: &mut Ui, group: &mut DuplicateGroup) {
    let frame_color = egui::Color32::from_gray(35);
    egui::Frame::none()
        .fill(frame_color)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            // ── Group header ──
            ui.horizontal(|ui| {
                // Collapse toggle
                let arrow = if group.collapsed { "▶" } else { "▼" };
                if ui.small_button(arrow).clicked() {
                    group.collapsed = !group.collapsed;
                }

                // Match kind badge
                let badge_text = format!("{} {}", group.kind.icon(), group.kind.label());
                let (r,g,b) = group.kind.color_rgb();
                ui.label(RichText::new(badge_text).color(Color32::from_rgb(r,g,b)).size(12.0).strong());

                // Similarity
                if group.similarity < 1.0 {
                    ui.label(
                        RichText::new(format!("{:.0}% similar", group.similarity * 100.0))
                            .color(Color32::GRAY).size(11.0),
                    );
                }

                ui.separator();

                // Representative filename
                if let Some(f) = group.files.first() {
                    ui.label(
                        RichText::new(
                            f.path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                        )
                        .size(13.0).strong(),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Wasted space
                    ui.label(
                        RichText::new(format!("−{}", group.wasted_str()))
                            .color(Color32::RED).size(12.0),
                    );
                    // Copy count
                    ui.label(
                        RichText::new(format!("{} copies", group.files.len()))
                            .color(Color32::YELLOW).size(12.0),
                    );
                    // Quick mark buttons
                    if ui.small_button("Keep newest").clicked() { group.auto_keep_newest(); }
                    if ui.small_button("Keep oldest").clicked() { group.auto_keep_oldest(); }
                    if ui.small_button("Clear").clicked() { group.clear_marks(); }
                });
            });

            // ── File rows (collapsible) ──
            if !group.collapsed {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(2.0);

                for file in &mut group.files {
                    draw_file_row(ui, file);
                }
            }
        });
}

fn draw_file_row(ui: &mut Ui, file: &mut grouper::DupeFile) {
    let row_color = match file.mark {
        FileMark::Keep       => Color32::from_rgba_unmultiplied(30, 100, 30, 180),
        FileMark::Delete     => Color32::from_rgba_unmultiplied(100, 20, 20, 180),
        FileMark::Quarantine => Color32::from_rgba_unmultiplied(80, 50, 10, 180),
        FileMark::None       => Color32::from_gray(28),
    };

    egui::Frame::none()
        .fill(row_color)
        .rounding(4.0)
        .inner_margin(egui::Margin { left: 8.0, right: 8.0, top: 4.0, bottom: 4.0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // File type icon
                ui.label(RichText::new(file.kind.icon()).size(16.0));

                // Filename + path
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(
                            file.path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                        ).size(12.0).strong(),
                    );
                    ui.label(
                        RichText::new(file.path.to_string_lossy())
                            .size(10.0)
                            .color(Color32::GRAY)
                            .monospace(),
                    );
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Mark buttons
                    let keep_active = file.mark == FileMark::Keep;
                    let del_active  = file.mark == FileMark::Delete;

                    if ui.add(egui::SelectableLabel::new(
                        del_active,
                        RichText::new("🗑 Delete").color(Color32::RED).size(11.0),
                    )).clicked() {
                        file.mark = if del_active { FileMark::None } else { FileMark::Delete };
                    }
                    if ui.add(egui::SelectableLabel::new(
                        keep_active,
                        RichText::new("✓ Keep").color(Color32::GREEN).size(11.0),
                    )).clicked() {
                        file.mark = if keep_active { FileMark::None } else { FileMark::Keep };
                    }

                    // Metadata
                    ui.label(
                        RichText::new(file.modified_str())
                            .color(Color32::GRAY).size(10.0),
                    );
                    ui.label(
                        RichText::new(file.size_str())
                            .color(Color32::LIGHT_GRAY).size(11.0)
                            .monospace(),
                    );
                });
            });
        });

    ui.add_space(2.0);
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn section(ui: &mut Ui, label: &str) {
    ui.label(RichText::new(label).size(13.0).strong().color(Color32::WHITE));
    ui.separator();
    ui.add_space(4.0);
}

// ── Entry point ────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DupeHunter")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([800.0, 500.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(
                    include_bytes!("../assets/icon.png")
                ).unwrap_or_default()
            ),
        ..Default::default()
    };

    eframe::run_native(
        "DupeHunter",
        options,
        Box::new(|cc| {
            // Slightly larger default font
            let mut style = (*cc.egui_ctx.style()).clone();
            style.text_styles.insert(
                egui::TextStyle::Body,
                FontId::proportional(13.5),
            );
            cc.egui_ctx.set_style(style);

            // Dark theme
            cc.egui_ctx.set_visuals(egui::Visuals::dark());

            Ok(Box::new(DupeHunter::default()))
        }),
    )
}
