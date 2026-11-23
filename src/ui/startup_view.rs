use std::path::Path;
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use eframe::egui::{self, Align, Label, Layout, RichText, ScrollArea, TextEdit, Ui};

use super::{
    ACCENT, DANGER, SUCCESS, TEAL, WARNING, badge, banner, card, icon_button, icon_label,
    page_header, toggle_switch,
};
use egui_material_icons::icons::{ICON_LOCK, ICON_OPEN_IN_NEW, ICON_REFRESH, ICON_SEARCH};

use crate::startup::{self, StartupItem, StartupKind, StartupScan};
use crate::util::{LOGIN_ITEMS_URL, display_path, open_url, reveal_in_finder};

struct ToggleResult {
    item: StartupItem,
    enable: bool,
    result: Result<(), String>,
}

pub struct StartupView {
    ctx: egui::Context,
    items: Vec<StartupItem>,
    login_items_error: Option<String>,
    scan_rx: Option<Receiver<StartupScan>>,
    toggle_rx: Option<Receiver<ToggleResult>>,
    busy_key: Option<String>,
    message: Option<(bool, String)>,
    /// Login items removed this session, kept so they can be added back.
    removed_login_items: Vec<StartupItem>,
    loaded: bool,
    filter: String,
}

impl StartupView {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            ctx,
            items: Vec::new(),
            login_items_error: None,
            scan_rx: None,
            toggle_rx: None,
            busy_key: None,
            message: None,
            removed_login_items: Vec::new(),
            loaded: false,
            filter: String::new(),
        }
    }

    fn refresh(&mut self) {
        let (tx, rx) = channel();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(startup::scan());
            ctx.request_repaint();
        });
        self.scan_rx = Some(rx);
    }

    fn toggle(&mut self, item: StartupItem, enable: bool) {
        let (tx, rx) = channel();
        let ctx = self.ctx.clone();
        self.busy_key = Some(item.key());
        thread::spawn(move || {
            let result = startup::set_enabled(&item, enable);
            let _ = tx.send(ToggleResult {
                item,
                enable,
                result,
            });
            ctx.request_repaint();
        });
        self.toggle_rx = Some(rx);
    }

    /// Sidebar summary.
    pub fn summary(&self) -> String {
        if !self.loaded {
            return "Not checked yet".to_owned();
        }
        let enabled = self.items.iter().filter(|i| i.enabled).count();
        format!("{enabled} of {} on", self.items.len())
    }

    fn poll(&mut self) {
        if let Some(scan) = self.scan_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.scan_rx = None;
            self.loaded = true;
            self.items = scan.items;
            self.login_items_error = scan.login_items_error;
            for removed in &self.removed_login_items {
                if !self.items.iter().any(|i| i.key() == removed.key()) {
                    self.items.push(removed.clone());
                }
            }
            self.items.sort_by_key(|i| (i.kind, i.name.to_lowercase()));
        }
        if let Some(done) = self.toggle_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.toggle_rx = None;
            self.busy_key = None;
            let ToggleResult {
                item,
                enable,
                result,
            } = done;
            match result {
                Ok(()) => {
                    if item.kind == StartupKind::LoginItem {
                        self.removed_login_items.retain(|i| i.key() != item.key());
                        if !enable {
                            self.removed_login_items.push(StartupItem {
                                enabled: false,
                                ..item.clone()
                            });
                        }
                    }
                    let state = if enable { "turned on" } else { "turned off" };
                    self.message = Some((true, format!("{} {state}.", item.name)));
                }
                Err(e) => {
                    self.message = Some((false, format!("Couldn't change {}: {e}", item.name)))
                }
            }
            self.refresh();
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if !self.loaded && self.scan_rx.is_none() {
            self.refresh();
        }
        self.poll();
        let loading = self.scan_rx.is_some();

        page_header(
            ui,
            "Startup Items",
            "Apps and services that start automatically. Turn off what you don't need to speed up login and save memory.",
            |ui| {
                if ui
                    .add_enabled(
                        !loading,
                        egui::Button::new(icon_label(ICON_REFRESH, "Refresh")),
                    )
                    .clicked()
                {
                    self.refresh();
                }
                if ui.button("Login Items Settings…").clicked() {
                    open_url(LOGIN_ITEMS_URL);
                }
            },
        );

        if let Some((ok, text)) = &self.message {
            if banner(ui, if *ok { SUCCESS } else { DANGER }, text) {
                self.message = None;
            }
            ui.add_space(6.0);
        }
        if let Some(error) = &self.login_items_error {
            if banner(ui, WARNING, error) {
                self.login_items_error = None;
            }
            ui.add_space(6.0);
        }

        if !self.loaded {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Looking for startup items…");
            });
            return;
        }

        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.filter)
                    .hint_text(icon_label(ICON_SEARCH, "Filter by name or label"))
                    .desired_width(240.0),
            );
            if loading {
                ui.spinner();
            }
            let running = self.items.iter().filter(|i| i.pid.is_some()).count();
            let enabled = self.items.iter().filter(|i| i.enabled).count();
            ui.label(
                RichText::new(format!(
                    "{} items · {enabled} on · {running} running now",
                    self.items.len()
                ))
                .color(ui.visuals().weak_text_color()),
            );
        });
        ui.label(
            RichText::new("Apps can also register background items that only System Settings › General › Login Items & Extensions can manage.")
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(6.0);

        let needle = self.filter.trim().to_lowercase();
        let mut request = None;
        let busy = self.busy_key.clone();
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for kind in StartupKind::ALL {
                    let items: Vec<&StartupItem> = self
                        .items
                        .iter()
                        .filter(|i| i.kind == kind)
                        .filter(|i| {
                            needle.is_empty()
                                || i.name.to_lowercase().contains(&needle)
                                || i.label.to_lowercase().contains(&needle)
                        })
                        .collect();
                    card(ui).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(kind.title()).strong().size(15.0));
                            ui.label(
                                RichText::new(items.len().to_string())
                                    .color(ui.visuals().weak_text_color()),
                            );
                            if kind == StartupKind::SystemDaemon {
                                badge(ui, &icon_label(ICON_LOCK, "Admin"), WARNING);
                            }
                        });
                        ui.label(
                            RichText::new(kind.description())
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                        if items.is_empty() {
                            ui.label(
                                RichText::new("Nothing here.")
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                            return;
                        }
                        ui.add_space(4.0);
                        for item in items {
                            ui.separator();
                            if let Some(enable) = item_row(ui, item, busy.as_deref()) {
                                request = Some((item.clone(), enable));
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
            });
        if let Some((item, enable)) = request {
            self.toggle(item, enable);
        }
    }
}

