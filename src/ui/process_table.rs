//! Per-app process table used by the Background and Memory views.

use std::cmp::Reverse;
use std::collections::HashSet;

use eframe::egui::{self, Align, Id, Label, Layout, Modal, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use super::{ACCENT, DANGER, PURPLE, SUCCESS, TEAL, WARNING, badge, danger_button, fraction_bar};
use crate::system::processes::{AppGroup, ProcKind, quit};
use crate::util::{display_path, format_memory};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Memory,
    Cpu,
    Name,
}

struct PendingQuit {
    group: AppGroup,
    force: bool,
}

pub struct ProcessTable {
    id: &'static str,
    expanded: HashSet<String>,
    sort: Sort,
    pending: Option<PendingQuit>,
    pub message: Option<(bool, String)>,
}

pub fn kind_color(kind: ProcKind) -> egui::Color32 {
    match kind {
        ProcKind::App => ACCENT,
        ProcKind::MenuBar => PURPLE,
        ProcKind::Background => TEAL,
    }
}

impl ProcessTable {
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            expanded: HashSet::new(),
            sort: Sort::Memory,
            pending: None,
            message: None,
        }
    }

    /// Draws the table. Returns true when processes were quit, so the caller can refresh.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        groups: &[&AppGroup],
        total_memory: u64,
        scroll: bool,
    ) -> bool {
        let mut sorted = groups.to_vec();
        match self.sort {
            Sort::Memory => sorted.sort_by_key(|g| Reverse(g.memory)),
            Sort::Cpu => sorted.sort_by(|a, b| b.cpu.total_cmp(&a.cpu)),
            Sort::Name => sorted.sort_by_key(|g| g.name.to_lowercase()),
        }

        let mut toggle = None;
        let mut request = None;
        let row_height = 26.0;
        let weak = ui.visuals().weak_text_color();
        TableBuilder::new(ui)
            .id_salt(self.id)
            .striped(true)
            .vscroll(scroll)
            .auto_shrink([false, true])
            .min_scrolled_height(220.0)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::remainder().at_least(220.0).clip(true))
            .column(Column::exact(96.0))
            .column(Column::exact(64.0))
            .column(Column::exact(170.0))
            .column(Column::exact(64.0))
            .column(Column::exact(170.0))
            .header(22.0, |mut header| {
                let mut sort_header = |header: &mut egui_extras::TableRow, label: &str, sort: Option<Sort>| {
                    header.col(|ui| match sort {
                        Some(sort) => {
                            let text = if self.sort == sort { format!("{label} ⏷") } else { label.to_owned() };
                            if ui.add(egui::Button::new(RichText::new(text).strong()).frame(false)).clicked() {
                                self.sort = sort;
                            }
                        }
                        None => {
                            ui.label(RichText::new(label).strong());
                        }
                    });
                };
                sort_header(&mut header, "Name", Some(Sort::Name));
                sort_header(&mut header, "Type", None);
                sort_header(&mut header, "Procs", None);
                sort_header(&mut header, "Memory", Some(Sort::Memory));
                sort_header(&mut header, "CPU", Some(Sort::Cpu));
                sort_header(&mut header, "", None);
            })
            .body(|mut body| {
                for group in &sorted {
                    let open = self.expanded.contains(&group.key);
                    body.row(row_height, |mut row| {
                        row.col(|ui| {
                            if group.procs.len() > 1 {
                                let arrow = if open { "⏷" } else { "⏵" };
                                if ui.add(egui::Button::new(arrow).frame(false)).clicked() {
                                    toggle = Some(group.key.clone());
                                }
                            } else {
                                ui.add_space(ui.spacing().interact_size.y);
                            }
                            let name = ui.add(
                                Label::new(RichText::new(&group.name).strong())
                                    .truncate()
                                    .sense(Sense::click()),
                            );
                            if name.clicked() && group.procs.len() > 1 {
                                toggle = Some(group.key.clone());
                            }
                            if let Some(path) = group.procs.first().and_then(|p| p.exe.as_deref()) {
                                name.on_hover_text(display_path(path));
                            }
                        });
                        row.col(|ui| {
                            badge(ui, group.kind.label(), kind_color(group.kind));
                        });
                        row.col(|ui| {
                            ui.label(group.procs.len().to_string());
                        });
                        row.col(|ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(72.0, row_height),
                                Layout::right_to_left(Align::Center),
                                |ui| ui.label(RichText::new(format_memory(group.memory)).monospace()),
                            );
                            let fraction = group.memory as f32 / total_memory.max(1) as f32;
                            fraction_bar(ui, fraction * 4.0, memory_color(fraction), 70.0)
                                .on_hover_text(format!("{:.1}% of RAM", fraction * 100.0));
                        });
                        row.col(|ui| {
                            ui.label(RichText::new(format!("{:.1}%", group.cpu)).monospace());
                        });
                        row.col(|ui| {
                            if group.protected {
                                ui.label(RichText::new("🔒 System").small().color(weak))
                                    .on_hover_text("Part of macOS or owned by another user. MacCleaner won't quit it.");
                            } else {
                                if ui.button("Quit").clicked() {
                                    request = Some(((*group).clone(), false));
                                }
                                if ui.button(RichText::new("Force Quit").color(DANGER)).clicked() {
                                    request = Some(((*group).clone(), true));
                                }
                            }
                        });
                    });
                    if !open {
                        continue;
                    }
                    for process in &group.procs {
                        body.row(22.0, |mut row| {
                            row.col(|ui| {
                                ui.add_space(ui.spacing().interact_size.y + 12.0);
                                let label = ui.add(
                                    Label::new(RichText::new(format!("{}  ·  PID {}", process.name, process.pid)).small())
                                        .truncate(),
                                );
                                if let Some(path) = &process.exe {
                                    label.on_hover_text(display_path(path));
                                }
                            });
                            row.col(|ui| {
                                ui.label(RichText::new(process.kind.label()).small().color(weak));
                            });
                            row.col(|ui| {
                                if let Some(label) = &process.launchd_label {
                                    ui.label(RichText::new("↻").color(WARNING)).on_hover_text(format!(
                                        "Started by launchd job {label}; it may restart after quitting."
                                    ));
                                }
                            });
                            row.col(|ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(72.0, 22.0),
                                    Layout::right_to_left(Align::Center),
                                    |ui| ui.label(RichText::new(format_memory(process.memory)).small().monospace()),
                                );
                            });
                            row.col(|ui| {
                                ui.label(RichText::new(format!("{:.1}%", process.cpu)).small().monospace());
                            });
                            row.col(|_| {});
                        });
                    }
                }
            });

        if let Some(key) = toggle
            && !self.expanded.remove(&key)
        {
            self.expanded.insert(key);
        }
        if let Some((group, force)) = request {
            // Quitting a regular app is the same as Quit in the Dock; anything else gets a prompt.
            if !force && group.kind == ProcKind::App {
                return self.perform(&group, false);
            }
            self.pending = Some(PendingQuit { group, force });
        }
        self.confirm_dialog(ui)
    }

    fn perform(&mut self, group: &AppGroup, force: bool) -> bool {
        match quit(group, force) {
            Ok(count) => {
                let verb = if force { "Force quit" } else { "Asked to quit" };
                let noun = if count == 1 { "process" } else { "processes" };
                self.message = Some((true, format!("{verb} {} ({count} {noun}).", group.name)));
                true
            }
            Err(e) => {
                self.message = Some((false, format!("Couldn't quit {}: {e}", group.name)));
                false
            }
        }
    }

    fn confirm_dialog(&mut self, ui: &mut Ui) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        let mut confirmed = false;
        let mut cancelled = false;
        let ctx = ui.ctx().clone();
        let response = Modal::new(Id::unique((self.id, "confirm_quit"))).show(&ctx, |ui| {
            ui.set_width(440.0);
            let group = &pending.group;
            ui.heading(if pending.force {
                format!("Force quit {}?", group.name)
            } else {
                format!("Quit {}?", group.name)
            });
            ui.label(format!(
                "{} processes using {}.",
                group.procs.len(),
                format_memory(group.memory)
            ));
            if pending.force {
                ui.label(RichText::new("Force quitting stops it immediately. Unsaved changes are lost.").color(DANGER));
            } else {
                ui.label("It will be asked to quit and can finish what it's doing first.");
            }
            if let Some(label) = group.launchd_label() {
                ui.label(
                    RichText::new(format!(
                        "↻ Started by launchd job “{label}”, so it may start again. To keep it off, turn it off in Startup Items."
                    ))
                    .color(WARNING),
                );
            }
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if pending.force { "Force Quit" } else { "Quit" };
                confirmed = ui.add(danger_button(label)).clicked();
                cancelled = ui.button("Cancel").clicked();
            });
        });
        if confirmed {
            let pending = self.pending.take().expect("pending quit");
            return self.perform(&pending.group, pending.force);
        }
        if cancelled || response.should_close() {
            self.pending = None;
        }
        false
    }

    /// Shows the outcome of the last quit, if any.
    pub fn message_ui(&mut self, ui: &mut Ui) {
        if let Some((ok, text)) = &self.message {
            let color = if *ok { SUCCESS } else { DANGER };
            if super::banner(ui, color, text) {
                self.message = None;
            }
            ui.add_space(6.0);
        }
    }
}

fn memory_color(fraction_of_ram: f32) -> egui::Color32 {
    if fraction_of_ram > 0.15 {
        DANGER
    } else if fraction_of_ram > 0.05 {
        WARNING
    } else {
        ACCENT
    }
}
