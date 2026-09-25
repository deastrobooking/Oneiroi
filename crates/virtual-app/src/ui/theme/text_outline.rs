//! Glyph outlines for the operator UI only, before egui tessellation.
use egui::{Color32, Shape, epaint::ClippedShape};
use std::sync::Arc;

pub fn outline_text(shapes: &mut [ClippedShape], width: f32, color: Color32) {
    if !width.is_finite() || width <= 0.0 {
        return;
    }
    for clipped in shapes {
        outline_shape(&mut clipped.shape, width.min(1.5), color);
    }
}

fn outline_shape(shape: &mut Shape, width: f32, color: Color32) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                outline_shape(shape, width, color);
            }
        }
        Shape::Text(text) => {
            let mut outline = text.clone();
            outline.underline = egui::Stroke::NONE;
            outline.override_text_color = Some(color);
            // Keep glyph triangles only: selection backgrounds and underlines
            // must never be duplicated and shifted along with the outline.
            let galley = Arc::make_mut(&mut outline.galley);
            for placed in &mut galley.rows {
                let visuals = &mut Arc::make_mut(&mut placed.row).visuals;
                let range = visuals.glyph_vertex_range.clone();
                let indices = visuals
                    .mesh
                    .indices
                    .chunks_exact(3)
                    .filter(|tri| tri.iter().all(|i| range.contains(&(*i as usize))))
                    .flatten()
                    .copied()
                    .collect();
                visuals.mesh.indices = indices;
            }
            let mut layers = Vec::with_capacity(5);
            for delta in [
                egui::vec2(-width, 0.0),
                egui::vec2(width, 0.0),
                egui::vec2(0.0, -width),
                egui::vec2(0.0, width),
            ] {
                let mut shifted = outline.clone();
                shifted.pos += delta;
                layers.push(Shape::Text(shifted));
            }
            layers.push(Shape::Text(text.clone()));
            *shape = Shape::Vec(layers);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_keeps_clipping_and_foreground_and_can_be_disabled() {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label("Definition");
        });
        let mut shapes = output.shapes;
        let original = shapes.clone();
        outline_text(&mut shapes, 0.0, Color32::BLACK);
        assert_eq!(shapes, original);
        outline_text(&mut shapes, 0.75, Color32::BLACK);
        for (outlined, source) in shapes.iter().zip(&original) {
            assert_eq!(outlined.clip_rect, source.clip_rect);
            if let Shape::Text(_) = source.shape {
                let Shape::Vec(layers) = &outlined.shape else {
                    panic!("missing outline");
                };
                assert_eq!(layers.len(), 5);
                assert_eq!(layers.last(), Some(&source.shape));
            }
        }
    }
}
