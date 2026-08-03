//! Program output window, display selection and topology tracking.

use std::sync::Arc;
use std::time::Instant;

use oneiroi_render::{
    MasterEffectProcessor, PresentSurface, ProgramPresenter, ProgramTarget, SurfaceAcquireStatus,
};
use winit::dpi::PhysicalPosition;
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window};

use super::State;

pub(crate) struct OutputMonitor {
    id: String,
    handle: MonitorHandle,
}

pub(crate) struct OutputHealth {
    pub(crate) status: &'static str,
    pub(crate) presented: u64,
    pub(crate) skipped: u64,
    pub(crate) reconfigurations: u64,
    pub(crate) recoveries: u64,
    pub(crate) timeouts: u64,
    pub(crate) occlusions: u64,
    pub(crate) validation_errors: u64,
    pub(crate) topology_changes: u64,
    awaiting_recovery: bool,
}

impl Default for OutputHealth {
    fn default() -> Self {
        Self {
            status: "Waiting for first frame",
            presented: 0,
            skipped: 0,
            reconfigurations: 0,
            recoveries: 0,
            timeouts: 0,
            occlusions: 0,
            validation_errors: 0,
            topology_changes: 0,
            awaiting_recovery: false,
        }
    }
}

impl OutputHealth {
    pub(crate) fn observe(&mut self, status: SurfaceAcquireStatus) {
        match status {
            SurfaceAcquireStatus::Healthy => {
                self.presented = self.presented.saturating_add(1);
                if self.awaiting_recovery {
                    self.recoveries = self.recoveries.saturating_add(1);
                    self.awaiting_recovery = false;
                }
                self.status = "Healthy";
            }
            SurfaceAcquireStatus::Suboptimal => {
                self.presented = self.presented.saturating_add(1);
                self.reconfigurations = self.reconfigurations.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Suboptimal · reconfigured";
            }
            SurfaceAcquireStatus::Outdated => {
                self.skipped = self.skipped.saturating_add(1);
                self.reconfigurations = self.reconfigurations.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Outdated · reconfiguring";
            }
            SurfaceAcquireStatus::Lost => {
                self.skipped = self.skipped.saturating_add(1);
                self.reconfigurations = self.reconfigurations.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Surface lost · reconfiguring";
            }
            SurfaceAcquireStatus::Timeout => {
                self.skipped = self.skipped.saturating_add(1);
                self.timeouts = self.timeouts.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Presentation timeout";
            }
            SurfaceAcquireStatus::Occluded => {
                self.skipped = self.skipped.saturating_add(1);
                self.occlusions = self.occlusions.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Output occluded";
            }
            SurfaceAcquireStatus::Validation => {
                self.skipped = self.skipped.saturating_add(1);
                self.validation_errors = self.validation_errors.saturating_add(1);
                self.awaiting_recovery = true;
                self.status = "Surface validation error";
            }
        }
    }
}

pub(crate) fn monitor_id(monitor: &MonitorHandle) -> String {
    let name = monitor.name().unwrap_or_else(|| "Display".to_owned());
    let size = monitor.size();
    let position = monitor.position();
    format!(
        "{name}|{}x{}|{},{}",
        size.width, size.height, position.x, position.y
    )
}

fn monitor_label(monitor: &MonitorHandle) -> String {
    let name = monitor.name().unwrap_or_else(|| "Display".to_owned());
    let size = monitor.size();
    let refresh = monitor
        .refresh_rate_millihertz()
        .map(|millihertz| format!(" · {:.1} Hz", millihertz as f64 / 1000.0))
        .unwrap_or_default();
    format!("{name} · {} × {}{refresh}", size.width, size.height)
}

pub(crate) fn describe_monitors(
    handles: Vec<MonitorHandle>,
) -> (Vec<OutputMonitor>, Vec<super::ui::OutputDisplay>) {
    let mut monitors = Vec::with_capacity(handles.len());
    let mut displays = Vec::with_capacity(handles.len());
    for handle in handles {
        let id = monitor_id(&handle);
        displays.push(super::ui::OutputDisplay {
            id: id.clone(),
            label: monitor_label(&handle),
        });
        monitors.push(OutputMonitor { id, handle });
    }
    monitors.sort_by(|left, right| left.id.cmp(&right.id));
    displays.sort_by(|left, right| left.id.cmp(&right.id));
    (monitors, displays)
}

/// Resources and health state that belong exclusively to the clean program
/// output. Keeping them together prevents unrelated app subsystems from
/// independently owning pieces of the output lifecycle.
pub(crate) struct OutputLifecycle {
    pub(crate) window: Arc<Window>,
    pub(crate) monitors: Vec<OutputMonitor>,
    pub(crate) displays: Vec<super::ui::OutputDisplay>,
    pub(crate) current_display: String,
    pub(crate) health: OutputHealth,
    pub(crate) last_display_refresh: Instant,
    pub(crate) surface: PresentSurface,
    pub(crate) presenter: ProgramPresenter,
}

impl OutputLifecycle {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        window: Arc<Window>,
        monitors: Vec<OutputMonitor>,
        displays: Vec<super::ui::OutputDisplay>,
        current_display: String,
        surface: PresentSurface,
        presenter: ProgramPresenter,
    ) -> Self {
        Self {
            window,
            monitors,
            displays,
            current_display,
            health: OutputHealth::default(),
            last_display_refresh: Instant::now(),
            surface,
            presenter,
        }
    }
}

