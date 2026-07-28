//! egui overlay.
//!
//! The UI never touches the GPU or mutates render state directly. It edits
//! plain values that get read into a per-frame snapshot, which is the same
//! path the parameter/modulation system takes later.

use oneiroi_core::FrameTime;
use oneiroi_media::{
    CrossfadeBus, DeckId, DeckState, DeckTransport, EndMode, FourDeckMixer, MediaHealth,
};
use oneiroi_render::DeckEffects;

/// Everything the overlay owns. All plain data — no GPU handles, no channels.
pub struct UiState {
    pub master_opacity: f32,
    pub blackout: bool,
    pub master_freeze: bool,
    pub crossfader: f32,
    pub equal_power: bool,
    pub effects: [DeckEffects; 4],
    fps: FpsMeter,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            master_opacity: 1.0,
            blackout: false,
            master_freeze: false,
            crossfader: 0.5,
            equal_power: true,
            effects: [DeckEffects::default(); 4],
            fps: FpsMeter::default(),
        }
    }
}

/// Exponentially smoothed frame rate.
///
/// Instantaneous 1/delta is unreadable and a rolling window costs an
/// allocation; neither is worth it for a number a human reads.
#[derive(Default)]
struct FpsMeter {
    smoothed_delta: f64,
}

impl FpsMeter {
    fn push(&mut self, delta: f64) {
        if delta <= 0.0 {
            return;
        }
        if self.smoothed_delta == 0.0 {
            self.smoothed_delta = delta;
        } else {
            self.smoothed_delta += (delta - self.smoothed_delta) * 0.1;
        }
    }

    fn fps(&self) -> f64 {
        if self.smoothed_delta > 0.0 {
            1.0 / self.smoothed_delta
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum UiAction {
    Restart(DeckId),
    Seek(DeckId),
}

pub fn draw(
    ctx: &egui::Context,
    state: &mut UiState,
    mixer: &mut FourDeckMixer,
    transports: &mut [DeckTransport; 4],
    time: &FrameTime,
    gpu_info: &str,
) -> Vec<UiAction> {
    state.fps.push(time.delta);
    let mut actions = Vec::new();

    egui::Window::new("oneiroi")
        .default_pos([16.0, 16.0])
        .default_size([920.0, 520.0])
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("ONEIROI · FOUR DECK VIDEO MIXER");
                ui.separator();
                ui.label(gpu_info);
            });
            ui.separator();

            ui.horizontal(|ui| {
                ui.label(format!("{:.1} fps", state.fps.fps()));
                ui.label(format!("frame {}", time.frame));
                ui.label(format!("{:.2}s", time.elapsed));
                ui.separator();
                ui.label("Select a deck, then drag a movie onto this window.");
            });

            ui.separator();
            egui::Grid::new("four-decks")
                .num_columns(2)
                .spacing([12.0, 12.0])
                .show(ui, |ui| {
                    for (index, deck_id) in DeckId::ALL.into_iter().enumerate() {
                        draw_deck(
                            ui,
                            mixer,
                            deck_id,
                            &mut transports[deck_id.index()],
                            &mut state.effects[deck_id.index()],
                            &mut actions,
                        );
                        if index % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("A");
                ui.add(
                    egui::Slider::new(&mut state.crossfader, 0.0..=1.0)
                        .text("crossfader")
                        .clamping(egui::SliderClamping::Always),
                );
                ui.label("B");
                ui.checkbox(&mut state.equal_power, "equal power");
                if ui.button("Center").clicked() {
                    state.crossfader = 0.5;
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::Slider::new(&mut state.master_opacity, 0.0..=1.0)
                        .text("master")
                        .clamping(egui::SliderClamping::Always),
                );
                if ui.selectable_label(state.blackout, "BLACKOUT").clicked() {
                    state.blackout = !state.blackout;
                }
                ui.checkbox(&mut state.master_freeze, "master freeze");
            });
        });
    actions
}

