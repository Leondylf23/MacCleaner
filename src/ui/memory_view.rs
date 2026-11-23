use std::sync::mpsc::{Receiver, channel};
use std::thread;

use eframe::egui::{self, Align, Color32, Layout, RichText, ScrollArea, TextEdit, Ui, vec2};

use super::process_table::ProcessTable;
use super::{
    ACCENT, DANGER, PURPLE, SUCCESS, TEAL, WARNING, banner, card, icon_label, page_header,
    primary_button, sparkline, stacked_bar, stat_card,
};
use egui_material_icons::icons::ICON_SEARCH;

use crate::system::memory::{MemoryStats, Pressure, purge};
use crate::system::monitor::{HISTORY_LEN, Monitor, Snapshot};
use crate::system::processes::{AppGroup, ProcKind};
use crate::util::format_memory;

const TOP_APPS: usize = 25;

pub struct MemoryView {
    table: ProcessTable,
    filter: String,
    show_all: bool,
    purge_rx: Option<Receiver<Result<u64, String>>>,
    purge_result: Option<Result<u64, String>>,
}

fn free_color(ui: &Ui) -> Color32 {
    ui.visuals().weak_text_color().gamma_multiply(0.35)
}

impl MemoryView {
    pub fn new() -> Self {
        Self {
            table: ProcessTable::new("memory_table"),
            filter: String::new(),
            show_all: false,
            purge_rx: None,
            purge_result: None,
        }
    }

    fn start_purge(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(purge());
            ctx.request_repaint();
        });
        self.purge_rx = Some(rx);
        self.purge_result = None;
    }

    pub fn ui(&mut self, ui: &mut Ui, snapshot: &Snapshot, monitor: &Monitor) {
        if let Some(result) = self.purge_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.purge_rx = None;
            self.purge_result = Some(result);
            monitor.refresh_now();
        }
        let purging = self.purge_rx.is_some();

        page_header(
            ui,
            "Memory",
            "How your RAM is used, what can be freed, and which apps use the most.",
            |ui| {
                let label = if purging {
                    "Freeing…"
                } else {
                    "Free Up RAM…"
                };
                if ui
                    .add_enabled(!purging, primary_button(label))
                    .on_hover_text("Runs macOS's purge command to drop cached files and purgeable memory. Asks for your administrator password.")
                    .clicked()
                {
                    self.start_purge(ui.ctx());
                }
            },
        );

        let Some(memory) = snapshot.memory else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading memory statistics…");
            });
            return;
        };

        match &self.purge_result {
            Some(Ok(freed)) => {
                let text = format!("Freed {} of cached memory.", format_memory(*freed));
                if banner(ui, SUCCESS, &text) {
                    self.purge_result = None;
                }
                ui.add_space(6.0);
            }
            Some(Err(e)) => {
                let text = format!("Couldn't free memory: {e}");
                if banner(ui, DANGER, &text) {
                    self.purge_result = None;
                }
                ui.add_space(6.0);
            }
            None => {}
        }
        self.table.message_ui(ui);

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                overview(ui, &memory);
                ui.add_space(8.0);
                ui.columns(2, |columns| {
                    composition_card(&mut columns[0], &memory);
                    history_card(&mut columns[1], &snapshot.history);
                });
                ui.add_space(8.0);
                reclaim_card(ui, &memory, &snapshot.groups, purging);
                ui.add_space(12.0);
                self.apps_section(ui, snapshot, &memory, monitor);
            });
    }

    fn apps_section(
        &mut self,
        ui: &mut Ui,
        snapshot: &Snapshot,
        memory: &MemoryStats,
        monitor: &Monitor,
    ) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Memory by app").strong().size(16.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.toggle_value(&mut self.show_all, "Show all");
                ui.add(
                    TextEdit::singleline(&mut self.filter)
                        .hint_text(icon_label(ICON_SEARCH, "Filter by name"))
                        .desired_width(200.0),
                );
            });
        });
        let needle = self.filter.trim().to_lowercase();
        let mut groups: Vec<&AppGroup> = snapshot
            .groups
            .iter()
            .filter(|g| needle.is_empty() || g.name.to_lowercase().contains(&needle))
            .collect();
        if groups.is_empty() {
            let text = if snapshot.groups.is_empty() {
                "Reading processes…"
            } else {
                "No apps match."
            };
            ui.label(RichText::new(text).color(ui.visuals().weak_text_color()));
            return;
        }
        let hidden = groups.len().saturating_sub(TOP_APPS);
        if !self.show_all && needle.is_empty() {
            groups.truncate(TOP_APPS);
        }
        if self.table.show(ui, &groups, memory.total, false) {
            monitor.refresh_now();
        }
        if !self.show_all && needle.is_empty() && hidden > 0 {
            ui.label(
                RichText::new(format!("{hidden} smaller apps and processes hidden."))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        }
    }
}

fn percent(part: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        part as f32 / total as f32 * 100.0
    }
}

