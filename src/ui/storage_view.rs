use std::cmp::Reverse;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use eframe::egui::{
    self, Align, Frame, Id, Label, Layout, Margin, Modal, ProgressBar, RichText, ScrollArea, Sense,
    Ui, vec2,
};

use super::{
    ACCENT, DANGER, SUCCESS, WARNING, badge, banner, card, danger_button, fraction_bar,
    page_header, primary_button, tri_checkbox,
};
use crate::storage::categories::{CategoryDef, Safety, builtin_categories, custom_category};
use crate::storage::cleaner::{CleanEvent, CleanReport, CleanTarget, DeleteMode, start_clean};
use crate::storage::safety::validate_custom_root;
use crate::storage::scanner::{ScanEvent, ScanProgress, load_children, start_scan};
use crate::storage::tree::{Category, Children, Node, ScanState, Selected, remove_path};
use crate::util::{FULL_DISK_ACCESS_URL, display_path, format_size, open_url, reveal_in_finder};

/// Rows shown per list before a "Show more" button.
const PAGE: usize = 150;
const SIZE_COLUMN: f32 = 76.0;
const INDENT: f32 = 20.0;

struct Cleaning {
    rx: Receiver<CleanEvent>,
    done: usize,
    total: usize,
    current: String,
}

/// Hands folder-expansion requests to the scanner.
struct Loader<'a> {
    tx: &'a Sender<ScanEvent>,
    ctx: &'a egui::Context,
}

impl Loader<'_> {
    fn load(&self, node: &mut Node) {
        node.children = Children::Loading;
        load_children(
            node.path.clone(),
            node.safety,
            self.tx.clone(),
            self.ctx.clone(),
        );
    }
}

pub struct StorageView {
    ctx: egui::Context,
    categories: Vec<Category>,
    tx: Sender<ScanEvent>,
    rx: Receiver<ScanEvent>,
    progress: Option<Arc<ScanProgress>>,
    last_scan: Option<(Duration, bool)>,
    mode: DeleteMode,
    confirm: Option<Vec<CleanTarget>>,
    cleaning: Option<Cleaning>,
    report: Option<CleanReport>,
    error: Option<String>,
    hide_empty: bool,
}

