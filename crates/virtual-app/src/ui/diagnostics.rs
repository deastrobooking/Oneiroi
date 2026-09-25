//! Operator-facing output, decoder, and frame-pipeline diagnostics.

use super::theme::ThemePalette;
use super::{PerformanceMetrics, UiState};

pub(super) fn draw_output_health(
    ui: &mut egui::Ui,
    state: &UiState,
    metrics: &PerformanceMetrics<'_>,
    palette: ThemePalette,
) {
    egui::CollapsingHeader::new("Output health")
        .default_open(false)
        .show(ui, |ui| {
            let health = &metrics.output_health;
            let (status, color) = if !state.output_enabled {
                ("Disabled", palette.idle)
            } else if metrics.output_displays.is_empty() {
                ("No connected display", palette.danger)
            } else if health.status == "Healthy" {
                (health.status, palette.success)
            } else if health.validation_errors > 0 {
                (health.status, palette.danger)
            } else {
                (health.status, palette.warning)
            };
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(color, status);
                ui.separator();
                ui.label(format!(
                    "surface {} × {} · composition {} × {} · FIFO",
                    health.surface_extent[0],
                    health.surface_extent[1],
                    state.composition_extent[0],
                    state.composition_extent[1]
                ));
            });
            ui.label(format!("Display: {}", health.current_display));
            ui.label(format!(
                "presented {} · skipped {} · recovered {} · reconfigured {}",
                health.presented, health.skipped, health.recoveries, health.reconfigurations
            ));
            ui.weak(format!(
                "timeouts {} · occluded {} · validation errors {} · display changes {}",
                health.timeouts,
                health.occlusions,
                health.validation_errors,
                health.topology_changes
            ));
        });
}

pub(super) fn draw_runtime_summary(
    ui: &mut egui::Ui,
    state: &UiState,
    metrics: &PerformanceMetrics<'_>,
) {
    ui.horizontal(|ui| {
        ui.label(format!("{:.1} fps", state.fps.fps()));
        ui.label(format!("frame {}", metrics.frame_time.frame));
        ui.label(format!("{:.2}s", metrics.frame_time.elapsed));
        ui.separator();
        ui.label("Select a slot, then drag a movie or folder onto this window.");
        if !metrics.folder_status.is_empty() {
            ui.separator();
            ui.weak(metrics.folder_status);
        }
        ui.separator();
        ui.label(format!("first-frame ready {}/32", state.preloaded_count()));
    });
}

pub(super) fn draw_pipeline_health(ui: &mut egui::Ui, metrics: &PerformanceMetrics<'_>) {
    let dropped: u64 = metrics
        .scheduler_stats
        .iter()
        .map(|stats| stats.dropped)
        .sum();
    let repeated: u64 = metrics
        .scheduler_stats
        .iter()
        .map(|stats| stats.repeated)
        .sum();
    let late: u64 = metrics.scheduler_stats.iter().map(|stats| stats.late).sum();
    ui.separator();
    ui.label(format!("drop {dropped} · repeat {repeated} · late {late}"));

    let allocated: u64 = metrics
        .frame_pool_stats
        .iter()
        .map(|stats| stats.allocations)
        .sum();
    let reused: u64 = metrics
        .frame_pool_stats
        .iter()
        .map(|stats| stats.reuses)
        .sum();
    let in_flight: u64 = metrics
        .frame_pool_stats
        .iter()
        .map(|stats| stats.in_flight)
        .sum();
    let discarded: u64 = metrics
        .frame_pool_stats
        .iter()
        .map(|stats| stats.discarded)
        .sum();
    let bytes: u64 = metrics
        .frame_pool_stats
        .iter()
        .map(|stats| stats.allocated_bytes)
        .sum();
    ui.separator();
    ui.label(format!(
        "RGBA pool alloc {allocated} · reuse {reused} · live {in_flight} · discard {discarded} · {:.1} MiB",
        bytes as f64 / (1024.0 * 1024.0)
    ));
    let packages = metrics.deck_package_stats;
    ui.separator();
    ui.label(format!(
        "deck packages selected {} · executed {} · invisible culled {} · bypass/dry {} · unavailable {}",
        packages.selected,
        packages.executed,
        packages.culled_invisible,
        packages.bypassed_or_dry,
        packages.unavailable
    ));
    let timings = metrics.deck_package_timings;
    if timings.supported {
        let samples = (0..4)
            .filter(|index| {
                timings.precomposition_ms[*index] > 0.0 || timings.package_ms[*index] > 0.0
            })
            .map(|index| {
                format!(
                    "{} pre {:.2} ms + package {:.2} ms",
                    char::from(b'A' + index as u8),
                    timings.precomposition_ms[index],
                    timings.package_ms[index]
                )
            })
            .collect::<Vec<_>>();
        if !samples.is_empty() {
            ui.weak(format!("deck GPU timing · {}", samples.join(" · ")));
        }
    } else {
        ui.weak("deck GPU timing unavailable on this adapter");
    }
}
