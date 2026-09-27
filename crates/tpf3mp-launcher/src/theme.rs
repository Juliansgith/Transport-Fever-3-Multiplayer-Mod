//! The launcher's look: that of tearded's launcher for Transport Fever 2
//! multiplayer (`tpf-multiplayer-launcher`, MIT, a web page in Tauri),
//! redrawn with egui for TPF3:
//!
//! - art across the whole window;
//! - the game's name large, over a band;
//! - glass panels with a short accent bar, and light main buttons with an
//!   arrow;
//! - outlined quiet ones, and small status pills;
//! - a bar for your game along the bottom.
//!
//! Its colours are taken from that launcher's style sheet. The status
//! colours, the checklist and the log box are TPF2MP's (tf2mod's launcher,
//! by Julian Cooper). The art is drawn here, not an image of the game's:
//! a railway viaduct at dusk.
//!
//! Text is set in the system's own fonts where it has them, such as
//! Bahnschrift and Segoe UI on Windows as that launcher does, and in
//! egui's otherwise. A font file egui could not read is never handed to
//! it.

use std::path::PathBuf;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Id,
    Margin, Mesh, Painter, Pos2, Rect, Response, RichText, ScrollArea, Sense, Shape, Stroke,
    StrokeKind, TextStyle, Ui, Vec2, Visuals, WidgetInfo, WidgetType,
    epaint::text::VariationCoords, pos2, vec2,
};

/// The window behind everything: #101415.
pub const BG: Color32 = Color32::from_rgb(16, 20, 21);
/// A raised surface: #222b2e.
pub const RAISED: Color32 = Color32::from_rgb(34, 43, 46);
pub const TEXT: Color32 = Color32::from_rgb(244, 245, 239);
pub const MUTED: Color32 = Color32::from_rgb(156, 169, 173);
pub const FAINT: Color32 = Color32::from_rgb(104, 116, 120);
/// A row's name, beside its value: #a6b5b9.
pub const LABEL: Color32 = Color32::from_rgb(166, 181, 185);
/// The main button: #d1d7da, with ink on it.
pub const LIGHT: Color32 = Color32::from_rgb(209, 215, 218);
pub const LIGHT_HOVER: Color32 = Color32::from_rgb(234, 238, 240);
pub const INK: Color32 = Color32::from_rgb(23, 26, 23);
/// TPF2MP's teal: the accent bar of a panel, a step done.
pub const ACCENT: Color32 = Color32::from_rgb(45, 190, 168);
pub const SUCCESS: Color32 = Color32::from_rgb(74, 199, 128);
pub const WARNING: Color32 = Color32::from_rgb(235, 181, 71);
pub const DANGER: Color32 = Color32::from_rgb(236, 96, 96);
pub const INFO: Color32 = Color32::from_rgb(92, 168, 230);
pub const LOG_TEXT: Color32 = Color32::from_rgb(178, 196, 208);

// Translucent colours, premultiplied: the colour times its opacity.
/// A hairline: white at 0x18.
pub const LINE: Color32 = Color32::from_rgba_premultiplied(24, 24, 24, 24);
/// A glass panel's border: #afc6c5 at 0x32.
const GLASS_BORDER: Color32 = Color32::from_rgba_premultiplied(34, 39, 39, 50);
/// A glass panel, top to bottom: #202e32 at 0xed to #0d171c at 0xe8.
const GLASS_TOP: Color32 = Color32::from_rgba_premultiplied(30, 43, 46, 237);
const GLASS_BOTTOM: Color32 = Color32::from_rgba_premultiplied(12, 21, 25, 232);
/// A quiet button's border: #bac8ce at 0x50.
const OUTLINE: Color32 = Color32::from_rgba_premultiplied(58, 63, 65, 80);
/// Under a hovered quiet button: white at 0x12.
const HOVER: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
/// A text field: #0a0f11 at 0xcc, with a white border at 0x22.
const FIELD: Color32 = Color32::from_rgba_premultiplied(8, 12, 14, 204);
const FIELD_BORDER: Color32 = Color32::from_rgba_premultiplied(34, 34, 34, 34);
/// The log box: #080b0d at 0xd8.
const LOG_BG: Color32 = Color32::from_rgba_premultiplied(7, 9, 11, 216);

/// Space around the window's content.
pub const GUTTER: i8 = 22;
/// The corners of controls, as in the TPF2 launcher.
const RADIUS: u8 = 5;

/// The font of the game's name: condensed and bold where the system has one.
pub const DISPLAY: &str = "display";
/// The font of headings and buttons: semibold where the system has one.
pub const HEADING: &str = "heading";
/// Whether the display font is a bold one, rather than egui's light one.
const DISPLAY_IS_BOLD: &str = "tpf3mp-display-is-bold";

pub fn display_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(DISPLAY.into()))
}

pub fn heading_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(HEADING.into()))
}