impl StorageView {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        Self {
            ctx,
            categories: builtin_categories()
                .into_iter()
                .map(Category::new)
                .collect(),
            tx,
            rx,
            progress: None,
            last_scan: None,
            mode: DeleteMode::Trash,
            confirm: None,
            cleaning: None,
            report: None,
            error: None,
            hide_empty: true,
        }
    }

    fn scanning(&self) -> bool {
        self.progress.is_some()
    }

    fn busy(&self) -> bool {
        self.scanning() || self.cleaning.is_some()
    }

    fn scan(&mut self, indices: Vec<usize>) {
        let mut jobs: Vec<(usize, CategoryDef)> = Vec::new();
        for index in indices {
            let category = &mut self.categories[index];
            category.items.clear();
            category.state = ScanState::Queued;
            category.unreadable = 0;
            jobs.push((index, category.def.clone()));
        }
        self.report = None;
        self.progress = Some(start_scan(jobs, self.tx.clone(), self.ctx.clone()));
    }

    fn scan_all(&mut self) {
        self.scan((0..self.categories.len()).collect());
    }

    fn add_custom_folder(&mut self) {
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Choose a folder to review")
            .set_directory(crate::util::home_dir())
            .pick_folder()
        else {
            return;
        };
        match validate_custom_root(&folder) {
            Ok(root) => {
                let duplicate = self.categories.iter().any(|c| {
                    c.def.custom && c.def.locations.first().is_some_and(|l| l.path == root)
                });
                if duplicate {
                    self.error = Some(format!("{} is already in the list", display_path(&root)));
                    return;
                }
                let mut category = Category::new(custom_category(root));
                category.expanded = true;
                self.categories.push(category);
                self.scan(vec![self.categories.len() - 1]);
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Everything currently ticked, with nested duplicates removed.
    fn selection(&self) -> Vec<CleanTarget> {
        let mut targets = Vec::new();
        for (index, category) in self.categories.iter().enumerate() {
            let mut picked: Vec<Selected> = Vec::new();
            category.collect_selected(&mut picked);
            targets.extend(picked.into_iter().map(|s| CleanTarget {
                path: s.path,
                size: s.size,
                category: index,
                safety: s.safety,
                force_permanent: category.def.permanent_only,
            }));
        }
        // A custom folder can overlap a built-in location; delete each path once.
        targets.sort_by(|a, b| a.path.cmp(&b.path));
        let mut unique: Vec<CleanTarget> = Vec::with_capacity(targets.len());
        for target in targets {
            if !unique
                .last()
                .is_some_and(|kept| target.path.starts_with(&kept.path))
            {
                unique.push(target);
            }
        }
        unique.sort_by_key(|t| Reverse(t.size));
        unique
    }

    /// One-line status for the sidebar.
    pub fn summary(&self) -> String {
        if self.scanning() {
            return "Scanning…".to_owned();
        }
        if self.last_scan.is_none() {
            return "Not scanned yet".to_owned();
        }
        let selected: u64 = self.categories.iter().map(Category::selected_size).sum();
        if selected > 0 {
            format!("{} selected", format_size(selected))
        } else {
            let found: u64 = self.categories.iter().map(Category::total_size).sum();
            format!("{} found", format_size(found))
        }
    }

    fn poll(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                ScanEvent::CategoryStarted(index) => {
                    if let Some(category) = self.categories.get_mut(index) {
                        category.state = ScanState::Scanning;
                    }
                }
                ScanEvent::CategoryDone {
                    index,
                    items,
                    unreadable,
                    missing,
                } => {
                    if let Some(category) = self.categories.get_mut(index) {
                        category.items = items;
                        category.unreadable = unreadable;
                        category.missing = missing;
                        category.state = ScanState::Done;
                    }
                }
                ScanEvent::ScanFinished { cancelled, elapsed } => {
                    self.progress = None;
                    self.last_scan = Some((elapsed, cancelled));
                    for category in &mut self.categories {
                        if matches!(category.state, ScanState::Queued | ScanState::Scanning) {
                            category.state = ScanState::Idle;
                        }
                    }
                }
                ScanEvent::ChildrenLoaded { path, result } => {
                    let node = self.categories.iter_mut().find_map(|c| c.find_mut(&path));
                    if let Some(node) = node {
                        node.children = match result {
                            Ok(mut kids) => {
                                let inherited = node.selected && !node.protected;
                                for kid in &mut kids {
                                    kid.set_selected(inherited);
                                }
                                Children::Loaded(kids)
                            }
                            Err(e) => Children::Failed(e),
                        };
                    }
                }
            }
        }

        let mut finished = None;
        if let Some(cleaning) = &mut self.cleaning {
            while let Ok(event) = cleaning.rx.try_recv() {
                match event {
                    CleanEvent::Progress {
                        done,
                        total,
                        current,
                    } => {
                        cleaning.done = done;
                        cleaning.total = total;
                        cleaning.current = current;
                    }
                    CleanEvent::Finished(report) => finished = Some(report),
                }
            }
        }
        if let Some(report) = finished {
            for target in &report.removed {
                if let Some(category) = self.categories.get_mut(target.category) {
                    remove_path(&mut category.items, &target.path);
                }
            }
            self.cleaning = None;
            self.report = Some(report);
        }
    }

    fn start_cleaning(&mut self, targets: Vec<CleanTarget>) {
        let allowed_roots = self
            .categories
            .iter()
            .flat_map(|c| c.def.allowed_roots().map(Path::to_path_buf))
            .collect();
        let (tx, rx) = channel();
        self.cleaning = Some(Cleaning {
            rx,
            done: 0,
            total: targets.len(),
            current: String::new(),
        });
        self.report = None;
        start_clean(targets, self.mode, allowed_roots, tx, self.ctx.clone());
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        self.poll();
        if self.busy() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }

        let selection = self.selection();
        egui::Panel::bottom("storage_actions")
            .frame(Frame::new().inner_margin(Margin {
                top: 10,
                ..Default::default()
            }))
            .show(ui, |ui| self.action_bar(ui, &selection));

        let scanning = self.scanning();
        let busy = self.busy();
        page_header(
            ui,
            "Storage",
            "Find caches, logs and leftovers, look inside them, then remove what you don't need.",
            |ui| {
                if scanning {
                    if ui.button("Cancel").clicked()
                        && let Some(progress) = &self.progress
                    {
                        progress.cancel.store(true, Ordering::Relaxed);
                    }
                } else {
                    let label = if self.last_scan.is_some() {
                        "⟳ Rescan"
                    } else {
                        "Scan"
                    };
                    if ui.add_enabled(!busy, primary_button(label)).clicked() {
                        self.scan_all();
                    }
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("📂 Add Folder…"))
                    .on_hover_text("Review the contents of any folder, e.g. a projects folder or an external drive")
                    .clicked()
                {
                    self.add_custom_folder();
                }
            },
        );

        self.status_area(ui);

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let loader = Loader {
                    tx: &self.tx,
                    ctx: &self.ctx,
                };
                let mut remove = None;
                let mut hidden = 0;
                for (index, category) in self.categories.iter_mut().enumerate() {
                    let empty = category.state == ScanState::Done && category.items.is_empty();
                    if self.hide_empty && empty && !category.def.custom {
                        hidden += 1;
                        continue;
                    }
                    if category_card(ui, category, &loader, !busy) {
                        remove = Some(index);
                    }
                    ui.add_space(6.0);
                }
                if let Some(index) = remove {
                    self.categories.remove(index);
                }
                if hidden > 0 || !self.hide_empty {
                    ui.horizontal(|ui| {
                        let text = if self.hide_empty {
                            format!("{hidden} empty categories hidden")
                        } else {
                            "Showing empty categories".to_owned()
                        };
                        ui.label(
                            RichText::new(text)
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                        let toggle = if self.hide_empty { "Show" } else { "Hide" };
                        if ui.small_button(toggle).clicked() {
                            self.hide_empty = !self.hide_empty;
                        }
                    });
                }
            });

        self.dialogs(ui);
    }

    fn status_area(&mut self, ui: &mut Ui) {
        if let Some(progress) = &self.progress {
            card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    let current = progress
                        .current
                        .lock()
                        .map(|c| c.clone())
                        .unwrap_or_default();
                    ui.label(RichText::new("Scanning").strong());
                    ui.add(
                        Label::new(RichText::new(current).color(ui.visuals().weak_text_color()))
                            .truncate(),
                    );
                });
                ui.label(format!(
                    "{} files checked · {} measured",
                    progress.files.load(Ordering::Relaxed),
                    format_size(progress.bytes.load(Ordering::Relaxed))
                ));
            });
            ui.add_space(6.0);
        } else if let Some((elapsed, cancelled)) = self.last_scan {
            let found: u64 = self.categories.iter().map(Category::total_size).sum();
            let text = format!(
                "{} {} reclaimable space in {:.1}s",
                if cancelled {
                    "Scan cancelled. Found"
                } else {
                    "Found"
                },
                format_size(found),
                elapsed.as_secs_f32()
            );
            ui.label(RichText::new(text).color(ui.visuals().weak_text_color()));
            ui.add_space(4.0);
        } else {
            card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Ready to scan").strong().size(16.0));
                ui.label("MacCleaner measures the locations below. Nothing is deleted until you pick items and confirm. Expand any item to look inside before you decide.");
            });
            ui.add_space(6.0);
        }

        let unreadable: u64 = self.categories.iter().map(|c| c.unreadable).sum();
        if unreadable > 0 && !self.scanning() {
            Frame::new()
                .fill(WARNING.gamma_multiply(0.12))
                .corner_radius(6)
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "⚠ {unreadable} folders couldn't be read. Grant Full Disk Access to MacCleaner to scan Mail, Trash and backups."
                            ))
                            .color(WARNING),
                        );
                        if ui.small_button("Open Settings").clicked() {
                            open_url(FULL_DISK_ACCESS_URL);
                        }
                    });
                });
            ui.add_space(6.0);
        }

        if let Some(report) = &self.report {
            let mut parts = Vec::new();
            if report.freed > 0 {
                parts.push(format!("Freed {}.", format_size(report.freed)));
            }
            if report.trashed > 0 {
                parts.push(format!(
                    "Moved {} to the Trash; empty the Trash to free the space.",
                    format_size(report.trashed)
                ));
            }
            if !report.failed.is_empty() {
                parts.push(format!(
                    "{} items couldn't be removed.",
                    report.failed.len()
                ));
            }
            if parts.is_empty() {
                parts.push("Nothing was removed.".to_owned());
            }
            let color = if report.failed.is_empty() {
                SUCCESS
            } else {
                WARNING
            };
            if banner(ui, color, &parts.join(" ")) {
                self.report = None;
            } else if !report.failed.is_empty() {
                egui::CollapsingHeader::new("Show items that failed")
                    .id_salt("failed_items")
                    .show(ui, |ui| {
                        for (path, error) in &report.failed {
                            ui.label(
                                RichText::new(format!("{}: {error}", display_path(path)))
                                    .small()
                                    .color(DANGER),
                            );
                        }
                    });
            }
            ui.add_space(6.0);
        }

        if let Some(error) = &self.error {
            if banner(ui, DANGER, error) {
                self.error = None;
            }
            ui.add_space(6.0);
        }
    }

    fn action_bar(&mut self, ui: &mut Ui, selection: &[CleanTarget]) {
        let total: u64 = selection.iter().map(|t| t.size).sum();
        let busy = self.busy();
        let has_items = self.categories.iter().any(|c| !c.items.is_empty());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("To be freed")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                ui.label(
                    RichText::new(format_size(total))
                        .size(22.0)
                        .strong()
                        .color(ACCENT),
                );
                ui.label(
                    RichText::new(format!("{} items selected", selection.len()))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
            ui.add_space(12.0);
            ui.separator();
            ui.vertical(|ui| {
                ui.radio_value(&mut self.mode, DeleteMode::Trash, "Move to Trash")
                    .on_hover_text("Recoverable. Space is freed when you empty the Trash.");
                ui.radio_value(&mut self.mode, DeleteMode::Permanent, "Delete permanently")
                    .on_hover_text("Frees space immediately. Can't be undone.");
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = match self.mode {
                    DeleteMode::Trash => "Clean Selected…",
                    DeleteMode::Permanent => "Delete Selected…",
                };
                if ui
                    .add_enabled(
                        !busy && !selection.is_empty(),
                        danger_button(label).min_size(vec2(140.0, 32.0)),
                    )
                    .clicked()
                {
                    self.confirm = Some(selection.to_vec());
                }
                ui.add_space(8.0);
                if ui
                    .add_enabled(!busy && has_items, egui::Button::new("Deselect All"))
                    .clicked()
                {
                    for category in &mut self.categories {
                        category.set_selected(false);
                    }
                }
                if ui
                    .add_enabled(!busy && has_items, egui::Button::new("Select All"))
                    .on_hover_text("Selects every item, including ones marked Review")
                    .clicked()
                {
                    for category in &mut self.categories {
                        category.set_selected(true);
                    }
                }
                if ui
                    .add_enabled(!busy && has_items, egui::Button::new("Select Safe"))
                    .on_hover_text("Selects only items apps re-create automatically")
                    .clicked()
                {
                    for category in &mut self.categories {
                        for item in &mut category.items {
                            item.set_selected(item.safety == Safety::Safe);
                        }
                    }
                }
            });
        });
    }

    fn dialogs(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if let Some(targets) = &self.confirm {
            let total: u64 = targets.iter().map(|t| t.size).sum();
            let review = targets
                .iter()
                .filter(|t| t.safety == Safety::Review)
                .count();
            let in_trash = targets.iter().filter(|t| t.force_permanent).count();
            let permanent = self.mode == DeleteMode::Permanent;
            let mut confirmed = false;
            let mut cancelled = false;
            let response = Modal::new(Id::unique("confirm_clean")).show(&ctx, |ui| {
                ui.set_width(560.0);
                ui.heading(if permanent { "Delete permanently?" } else { "Move to Trash?" });
                ui.label(format!("{} items · {}", targets.len(), format_size(total)));
                ui.add_space(4.0);
                if permanent {
                    ui.label(RichText::new("These items will be deleted immediately. This can't be undone.").color(DANGER));
                } else {
                    ui.label("You can put them back from the Trash. The space is freed once the Trash is emptied.");
                    if in_trash > 0 {
                        ui.label(RichText::new(format!("{in_trash} items already in the Trash will be deleted permanently.")).color(WARNING));
                    }
                }
                if review > 0 {
                    ui.label(RichText::new(format!("⚠ {review} items are marked Review (personal files or backups). Check them below.")).color(WARNING));
                }
                ui.add_space(6.0);
                card(ui).show(ui, |ui| {
                    ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for target in targets {
                            ui.horizontal(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(RichText::new(format_size(target.size)).monospace());
                                    if target.safety == Safety::Review {
                                        badge(ui, "Review", WARNING);
                                    }
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add(Label::new(display_path(&target.path)).truncate());
                                    });
                                });
                            });
                        }
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let label = if permanent { "Delete Permanently" } else { "Move to Trash" };
                        confirmed = ui.add(danger_button(label)).clicked();
                        cancelled = ui.button("Cancel").clicked();
                    });
                });
            });
            if confirmed {
                let targets = self.confirm.take().unwrap_or_default();
                self.start_cleaning(targets);
            } else if cancelled || response.should_close() {
                self.confirm = None;
            }
        }

        if let Some(cleaning) = &self.cleaning {
            Modal::new(Id::unique("cleaning")).show(&ctx, |ui| {
                ui.set_width(420.0);
                ui.heading("Cleaning…");
                let fraction = if cleaning.total == 0 {
                    0.0
                } else {
                    cleaning.done as f32 / cleaning.total as f32
                };
                ui.add(ProgressBar::new(fraction).show_percentage().animate(true));
                ui.add(Label::new(RichText::new(&cleaning.current).small()).truncate());
            });
        }
    }
}

