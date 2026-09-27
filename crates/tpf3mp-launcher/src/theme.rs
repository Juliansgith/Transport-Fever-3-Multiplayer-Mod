//! The launcher's look, carried over from TPF2MP's launcher: its colours,
//! its cards with an uppercase heading, its primary, secondary, ghost and
//! danger buttons, its status pills, its numbered checklist and its log
//! box. Taken from `tools/launcher_theme.ps1` and
//! `tools/launcher_checklist.ps1` in TPF2MP (tf2mod, by Julian Cooper), and
//! redrawn with egui.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Pos2, Response,
    RichText, ScrollArea, Sense, Stroke, StrokeKind, TextStyle, Ui, Vec2, Visuals, WidgetInfo,
    WidgetType,
};

pub const BG: Color32 = Color32::from_rgb(15, 19, 24);
pub const SURFACE: Color32 = Color32::from_rgb(23, 29, 36);
pub const SURFACE_ALT: Color32 = Color32::from_rgb(30, 38, 47);
pub const BORDER: Color32 = Color32::from_rgb(43, 54, 66);
pub const FIELD: Color32 = Color32::from_rgb(11, 15, 19);
pub const FIELD_BORDER: Color32 = Color32::from_rgb(52, 65, 79);
pub const TEXT: Color32 = Color32::from_rgb(233, 238, 243);
pub const MUTED: Color32 = Color32::from_rgb(140, 154, 168);
pub const FAINT: Color32 = Color32::from_rgb(96, 108, 120);
pub const ACCENT: Color32 = Color32::from_rgb(45, 190, 168);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(70, 210, 188);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(6, 24, 22);
pub const SECONDARY: Color32 = Color32::from_rgb(36, 46, 57);
pub const SECONDARY_HOVER: Color32 = Color32::from_rgb(48, 60, 73);
pub const SUCCESS: Color32 = Color32::from_rgb(74, 199, 128);
pub const WARNING: Color32 = Color32::from_rgb(235, 181, 71);
pub const DANGER: Color32 = Color32::from_rgb(236, 96, 96);
pub const INFO: Color32 = Color32::from_rgb(92, 168, 230);
pub const LOG_BG: Color32 = Color32::from_rgb(10, 13, 17);
pub const LOG_TEXT: Color32 = Color32::from_rgb(178, 196, 208);
const DANGER_HOVER: Color32 = Color32::from_rgb(74, 32, 36);
const DANGER_BORDER: Color32 = Color32::from_rgb(110, 48, 52);

/// Space around the window's content.
pub const GUTTER: i8 = 18;

/// Sets the look on the whole window.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals());
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(11.5, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(21.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(12.5, FontFamily::Monospace),
            ),
        ]
        .into();
    });
}

fn visuals() -> Visuals {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = SURFACE;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.extreme_bg_color = FIELD;
    visuals.text_edit_bg_color = Some(FIELD);
    visuals.faint_bg_color = SURFACE_ALT;
    visuals.code_bg_color = LOG_BG;
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = DANGER;
    visuals.weak_text_color = Some(MUTED);
    visuals.selection.bg_fill = alpha(ACCENT, 90);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    let radius = CornerRadius::same(6);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = SURFACE;
    widgets.noninteractive.weak_bg_fill = SURFACE;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.noninteractive.corner_radius = radius;
    for (state, fill) in [
        (&mut widgets.inactive, SECONDARY),
        (&mut widgets.hovered, SECONDARY_HOVER),
        (&mut widgets.active, SECONDARY_HOVER),
        (&mut widgets.open, SECONDARY_HOVER),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, FIELD_BORDER);
        state.fg_stroke = Stroke::new(1.0, TEXT);
        state.corner_radius = radius;
        // Flat, as TPF2MP's: widgets change colour, not size.
        state.expansion = 0.0;
    }
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals
}

/// `color` at this opacity.
pub fn alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// A card: a flat surface with a hairline border and an uppercase heading.
pub fn card<R>(ui: &mut Ui, heading: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let inner = Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(6)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !heading.is_empty() {
                ui.label(
                    RichText::new(heading.to_uppercase())
                        .size(11.0)
                        .strong()
                        .color(MUTED),
                );
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner;
    ui.add_space(10.0);
    inner
}

/// A banner across the window in one colour: an update, a warning, a
/// message from the server, an error.
pub fn banner(ui: &mut Ui, color: Color32, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(alpha(color, 22))
        .stroke(Stroke::new(1.0, alpha(color, 150)))
        .corner_radius(6)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(10.0);
}

/// What a button does, which sets its colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The step to take next: filled with the accent.
    Primary,
    Secondary,
    /// A quiet one: only a border.
    Ghost,
    /// Removes or leaves something.
    Danger,
}

/// A button of this kind, of the height TPF2MP's are.
pub fn button(ui: &mut Ui, enabled: bool, text: &str, kind: Kind) -> Response {
    styled_button(ui, enabled, RichText::new(text).strong(), kind, 32.0)
}

/// A small button of this kind, for a row of a table or a line of text.
pub fn small_button(ui: &mut Ui, enabled: bool, text: &str, kind: Kind) -> Response {
    styled_button(ui, enabled, RichText::new(text).small(), kind, 22.0)
}