impl State {
    pub(crate) fn apply_output_settings(&mut self) -> bool {
        if self.program.extent() != self.ui.composition_extent {
            if let Err(error) = self
                .performance_runtime
                .set_composition_extent(self.ui.composition_extent)
            {
                self.project_status = format!("Output graph rejected: {error:#}");
                self.ui.composition_extent = self.program.extent();
                self.ui.custom_composition_extent = self.program.extent();
                return false;
            }
            self.program = ProgramTarget::new(&self.gpu.device, self.ui.composition_extent);
            self.master_effect_processor =
                MasterEffectProcessor::new(&self.gpu.device, &self.program);
            let manifest_paths = self.effect_manifest_paths();
            self.master_effect_processor
                .watch_effect_manifests(manifest_paths);
            self.ui.effect_reload_status = self.master_effect_processor.reload_status().to_owned();
            self.operator_presenter =
                ProgramPresenter::new(&self.gpu.device, &self.program, self.gpu.content_format());
            self.output.presenter = ProgramPresenter::new(
                &self.gpu.device,
                &self.program,
                self.output.surface.content_format(),
            );
        }
        self.output.window.set_visible(self.ui.output_enabled);
        self.apply_output_monitor();
        true
    }

    pub(crate) fn apply_output_monitor(&mut self) {
        if !self
            .output
            .monitors
            .iter()
            .any(|monitor| monitor.id == self.ui.output_display_id)
        {
            let current_id = self
                .output
                .window
                .current_monitor()
                .map(|monitor| monitor_id(&monitor));
            self.ui.output_display_id = current_id
                .filter(|id| self.output.monitors.iter().any(|monitor| &monitor.id == id))
                .or_else(|| {
                    self.output
                        .monitors
                        .first()
                        .map(|monitor| monitor.id.clone())
                })
                .unwrap_or_default();
        }
        let monitor = self
            .output
            .monitors
            .iter()
            .find(|monitor| monitor.id == self.ui.output_display_id)
            .map(|monitor| monitor.handle.clone());
        if self.ui.output_fullscreen {
            self.output
                .window
                .set_fullscreen(Some(Fullscreen::Borderless(monitor)));
        } else {
            self.output.window.set_fullscreen(None);
            if let Some(monitor) = monitor {
                let position = monitor.position();
                self.output.window.set_outer_position(PhysicalPosition::new(
                    position.x.saturating_add(40),
                    position.y.saturating_add(40),
                ));
            }
        }
        self.output.current_display = self
            .output
            .displays
            .iter()
            .find(|display| display.id == self.ui.output_display_id)
            .map(|display| display.label.clone())
            .unwrap_or_else(|| "No connected display".to_owned());
    }

    pub(crate) fn refresh_output_displays(&mut self) {
        let previous_ids: Vec<_> = self
            .output
            .monitors
            .iter()
            .map(|monitor| monitor.id.clone())
            .collect();
        let handles: Vec<_> = self.output.window.available_monitors().collect();
        (self.output.monitors, self.output.displays) = describe_monitors(handles);
        let current_ids: Vec<_> = self
            .output
            .monitors
            .iter()
            .map(|monitor| monitor.id.clone())
            .collect();
        if current_ids != previous_ids {
            self.output.health.topology_changes =
                self.output.health.topology_changes.saturating_add(1);
            self.apply_output_monitor();
        } else {
            self.update_current_output_display();
        }
        self.output.last_display_refresh = Instant::now();
    }

    pub(crate) fn update_current_output_display(&mut self) {
        self.output.current_display = self
            .output
            .window
            .current_monitor()
            .map(|monitor| {
                let id = monitor_id(&monitor);
                self.output
                    .displays
                    .iter()
                    .find(|display| display.id == id)
                    .map(|display| display.label.clone())
                    .unwrap_or_else(|| monitor_label(&monitor))
            })
            .unwrap_or_else(|| "No connected display".to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_surface_failures_and_the_next_healthy_recovery() {
        let mut health = OutputHealth::default();
        health.observe(SurfaceAcquireStatus::Lost);
        health.observe(SurfaceAcquireStatus::Timeout);
        assert_eq!(health.skipped, 2);
        assert_eq!(health.reconfigurations, 1);
        assert_eq!(health.timeouts, 1);
        assert_eq!(health.recoveries, 0);

        health.observe(SurfaceAcquireStatus::Healthy);
        assert_eq!(health.presented, 1);
        assert_eq!(health.recoveries, 1);
        assert_eq!(health.status, "Healthy");

        health.observe(SurfaceAcquireStatus::Healthy);
        assert_eq!(health.recoveries, 1);
    }

    #[test]
    fn suboptimal_frames_are_presented_and_reconfigured() {
        let mut health = OutputHealth::default();
        health.observe(SurfaceAcquireStatus::Suboptimal);
        assert_eq!(health.presented, 1);
        assert_eq!(health.skipped, 0);
        assert_eq!(health.reconfigurations, 1);
        assert!(health.awaiting_recovery);
    }
}
