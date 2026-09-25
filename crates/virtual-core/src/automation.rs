//! Clip-level automation envelopes.
//!
//! A lane is a curve drawn against a clip's own timeline rather than against
//! wall time or the tempo grid: position 0 is the clip's in point and 1 is its
//! out point, so a trimmed, warped or half-speed clip carries its automation
//! with it and repeats it exactly on every loop.
//!
//! Evaluation is pure and bounded. The render thread asks a lane for its value
//! at a position every frame, so nothing here allocates or searches beyond the
//! keyframe list.

use crate::ControlTarget;

/// Lanes one clip may hold. Each lane costs one control write per frame.
pub const MAX_AUTOMATION_LANES: usize = 8;

/// Keyframes one lane may hold.
pub const MAX_AUTOMATION_KEYFRAMES: usize = 128;

/// How a segment reaches the value of the keyframe that follows it.
///
/// The curve belongs to the keyframe the segment *starts* at, which is what a
/// drawing tool implies: grabbing a point and changing its curve reshapes the
/// span to its right.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CurveType {
    #[default]
    Linear,
    /// Smoothstep: flat departure and arrival, for moves that should not read
    /// as a machine ramp.
    Smooth,
    /// Hold this value until the next keyframe. Rhythmic gates and step
    /// sequences are built from these.
    Step,
    /// Slow departure, fast arrival.
    Exponential,
}

impl CurveType {
    pub const ALL: [Self; 4] = [Self::Linear, Self::Smooth, Self::Step, Self::Exponential];

    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Smooth => "Smooth",
            Self::Step => "Step",
            Self::Exponential => "Exponential",
        }
    }

    /// Shape a normalized 0–1 span position into a 0–1 blend weight.
    fn shape(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Smooth => t * t * (3.0 - 2.0 * t),
            Self::Step => 0.0,
            Self::Exponential => t * t,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutomationKeyframe {
    /// Normalized position in the clip's play range, 0–1.
    pub position: f64,
    pub value: f32,
    pub interpolation: CurveType,
}

impl AutomationKeyframe {
    pub fn new(position: f64, value: f32) -> Self {
        Self {
            position,
            value,
            interpolation: CurveType::Linear,
        }
    }

    fn is_usable(&self) -> bool {
        self.position.is_finite() && self.value.is_finite()
    }
}

/// One parameter's curve across a clip.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipAutomationLane {
    pub target: ControlTarget,
    pub enabled: bool,
    /// Keyframes in ascending position order. `sanitized` is what guarantees
    /// that; the evaluator assumes it rather than sorting every frame.
    pub keyframes: Vec<AutomationKeyframe>,
}

impl ClipAutomationLane {
    pub fn new(target: ControlTarget) -> Self {
        Self {
            target,
            enabled: true,
            keyframes: Vec::new(),
        }
    }

    /// A flat lane at `value`, the shape a freshly added lane starts from.
    pub fn flat(target: ControlTarget, value: f32) -> Self {
        Self {
            target,
            enabled: true,
            keyframes: vec![
                AutomationKeyframe::new(0.0, value),
                AutomationKeyframe::new(1.0, value),
            ],
        }
    }

