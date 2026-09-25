//! Operator theme: palette presets, accent override, control density and the
//! deck-strip layout preference.
//!
//! Every colour the UI paints comes from a [`ThemePalette`] so a preset change
//! restyles the whole operator surface at once. The palette is resolved once
//! per frame and applied to the egui style only when something actually
//! changed, because rebuilding the style each frame invalidates egui's style
//! cache for no benefit.

use virtual_io::{ThemeAppearanceProject, ThemeProject};
use virtual_media::DeckId;

mod presets;
mod text_outline;
pub use text_outline::outline_text;

/// A complete operator colour scheme.
#[derive(Clone, Copy, PartialEq)]
pub struct ThemePalette {
    pub text: egui::Color32,
    pub muted_text: egui::Color32,
    pub text_outline: egui::Color32,
    pub outline_width: f32,
    pub background: egui::Color32,
    pub surface: egui::Color32,
    pub control: egui::Color32,
    pub faint: egui::Color32,
    pub extreme: egui::Color32,
    pub code: egui::Color32,
    pub stroke: egui::Color32,
    pub accent: egui::Color32,
    pub secondary: egui::Color32,
    pub danger: egui::Color32,
    pub success: egui::Color32,
    pub warning: egui::Color32,
    pub idle: egui::Color32,
    /// One hue per deck, in `DeckId::ALL` order. Doubles as the channel-strip
    /// banding so an operator can find deck C with peripheral vision.
    pub deck: [egui::Color32; 4],
    /// Whether the palette builds on egui's dark or light visuals.
    pub dark: bool,
}

impl ThemePalette {
    pub fn deck_color(&self, id: DeckId) -> egui::Color32 {
        self.deck[id.index()]
    }

    /// Hover shade of the accent: pushed toward white rather than scaled,
    /// so it stays visible on light themes too.
    pub fn accent_hover(&self) -> egui::Color32 {
        blend(self.accent, egui::Color32::WHITE, 0.25)
    }

    /// A restrained semantic tint over the normal control colour. Blending
    /// from the control (rather than multiplying RGB) preserves readable
    /// foreground contrast in both dark and light presets.
    pub fn control_tint(&self, color: egui::Color32, amount: f32) -> egui::Color32 {
        blend(self.control, color, amount)
    }

    /// A restrained deck tint over the panel surface.
    pub fn surface_tint(&self, color: egui::Color32, amount: f32) -> egui::Color32 {
        blend(self.surface, color, amount)
    }

    /// Fill used by an accent-selected widget on this background.
    pub fn selection_fill(&self) -> egui::Color32 {
        self.control_tint(self.accent, if self.dark { 0.45 } else { 0.18 })
    }
}

fn blend(a: egui::Color32, b: egui::Color32, amount: f32) -> egui::Color32 {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * amount) as u8;
    egui::Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThemePreset {
    /// The original operator look: cold cyan on blue-black.
    #[default]
    Nocturne,
    Ultraviolet,
    Ember,
    Cathode,
    /// Light theme for daylight patching and rehearsal, not for show mode.
    Daylight,
}