fn overview(ui: &mut Ui, memory: &MemoryStats) {
    let (pressure_text, pressure_color, pressure_detail) = match memory.pressure {
        Pressure::Normal => ("Normal", SUCCESS, "RAM is used efficiently"),
        Pressure::Warning => ("Elevated", WARNING, "Consider quitting some apps"),
        Pressure::Critical => ("High", DANGER, "Your Mac needs more free memory"),
        Pressure::Unknown => ("Unknown", ui.visuals().weak_text_color(), ""),
    };
    let text_color = ui.visuals().text_color();
    ui.columns(5, |columns| {
        stat_card(
            &mut columns[0],
            "Physical memory",
            &format_memory(memory.total),
            "installed RAM",
            text_color,
        );
        stat_card(
            &mut columns[1],
            "Memory used",
            &format_memory(memory.used()),
            &format!(
                "{:.0}% · apps, wired, compressed",
                percent(memory.used(), memory.total)
            ),
            ACCENT,
        );
        stat_card(
            &mut columns[2],
            "Cached files",
            &format_memory(memory.cached),
            "released when apps need it",
            TEAL,
        );
        stat_card(
            &mut columns[3],
            "Swap used",
            &format_memory(memory.swap_used),
            &format!("of {} swap file", format_memory(memory.swap_total)),
            if memory.swap_used > 1 << 30 {
                WARNING
            } else {
                text_color
            },
        );
        stat_card(
            &mut columns[4],
            "Memory pressure",
            pressure_text,
            pressure_detail,
            pressure_color,
        );
    });
}

fn legend(ui: &mut Ui, color: Color32, label: &str, value: u64, total: u64) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2, color);
        ui.label(label);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{:.0}%", percent(value, total)))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.label(RichText::new(format_memory(value)).monospace());
        });
    });
}

fn composition_card(ui: &mut Ui, memory: &MemoryStats) {
    card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Where your RAM goes").strong());
        ui.add_space(4.0);
        let total = memory.total.max(1) as f32;
        let free = free_color(ui);
        stacked_bar(
            ui,
            &[
                (memory.app as f32 / total, ACCENT),
                (memory.wired as f32 / total, WARNING),
                (memory.compressed as f32 / total, PURPLE),
                (memory.cached as f32 / total, TEAL),
            ],
            16.0,
        );
        ui.add_space(6.0);
        legend(ui, ACCENT, "App memory", memory.app, memory.total);
        legend(
            ui,
            WARNING,
            "Wired (kernel, can't be freed)",
            memory.wired,
            memory.total,
        );
        legend(ui, PURPLE, "Compressed", memory.compressed, memory.total);
        legend(ui, TEAL, "Cached files", memory.cached, memory.total);
        legend(ui, free, "Free", memory.free(), memory.total);
    });
}

fn history_card(ui: &mut Ui, history: &[f32]) {
    card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Memory used").strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new("last 5 minutes")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
        });
        ui.add_space(4.0);
        sparkline(ui, history, HISTORY_LEN, ACCENT, 112.0);
        if let Some(latest) = history.last() {
            let peak = history.iter().copied().fold(0.0f32, f32::max);
            ui.label(
                RichText::new(format!(
                    "Now {:.0}% · peak {:.0}%",
                    latest * 100.0,
                    peak * 100.0
                ))
                .small()
                .color(ui.visuals().weak_text_color()),
            );
        }
    });
}

fn reclaim_card(ui: &mut Ui, memory: &MemoryStats, groups: &[AppGroup], purging: bool) {
    let quittable: Vec<&AppGroup> = groups
        .iter()
        .filter(|g| !g.protected && g.kind != ProcKind::App)
        .collect();
    let quittable_memory: u64 = quittable.iter().map(|g| g.memory).sum();
    let open_apps: u64 = groups
        .iter()
        .filter(|g| !g.protected && g.kind == ProcKind::App)
        .map(|g| g.memory)
        .sum();
    card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Memory that can be freed").strong());
            if purging {
                ui.spinner();
            }
        });
        ui.add_space(4.0);
        let row = |ui: &mut Ui, color: Color32, title: &str, detail: &str, value: u64| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(color));
                ui.vertical(|ui| {
                    ui.label(title);
                    ui.label(
                        RichText::new(detail)
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format_memory(value)).strong().monospace());
                });
            });
        };
        row(
            ui,
            TEAL,
            "Cached files & purgeable memory",
            "macOS releases this automatically. Free Up RAM releases it right away.",
            memory.cached,
        );
        let detail = if groups.is_empty() {
            "Measuring…".to_owned()
        } else {
            format!(
                "{} background helpers and menu bar apps you can quit.",
                quittable.len()
            )
        };
        row(
            ui,
            WARNING,
            "Background & menu bar processes",
            &detail,
            quittable_memory,
        );
        row(
            ui,
            ACCENT,
            "Open apps",
            "Quit apps you're not using from the list below.",
            open_apps,
        );
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Up to").strong());
            ui.label(
                RichText::new(format_memory(memory.cached + quittable_memory))
                    .strong()
                    .size(18.0)
                    .color(SUCCESS),
            );
            ui.label(
                RichText::new("can be freed without closing open apps")
                    .color(ui.visuals().weak_text_color()),
            );
        });
    });
}