fn styled_button(ui: &mut Ui, enabled: bool, text: RichText, kind: Kind, height: f32) -> Response {
    let (fill, hover, fore, border) = match kind {
        Kind::Primary => (ACCENT, ACCENT_HOVER, ACCENT_TEXT, ACCENT),
        Kind::Secondary => (SECONDARY, SECONDARY_HOVER, TEXT, SECONDARY),
        Kind::Ghost => (SURFACE, SURFACE_ALT, TEXT, FIELD_BORDER),
        Kind::Danger => (SURFACE, DANGER_HOVER, DANGER, DANGER_BORDER),
    };
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for (state, fill) in [
            (&mut widgets.inactive, fill),
            (&mut widgets.hovered, hover),
            (&mut widgets.active, hover),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::new(1.0, border);
            state.fg_stroke = Stroke::new(1.0, fore);
        }
        if height < 32.0 {
            ui.spacing_mut().button_padding = Vec2::new(8.0, 2.0);
        }
        ui.add_enabled(
            enabled,
            egui::Button::new(text.color(fore)).min_size(Vec2::new(0.0, height)),
        )
    })
    .inner
}

/// A small status pill: the colour's own text on a faint capsule of it.
pub fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_uppercase(),
        FontId::new(10.5, FontFamily::Proportional),
        color,
    );
    let size = galley.size() + Vec2::new(18.0, 8.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(u8::try_from(rect.height() as u32 / 2).unwrap_or(u8::MAX));
        ui.painter().rect(
            rect,
            radius,
            alpha(color, 38),
            Stroke::new(1.0, alpha(color, 140)),
            StrokeKind::Inside,
        );
        let at = rect.center() - galley.size() / 2.0;
        ui.painter().galley(at, galley, color);
    }
    let label = text.to_owned();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &label));
    response
}

/// Where a checklist step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Done,
    /// The first step not yet done.
    Current,
    Pending,
}

impl Step {
    /// Each step's standing from which are done. As in TPF2MP's checklist,
    /// a later step done proves every earlier one, whatever this launcher
    /// saw of them.
    pub fn of(done: &[bool]) -> Vec<Self> {
        let mut done = done.to_vec();
        for index in (0..done.len().saturating_sub(1)).rev() {
            done[index] |= done[index + 1];
        }
        let current = done.iter().position(|done| !done);
        done.iter()
            .enumerate()
            .map(|(index, done)| match (done, Some(index) == current) {
                (true, _) => Self::Done,
                (false, true) => Self::Current,
                (false, false) => Self::Pending,
            })
            .collect()
    }
}

/// The numbered steps to a game in one row, each done, current or still to
/// come, joined by short rules.
pub fn checklist(ui: &mut Ui, steps: &[(&str, Step)]) {
    ui.horizontal_wrapped(|ui| {
        for (index, (text, step)) in steps.iter().enumerate() {
            if index > 0 {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(16.0, 22.0), Sense::hover());
                ui.painter().line_segment(
                    [rect.left_center(), rect.right_center()],
                    Stroke::new(1.0, BORDER),
                );
            }
            step_mark(ui, index + 1, *step);
            let text = RichText::new(*text);
            ui.label(match step {
                Step::Done => text.color(MUTED),
                Step::Current => text.strong().color(TEXT),
                Step::Pending => text.color(FAINT),
            });
        }
    });
}

/// A step's circle: ticked when done, numbered otherwise.
fn step_mark(ui: &mut Ui, number: usize, step: Step) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
    let center = rect.center();
    let painter = ui.painter();
    match step {
        Step::Done => {
            painter.circle(center, 10.0, ACCENT, Stroke::NONE);
            let tick = Stroke::new(2.0, ACCENT_TEXT);
            painter.line_segment(
                [center + Vec2::new(-4.5, 0.0), center + Vec2::new(-1.0, 3.5)],
                tick,
            );
            painter.line_segment(
                [center + Vec2::new(-1.0, 3.5), center + Vec2::new(5.0, -3.5)],
                tick,
            );
        }
        Step::Current => {
            painter.circle(center, 10.0, alpha(ACCENT, 38), Stroke::new(1.5, ACCENT));
            numeral(painter, center, number, ACCENT);
        }
        Step::Pending => {
            painter.circle_stroke(center, 10.0, Stroke::new(1.0, FAINT));
            numeral(painter, center, number, FAINT);
        }
    }
}

fn numeral(painter: &egui::Painter, center: Pos2, number: usize, color: Color32) {
    painter.text(
        center,
        Align2::CENTER_CENTER,
        number.to_string(),
        FontId::new(11.5, FontFamily::Proportional),
        color,
    );
}

/// A dark log box, newest line last, in the log's monospace.
pub fn log_box<'a>(ui: &mut Ui, id: &str, lines: impl Iterator<Item = &'a str>, height: f32) {
    Frame::new()
        .fill(LOG_BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(4)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ScrollArea::vertical()
                .id_salt(id)
                .max_height(height)
                .stick_to_bottom(true)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for line in lines {
                        ui.label(RichText::new(line).monospace().color(LOG_TEXT));
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::Step::{self, Current, Done, Pending};

    #[test]
    fn a_later_step_done_proves_the_earlier_ones() {
        assert_eq!(
            Step::of(&[false, false, false]),
            [Current, Pending, Pending]
        );
        assert_eq!(Step::of(&[true, false, false]), [Done, Current, Pending]);
        // The launcher restarted in a running game: it saw no connect.
        assert_eq!(Step::of(&[false, false, true]), [Done, Done, Done]);
        assert_eq!(
            Step::of(&[true, false, true, false]),
            [Done, Done, Done, Current]
        );
        assert_eq!(Step::of(&[]), []);
    }
}
