//! egui overlay.
//!
//! The UI never touches the GPU or mutates render state directly. It edits
//! plain values that get read into a per-frame snapshot, which is the same
//! path the parameter/modulation system takes later.

use oneiroi_core::FrameTime;

/// Everything the overlay owns. All plain data — no GPU handles, no channels.
pub struct UiState {
    pub spin: f32,
    fps: FpsMeter,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            spin: 0.6,
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

pub fn draw(ctx: &egui::Context, state: &mut UiState, time: &FrameTime, gpu_info: &str) {
    state.fps.push(time.delta);

    egui::Window::new("oneiroi")
        .default_pos([16.0, 16.0])
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(gpu_info);
            ui.separator();

            egui::Grid::new("stats").num_columns(2).show(ui, |ui| {
                ui.label("fps");
                ui.label(format!("{:.1}", state.fps.fps()));
                ui.end_row();

                ui.label("frame");
                ui.label(time.frame.to_string());
                ui.end_row();

                ui.label("elapsed");
                ui.label(format!("{:.2}s", time.elapsed));
                ui.end_row();
            });

            ui.separator();
            ui.add(
                egui::Slider::new(&mut state.spin, -4.0..=4.0)
                    .text("spin")
                    .clamping(egui::SliderClamping::Always),
            );
        });
}