/// Sets the look on the whole window.
pub fn apply(ctx: &egui::Context) {
    let bold = install_fonts(ctx);
    ctx.data_mut(|data| data.insert_temp(Id::new(DISPLAY_IS_BOLD), bold));
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals());
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(16.0, 6.0);
        style.spacing.interact_size.y = 34.0;
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, heading_font(14.0)),
            (TextStyle::Heading, heading_font(20.0)),
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
    visuals.window_fill = Color32::from_rgb(21, 27, 29);
    visuals.window_stroke = Stroke::new(1.0, GLASS_BORDER);
    visuals.extreme_bg_color = FIELD;
    visuals.text_edit_bg_color = Some(FIELD);
    visuals.faint_bg_color = RAISED;
    visuals.code_bg_color = LOG_BG;
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = DANGER;
    visuals.weak_text_color = Some(MUTED);
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = alpha(ACCENT, 90);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    let radius = CornerRadius::same(RADIUS);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = Color32::TRANSPARENT;
    widgets.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.noninteractive.corner_radius = radius;
    for (state, fill) in [
        (&mut widgets.inactive, FIELD),
        (&mut widgets.hovered, RAISED),
        (&mut widgets.active, RAISED),
        (&mut widgets.open, RAISED),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, FIELD_BORDER);
        state.fg_stroke = Stroke::new(1.0, TEXT);
        state.corner_radius = radius;
        // Flat: widgets change colour, not size.
        state.expansion = 0.0;
    }
    widgets.hovered.bg_stroke = Stroke::new(1.0, OUTLINE);
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals
}

/// `color` at this opacity.
pub fn alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// A variable font's axes to set, such as its weight (`wght`).
type Axes = &'static [(&'static [u8; 4], f32)];

/// A font file on this system, and the axes to set when it is variable.
struct SystemFont {
    path: PathBuf,
    axes: Axes,
}

/// Puts the system's fonts first in each family, egui's after them for
/// what they lack. Returns whether a bold display font was found.
fn install_fonts(ctx: &egui::Context) -> bool {
    let mut fonts = FontDefinitions::default();
    let defaults = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut bold = false;
    for (role, candidates) in [
        ("tpf3mp-body", body_fonts()),
        ("tpf3mp-heading", heading_fonts()),
        ("tpf3mp-display", display_fonts()),
        ("tpf3mp-mono", mono_fonts()),
    ] {
        let family = match role {
            "tpf3mp-body" => FontFamily::Proportional,
            "tpf3mp-heading" => FontFamily::Name(HEADING.into()),
            "tpf3mp-display" => FontFamily::Name(DISPLAY.into()),
            _ => FontFamily::Monospace,
        };
        let list = fonts
            .families
            .entry(family)
            .or_insert_with(|| defaults.clone());
        if let Some(font) = first_readable(&candidates) {
            fonts.font_data.insert(role.to_owned(), font.into());
            list.insert(0, role.to_owned());
            bold |= role == "tpf3mp-display";
        }
    }
    // Headings and the name fall back to the body's font, then egui's.
    let body = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for name in [HEADING, DISPLAY] {
        if let Some(list) = fonts.families.get_mut(&FontFamily::Name(name.into())) {
            for font in &body {
                if !list.contains(font) {
                    list.push(font.clone());
                }
            }
        }
    }
    ctx.set_fonts(fonts);
    bold
}

/// The first of `candidates` that is there and that egui can read: egui
/// stops the program on a font it cannot parse, so each is parsed here
/// first, as egui would.
fn first_readable(candidates: &[SystemFont]) -> Option<FontData> {
    candidates.iter().find_map(|font| {
        let bytes = std::fs::read(&font.path).ok()?;
        skrifa::FontRef::from_index(&bytes, 0).ok()?;
        let mut data = FontData::from_owned(bytes);
        data.tweak.coords = VariationCoords::new(font.axes.iter().copied());
        Some(data)
    })
}

fn fonts_in(dirs: &[PathBuf], files: &[(&str, Axes)]) -> Vec<SystemFont> {
    files
        .iter()
        .flat_map(|(file, axes)| {
            dirs.iter().map(move |dir| SystemFont {
                path: dir.join(file),
                axes,
            })
        })
        .collect()
}

#[cfg(windows)]
fn font_dirs() -> Vec<PathBuf> {
    let windows = std::env::var_os("WINDIR")
        .or_else(|| std::env::var_os("SystemRoot"))
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    vec![windows.join("Fonts")]
}