    /// Value at a normalized clip position, or `None` if the lane is empty.
    ///
    /// Positions outside 0–1 clamp to the end values rather than extrapolating:
    /// a clip resuming mid-range must not be driven somewhere the curve never
    /// goes.
    pub fn value_at(&self, position: f64) -> Option<f32> {
        let first = self.keyframes.first()?;
        let last = self.keyframes.last()?;
        let position = if position.is_finite() {
            position.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if position <= first.position {
            return Some(first.value);
        }
        if position >= last.position {
            return Some(last.value);
        }
        let index = self
            .keyframes
            .partition_point(|keyframe| keyframe.position <= position)
            .saturating_sub(1);
        let start = self.keyframes.get(index)?;
        let Some(end) = self.keyframes.get(index + 1) else {
            return Some(start.value);
        };
        let span = end.position - start.position;
        if span <= 0.0 {
            return Some(end.value);
        }
        let weight = start
            .interpolation
            .shape((position - start.position) / span);
        Some(start.value + (end.value - start.value) * weight as f32)
    }

    /// Order, clamp and bound the lane. Nothing else may assume the invariants.
    pub fn sanitized(mut self) -> Self {
        self.keyframes.retain(AutomationKeyframe::is_usable);
        for keyframe in &mut self.keyframes {
            keyframe.position = keyframe.position.clamp(0.0, 1.0);
        }
        self.keyframes
            .sort_by(|a, b| a.position.total_cmp(&b.position));
        self.keyframes.truncate(MAX_AUTOMATION_KEYFRAMES);
        self
    }

    /// Add a keyframe, replacing one at a position too close to distinguish.
    ///
    /// Clicking a curve twice in the same pixel should move that point, not
    /// stack two points that can never be separated again.
    pub fn set_keyframe(&mut self, keyframe: AutomationKeyframe) {
        const MERGE_DISTANCE: f64 = 0.002;
        if !keyframe.is_usable() {
            return;
        }
        let keyframe = AutomationKeyframe {
            position: keyframe.position.clamp(0.0, 1.0),
            ..keyframe
        };
        let existing = self
            .keyframes
            .iter()
            .position(|existing| (existing.position - keyframe.position).abs() < MERGE_DISTANCE);
        match existing {
            Some(index) => self.keyframes[index] = keyframe,
            None if self.keyframes.len() < MAX_AUTOMATION_KEYFRAMES => {
                self.keyframes.push(keyframe);
                self.keyframes
                    .sort_by(|a, b| a.position.total_cmp(&b.position));
            }
            None => {}
        }
    }

    pub fn remove_keyframe(&mut self, index: usize) {
        if index < self.keyframes.len() {
            self.keyframes.remove(index);
        }
    }

    /// Replace the lane with an evenly spaced rhythmic gate.
    ///
    /// `steps` values are held across the clip, which is the step-sequencer
    /// shape: a trailing keyframe pins the last step's length.
    pub fn set_steps(&mut self, steps: &[f32]) {
        self.keyframes.clear();
        if steps.is_empty() {
            return;
        }
        let count = steps.len().min(MAX_AUTOMATION_KEYFRAMES - 1);
        for (index, value) in steps.iter().take(count).enumerate() {
            self.keyframes.push(AutomationKeyframe {
                position: index as f64 / count as f64,
                value: *value,
                interpolation: CurveType::Step,
            });
        }
        self.keyframes.push(AutomationKeyframe {
            position: 1.0,
            value: steps[count - 1],
            interpolation: CurveType::Step,
        });
    }

    pub fn is_active(&self) -> bool {
        self.enabled && !self.keyframes.is_empty()
    }
}

/// Every lane a clip owns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipAutomation {
    pub lanes: Vec<ClipAutomationLane>,
}

impl ClipAutomation {
    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }

    /// Values every active lane produces at `position`.
    pub fn values_at(&self, position: f64) -> impl Iterator<Item = (ControlTarget, f32)> + '_ {
        self.lanes
            .iter()
            .filter(|lane| lane.is_active())
            .filter_map(move |lane| Some((lane.target, lane.value_at(position)?)))
    }

    /// Add a lane for `target`, or return the existing one.
    ///
    /// One target may not hold two lanes: they would write the same parameter
    /// in list order every frame and only the last would be visible.
    pub fn lane_for(
        &mut self,
        target: ControlTarget,
        initial: f32,
    ) -> Option<&mut ClipAutomationLane> {
        if let Some(index) = self.lanes.iter().position(|lane| lane.target == target) {
            return self.lanes.get_mut(index);
        }
        if self.lanes.len() >= MAX_AUTOMATION_LANES {
            return None;
        }
        self.lanes.push(ClipAutomationLane::flat(target, initial));
        self.lanes.last_mut()
    }

    pub fn remove_lane(&mut self, index: usize) {
        if index < self.lanes.len() {
            self.lanes.remove(index);
        }
    }

    pub fn sanitized(mut self) -> Self {
        self.lanes.truncate(MAX_AUTOMATION_LANES);
        let mut seen = Vec::with_capacity(self.lanes.len());
        self.lanes.retain(|lane| {
            let fresh = !seen.contains(&lane.target);
            seen.push(lane.target);
            fresh
        });
        self.lanes = self
            .lanes
            .drain(..)
            .map(ClipAutomationLane::sanitized)
            .collect();
        self
    }
}

