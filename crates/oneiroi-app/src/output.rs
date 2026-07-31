//! Program output window, display selection and topology tracking.

use std::time::Instant;

use oneiroi_render::{MasterEffectProcessor, ProgramPresenter, ProgramTarget};
use winit::dpi::PhysicalPosition;
use winit::window::Fullscreen;

use super::{State, describe_monitors, monitor_id, monitor_label};

impl State {
    pub(crate) fn apply_output_settings(&mut self) {
        if self.program.extent() != self.ui.composition_extent {
            if let Err(error) = self
                .performance_runtime
                .set_composition_extent(self.ui.composition_extent)
            {
                self.project_status = format!("Output graph rejected: {error:#}");
                self.ui.composition_extent = self.program.extent();
                self.ui.custom_composition_extent = self.program.extent();
                return;
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
            self.output_presenter = ProgramPresenter::new(
                &self.gpu.device,
                &self.program,
                self.output_surface.content_format(),
            );
        }
        self.output_window.set_visible(self.ui.output_enabled);
        self.apply_output_monitor();
    }

    pub(crate) fn apply_output_monitor(&mut self) {
        if !self
            .output_monitors
            .iter()
            .any(|monitor| monitor.id == self.ui.output_display_id)
        {
            let current_id = self
                .output_window
                .current_monitor()
                .map(|monitor| monitor_id(&monitor));
            self.ui.output_display_id = current_id
                .filter(|id| self.output_monitors.iter().any(|monitor| &monitor.id == id))
                .or_else(|| {
                    self.output_monitors
                        .first()
                        .map(|monitor| monitor.id.clone())
                })
                .unwrap_or_default();
        }
        let monitor = self
            .output_monitors
            .iter()
            .find(|monitor| monitor.id == self.ui.output_display_id)
            .map(|monitor| monitor.handle.clone());
        if self.ui.output_fullscreen {
            self.output_window
                .set_fullscreen(Some(Fullscreen::Borderless(monitor)));
        } else {
            self.output_window.set_fullscreen(None);
            if let Some(monitor) = monitor {
                let position = monitor.position();
                self.output_window.set_outer_position(PhysicalPosition::new(
                    position.x.saturating_add(40),
                    position.y.saturating_add(40),
                ));
            }
        }
        self.output_current_display = self
            .output_displays
            .iter()
            .find(|display| display.id == self.ui.output_display_id)
            .map(|display| display.label.clone())
            .unwrap_or_else(|| "No connected display".to_owned());
    }

    pub(crate) fn refresh_output_displays(&mut self) {
        let previous_ids: Vec<_> = self
            .output_monitors
            .iter()
            .map(|monitor| monitor.id.clone())
            .collect();
        let handles: Vec<_> = self.output_window.available_monitors().collect();
        (self.output_monitors, self.output_displays) = describe_monitors(handles);
        let current_ids: Vec<_> = self
            .output_monitors
            .iter()
            .map(|monitor| monitor.id.clone())
            .collect();
        if current_ids != previous_ids {
            self.output_health.topology_changes =
                self.output_health.topology_changes.saturating_add(1);
            self.apply_output_monitor();
        } else {
            self.update_current_output_display();
        }
        self.last_display_refresh = Instant::now();
    }

    pub(crate) fn update_current_output_display(&mut self) {
        self.output_current_display = self
            .output_window
            .current_monitor()
            .map(|monitor| {
                let id = monitor_id(&monitor);
                self.output_displays
                    .iter()
                    .find(|display| display.id == id)
                    .map(|display| display.label.clone())
                    .unwrap_or_else(|| monitor_label(&monitor))
            })
            .unwrap_or_else(|| "No connected display".to_owned());
    }
}