#[cfg(target_os = "macos")]
fn font_dirs() -> Vec<PathBuf> {
    [
        "/System/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
        "/Library/Fonts",
    ]
    .map(PathBuf::from)
    .to_vec()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn font_dirs() -> Vec<PathBuf> {
    // Debian and Ubuntu, Fedora, Arch.
    [
        "/usr/share/fonts/truetype/dejavu",
        "/usr/share/fonts/dejavu-sans-fonts",
        "/usr/share/fonts/dejavu-sans-mono-fonts",
        "/usr/share/fonts/TTF",
        "/usr/share/fonts/dejavu",
        "/usr/share/fonts/truetype/noto",
        "/usr/share/fonts/noto",
        "/usr/share/fonts/google-noto",
        "/usr/share/fonts/truetype/liberation",
        "/usr/share/fonts/liberation-sans",
        "/usr/share/fonts/liberation-mono",
        "/usr/share/fonts/liberation",
    ]
    .map(PathBuf::from)
    .to_vec()
}

const PLAIN: Axes = &[];

fn display_fonts() -> Vec<SystemFont> {
    fonts_in(
        &font_dirs(),
        if cfg!(windows) {
            // As the TPF2 launcher sets its name: Bahnschrift, bold and
            // condensed.
            &[
                ("bahnschrift.ttf", &[(b"wght", 700.0), (b"wdth", 75.0)]),
                ("seguibl.ttf", PLAIN),
                ("impact.ttf", PLAIN),
            ]
        } else if cfg!(target_os = "macos") {
            &[
                ("DIN Condensed Bold.ttf", PLAIN),
                ("DIN Alternate Bold.ttf", PLAIN),
                ("Arial Black.ttf", PLAIN),
            ]
        } else {
            &[
                ("DejaVuSansCondensed-Bold.ttf", PLAIN),
                ("LiberationSansNarrow-Bold.ttf", PLAIN),
                ("NotoSans-Bold.ttf", PLAIN),
                ("DejaVuSans-Bold.ttf", PLAIN),
            ]
        },
    )
}

fn heading_fonts() -> Vec<SystemFont> {
    fonts_in(
        &font_dirs(),
        if cfg!(windows) {
            &[
                ("bahnschrift.ttf", &[(b"wght", 600.0), (b"wdth", 100.0)]),
                ("seguisb.ttf", PLAIN),
                ("segoeuib.ttf", PLAIN),
            ]
        } else if cfg!(target_os = "macos") {
            &[("DIN Alternate Bold.ttf", PLAIN), ("Arial Bold.ttf", PLAIN)]
        } else {
            &[
                ("DejaVuSans-Bold.ttf", PLAIN),
                ("NotoSans-Bold.ttf", PLAIN),
                ("LiberationSans-Bold.ttf", PLAIN),
            ]
        },
    )
}

fn body_fonts() -> Vec<SystemFont> {
    fonts_in(
        &font_dirs(),
        if cfg!(windows) {
            &[("segoeui.ttf", PLAIN), ("arial.ttf", PLAIN)]
        } else if cfg!(target_os = "macos") {
            &[("SFNS.ttf", PLAIN), ("Arial.ttf", PLAIN)]
        } else {
            &[
                ("DejaVuSans.ttf", PLAIN),
                ("NotoSans-Regular.ttf", PLAIN),
                ("LiberationSans-Regular.ttf", PLAIN),
            ]
        },
    )
}

fn mono_fonts() -> Vec<SystemFont> {
    fonts_in(
        &font_dirs(),
        if cfg!(windows) {
            &[("consola.ttf", PLAIN)]
        } else if cfg!(target_os = "macos") {
            &[("SFNSMono.ttf", PLAIN), ("Menlo.ttc", PLAIN)]
        } else {
            &[
                ("DejaVuSansMono.ttf", PLAIN),
                ("LiberationMono-Regular.ttf", PLAIN),
            ]
        },
    )
}

/// A glass panel: dark, a little see-through, with a hairline border, a
/// short accent bar at its top left, and a heading.
pub fn glass<R>(ui: &mut Ui, heading: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let background = ui.painter().add(Shape::Noop);
    let inner = Frame::new().inner_margin(Margin::same(22)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if !heading.is_empty() {
            ui.label(RichText::new(heading).font(heading_font(15.0)).color(TEXT));
            ui.add_space(4.0);
        }
        add(ui)
    });
    let rect = inner.response.rect;
    ui.painter().set(background, glass_shape(rect, ACCENT));
    ui.add_space(14.0);
    inner.inner
}

/// A panel as [`glass`] draws one.
pub fn card<R>(ui: &mut Ui, heading: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    glass(ui, heading, add)
}

/// A low glass bar across the window, as the TPF2 launcher's for your
/// game.
pub fn bar<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let background = ui.painter().add(Shape::Noop);
    let inner = Frame::new()
        .inner_margin(Margin::symmetric(18, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        });
    ui.painter()
        .set(background, glass_shape(inner.response.rect, ACCENT));
    inner.inner
}

fn glass_shape(rect: Rect, bar: Color32) -> Shape {
    let radius = 8.0;
    Shape::Vec(vec![
        Shape::mesh(rounded_gradient(rect, radius, GLASS_TOP, GLASS_BOTTOM)),
        Shape::rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0, GLASS_BORDER),
            StrokeKind::Inside,
        ),
        Shape::rect_filled(
            Rect::from_min_size(rect.min + vec2(22.0, 0.0), vec2(42.0, 2.0)),
            CornerRadius::ZERO,
            bar,
        ),
    ])
}

