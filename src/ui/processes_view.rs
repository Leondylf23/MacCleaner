use eframe::egui::{RichText, TextEdit, Ui};

use super::process_table::{ProcessTable, kind_color};
use super::{page_header, stat_card};
use crate::app::Tab;
use crate::system::monitor::{Monitor, Snapshot};
use crate::system::processes::{AppGroup, ProcKind};
use crate::util::format_memory;

pub struct ProcessesView {
    table: ProcessTable,
    filter: String,
    show_background: bool,
    show_menu_bar: bool,
    show_apps: bool,
    show_protected: bool,
}

impl ProcessesView {
    pub fn new() -> Self {
        Self {
            table: ProcessTable::new("background_table"),
            filter: String::new(),
            show_background: true,
            show_menu_bar: true,
            show_apps: false,
            show_protected: false,
        }
    }

    fn matches(&self, group: &AppGroup) -> bool {
        let kind_shown = match group.kind {
            ProcKind::App => self.show_apps,
            ProcKind::MenuBar => self.show_menu_bar,
            ProcKind::Background => self.show_background,
        };
        if !kind_shown || (group.protected && !self.show_protected) {
            return false;
        }
        let needle = self.filter.trim().to_lowercase();
        needle.is_empty()
            || group.name.to_lowercase().contains(&needle)
            || group
                .procs
                .iter()
                .any(|p| p.name.to_lowercase().contains(&needle))
    }

    pub fn ui(&mut self, ui: &mut Ui, snapshot: &Snapshot, monitor: &Monitor, tab: &mut Tab) {
        page_header(
            ui,
            "Background Processes",
            "Helpers, agents and menu bar apps running without a window. Quit the ones you don't need.",
            |ui| {
                if ui.button("⟳ Refresh").clicked() {
                    monitor.refresh_now();
                }
            },
        );

        let groups = &snapshot.groups;
        if groups.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading processes…");
            });
            return;
        }

        let count_and_memory = |kind: ProcKind| {
            let list: Vec<&AppGroup> = groups
                .iter()
                .filter(|g| g.kind == kind && !g.protected)
                .collect();
            (list.len(), list.iter().map(|g| g.memory).sum::<u64>())
        };
        let (background_count, background_memory) = count_and_memory(ProcKind::Background);
        let (menu_count, menu_memory) = count_and_memory(ProcKind::MenuBar);
        let system_count = groups.iter().filter(|g| g.protected).count();

        let weak = ui.visuals().weak_text_color();
        ui.columns(3, |columns| {
            stat_card(
                &mut columns[0],
                "Background items you can quit",
                &background_count.to_string(),
                &format!("using {}", format_memory(background_memory)),
                kind_color(ProcKind::Background),
            );
            stat_card(
                &mut columns[1],
                "Menu bar & accessory apps",
                &menu_count.to_string(),
                &format!("using {}", format_memory(menu_memory)),
                kind_color(ProcKind::MenuBar),
            );
            stat_card(
                &mut columns[2],
                "System items (protected)",
                &system_count.to_string(),
                "part of macOS or other users",
                weak,
            );
        });
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.filter)
                    .hint_text("🔍 Filter by name")
                    .desired_width(220.0),
            );
            ui.separator();
            ui.toggle_value(&mut self.show_background, "Background");
            ui.toggle_value(&mut self.show_menu_bar, "Menu bar");
            ui.toggle_value(&mut self.show_apps, "Apps");
            ui.toggle_value(&mut self.show_protected, "System");
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(
                    "Quitting only lasts until the process is started again. Processes marked ↻ belong to a startup item;",
                )
                .small()
                .color(ui.visuals().weak_text_color()),
            );
            if ui.small_button("Manage Startup Items").clicked() {
                *tab = Tab::Startup;
            }
        });
        ui.add_space(4.0);
        self.table.message_ui(ui);

        let visible: Vec<&AppGroup> = groups.iter().filter(|g| self.matches(g)).collect();
        if visible.is_empty() {
            ui.label(
                RichText::new("No processes match the filters.")
                    .color(ui.visuals().weak_text_color()),
            );
            return;
        }
        let total = snapshot.memory.map_or(0, |m| m.total);
        if self.table.show(ui, &visible, total, true) {
            monitor.refresh_now();
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(600));
        }
    }
}
