//! Views and shared widgets.

pub mod memory_view;
pub mod process_table;
pub mod processes_view;
pub mod startup_view;
pub mod storage_view;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Mesh, Pos2, Rect,
    Response, RichText, Sense, Shape, Stroke, TextStyle, Ui, pos2, vec2,
};

use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{ICON_CHEVRON_RIGHT, ICON_CLOSE, ICON_EXPAND_MORE};

use crate::storage::tree::Check;

pub const ACCENT: Color32 = Color32::from_rgb(0x3B, 0x82, 0xF6);
pub const DANGER: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
pub const SUCCESS: Color32 = Color32::from_rgb(0x30, 0xA4, 0x6C);
pub const WARNING: Color32 = Color32::from_rgb(0xF0, 0x8C, 0x00);
pub const PURPLE: Color32 = Color32::from_rgb(0x8E, 0x4E, 0xC6);
pub const TEAL: Color32 = Color32::from_rgb(0x12, 0xA5, 0x94);

pub fn apply_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(10.0, 4.0);
        style.spacing.interact_size.y = 22.0;
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(22.0));
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(13.5));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(13.5));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(11.0));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(12.0));
        style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.75);
    });
}

/// A rounded, slightly raised container.
pub fn card(ui: &Ui) -> Frame {
    Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(8)
        .inner_margin(Margin::same(12))
}

/// Title and subtitle on the left, `actions` laid out from the right edge.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str, actions: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.heading(RichText::new(title).strong());
            ui.label(RichText::new(subtitle).color(ui.visuals().weak_text_color()));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), actions);
    });
    ui.add_space(8.0);
}

/// Width of the expand/collapse arrow in tree rows.
pub const EXPANDER_WIDTH: f32 = 22.0;

/// An icon followed by a label, for buttons, badges and hints.
pub fn icon_label(icon: MaterialIcon, text: &str) -> String {
    format!("{} {text}", icon.codepoint)
}

/// A frameless icon button with a tooltip.
pub fn icon_button(ui: &mut Ui, icon: MaterialIcon, tooltip: &str) -> Response {
    ui.add(egui::Button::new(icon.rich_text().size(18.0)).frame(false))
        .on_hover_text(tooltip)
}

/// The arrow that opens and closes a row.
pub fn expander(ui: &mut Ui, open: bool, enabled: bool) -> Response {
    let icon = if open {
        ICON_EXPAND_MORE
    } else {
        ICON_CHEVRON_RIGHT
    };
    ui.add_enabled_ui(enabled, |ui| {
        ui.add_sized(
            [EXPANDER_WIDTH, EXPANDER_WIDTH],
            egui::Button::new(icon.rich_text().size(20.0)).frame(false),
        )
    })
    .inner
}

/// Keeps rows without an expander aligned with rows that have one.
pub fn expander_space(ui: &mut Ui) {
    ui.add_space(EXPANDER_WIDTH + ui.spacing().item_spacing.x);
}

/// An icon on a tinted rounded square, like a Material avatar.
pub fn icon_tile(ui: &mut Ui, icon: MaterialIcon, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 9, color.gamma_multiply(0.16));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon.codepoint,
        FontId::new(22.0, icon.font_family()),
        color,
    );
    response
}

pub fn primary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).color(Color32::WHITE).strong()).fill(ACCENT)
}

pub fn danger_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).color(Color32::WHITE).strong()).fill(DANGER)
}

/// A checkbox that shows a dash for partially selected folders. Returns the new value if the
/// user clicked it.
pub fn tri_checkbox(ui: &mut Ui, state: Check, enabled: bool) -> Option<bool> {
    let mut on = state == Check::On;
    let response = ui.add_enabled(
        enabled,
        egui::Checkbox::without_text(&mut on).indeterminate(state == Check::Mixed),
    );
    response.changed().then_some(on)
}