/// A rounded rectangle filled from `top` to `bottom`: a fan of triangles
/// from its middle, each corner a few points of its arc.
fn rounded_gradient(rect: Rect, radius: f32, top: Color32, bottom: Color32) -> Mesh {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let color_at = |y: f32| {
        let t = if rect.height() > 0.0 {
            ((y - rect.top()) / rect.height()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        mix(top, bottom, t)
    };
    let mut mesh = Mesh::default();
    let middle = rect.center();
    mesh.colored_vertex(middle, color_at(middle.y));
    let corners = [
        (rect.right_top() + vec2(-radius, radius), -90.0_f32),
        (rect.right_bottom() + vec2(-radius, -radius), 0.0),
        (rect.left_bottom() + vec2(radius, -radius), 90.0),
        (rect.left_top() + vec2(radius, radius), 180.0),
    ];
    const STEPS: u32 = 6;
    for (center, start) in corners {
        for step in 0..=STEPS {
            let angle = (start + 90.0 * step as f32 / STEPS as f32).to_radians();
            let point = center + radius * vec2(angle.cos(), angle.sin());
            mesh.colored_vertex(point, color_at(point.y));
        }
    }
    let points = 4 * (STEPS + 1);
    for index in 0..points {
        mesh.add_triangle(0, 1 + index, 1 + (index + 1) % points);
    }
    mesh
}

/// Between two premultiplied colours.
fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    let channel = |a: u8, b: u8| {
        (f32::from(a) + (f32::from(b) - f32::from(a)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgba_premultiplied(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
        channel(from.a(), to.a()),
    )
}

/// A banner across the column in one colour: an update, a warning, a
/// message from the server, an error. Glass with that colour's bar.
pub fn banner(ui: &mut Ui, color: Color32, add: impl FnOnce(&mut Ui)) {
    let background = ui.painter().add(Shape::Noop);
    let response = Frame::new()
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        })
        .response;
    let rect = response.rect;
    ui.painter().set(
        background,
        Shape::Vec(vec![
            glass_shape(rect, color),
            Shape::rect_filled(rect, CornerRadius::same(8), alpha(color, 20)),
            Shape::rect_stroke(
                rect,
                CornerRadius::same(8),
                Stroke::new(1.0, alpha(color, 120)),
                StrokeKind::Inside,
            ),
        ]),
    );
    ui.add_space(12.0);
}

/// What a button does, which sets its look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The step to take next: light, with ink.
    Primary,
    /// Outlined.
    Secondary,
    /// Only its words, until hovered.
    Ghost,
    /// Removes or leaves something.
    Danger,
}

/// A button of this kind, as tall as the TPF2 launcher's controls.
pub fn button(ui: &mut Ui, enabled: bool, text: &str, kind: Kind) -> Response {
    styled_button(
        ui,
        enabled,
        RichText::new(text).font(heading_font(14.0)),
        kind,
        38.0,
        0.0,
    )
}

/// A small button of this kind, for a row or a line of text.
pub fn small_button(ui: &mut Ui, enabled: bool, text: &str, kind: Kind) -> Response {
    styled_button(ui, enabled, RichText::new(text).size(12.0), kind, 26.0, 0.0)
}

/// The panel's main button, across it, with an arrow after its words; the
/// arrow is drawn, so the button's name stays its words.
pub fn big_button(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    let width = ui.available_width();
    let response = styled_button(
        ui,
        enabled,
        RichText::new(text).font(heading_font(16.0)),
        Kind::Primary,
        50.0,
        width,
    );
    if ui.is_rect_visible(response.rect) {
        let color = if enabled { INK } else { alpha(INK, 120) };
        let tip = pos2(response.rect.right() - 20.0, response.rect.center().y);
        let stroke = Stroke::new(1.8, color);
        let painter = ui.painter();
        painter.line_segment([tip - vec2(14.0, 0.0), tip], stroke);
        painter.line_segment([tip + vec2(-5.5, -5.5), tip], stroke);
        painter.line_segment([tip + vec2(-5.5, 5.5), tip], stroke);
    }
    response
}

fn styled_button(
    ui: &mut Ui,
    enabled: bool,
    text: RichText,
    kind: Kind,
    height: f32,
    width: f32,
) -> Response {
    let (fill, hover, fore, border) = match kind {
        Kind::Primary => (LIGHT, LIGHT_HOVER, INK, LIGHT),
        Kind::Secondary => (Color32::TRANSPARENT, HOVER, TEXT, OUTLINE),
        Kind::Ghost => (Color32::TRANSPARENT, HOVER, MUTED, Color32::TRANSPARENT),
        Kind::Danger => (
            Color32::TRANSPARENT,
            alpha(DANGER, 34),
            DANGER,
            alpha(DANGER, 110),
        ),
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
            state.corner_radius = CornerRadius::same(RADIUS);
        }
        if height < 30.0 {
            ui.spacing_mut().button_padding = Vec2::new(10.0, 2.0);
        }
        ui.add_enabled(
            enabled,
            egui::Button::new(text.color(fore)).min_size(Vec2::new(width, height)),
        )
    })
    .inner
}

/// A small status pill: the colour's own words, in capitals, on a faint
/// capsule of it.
pub fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_uppercase(), heading_font(10.5), color);
    let size = galley.size() + Vec2::new(18.0, 9.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(u8::try_from(rect.height() as u32 / 2).unwrap_or(u8::MAX));
        ui.painter().rect(
            rect,
            radius,
            alpha(color, 30),
            Stroke::new(1.0, alpha(color, 120)),
            StrokeKind::Inside,
        );
        let at = rect.center() - galley.size() / 2.0;
        ui.painter().galley(at, galley, color);
    }
    let label = text.to_owned();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &label));
    response
}