/// Draws one startup item. Returns the requested new state if the switch was flipped.
fn item_row(ui: &mut Ui, item: &StartupItem, busy: Option<&str>) -> Option<bool> {
    let mut request = None;
    ui.horizontal(|ui| {
        let this_busy = busy == Some(item.key().as_str());
        if this_busy {
            ui.add_sized([36.0, 20.0], egui::Spinner::new());
        } else {
            let hint = if item.enabled { "Turn off" } else { "Turn on" };
            if toggle_switch(ui, item.enabled, busy.is_none())
                .on_hover_text(hint)
                .clicked()
            {
                request = Some(!item.enabled);
            }
        }
        ui.add_space(4.0);
        // The reveal button is laid out first so the truncating detail line stops short of it.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let target = item
                .plist
                .clone()
                .or_else(|| item.program.as_ref().map(Into::into));
            if let Some(target) = target
                && icon_button(ui, ICON_OPEN_IN_NEW, "Reveal in Finder").clicked()
            {
                reveal_in_finder(&target);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.vertical(|ui| item_details(ui, item));
            });
        });
    });
    request
}

fn item_details(ui: &mut Ui, item: &StartupItem) {
    ui.horizontal(|ui| {
        let name = RichText::new(&item.name).strong();
        ui.label(if item.enabled {
            name
        } else {
            name.color(ui.visuals().weak_text_color())
        });
        if let Some(pid) = item.pid {
            badge(ui, &format!("Running · PID {pid}"), SUCCESS);
        } else if item.loaded && item.kind != StartupKind::LoginItem {
            badge(ui, "Loaded", TEAL);
        }
        if item.run_at_load && item.kind != StartupKind::LoginItem {
            badge(ui, "Runs at login", ACCENT);
        }
        if item.keep_alive {
            badge(ui, "Kept alive", WARNING).on_hover_text("launchd restarts it if it quits");
        }
        if item.label.starts_with("com.apple.") {
            badge(ui, "Apple", DANGER)
                .on_hover_text("Part of macOS; turning it off may break features");
        }
    });
    let location = item
        .program
        .as_deref()
        .map(|p| display_path(Path::new(p)))
        .or_else(|| item.plist.as_deref().map(display_path));
    let detail = match location {
        Some(location) if item.kind == StartupKind::LoginItem || item.name == item.label => {
            location
        }
        Some(location) => format!("{} · {location}", item.label),
        None => item.label.clone(),
    };
    ui.add(
        Label::new(
            RichText::new(detail)
                .small()
                .monospace()
                .color(ui.visuals().weak_text_color()),
        )
        .truncate(),
    );
}
