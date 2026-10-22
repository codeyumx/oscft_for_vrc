//! Calibration state: the learned peak and learning progress of every shape (spec/mutation.md, stage 1).

use fb_tracking::{Expression, SHAPE_COUNT};
use serde::{Deserialize, Serialize};

/// Time constant of the moving average that ignores single-frame spikes, seconds.
const HOLD_TAU_S: f64 = 0.05;
/// Time a shape has to be used at no less than half its peak to reach full progress, seconds.
const PROGRESS_FULL_S: f64 = 5.0;
/// Below this smoothed value a shape counts as not being used.
const USE_THRESHOLD: f32 = 0.1;
/// The peak used for scaling is never below this, so amplification is limited to 5x.
const PEAK_FLOOR: f32 = 0.2;

/// Learned state of one shape, as stored in `calibration.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShapeCalibration {
    /// `Expression` name, e.g. `"JawOpen"`.
    pub name: String,
    /// Largest sustained value seen, 0..1.
    pub peak: f32,
    /// How well the peak is established, 0..1 (full after about 5 s of use).
    pub progress: f32,
}

impl Default for ShapeCalibration {
    fn default() -> Self {
        Self { name: String::new(), peak: 0.0, progress: 0.0 }
    }
}

/// Calibration data of all shapes (one entry per shape, in `Expression` order, once held by a `Mutator`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalibrationData {
    pub shapes: Vec<ShapeCalibration>,
}

impl CalibrationData {
    /// The entry of `shape`, if present.
    pub fn get(&self, shape: Expression) -> Option<&ShapeCalibration> {
        self.shapes.iter().find(|s| s.name == shape.name())
    }

    /// Mean `progress` over the shapes with `peak > 0` (0 when there are none).
    pub fn mean_progress(&self) -> f32 {
        let (sum, n) = self.shapes.iter().filter(|s| s.peak > 0.0).fold((0.0, 0u32), |(s, n), e| (s + e.progress, n + 1));
        if n == 0 { 0.0 } else { sum / n as f32 }
    }

    /// One entry per shape in `Expression` order: unknown names are dropped, missing shapes start at zero, and
    /// numbers are clamped to 0..1 (non-finite = 0).
    pub(crate) fn normalized(&self) -> Self {
        let mut shapes: Vec<ShapeCalibration> = Expression::ALL
            .iter()
            .map(|e| ShapeCalibration { name: e.name().to_owned(), peak: 0.0, progress: 0.0 })
            .collect();
        let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        for s in &self.shapes {
            if let Some(e) = Expression::from_name(&s.name) {
                let slot = &mut shapes[e.index()];
                slot.peak = unit(s.peak);
                slot.progress = unit(s.progress);
            }
        }
        Self { shapes }
    }
}

/// Learning and applying. `data.shapes` always has [`SHAPE_COUNT`] entries in `Expression` order.
pub(crate) struct Calibrator {
    pub data: CalibrationData,
    hold: [f32; SHAPE_COUNT],
    dirty: bool,
}

impl Calibrator {
    pub fn new(data: &CalibrationData) -> Self {
        Self { data: data.normalized(), hold: [0.0; SHAPE_COUNT], dirty: false }
    }

    pub fn reset(&mut self, shape: Option<Expression>) {
        let range = match shape {
            Some(e) => e.index()..e.index() + 1,
            None => 0..SHAPE_COUNT,
        };
        for i in range {
            let s = &mut self.data.shapes[i];
            if s.peak != 0.0 || s.progress != 0.0 {
                s.peak = 0.0;
                s.progress = 0.0;
                self.dirty = true;
            }
            self.hold[i] = 0.0;
        }
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Learns from one frame of raw input; `dt` is the time since the previous frame, seconds (> 0).
    pub fn learn(&mut self, shapes: &[f32; SHAPE_COUNT], dt: f64) {
        let k = (1.0 - (-dt / HOLD_TAU_S).exp()) as f32;
        let step = (dt / PROGRESS_FULL_S) as f32;
        for ((s, hold), &v) in self.data.shapes.iter_mut().zip(&mut self.hold).zip(shapes) {
            if !v.is_finite() {
                continue;
            }
            *hold += (v.clamp(0.0, 1.0) - *hold) * k;
            if *hold > s.peak {
                s.peak = *hold;
                self.dirty = true;
            }
            if *hold >= 0.5 * s.peak && *hold >= USE_THRESHOLD && s.progress < 1.0 {
                s.progress = (s.progress + step).min(1.0);
                self.dirty = true;
            }
        }
    }

    /// Rescales every shape by its learned peak: `v + (clamp(v / max(peak, 0.2)) - v) * blend * progress`.
    pub fn apply(&self, shapes: &mut [f32; SHAPE_COUNT], blend: f32) {
        for (v, s) in shapes.iter_mut().zip(&self.data.shapes) {
            let weight = blend * s.progress;
            if weight > 0.0 {
                let calibrated = (*v / s.peak.max(PEAK_FLOOR)).clamp(0.0, 1.0);
                *v = (*v + (calibrated - *v) * weight).clamp(0.0, 1.0);
            }
        }
    }
}