/// Rows of a name and its value, the value in the log's type, as the TPF2
/// launcher lists versions.
pub fn rows(ui: &mut Ui, id: &str, rows: &[(&str, &str)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([18.0, 6.0])
        .show(ui, |ui| {
            for (name, value) in rows {
                ui.label(RichText::new(*name).size(12.0).color(LABEL));
                ui.label(RichText::new(*value).monospace().color(TEXT));
                ui.end_row();
            }
        });
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
                    Stroke::new(1.0, LINE),
                );
            }
            step_mark(ui, index + 1, *step);
            ui.label(step_text(text, *step));
        }
    });
}

/// The same steps one under the other, joined by a line, for the column
/// under the game's name.
pub fn checklist_column(ui: &mut Ui, steps: &[(&str, Step)]) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.spacing_mut().interact_size.y = 22.0;
    for (index, (text, step)) in steps.iter().enumerate() {
        if index > 0 {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(22.0, 9.0), Sense::hover());
            ui.painter().line_segment(
                [rect.center_top(), rect.center_bottom()],
                Stroke::new(1.0, LINE),
            );
        }
        ui.horizontal(|ui| {
            step_mark(ui, index + 1, *step);
            ui.label(step_text(text, *step));
        });
    }
}

fn step_text(text: &str, step: Step) -> RichText {
    let text = RichText::new(text);
    match step {
        Step::Done => text.color(MUTED),
        Step::Current => text.font(heading_font(14.0)).color(TEXT),
        Step::Pending => text.color(FAINT),
    }
}

/// A step's circle: ticked when done, numbered otherwise.
fn step_mark(ui: &mut Ui, number: usize, step: Step) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
    let center = rect.center();
    let painter = ui.painter();
    match step {
        Step::Done => {
            painter.circle(center, 10.0, ACCENT, Stroke::NONE);
            let tick = Stroke::new(2.0, INK);
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
            painter.circle(center, 10.0, LIGHT, Stroke::NONE);
            numeral(painter, center, number, INK);
        }
        Step::Pending => {
            painter.circle(center, 10.0, alpha(BG, 160), Stroke::new(1.0, FAINT));
            numeral(painter, center, number, FAINT);
        }
    }
}

fn numeral(painter: &Painter, center: Pos2, number: usize, color: Color32) {
    painter.text(
        center,
        Align2::CENTER_CENTER,
        number.to_string(),
        heading_font(11.5),
        color,
    );
}