/// Normalized position of `seconds` inside a clip's play range.
///
/// `end` is the effective out point, which already folds in trim, media length
/// and any musical duration. A range that has not resolved yet — a clip whose
/// media is still probing — has no meaningful position, so automation holds at
/// its start rather than jumping when the length arrives.
pub fn clip_position(seconds: f64, start: f64, end: Option<f64>) -> f64 {
    let Some(end) = end.filter(|end| end.is_finite() && *end > start) else {
        return 0.0;
    };
    if !seconds.is_finite() {
        return 0.0;
    }
    ((seconds - start) / (end - start)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane() -> ClipAutomationLane {
        let mut lane = ClipAutomationLane::new(ControlTarget::MasterOpacity);
        lane.keyframes = vec![
            AutomationKeyframe::new(0.0, 0.0),
            AutomationKeyframe::new(0.5, 1.0),
            AutomationKeyframe::new(1.0, 0.0),
        ];
        lane
    }

    #[test]
    fn interpolates_linearly_between_keyframes() {
        let lane = lane();
        assert_eq!(lane.value_at(0.0), Some(0.0));
        assert_eq!(lane.value_at(0.25), Some(0.5));
        assert_eq!(lane.value_at(0.5), Some(1.0));
        assert_eq!(lane.value_at(0.75), Some(0.5));
        assert_eq!(lane.value_at(1.0), Some(0.0));
    }

    #[test]
    fn positions_outside_the_clip_clamp_to_the_end_values() {
        let lane = lane();
        assert_eq!(lane.value_at(-4.0), Some(0.0));
        assert_eq!(lane.value_at(9.0), Some(0.0));
        assert_eq!(lane.value_at(f64::NAN), Some(0.0));
        assert_eq!(
            ClipAutomationLane::new(ControlTarget::Crossfader).value_at(0.5),
            None
        );
    }

    #[test]
    fn step_curves_hold_until_the_next_keyframe() {
        let mut lane = lane();
        for keyframe in &mut lane.keyframes {
            keyframe.interpolation = CurveType::Step;
        }
        assert_eq!(lane.value_at(0.49), Some(0.0));
        assert_eq!(lane.value_at(0.5), Some(1.0));
        assert_eq!(lane.value_at(0.99), Some(1.0));
    }

    #[test]
    fn smooth_and_exponential_curves_stay_inside_their_segment() {
        let mut lane = ClipAutomationLane::new(ControlTarget::MasterOpacity);
        lane.keyframes = vec![
            AutomationKeyframe {
                position: 0.0,
                value: 0.0,
                interpolation: CurveType::Smooth,
            },
            AutomationKeyframe::new(1.0, 1.0),
        ];
        let smooth = lane.value_at(0.5).unwrap();
        assert!(
            (smooth - 0.5).abs() < 1e-6,
            "smoothstep is symmetric: {smooth}"
        );
        assert!(lane.value_at(0.25).unwrap() < 0.25, "flat departure");

        lane.keyframes[0].interpolation = CurveType::Exponential;
        assert!((lane.value_at(0.5).unwrap() - 0.25).abs() < 1e-6);
        assert!(lane.value_at(0.9).unwrap() < 1.0);
    }

    #[test]
    fn sanitizing_orders_clamps_and_bounds_keyframes() {
        let mut lane = ClipAutomationLane::new(ControlTarget::MasterOpacity);
        lane.keyframes = vec![
            AutomationKeyframe::new(0.8, 1.0),
            AutomationKeyframe::new(-3.0, 0.25),
            AutomationKeyframe::new(f64::NAN, 0.5),
            AutomationKeyframe::new(0.2, f32::INFINITY),
            AutomationKeyframe::new(4.0, 0.75),
        ];

        let lane = lane.sanitized();

        let positions: Vec<f64> = lane.keyframes.iter().map(|k| k.position).collect();
        assert_eq!(positions, vec![0.0, 0.8, 1.0]);
        assert!(lane.keyframes.iter().all(|k| k.value.is_finite()));
    }

    #[test]
    fn setting_a_keyframe_on_top_of_another_moves_it() {
        let mut lane = lane();
        lane.set_keyframe(AutomationKeyframe::new(0.5001, 0.25));
        assert_eq!(lane.keyframes.len(), 3);
        assert_eq!(lane.value_at(0.5001), Some(0.25));

        lane.set_keyframe(AutomationKeyframe::new(0.75, 0.5));
        assert_eq!(lane.keyframes.len(), 4);
        let positions: Vec<f64> = lane.keyframes.iter().map(|k| k.position).collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn keyframes_are_capped_so_a_drawing_tool_cannot_grow_without_bound() {
        let mut lane = ClipAutomationLane::new(ControlTarget::MasterOpacity);
        for index in 0..(MAX_AUTOMATION_KEYFRAMES * 2) {
            lane.set_keyframe(AutomationKeyframe::new(
                index as f64 / (MAX_AUTOMATION_KEYFRAMES * 2) as f64,
                0.5,
            ));
        }
        assert_eq!(lane.keyframes.len(), MAX_AUTOMATION_KEYFRAMES);
    }

    #[test]
    fn step_sequences_produce_evenly_spaced_gates() {
        let mut lane = ClipAutomationLane::new(ControlTarget::MasterOpacity);
        lane.set_steps(&[1.0, 0.0, 1.0, 0.0]);

        assert_eq!(lane.value_at(0.0), Some(1.0));
        assert_eq!(lane.value_at(0.24), Some(1.0));
        assert_eq!(lane.value_at(0.26), Some(0.0));
        assert_eq!(lane.value_at(0.51), Some(1.0));
        assert_eq!(lane.value_at(0.99), Some(0.0));
    }

    #[test]
    fn one_target_holds_one_lane() {
        let mut automation = ClipAutomation::default();
        automation.lane_for(ControlTarget::MasterOpacity, 1.0);
        automation.lane_for(ControlTarget::MasterOpacity, 0.0);
        assert_eq!(automation.lanes.len(), 1);

        for deck in 0..MAX_AUTOMATION_LANES as u8 {
            automation.lane_for(ControlTarget::DeckLevel(deck.min(3)), 1.0);
        }
        assert!(automation.lanes.len() <= MAX_AUTOMATION_LANES);
    }

    #[test]
    fn sanitizing_drops_duplicate_target_lanes() {
        let mut automation = ClipAutomation::default();
        automation
            .lanes
            .push(ClipAutomationLane::flat(ControlTarget::Crossfader, 0.2));
        automation
            .lanes
            .push(ClipAutomationLane::flat(ControlTarget::Crossfader, 0.9));

        let automation = automation.sanitized();

        assert_eq!(automation.lanes.len(), 1);
        assert_eq!(automation.lanes[0].value_at(0.5), Some(0.2));
    }

    #[test]
    fn disabled_and_empty_lanes_produce_nothing() {
        let mut automation = ClipAutomation::default();
        let mut lane = ClipAutomationLane::flat(ControlTarget::MasterOpacity, 0.5);
        lane.enabled = false;
        automation.lanes.push(lane);
        automation
            .lanes
            .push(ClipAutomationLane::new(ControlTarget::Crossfader));

        assert_eq!(automation.values_at(0.5).count(), 0);
    }

    #[test]
    fn clip_position_maps_the_effective_play_range() {
        assert_eq!(clip_position(2.0, 1.0, Some(3.0)), 0.5);
        assert_eq!(clip_position(0.0, 1.0, Some(3.0)), 0.0);
        assert_eq!(clip_position(90.0, 1.0, Some(3.0)), 1.0);
        // An unresolved range holds at the start instead of guessing.
        assert_eq!(clip_position(2.0, 1.0, None), 0.0);
        assert_eq!(clip_position(2.0, 3.0, Some(3.0)), 0.0);
    }
}