impl ThemePreset {
    pub const ALL: [Self; 5] = [
        Self::Nocturne,
        Self::Ultraviolet,
        Self::Ember,
        Self::Cathode,
        Self::Daylight,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Nocturne => "Nocturne",
            Self::Ultraviolet => "Ultraviolet",
            Self::Ember => "Ember",
            Self::Cathode => "Cathode",
            Self::Daylight => "Daylight",
        }
    }

    /// Stable identifier written into project files.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Nocturne => "nocturne",
            Self::Ultraviolet => "ultraviolet",
            Self::Ember => "ember",
            Self::Cathode => "cathode",
            Self::Daylight => "daylight",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.name() == name)
    }

    pub fn palette(self) -> ThemePalette {
        let rgb = egui::Color32::from_rgb;
        match self {
            Self::Nocturne => ThemePalette {
                text: rgb(236, 240, 248),
                muted_text: rgb(165, 174, 192),
                text_outline: rgb(0, 0, 0),
                outline_width: 1.25,
                background: rgb(12, 13, 20),
                surface: rgb(22, 24, 35),
                control: rgb(33, 36, 51),
                faint: rgb(27, 29, 42),
                extreme: rgb(8, 9, 15),
                code: rgb(18, 20, 31),
                stroke: rgb(49, 53, 72),
                accent: rgb(71, 214, 255),
                secondary: rgb(176, 113, 255),
                danger: rgb(255, 70, 98),
                success: rgb(92, 226, 146),
                warning: rgb(255, 194, 79),
                idle: rgb(105, 110, 129),
                deck: [
                    rgb(71, 214, 255),
                    rgb(176, 113, 255),
                    rgb(255, 151, 74),
                    rgb(84, 224, 155),
                ],
                dark: true,
            },
            Self::Ultraviolet => ThemePalette {
                text: rgb(236, 240, 248),
                muted_text: rgb(165, 174, 192),
                text_outline: rgb(0, 0, 0),
                outline_width: 1.25,
                background: rgb(14, 10, 22),
                surface: rgb(24, 18, 38),
                control: rgb(36, 27, 56),
                faint: rgb(30, 23, 46),
                extreme: rgb(9, 6, 15),
                code: rgb(20, 15, 32),
                stroke: rgb(58, 45, 86),
                accent: rgb(186, 124, 255),
                secondary: rgb(255, 110, 199),
                danger: rgb(255, 82, 120),
                success: rgb(126, 231, 170),
                warning: rgb(255, 203, 107),
                idle: rgb(118, 105, 143),
                deck: [
                    rgb(186, 124, 255),
                    rgb(255, 110, 199),
                    rgb(120, 190, 255),
                    rgb(126, 231, 170),
                ],
                dark: true,
            },
            Self::Ember => ThemePalette {
                text: rgb(236, 240, 248),
                muted_text: rgb(165, 174, 192),
                text_outline: rgb(0, 0, 0),
                outline_width: 1.25,
                background: rgb(18, 12, 10),
                surface: rgb(30, 20, 17),
                control: rgb(46, 30, 25),
                faint: rgb(38, 26, 22),
                extreme: rgb(12, 8, 7),
                code: rgb(26, 17, 14),
                stroke: rgb(70, 48, 40),
                accent: rgb(255, 138, 66),
                secondary: rgb(255, 90, 95),
                danger: rgb(255, 64, 64),
                success: rgb(178, 220, 110),
                warning: rgb(255, 203, 79),
                idle: rgb(134, 112, 102),
                deck: [
                    rgb(255, 138, 66),
                    rgb(255, 90, 95),
                    rgb(255, 203, 79),
                    rgb(178, 220, 110),
                ],
                dark: true,
            },
            Self::Cathode => ThemePalette {
                text: rgb(236, 240, 248),
                muted_text: rgb(165, 174, 192),
                text_outline: rgb(0, 0, 0),
                outline_width: 1.25,
                background: rgb(8, 14, 10),
                surface: rgb(14, 24, 17),
                control: rgb(21, 36, 26),
                faint: rgb(18, 30, 22),
                extreme: rgb(5, 10, 7),
                code: rgb(11, 20, 14),
                stroke: rgb(38, 64, 46),
                accent: rgb(94, 255, 146),
                secondary: rgb(94, 240, 255),
                danger: rgb(255, 92, 92),
                success: rgb(94, 255, 146),
                warning: rgb(240, 255, 120),
                idle: rgb(96, 122, 104),
                deck: [
                    rgb(94, 255, 146),
                    rgb(94, 240, 255),
                    rgb(240, 255, 120),
                    rgb(255, 168, 94),
                ],
                dark: true,
            },
            Self::Daylight => ThemePalette {
                text: rgb(25, 31, 43),
                muted_text: rgb(85, 93, 111),
                text_outline: rgb(255, 255, 255),
                outline_width: 1.25,
                background: rgb(236, 238, 244),
                surface: rgb(248, 249, 252),
                control: rgb(222, 226, 236),
                faint: rgb(228, 231, 240),
                extreme: rgb(255, 255, 255),
                code: rgb(240, 242, 247),
                stroke: rgb(190, 196, 212),
                accent: rgb(0, 122, 204),
                secondary: rgb(146, 86, 220),
                danger: rgb(211, 47, 72),
                success: rgb(46, 160, 90),
                warning: rgb(204, 142, 0),
                idle: rgb(148, 155, 172),
                deck: [
                    rgb(0, 122, 204),
                    rgb(146, 86, 220),
                    rgb(230, 120, 30),
                    rgb(46, 160, 90),
                ],
                dark: false,
            },
        }
    }
}