/// A dark log box, newest line last, in the log's type.
pub fn log_box<'a>(ui: &mut Ui, id: &str, lines: impl Iterator<Item = &'a str>, height: f32) {
    Frame::new()
        .fill(LOG_BG)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(RADIUS)
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

/// The game's name, TRANSPORT FEVER 3, in two lines on dark blocks, over a
/// band that says MULTIPLAYER, and under it a line naming this launcher:
/// as the TPF2 launcher shows its game's name. `size` is the name's.
pub fn wordmark(ui: &mut Ui, size: f32, byline: &str) -> Response {
    let bold = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(Id::new(DISPLAY_IS_BOLD)))
        .unwrap_or(false);
    let painter = ui.painter().clone();
    let lines: Vec<_> = ["TRANSPORT", "FEVER 3"]
        .iter()
        .map(|line| painter.layout_no_wrap((*line).to_owned(), display_font(size), TEXT))
        .collect();
    // Letters spaced by thin spaces, as a band's are.
    let spaced: String = "MULTIPLAYER"
        .chars()
        .map(String::from)
        .collect::<Vec<_>>()
        .join("\u{2009}");
    let band = painter.layout_no_wrap(spaced, heading_font(size * 0.3), TEXT);
    let byline = painter.layout_no_wrap(byline.to_owned(), FontId::proportional(12.5), MUTED);

    let pad = vec2(size * 0.12, size * 0.02);
    let leading = size * 0.02;
    let line_height = |galley: &std::sync::Arc<egui::Galley>| galley.size().y * 0.86;
    let name_width = lines
        .iter()
        .map(|galley| galley.size().x)
        .fold(0.0_f32, f32::max);
    let height = lines
        .iter()
        .map(|galley| line_height(galley) + leading)
        .sum::<f32>()
        + band.size().y
        + pad.y * 2.0
        + size * 0.1
        + 18.0
        + byline.size().y;
    let width = (name_width + pad.x * 2.0).max(band.size().x + pad.x * 2.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    response.widget_info(|| {
        WidgetInfo::labeled(WidgetType::Label, true, "Transport Fever 3 Multiplayer")
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let block = Color32::from_rgba_premultiplied(6, 8, 9, 225);
    let shadow = size * 0.05;
    let mut y = rect.top();
    for galley in &lines {
        let height = line_height(galley);
        let back = Rect::from_min_size(
            pos2(rect.left(), y),
            vec2(galley.size().x + pad.x * 2.0, height + pad.y * 2.0),
        );
        painter.rect_filled(back.translate(vec2(shadow, shadow)), 0.0, alpha(BG, 170));
        painter.rect_filled(back, 0.0, block);
        let at = pos2(rect.left() + pad.x, y + pad.y - galley.size().y * 0.07);
        if bold {
            painter.galley(at, galley.clone(), TEXT);
        } else {
            // egui's own font is light: set it several times over, a hair
            // apart, to make it bold.
            let step = (size / 60.0).max(0.6);
            for offset in [
                vec2(0.0, 0.0),
                vec2(step, 0.0),
                vec2(0.0, step),
                vec2(step, step),
            ] {
                painter.galley(at + offset, galley.clone(), TEXT);
            }
        }
        y += height + pad.y * 2.0 + leading - pad.y * 2.0 + leading;
    }
    y += size * 0.1;
    let band_rect = Rect::from_min_size(
        pos2(rect.left(), y),
        vec2(
            band.size().x + pad.x * 2.0,
            band.size().y + pad.y * 2.0 + 4.0,
        ),
    );
    painter.rect_filled(band_rect, 0.0, block);
    painter.rect_filled(
        Rect::from_min_size(band_rect.left_top(), vec2(4.0, band_rect.height())),
        0.0,
        ACCENT,
    );
    painter.galley(
        pos2(band_rect.left() + pad.x, band_rect.top() + pad.y + 2.0),
        band,
        TEXT,
    );
    let rule_y = band_rect.bottom() + 18.0 + byline.size().y / 2.0;
    painter.line_segment(
        [pos2(rect.left(), rule_y), pos2(rect.left() + 36.0, rule_y)],
        Stroke::new(1.0, MUTED),
    );
    painter.galley(
        pos2(rect.left() + 46.0, rule_y - byline.size().y / 2.0),
        byline,
        MUTED,
    );
    response
}

/// The launcher's mark, as its icon: rails and sleepers on a blue tile.
pub fn mark(ui: &mut Ui, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let unit = size / 64.0;
        painter.rect_filled(
            rect,
            CornerRadius::same((12.0 * unit) as u8),
            Color32::from_rgb(38, 110, 196),
        );
        for x in (6..=57).step_by(10) {
            painter.rect_filled(
                Rect::from_min_size(
                    rect.min + vec2(x as f32 * unit, 18.0 * unit),
                    vec2(4.0 * unit, 28.0 * unit),
                ),
                0.0,
                Color32::from_rgb(120, 84, 52),
            );
        }
        for y in [22.0, 38.0] {
            painter.rect_filled(
                Rect::from_min_size(
                    rect.min + vec2(4.0 * unit, y * unit),
                    vec2(56.0 * unit, 4.0 * unit),
                ),
                0.0,
                Color32::from_rgb(236, 240, 245),
            );
        }
    }
    response
}

/// A small numbers generator, so the art is the same in every frame and on
/// every machine.
struct Seeded(u64);

impl Seeded {
    /// A number in `0.0..1.0`.
    fn next(&mut self) -> f32 {
        // Knuth's MMIX constants.
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
}

/// The window's background: a railway viaduct crossing a city at dusk,
/// drawn for TPF3-MP; no image of the game's is used. It is shaded on the
/// left, under the game's name, and along the top and bottom, under the
/// header and the bar.
pub fn art(painter: &Painter, rect: Rect) {
    let painter = &painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let (w, h) = (rect.width(), rect.height());
    let at = |x: f32, y: f32| rect.min + vec2(x * w, y * h);
    let band = |top: f32, bottom: f32| Rect::from_min_max(at(0.0, top), at(1.0, bottom));
    let horizon = 0.64;

    // The sky, darkening upwards from a warm horizon.
    painter.rect_filled(rect, 0.0, BG);
    vertical(
        painter,
        band(0.0, 0.42),
        Color32::from_rgb(9, 17, 23),
        Color32::from_rgb(19, 38, 48),
    );
    vertical(
        painter,
        band(0.42, horizon),
        Color32::from_rgb(19, 38, 48),
        Color32::from_rgb(84, 74, 62),
    );
    // The evening glow behind the city, between the columns.
    let sky = painter.with_clip_rect(band(0.0, horizon).intersect(painter.clip_rect()));
    glow(
        &sky,
        at(0.56, horizon),
        0.42 * w,
        alpha(Color32::from_rgb(236, 150, 82), 70),
    );
    glow(
        &sky,
        at(0.56, horizon),
        0.16 * w,
        alpha(Color32::from_rgb(255, 196, 120), 60),
    );
    // A few stars.
    let mut seed = Seeded(0x7f3a_2c11_9d05_e3b1);
    for _ in 0..70 {
        let star = at(seed.next(), seed.between(0.02, 0.36));
        sky.circle_filled(
            star,
            seed.between(0.5, 1.2),
            alpha(TEXT, (seed.between(30.0, 110.0)) as u8),
        );
    }

    // The city: a far row, paler in the haze, then a near one with lit
    // windows.
    skyline(
        painter,
        rect,
        &mut seed,
        horizon,
        (0.05, 0.17),
        (0.012, 0.034),
        Color32::from_rgb(35, 46, 50),
        0.03,
    );
    skyline(
        painter,
        rect,
        &mut seed,
        horizon + 0.02,
        (0.08, 0.32),
        (0.022, 0.058),
        Color32::from_rgb(13, 21, 25),
        0.10,
    );

    // The ground in front.
    vertical(
        painter,
        band(horizon + 0.02, 1.0),
        Color32::from_rgb(9, 13, 15),
        Color32::from_rgb(5, 7, 8),
    );

    // The viaduct: a deck on piers, arches between them.
    let deck_top = 0.70;
    let deck_bottom = 0.722;
    let foot = 0.90;
    let stone = Color32::from_rgb(8, 12, 15);
    painter.rect_filled(
        Rect::from_min_max(at(0.0, deck_top), at(1.0, deck_bottom)),
        0.0,
        stone,
    );
    let span = 0.105;
    let pier = 0.014;
    let spring = 0.80;
    let mut x = -0.03;
    while x < 1.03 {
        painter.rect_filled(
            Rect::from_min_max(at(x, deck_bottom), at(x + pier, foot)),
            0.0,
            stone,
        );
        // The spandrels over the arch, strip by strip.
        let (left, right) = (x + pier, x + span);
        let middle = (left + right) / 2.0;
        let half = (right - left) / 2.0;
        const STRIPS: usize = 14;
        for strip in 0..STRIPS {
            let x0 = left + (right - left) * strip as f32 / STRIPS as f32;
            let x1 = left + (right - left) * (strip + 1) as f32 / STRIPS as f32;
            let arch = |x: f32| {
                let across = ((x - middle) / half).clamp(-1.0, 1.0);
                spring - (spring - deck_bottom - 0.012) * (1.0 - across * across).sqrt()
            };
            painter.add(Shape::convex_polygon(
                vec![
                    at(x0, deck_bottom),
                    at(x1, deck_bottom),
                    at(x1, arch(x1)),
                    at(x0, arch(x0)),
                ],
                stone,
                Stroke::NONE,
            ));
        }
        x += span;
    }
    // A parapet catching the last light.
    painter.line_segment(
        [at(0.0, deck_top), at(1.0, deck_top)],
        Stroke::new(1.0, alpha(ACCENT, 50)),
    );

    // A train crossing, lit, its headlight towards the glow.
    train(painter, rect, deck_top);

    // Shade for what is drawn over the art.
    horizontal(
        painter,
        Rect::from_min_max(at(0.0, 0.0), at(0.64, 1.0)),
        alpha(BG, 228),
        alpha(BG, 60),
    );
    horizontal(
        painter,
        Rect::from_min_max(at(0.64, 0.0), at(1.0, 1.0)),
        alpha(BG, 60),
        alpha(BG, 120),
    );
    vertical(painter, band(0.0, 0.16), alpha(BG, 190), alpha(BG, 0));
    vertical(painter, band(0.78, 1.0), alpha(BG, 0), alpha(BG, 235));
}

/// A row of buildings standing on `base`, of the heights and widths given
/// as parts of the window's, some windows lit.
#[allow(clippy::too_many_arguments)]
fn skyline(
    painter: &Painter,
    rect: Rect,
    seed: &mut Seeded,
    base: f32,
    heights: (f32, f32),
    widths: (f32, f32),
    color: Color32,
    lit: f32,
) {
    let (w, h) = (rect.width(), rect.height());
    let mut x = -0.02;
    while x < 1.02 {
        let width = seed.between(widths.0, widths.1);
        let height = seed.between(heights.0, heights.1);
        let building = Rect::from_min_max(
            rect.min + vec2(x * w, (base - height) * h),
            rect.min + vec2((x + width) * w, base * h),
        );
        painter.rect_filled(building, 0.0, color);
        // A roof on some: a step, or a mast.
        if seed.next() < 0.3 {
            let step = Rect::from_min_size(
                building.left_top() + vec2(building.width() * 0.2, -h * 0.015),
                vec2(building.width() * 0.5, h * 0.015),
            );
            painter.rect_filled(step, 0.0, color);
        } else if seed.next() < 0.15 {
            painter.line_segment(
                [
                    building.center_top(),
                    building.center_top() - vec2(0.0, h * 0.04),
                ],
                Stroke::new(1.0, color),
            );
        }
        let (cell_w, cell_h) = (7.0, 10.0);
        let mut row = building.top() + 6.0;
        while row + 4.0 < building.bottom() - 4.0 {
            let mut column = building.left() + 4.0;
            while column + 3.0 < building.right() - 3.0 {
                if seed.next() < lit {
                    let warmth = seed.between(0.0, 1.0);
                    let light = mix(
                        Color32::from_rgb(255, 204, 128),
                        Color32::from_rgb(190, 225, 255),
                        warmth * warmth * warmth,
                    );
                    painter.rect_filled(
                        Rect::from_min_size(pos2(column, row), vec2(3.0, 4.0)),
                        0.0,
                        alpha(light, seed.between(90.0, 200.0) as u8),
                    );
                }
                column += cell_w;
            }
            row += cell_h;
        }
        x += width + seed.between(0.0, 0.006);
    }
}

/// A train of five cars on the viaduct, windows lit, headlight on.
fn train(painter: &Painter, rect: Rect, deck: f32) {
    let (w, h) = (rect.width(), rect.height());
    let at = |x: f32, y: f32| rect.min + vec2(x * w, y * h);
    let body = Color32::from_rgb(17, 26, 31);
    let height = 0.034;
    let (start, car, gap) = (0.28, 0.082, 0.004);
    for index in 0..5 {
        let left = start + index as f32 * (car + gap);
        let right = left + car;
        let top = deck - height;
        let front = index == 4;
        let mut outline = vec![at(left, deck), at(left, top + 0.006), at(left + 0.004, top)];
        if front {
            // The nose, sloped.
            outline.push(at(right - 0.018, top));
            outline.push(at(right, deck - 0.010));
            outline.push(at(right, deck));
        } else {
            outline.push(at(right - 0.004, top));
            outline.push(at(right, top + 0.006));
            outline.push(at(right, deck));
        }
        painter.add(Shape::convex_polygon(outline, body, Stroke::NONE));
        painter.line_segment(
            [
                at(left + 0.004, top),
                at(right - if front { 0.018 } else { 0.004 }, top),
            ],
            Stroke::new(1.0, alpha(ACCENT, 90)),
        );
        // A band of lit windows.
        let windows = Rect::from_min_max(
            at(left + 0.006, top + height * 0.28),
            at(
                right - if front { 0.022 } else { 0.006 },
                top + height * 0.52,
            ),
        );
        painter.rect_filled(windows, 1.0, alpha(Color32::from_rgb(255, 214, 150), 150));
        let mut post = windows.left() + 9.0;
        while post < windows.right() - 2.0 {
            painter.line_segment(
                [pos2(post, windows.top()), pos2(post, windows.bottom())],
                Stroke::new(1.5, body),
            );
            post += 11.0;
        }
    }
    // The headlight and its beam.
    let lamp = at(start + 5.0 * car + 4.0 * gap - 0.002, deck - 0.008);
    let beam = Color32::from_rgb(255, 238, 205);
    painter.add(Shape::convex_polygon(
        vec![
            lamp,
            lamp + vec2(w * 0.16, -h * 0.03),
            lamp + vec2(w * 0.16, h * 0.03),
        ],
        alpha(beam, 16),
        Stroke::NONE,
    ));
    for (radius, opacity) in [(18.0, 20), (9.0, 50), (4.0, 120)] {
        painter.circle_filled(lamp, radius, alpha(beam, opacity));
    }
    painter.circle_filled(lamp, 2.0, beam);
}

/// A soft round glow: `color` at its middle, fading to nothing at
/// `radius`.
fn glow(painter: &Painter, center: Pos2, radius: f32, color: Color32) {
    const POINTS: u32 = 64;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    for point in 0..POINTS {
        let angle = std::f32::consts::TAU * point as f32 / POINTS as f32;
        mesh.colored_vertex(
            center + radius * vec2(angle.cos(), angle.sin()),
            Color32::TRANSPARENT,
        );
    }
    for point in 0..POINTS {
        mesh.add_triangle(0, 1 + point, 1 + (point + 1) % POINTS);
    }
    painter.add(Shape::mesh(mesh));
}

/// `rect` filled from `top` to `bottom`.
fn vertical(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

/// `rect` filled from `left` to `right`.
fn horizontal(painter: &Painter, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::Step::{self, Current, Done, Pending};
    use super::*;

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

    #[test]
    fn a_font_egui_could_not_read_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken.ttf");
        std::fs::write(&broken, b"not a font").unwrap();
        let missing = dir.path().join("missing.ttf");
        let candidates = [
            SystemFont {
                path: missing,
                axes: PLAIN,
            },
            SystemFont {
                path: broken,
                axes: PLAIN,
            },
        ];
        assert!(first_readable(&candidates).is_none());
    }

    #[test]
    fn the_art_is_the_same_every_time() {
        let mut first = Seeded(1);
        let mut second = Seeded(1);
        let a: Vec<f32> = (0..100).map(|_| first.next()).collect();
        let b: Vec<f32> = (0..100).map(|_| second.next()).collect();
        assert_eq!(a, b);
        assert!(a.iter().all(|value| (0.0..1.0).contains(value)));
    }

    #[test]
    fn glass_fades_from_its_top_to_its_bottom() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 100.0));
        let mesh = rounded_gradient(rect, 8.0, GLASS_TOP, GLASS_BOTTOM);
        let top = mesh
            .vertices
            .iter()
            .find(|vertex| vertex.pos.y == 0.0)
            .unwrap();
        let bottom = mesh
            .vertices
            .iter()
            .find(|vertex| vertex.pos.y == 100.0)
            .unwrap();
        assert_eq!(top.color, GLASS_TOP);
        assert_eq!(bottom.color, GLASS_BOTTOM);
        assert!(mesh.is_valid());
    }
}