/// An iOS-style switch.
pub fn toggle_switch(ui: &mut Ui, on: bool, enabled: bool) -> Response {
    let size = vec2(36.0, 20.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(response.id, on);
        let off = ui.visuals().widgets.inactive.bg_fill;
        let mut fill = off.lerp_to_gamma(SUCCESS, t);
        if !enabled {
            fill = fill.gamma_multiply(0.5);
        }
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, radius, fill);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter()
            .circle_filled(pos2(x, rect.center().y), radius - 2.5, Color32::WHITE);
    }
    response
}

/// A small colored tag.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    Frame::new()
        .fill(color.gamma_multiply(0.16))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(color)))
        .response
}

/// A horizontal bar split into colored segments; fractions are of the full width.
pub fn stacked_bar(ui: &mut Ui, segments: &[(f32, Color32)], height: f32) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    let radius = (height / 2.0).min(6.0) as u8;
    painter.rect_filled(rect, radius, ui.visuals().extreme_bg_color);
    let mut x = rect.left();
    for (i, (fraction, color)) in segments.iter().enumerate() {
        let width = rect.width() * fraction.clamp(0.0, 1.0);
        if width <= 0.0 {
            continue;
        }
        let right = (x + width).min(rect.right());
        let corners = CornerRadius {
            nw: if i == 0 { radius } else { 0 },
            sw: if i == 0 { radius } else { 0 },
            ne: if right >= rect.right() - 0.5 {
                radius
            } else {
                0
            },
            se: if right >= rect.right() - 0.5 {
                radius
            } else {
                0
            },
        };
        painter.rect_filled(
            Rect::from_min_max(pos2(x, rect.top()), pos2(right, rect.bottom())),
            corners,
            *color,
        );
        x = right;
    }
    response
}

/// A thin proportional bar, used next to sizes in lists.
pub fn fraction_bar(ui: &mut Ui, fraction: f32, color: Color32, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3, ui.visuals().extreme_bg_color);
    let filled = rect.width() * fraction.clamp(0.0, 1.0);
    if filled > 0.5 {
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(filled.max(3.0), rect.height())),
            3,
            color,
        );
    }
    response
}

/// A filled line chart of values in 0..=1, newest on the right. `capacity` fixes the x scale.
pub fn sparkline(ui: &mut Ui, values: &[f32], capacity: usize, color: Color32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6, ui.visuals().extreme_bg_color);
    for fraction in [0.25, 0.5, 0.75] {
        let y = rect.bottom() - rect.height() * fraction;
        painter.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(0.5, ui.visuals().weak_text_color().gamma_multiply(0.3)),
        );
    }
    if values.len() < 2 {
        return;
    }
    let step = rect.width() / (capacity.max(values.len()) - 1) as f32;
    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            pos2(
                rect.right() - (values.len() - 1 - i) as f32 * step,
                rect.bottom() - v.clamp(0.0, 1.0) * (rect.height() - 2.0),
            )
        })
        .collect();
    let mut mesh = Mesh::default();
    let fill = color.gamma_multiply(0.22);
    for point in &points {
        mesh.colored_vertex(*point, fill);
        mesh.colored_vertex(pos2(point.x, rect.bottom()), fill);
    }
    for i in 0..points.len() as u32 - 1 {
        let base = i * 2;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    painter.add(mesh);
    painter.add(Shape::line(points, Stroke::new(1.5, color)));
}

/// A headline number with a caption.
pub fn stat_card(ui: &mut Ui, title: &str, value: &str, detail: &str, color: Color32) {
    card(ui).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.label(
            RichText::new(title)
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.label(RichText::new(value).size(20.0).strong().color(color));
        ui.label(
            RichText::new(detail)
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    });
}

/// A dismissible message strip.
pub fn banner(ui: &mut Ui, color: Color32, text: &str) -> bool {
    let mut dismissed = false;
    Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.5)))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    dismissed = icon_button(ui, ICON_CLOSE, "Dismiss").clicked();
                });
            });
        });
    dismissed
}