/// Control sizing. Compact fits a laptop beside a DAW; Roomy suits a touch
/// screen or standing at an FOH desk.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Density {
    Compact,
    #[default]
    Cozy,
    Roomy,
}

impl Density {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Cozy, Self::Roomy];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Cozy => "Cozy",
            Self::Roomy => "Roomy",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Cozy => "cozy",
            Self::Roomy => "roomy",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|density| density.name() == name)
    }

    fn item_spacing(self) -> egui::Vec2 {
        match self {
            Self::Compact => egui::vec2(6.0, 4.0),
            Self::Cozy => egui::vec2(8.0, 7.0),
            Self::Roomy => egui::vec2(10.0, 9.0),
        }
    }

    fn button_padding(self) -> egui::Vec2 {
        match self {
            Self::Compact => egui::vec2(8.0, 4.0),
            Self::Cozy => egui::vec2(10.0, 6.0),
            Self::Roomy => egui::vec2(12.0, 8.0),
        }
    }

    fn interact_height(self) -> f32 {
        match self {
            Self::Compact => 22.0,
            Self::Cozy => 26.0,
            Self::Roomy => 30.0,
        }
    }

    pub fn slider_width(self) -> f32 {
        match self {
            Self::Compact => 140.0,
            Self::Cozy => 170.0,
            Self::Roomy => 210.0,
        }
    }
}

/// How the four deck channel strips are arranged.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeckLayout {
    /// Pick per frame from the available width.
    #[default]
    Auto,
    /// Two-by-two grid.
    Grid,
    /// All four strips side by side in a horizontally scrollable cascade,
    /// like channel strips on a desk.
    Cascade,
}

impl DeckLayout {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Grid, Self::Cascade];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Grid => "Grid 2×2",
            Self::Cascade => "Cascade",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Grid => "grid",
            Self::Cascade => "cascade",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layout| layout.name() == name)
    }
}

/// Concrete arrangement after Auto is resolved against a width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolvedDeckLayout {
    Grid,
    Cascade,
    /// One strip per row; the narrow-window fallback.
    Stack,
}

/// Width of one deck strip in the cascade, chosen so four strips plus
/// spacing fit a 1600 px window without scrolling.
pub const CASCADE_STRIP_WIDTH: f32 = 380.0;

impl DeckLayout {
    pub fn resolve(self, available_width: f32) -> ResolvedDeckLayout {
        match self {
            Self::Grid => ResolvedDeckLayout::Grid,
            Self::Cascade => ResolvedDeckLayout::Cascade,
            Self::Auto => {
                if available_width >= 4.0 * CASCADE_STRIP_WIDTH + 48.0 {
                    ResolvedDeckLayout::Cascade
                } else if available_width >= 900.0 {
                    ResolvedDeckLayout::Grid
                } else {
                    ResolvedDeckLayout::Stack
                }
            }
        }
    }
}

/// Theme choices are saved in projects; the preset library is shared across shows.
#[derive(Default)]
pub struct ThemeState {
    pub preset: ThemePreset,
    pub accent_override: Option<egui::Color32>,
    pub density: Density,
    pub deck_layout: DeckLayout,
    pub appearance: ThemeAppearanceProject,
    pub editor_open: bool,
    library: presets::PresetLibrary,
    preset_name: String,
    selected_preset: String,
    applied: Option<ThemeProject>,
}

impl ThemeState {
    pub fn snapshot(&self) -> ThemeProject {
        ThemeProject {
            preset: self.preset.name().to_owned(),
            accent: self.accent_override.map(|c| [c.r(), c.g(), c.b()]),
            density: self.density.name().to_owned(),
            deck_layout: self.deck_layout.name().to_owned(),
            appearance: self.appearance.sanitized(),
        }
    }

