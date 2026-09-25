//! Shared camera/capture-card controls for setup and the selected deck.

use oneiroi_media::{CameraConfig, CameraDevice, CapturePixelFormat, DeckId};

use super::{UiAction, UiState};

pub(super) fn draw_video_input(
    ui: &mut egui::Ui,
    state: &mut UiState,
    deck: DeckId,
    devices: &[CameraDevice],
    status: &str,
    actions: &mut Vec<UiAction>,
) {
    ui.push_id(("video-input", deck.index()), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Video input · camera / capture card");
            let selected = devices.iter().find(|device| device.id == state.camera_device_id);
            egui::ComboBox::from_id_salt("device")
                .selected_text(selected.map_or(state.camera_device_id.as_str(), |device| device.label.as_str()))
                .show_ui(ui, |ui| {
                    for device in devices {
                        ui.selectable_value(&mut state.camera_device_id, device.id.clone(), &device.label);
                    }
                });
            if ui.button("Refresh").clicked() { actions.push(UiAction::RefreshCameras); }
            if ui.add_enabled(!state.camera_device_id.trim().is_empty(),
                egui::Button::new(format!("Connect to Deck {}", deck.label()))).clicked() {
                let device = devices.iter().find(|device| device.id == state.camera_device_id)
                    .cloned().unwrap_or_else(|| CameraDevice {
                        id: state.camera_device_id.trim().to_owned(),
                        label: format!("Video input {}", state.camera_device_id.trim()),
                        backend: "avfoundation".to_owned(),
                    });
                actions.push(UiAction::ConnectCamera { deck, config: CameraConfig {
                    device,
                    requested_extent: Some([state.camera_width, state.camera_height]),
                    requested_fps: Some(state.camera_fps),
                    fps_denominator: state.camera_fps_denominator,
                    pixel_format: state.capture_pixel_format,
                }});
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Capture format");
            egui::ComboBox::from_id_salt("resolution")
                .selected_text(format!("{} × {}", state.camera_width, state.camera_height))
                .show_ui(ui, |ui| {
                    for (label, width, height) in [("720p",1280,720),("1080p",1920,1080),("2160p",3840,2160)] {
                        if ui.selectable_label(state.camera_width == width && state.camera_height == height, label).clicked() {
                            state.camera_width = width; state.camera_height = height;
                        }
                    }
                });
            let rate = f64::from(state.camera_fps) / f64::from(state.camera_fps_denominator.max(1));
            egui::ComboBox::from_id_salt("rate").selected_text(format!("{rate:.2} fps"))
                .show_ui(ui, |ui| {
                    for (label, numerator, denominator) in [
                        ("23.976",24000,1001),("24",24,1),("25",25,1),("29.97",30000,1001),
                        ("30",30,1),("50",50,1),("59.94",60000,1001),("60",60,1),("120",120,1),
                    ] {
                        if ui.selectable_label(state.camera_fps == numerator && state.camera_fps_denominator == denominator, label).clicked() {
                            state.camera_fps = numerator; state.camera_fps_denominator = denominator;
                        }
                    }
                });
            egui::ComboBox::from_id_salt("pixels").selected_text(state.capture_pixel_format.label())
                .show_ui(ui, |ui| {
                    for format in CapturePixelFormat::ALL {
                        ui.selectable_value(&mut state.capture_pixel_format, format, format.label());
                    }
                });
        });
        egui::CollapsingHeader::new("Manual input / custom size").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut state.camera_device_id).hint_text("Device name or index"));
                ui.add(egui::DragValue::new(&mut state.camera_width).range(160..=7680).suffix(" w"));
                ui.add(egui::DragValue::new(&mut state.camera_height).range(120..=4320).suffix(" h"));
            });
            ui.weak("Match the card's input signal and supported capture mode. Video only; audio input is selected separately.");
        });
        if !status.is_empty() { ui.weak(status); }
    });
}