fn draw_deck(
    ui: &mut egui::Ui,
    mixer: &mut FourDeckMixer,
    id: DeckId,
    transport: &mut DeckTransport,
    effects: &mut DeckEffects,
    actions: &mut Vec<UiAction>,
) {
    let selected = mixer.selected() == id;
    let frame = egui::Frame::group(ui.style())
        .fill(if selected {
            ui.visuals().selection.bg_fill.linear_multiply(0.35)
        } else {
            ui.visuals().faint_bg_color
        })
        .inner_margin(10.0);

    frame.show(ui, |ui| {
        ui.set_min_size([420.0, 165.0].into());
        ui.horizontal(|ui| {
            if ui
                .selectable_label(selected, format!("DECK {}", id.label()))
                .clicked()
            {
                mixer.select(id);
            }
            ui.weak(if selected {
                "drop target"
            } else {
                "click to target"
            });
            let eject_enabled = !matches!(mixer.deck(id).state, DeckState::Empty);
            if ui
                .add_enabled(eject_enabled, egui::Button::new("Eject"))
                .clicked()
            {
                mixer.eject(id);
            }
        });
        ui.separator();

        match &mixer.deck(id).state {
            DeckState::Empty => {
                ui.label("Empty");
                ui.weak("Select this deck and drop MOV, MP4, MKV, AVI, WebM, or MXF footage.");
            }
            DeckState::Loading { path } => {
                ui.spinner();
                ui.label(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Loading movie…"),
                );
                ui.weak("Probing codec and performance metadata…");
            }
            DeckState::Ready(movie) => {
                ui.strong(&movie.display_name);
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} × {}",
                        movie.visible_extent[0], movie.visible_extent[1]
                    ));
                    ui.label(movie.codec.to_uppercase());
                    if let Some(rate) = movie.frame_rate {
                        ui.label(format!(
                            "{:.2} fps",
                            rate.numerator as f64 / rate.denominator as f64
                        ));
                    }
                    if let Some(duration) = movie.duration {
                        ui.label(format!("{:.1}s", duration.as_seconds()));
                    }
                });
                let (label, color) = match movie.health {
                    MediaHealth::StageReady => ("STAGE READY", egui::Color32::LIGHT_GREEN),
                    MediaHealth::Usable => ("USABLE", egui::Color32::from_rgb(130, 210, 255)),
                    MediaHealth::Caution => ("CAUTION", egui::Color32::YELLOW),
                    MediaHealth::Problem => ("PROBLEM", egui::Color32::LIGHT_RED),
                };
                ui.colored_label(color, label);
                ui.weak(&movie.health_reason);
            }
            DeckState::Error { path, message } => {
                ui.colored_label(egui::Color32::LIGHT_RED, "IMPORT ERROR");
                ui.label(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Unknown file"),
                );
                ui.weak(message);
            }
        }

        let deck = mixer.deck_mut(id);
        ui.horizontal(|ui| {
            ui.add(
                egui::Slider::new(&mut deck.level, 0.0..=1.0)
                    .text("level")
                    .clamping(egui::SliderClamping::Always),
            );
            ui.selectable_value(&mut deck.bus, CrossfadeBus::Left, "Bus A");
            ui.selectable_value(&mut deck.bus, CrossfadeBus::Right, "Bus B");
        });
        ui.horizontal(|ui| {
            if ui
                .button(if transport.playing { "Pause" } else { "Play" })
                .clicked()
            {
                transport.playing = !transport.playing;
            }
            if ui.button("Restart").clicked() {
                transport.restart();
                actions.push(UiAction::Restart(id));
            }
            ui.checkbox(&mut transport.frozen, "Freeze");
            let mut looping = transport.end_mode == EndMode::Loop;
            if ui.checkbox(&mut looping, "Loop").changed() {
                transport.end_mode = if looping {
                    EndMode::Loop
                } else {
                    EndMode::OneShot
                };
            }
            ui.add(
                egui::Slider::new(&mut transport.speed, 0.25..=4.0)
                    .text("speed")
                    .logarithmic(true),
            );
        });
        if let Some(duration) = transport.duration.filter(|duration| *duration > 0.0) {
            let mut progress = (transport.position / duration).clamp(0.0, 1.0) as f32;
            if ui
                .add(egui::Slider::new(&mut progress, 0.0..=1.0).text("playhead"))
                .changed()
            {
                transport.seek_normalized(progress);
                actions.push(UiAction::Seek(id));
            }
        }
        egui::CollapsingHeader::new("GPU effects")
            .id_salt(format!("effects-{}", id.label()))
            .show(ui, |ui| {
                ui.add(egui::Slider::new(&mut effects.contrast, 0.0..=2.0).text("contrast"));
                ui.add(egui::Slider::new(&mut effects.saturation, 0.0..=2.0).text("saturation"));
                ui.add(egui::Slider::new(&mut effects.pixelate, 0.0..=0.1).text("pixelate"));
                ui.add(egui::Slider::new(&mut effects.luma_key, 0.0..=1.0).text("luma key"));
                ui.checkbox(&mut effects.mirror, "mirror");
            });
    });
}