fn uniform_safety(def: &CategoryDef) -> Option<Safety> {
    let first = def.locations.first()?.safety;
    def.locations
        .iter()
        .all(|l| l.safety == first)
        .then_some(first)
}

/// Draws one category. Returns true if the user removed it (custom folders only).
fn category_card(ui: &mut Ui, category: &mut Category, loader: &Loader, interactive: bool) -> bool {
    let mut remove = false;
    let total = category.total_size();
    let selected = category.selected_size();
    let safety = uniform_safety(&category.def);
    card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let can_expand = !category.items.is_empty();
            let arrow = if category.expanded && can_expand {
                "⏷"
            } else {
                "⏵"
            };
            if ui
                .add_enabled(can_expand, egui::Button::new(arrow).frame(false))
                .clicked()
            {
                category.expanded = !category.expanded;
            }
            if let Some(on) = tri_checkbox(ui, category.check(), interactive && can_expand) {
                category.set_selected(on);
            }
            ui.label(RichText::new(category.def.icon).size(20.0));
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let title = ui.add(
                        Label::new(RichText::new(&category.def.name).strong().size(15.0))
                            .sense(Sense::click()),
                    );
                    if title.clicked() && can_expand {
                        category.expanded = !category.expanded;
                    }
                    match safety {
                        Some(Safety::Safe) => badge(ui, "Safe", SUCCESS),
                        Some(Safety::Review) => badge(ui, "Review", WARNING),
                        None => badge(ui, "Mixed", ACCENT),
                    }
                    .on_hover_text(match safety {
                        Some(Safety::Safe) => "Apps and tools re-create these automatically.",
                        Some(Safety::Review) => "Personal files or backups. Look before deleting.",
                        None => "Some items here are marked Review.",
                    });
                    if category.def.permanent_only {
                        badge(ui, "Deleted permanently", DANGER);
                    }
                });
                ui.add(
                    Label::new(
                        RichText::new(&category.def.description)
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    )
                    .truncate(),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if category.def.custom
                    && interactive
                    && ui
                        .small_button("✕")
                        .on_hover_text("Remove this folder from the list")
                        .clicked()
                {
                    remove = true;
                }
                ui.vertical(|ui| {
                    ui.with_layout(Layout::top_down(Align::Max), |ui| match category.state {
                        ScanState::Idle if category.items.is_empty() => {
                            ui.label(RichText::new("—").size(17.0).strong());
                            ui.label(
                                RichText::new("Not scanned")
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                        }
                        ScanState::Queued => {
                            ui.label(
                                RichText::new("Waiting…").color(ui.visuals().weak_text_color()),
                            );
                        }
                        ScanState::Scanning => {
                            ui.spinner();
                        }
                        _ => {
                            ui.label(RichText::new(format_size(total)).size(17.0).strong());
                            let detail = if category.missing {
                                "Not found on this Mac".to_owned()
                            } else if selected > 0 {
                                format!("{} selected", format_size(selected))
                            } else {
                                format!("{} items", category.items.len())
                            };
                            let color = if selected > 0 {
                                ACCENT
                            } else {
                                ui.visuals().weak_text_color()
                            };
                            ui.label(RichText::new(detail).small().color(color));
                        }
                    });
                });
            });
        });

        if category.expanded && !category.items.is_empty() {
            ui.separator();
            let id = Id::unique(("category_rows", &category.def.name, category.def.custom));
            paged_rows(ui, id, &mut category.items, |ui, node| {
                node_rows(ui, node, 0, total, loader, interactive);
            });
        }
    });
    remove
}