    pub fn restore(&mut self, project: &ThemeProject) {
        if let Some(preset) = ThemePreset::from_name(&project.preset) {
            self.preset = preset;
        }
        self.accent_override = project
            .accent
            .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b));
        if let Some(density) = Density::from_name(&project.density) {
            self.density = density;
        }
        if let Some(layout) = DeckLayout::from_name(&project.deck_layout) {
            self.deck_layout = layout;
        }
        self.appearance = project.appearance.sanitized();
    }

    pub fn load_library(&mut self) {
        self.library = presets::PresetLibrary::load_default();
        if let Some(theme) = self.library.active().cloned() {
            self.restore(&theme);
        }
    }

    pub fn palette(&self) -> ThemePalette {
        let mut p = self.preset.palette();
        p.stroke = blend(p.stroke, p.text, 0.22);
        p.outline_width = self.appearance.element_outline;
        if let Some(accent) = self.accent_override {
            p.accent = accent;
        }
        for (key, field) in palette_fields(&mut p) {
            if let Some([r, g, b]) = self.appearance.colors.get(key) {
                *field = egui::Color32::from_rgb(*r, *g, *b);
            }
        }
        p
    }

    pub fn ensure_applied(&mut self, ctx: &egui::Context) {
        self.library.poll();
        let key = self.snapshot();
        if self.applied.as_ref() == Some(&key) {
            return;
        }
        apply(ctx, &self.palette(), self.density, &self.appearance);
        self.applied = Some(key);
    }

    pub fn editor_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.editor_open;
        egui::Window::new("Appearance")
            .open(&mut open)
            .default_width(440.0)
            .default_height(620.0)
            .resizable(true)
            .vscroll(true)
            .show(ctx, |ui| self.picker_ui(ui));
        self.editor_open = open;
    }

    pub fn picker_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Make it yours");
        ui.weak("Changes preview live. Save a named preset to reuse it across shows.");
        ui.label("Built-in starting points");
        ui.horizontal_wrapped(|ui| {
            for preset in ThemePreset::ALL {
                if ui
                    .selectable_label(self.preset == preset, preset.label())
                    .clicked()
                {
                    self.preset = preset;
                    self.accent_override = None;
                    self.appearance.colors.clear();
                }
            }
        });
        ui.separator();
        ui.strong("Definition & sizing");
        ui.add(
            egui::Slider::new(&mut self.appearance.element_outline, 0.0..=3.0)
                .text("Element outlines"),
        );
        ui.add(
            egui::Slider::new(&mut self.appearance.text_outline, 0.0..=1.5).text("Text outlines"),
        );
        ui.add(egui::Slider::new(&mut self.appearance.text_scale, 0.8..=1.5).text("Text size"));
        ui.add(
            egui::Slider::new(&mut self.appearance.corner_radius, 0..=16).text("Corner rounding"),
        );
        ui.weak("Set an outline width to zero to turn it off.");
        ui.horizontal_wrapped(|ui| {
            ui.label("Spacing");
            for density in Density::ALL {
                ui.selectable_value(&mut self.density, density, density.label());
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Deck layout");
            for layout in DeckLayout::ALL {
                ui.selectable_value(&mut self.deck_layout, layout, layout.label());
            }
        });
        ui.separator();
        egui::CollapsingHeader::new("Custom colors")
            .default_open(true)
            .show(ui, |ui| {
                let mut palette = self.palette();
                egui::Grid::new("theme-colors")
                    .num_columns(3)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for (key, color) in palette_fields(&mut palette) {
                            ui.label(color_label(key));
                            let mut rgb = [color.r(), color.g(), color.b()];
                            if ui.color_edit_button_srgb(&mut rgb).changed() {
                                if key == "accent" {
                                    self.accent_override = None;
                                }
                                self.appearance.colors.insert(key.to_owned(), rgb);
                            }
                            let custom = self.appearance.colors.contains_key(key)
                                || (key == "accent" && self.accent_override.is_some());
                            if ui.add_enabled(custom, egui::Button::new("Reset")).clicked() {
                                self.appearance.colors.remove(key);
                                if key == "accent" {
                                    self.accent_override = None;
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
        if ui.button("Reset appearance to built-in").clicked() {
            self.appearance = ThemeAppearanceProject::default();
            self.accent_override = None;
        }
        ui.separator();
        ui.strong("My presets");
        let entries = self.library.entries().to_vec();
        let busy = self.library.busy();
        ui.add_enabled_ui(!busy, |ui| {
            egui::ComboBox::from_id_salt("saved-theme")
                .selected_text(if self.selected_preset.is_empty() {
                    "Choose a saved preset"
                } else {
                    &self.selected_preset
                })
                .show_ui(ui, |ui| {
                    for entry in &entries {
                        ui.selectable_value(
                            &mut self.selected_preset,
                            entry.name.clone(),
                            &entry.name,
                        );
                    }
                });
            ui.horizontal_wrapped(|ui| {
                let selected = entries.iter().find(|e| e.name == self.selected_preset);
                if ui
                    .add_enabled(selected.is_some(), egui::Button::new("Load"))
                    .clicked()
                    && let Some(entry) = selected
                {
                    self.restore(&entry.theme);
                    self.preset_name.clone_from(&entry.name);
                    self.library.select(entry.theme.clone());
                }
                if ui
                    .add_enabled(selected.is_some(), egui::Button::new("Update selected"))
                    .clicked()
                {
                    self.library
                        .save(&self.selected_preset, self.snapshot(), true);
                }
                if ui
                    .add_enabled(selected.is_some(), egui::Button::new("Delete"))
                    .clicked()
                {
                    self.library.delete(&self.selected_preset);
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.preset_name)
                        .hint_text("New preset name")
                        .char_limit(64)
                        .desired_width(230.0),
                );
                if ui.button("Save new").clicked()
                    && self.library.save(&self.preset_name, self.snapshot(), false)
                {
                    self.selected_preset = self.preset_name.trim().to_owned();
                }
            });
        });
        if busy {
            ui.spinner();
        }
        ui.label(self.library.status());
    }
}

fn palette_fields(p: &mut ThemePalette) -> [(&'static str, &mut egui::Color32); 20] {
    let [a, b, c, d] = &mut p.deck;
    [
        ("background", &mut p.background),
        ("surface", &mut p.surface),
        ("control", &mut p.control),
        ("faint", &mut p.faint),
        ("extreme", &mut p.extreme),
        ("code", &mut p.code),
        ("text", &mut p.text),
        ("muted_text", &mut p.muted_text),
        ("stroke", &mut p.stroke),
        ("text_outline", &mut p.text_outline),
        ("accent", &mut p.accent),
        ("secondary", &mut p.secondary),
        ("danger", &mut p.danger),
        ("success", &mut p.success),
        ("warning", &mut p.warning),
        ("idle", &mut p.idle),
        ("deck_a", a),
        ("deck_b", b),
        ("deck_c", c),
        ("deck_d", d),
    ]
}

fn color_label(key: &str) -> &str {
    match key {
        "background" => "Background",
        "surface" => "Panels",
        "control" => "Controls",
        "faint" => "Inset panels",
        "extreme" => "Input fields",
        "code" => "Code fields",
        "text" => "Text",
        "muted_text" => "Secondary text",
        "stroke" => "Element outlines",
        "text_outline" => "Text outlines",
        "accent" => "Accent",
        "secondary" => "Secondary accent",
        "danger" => "Alerts / blackout",
        "success" => "Active / healthy",
        "warning" => "Warnings",
        "idle" => "Idle indicators",
        "deck_a" => "Deck A",
        "deck_b" => "Deck B",
        "deck_c" => "Deck C",
        "deck_d" => "Deck D",
        _ => key,
    }
}

fn apply(
    ctx: &egui::Context,
    palette: &ThemePalette,
    density: Density,
    appearance: &ThemeAppearanceProject,
) {
    let theme = ctx.theme();
    let mut style = (*ctx.style_of(theme)).clone();

    // Start from base font sizes so repeated edits never compound the scale.
    style.text_styles = egui::Style::default().text_styles;
    for font in style.text_styles.values_mut() {
        font.size *= appearance.text_scale;
    }
    style.spacing.item_spacing = density.item_spacing();
    style.spacing.button_padding = density.button_padding();
    style.spacing.interact_size.y = density.interact_height();
    style.spacing.slider_width = density.slider_width();

    style.visuals = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    style.visuals.override_text_color = Some(palette.text);
    style.visuals.weak_text_color = Some(palette.muted_text);
    style.visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(palette.outline_width, palette.stroke);
    style.visuals.widgets.inactive.bg_stroke =
        egui::Stroke::new(palette.outline_width, palette.stroke);
    style.visuals.panel_fill = palette.background;
    style.visuals.window_fill = palette.surface;
    style.visuals.extreme_bg_color = palette.extreme;
    style.visuals.faint_bg_color = palette.faint;
    style.visuals.code_bg_color = palette.code;
    style.visuals.warn_fg_color = palette.warning;
    style.visuals.error_fg_color = palette.danger;
    style.visuals.hyperlink_color = palette.accent;
    style.visuals.selection.bg_fill = palette.selection_fill();
    style.visuals.selection.stroke = egui::Stroke::new(
        1.0,
        if palette.dark {
            egui::Color32::WHITE
        } else {
            egui::Color32::BLACK
        },
    );
    style.visuals.widgets.inactive.weak_bg_fill = palette.control;
    style.visuals.widgets.inactive.bg_fill = palette.control;
    let hovered = blend(palette.control, palette.accent, 0.18);
    style.visuals.widgets.hovered.weak_bg_fill = hovered;
    style.visuals.widgets.hovered.bg_fill = hovered;
    style.visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(palette.outline_width, palette.accent_hover());
    style.visuals.widgets.active.bg_fill = blend(palette.control, palette.accent, 0.35);
    style.visuals.widgets.active.bg_stroke =
        egui::Stroke::new(palette.outline_width, palette.accent);
    style.visuals.widgets.open.bg_stroke =
        egui::Stroke::new(palette.outline_width, palette.secondary);
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(appearance.corner_radius);
    }
    style.visuals.window_stroke = egui::Stroke::new(palette.outline_width, palette.stroke);
    style.visuals.window_corner_radius =
        egui::CornerRadius::same(appearance.corner_radius.saturating_add(4));
    style.visuals.menu_corner_radius =
        egui::CornerRadius::same(appearance.corner_radius.saturating_add(2));
    style.visuals.collapsing_header_frame = true;

    ctx.set_style_of(theme, style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_names_round_trip() {
        for preset in ThemePreset::ALL {
            assert_eq!(ThemePreset::from_name(preset.name()), Some(preset));
        }
        for density in Density::ALL {
            assert_eq!(Density::from_name(density.name()), Some(density));
        }
        for layout in DeckLayout::ALL {
            assert_eq!(DeckLayout::from_name(layout.name()), Some(layout));
        }
        assert_eq!(ThemePreset::from_name("unknown"), None);
    }

    #[test]
    fn auto_layout_resolves_by_width() {
        assert_eq!(
            DeckLayout::Auto.resolve(1920.0),
            ResolvedDeckLayout::Cascade
        );
        assert_eq!(DeckLayout::Auto.resolve(1100.0), ResolvedDeckLayout::Grid);
        assert_eq!(DeckLayout::Auto.resolve(700.0), ResolvedDeckLayout::Stack);
        assert_eq!(DeckLayout::Grid.resolve(700.0), ResolvedDeckLayout::Grid);
        assert_eq!(
            DeckLayout::Cascade.resolve(700.0),
            ResolvedDeckLayout::Cascade
        );
    }

    #[test]
    fn accent_override_replaces_only_the_accent() {
        let mut theme = ThemeState::default();
        let stock = theme.palette();
        theme.accent_override = Some(egui::Color32::from_rgb(255, 0, 0));
        let overridden = theme.palette();
        assert_eq!(overridden.accent, egui::Color32::from_rgb(255, 0, 0));
        assert_eq!(overridden.surface, stock.surface);
        assert_eq!(overridden.deck, stock.deck);
    }
}
