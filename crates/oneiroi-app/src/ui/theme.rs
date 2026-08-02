//! Operator theme: palette presets, accent override, control density and the
//! deck-strip layout preference.
//!
//! Every colour the UI paints comes from a [`ThemePalette`] so a preset change
//! restyles the whole operator surface at once. The palette is resolved once
//! per frame and applied to the egui style only when something actually
//! changed, because rebuilding the style each frame invalidates egui's style
//! cache for no benefit.

use oneiroi_media::DeckId;

/// A complete operator colour scheme.
#[derive(Clone, Copy, PartialEq)]
pub struct ThemePalette {
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

    /// Fill used by an accent-selected widget on this background.
    pub fn selection_fill(&self) -> egui::Color32 {
        blend(
            self.accent,
            self.background,
            if self.dark { 0.35 } else { 0.15 },
        )
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

/// The operator's theme choices plus the bookkeeping to apply them lazily.
#[derive(Default)]
pub struct ThemeState {
    pub preset: ThemePreset,
    pub accent_override: Option<egui::Color32>,
    pub density: Density,
    pub deck_layout: DeckLayout,
    applied: Option<(ThemePreset, Option<egui::Color32>, Density)>,
}

impl ThemeState {
    /// The preset palette with the operator's accent override folded in.
    pub fn palette(&self) -> ThemePalette {
        let mut palette = self.preset.palette();
        if let Some(accent) = self.accent_override {
            palette.accent = accent;
        }
        palette
    }

    /// Re-style the context if the theme changed since the last frame.
    pub fn ensure_applied(&mut self, ctx: &egui::Context) {
        let key = (self.preset, self.accent_override, self.density);
        if self.applied == Some(key) {
            return;
        }
        self.applied = Some(key);
        apply(ctx, &self.palette(), self.density);
    }

    /// Body of the header's theme menu.
    pub fn picker_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Preset").weak().small());
        for preset in ThemePreset::ALL {
            let palette = preset.palette();
            ui.horizontal(|ui| {
                for color in [palette.accent, palette.secondary, palette.deck[2]] {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.0, color);
                }
                if ui
                    .selectable_label(self.preset == preset, preset.label())
                    .clicked()
                {
                    self.preset = preset;
                }
            });
        }
        ui.separator();

        ui.label(egui::RichText::new("Accent").weak().small());
        ui.horizontal(|ui| {
            let mut accent = self
                .accent_override
                .unwrap_or_else(|| self.preset.palette().accent);
            if ui.color_edit_button_srgba(&mut accent).changed() {
                self.accent_override = Some(accent);
            }
            if self.accent_override.is_some() && ui.button("Reset").clicked() {
                self.accent_override = None;
            }
        });
        ui.separator();

        ui.label(egui::RichText::new("Density").weak().small());
        ui.horizontal(|ui| {
            for density in Density::ALL {
                if ui
                    .selectable_label(self.density == density, density.label())
                    .clicked()
                {
                    self.density = density;
                }
            }
        });
        ui.separator();

        ui.label(egui::RichText::new("Deck layout").weak().small());
        ui.horizontal(|ui| {
            for layout in DeckLayout::ALL {
                if ui
                    .selectable_label(self.deck_layout == layout, layout.label())
                    .clicked()
                {
                    self.deck_layout = layout;
                }
            }
        });
    }
}

fn apply(ctx: &egui::Context, palette: &ThemePalette, density: Density) {
    let theme = ctx.theme();
    let mut style = (*ctx.style_of(theme)).clone();

    style.spacing.item_spacing = density.item_spacing();
    style.spacing.button_padding = density.button_padding();
    style.spacing.interact_size.y = density.interact_height();
    style.spacing.slider_width = density.slider_width();

    style.visuals = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    style.visuals.panel_fill = palette.background;
    style.visuals.window_fill = palette.surface;
    style.visuals.extreme_bg_color = palette.extreme;
    style.visuals.faint_bg_color = palette.faint;
    style.visuals.code_bg_color = palette.code;
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
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, palette.accent_hover());
    style.visuals.widgets.active.bg_fill = blend(palette.control, palette.accent, 0.35);
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, palette.accent);
    style.visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, palette.secondary);
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(6);
    }
    style.visuals.window_stroke = egui::Stroke::new(1.0, palette.stroke);
    style.visuals.window_corner_radius = egui::CornerRadius::same(10);
    style.visuals.menu_corner_radius = egui::CornerRadius::same(8);
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