/// Shows the first rows of a list with a button that reveals more.
fn paged_rows(ui: &mut Ui, id: Id, nodes: &mut [Node], mut row: impl FnMut(&mut Ui, &mut Node)) {
    let limit = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(PAGE);
    for node in nodes.iter_mut().take(limit) {
        row(ui, node);
    }
    if nodes.len() > limit {
        let rest = &nodes[limit..];
        let rest_size: u64 = rest.iter().map(|n| n.size).sum();
        ui.horizontal(|ui| {
            ui.add_space(INDENT * 2.0);
            let text = format!(
                "Show {} more ({} smaller items, {})",
                rest.len().min(PAGE),
                rest.len(),
                format_size(rest_size)
            );
            if ui.small_button(text).clicked() {
                ui.data_mut(|d| d.insert_temp(id, limit + PAGE));
            }
        });
    }
}

fn node_rows(
    ui: &mut Ui,
    node: &mut Node,
    depth: usize,
    parent_size: u64,
    loader: &Loader,
    interactive: bool,
) {
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * INDENT);
        if node.is_dir {
            let arrow = if node.expanded { "⏷" } else { "⏵" };
            if ui
                .add(egui::Button::new(arrow).frame(false))
                .on_hover_text("Look inside")
                .clicked()
            {
                node.expanded = !node.expanded;
                if node.expanded
                    && matches!(node.children, Children::NotLoaded | Children::Failed(_))
                {
                    loader.load(node);
                }
            }
        } else {
            ui.add_space(ui.spacing().interact_size.y);
        }
        let checkbox = tri_checkbox(ui, node.check(), interactive && node.selectable());
        if let Some(on) = checkbox {
            node.set_selected(on);
        }
        ui.label(if node.is_dir { "📁" } else { "📄" });

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button("🔍")
                .on_hover_text("Reveal in Finder")
                .clicked()
            {
                reveal_in_finder(&node.path);
            }
            ui.allocate_ui_with_layout(
                vec2(SIZE_COLUMN, ui.spacing().interact_size.y),
                Layout::right_to_left(Align::Center),
                |ui| ui.label(RichText::new(format_size(node.size)).monospace()),
            );
            let fraction = if parent_size == 0 {
                0.0
            } else {
                node.size as f32 / parent_size as f32
            };
            fraction_bar(ui, fraction, ACCENT, 60.0);
            if node.protected {
                ui.label("🔒")
                    .on_hover_text("This folder itself is kept. Expand it to choose items inside.");
            }
            if node.safety == Safety::Review {
                badge(ui, "Review", WARNING);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let name = ui
                    .add(Label::new(&node.name).truncate().sense(Sense::click()))
                    .on_hover_text(display_path(&node.path));
                if name.double_clicked() {
                    reveal_in_finder(&node.path);
                } else if name.clicked() && node.is_dir {
                    node.expanded = !node.expanded;
                    if node.expanded
                        && matches!(node.children, Children::NotLoaded | Children::Failed(_))
                    {
                        loader.load(node);
                    }
                }
                name.context_menu(|ui| {
                    if ui.button("Reveal in Finder").clicked() {
                        reveal_in_finder(&node.path);
                        ui.close();
                    }
                    if ui.button("Copy Path").clicked() {
                        ui.ctx().copy_text(node.path.display().to_string());
                        ui.close();
                    }
                });
            });
        });
    });

    if !node.expanded {
        return;
    }
    let size = node.size;
    let child_indent = (depth + 1) as f32 * INDENT + ui.spacing().interact_size.y;
    match &mut node.children {
        Children::Loading => {
            ui.horizontal(|ui| {
                ui.add_space(child_indent);
                ui.spinner();
                ui.label(RichText::new("Measuring…").small());
            });
        }
        Children::Failed(error) => {
            ui.horizontal(|ui| {
                ui.add_space(child_indent);
                ui.label(
                    RichText::new(format!("Can't open: {error}"))
                        .small()
                        .color(DANGER),
                );
            });
        }
        Children::Loaded(kids) if kids.is_empty() => {
            ui.horizontal(|ui| {
                ui.add_space(child_indent);
                ui.label(
                    RichText::new("Empty folder")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
        }
        Children::Loaded(kids) => {
            let id = Id::unique(("node_rows", &node.path));
            paged_rows(ui, id, kids, |ui, kid| {
                node_rows(ui, kid, depth + 1, size, loader, interactive);
            });
        }
        Children::NotLoaded => {}
    }
}
