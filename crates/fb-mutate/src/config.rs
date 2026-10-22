//! Mutation configuration (`config.json` → `"mutation"`) and its sanitizing rules (spec/mutation.md).

use crate::adjust::AdjustGroup;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

/// Minimum distance between a range's `min` and `max`.
const MIN_SPAN: f32 = 0.01;
/// Tolerance for the span check, so that an already sanitized range is left alone (f32 rounding).
const SPAN_EPS: f32 = 1e-6;

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// Master switch plus the four stages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MutationConfig {
    /// Master switch; off = the output equals the input.
    pub enabled: bool,
    pub calibration: CalibrationConfig,
    pub correctors: CorrectorsConfig,
    pub adjustment: AdjustmentConfig,
    pub filter: FilterConfig,
}

impl Default for MutationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            calibration: CalibrationConfig::default(),
            correctors: CorrectorsConfig::default(),
            adjustment: AdjustmentConfig::default(),
            filter: FilterConfig::default(),
        }
    }
}

impl MutationConfig {
    /// Clamps out-of-range numbers, fixes ranges and replaces non-finite numbers by their default.
    pub fn sanitize(&mut self) {
        let c = &mut self.calibration;
        c.blend = finite_or(c.blend, CalibrationConfig::default().blend).clamp(0.0, 1.0);
        let k = &mut self.correctors;
        k.eye_lid_blend = finite_or(k.eye_lid_blend, 0.0).clamp(0.0, 1.0);
        for r in self.adjustment.ranges.values_mut() {
            r.sanitize();
        }
        self.filter.sanitize();
    }

    /// [`MutationConfig::sanitize`] by value.
    pub fn sanitized(mut self) -> Self {
        self.sanitize();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalibrationConfig {
    pub enabled: bool,
    /// How much of the calibrated value replaces the raw one, 0..1 (scaled further by learning progress).
    pub blend: f32,
    /// Keep learning the peaks while running.
    pub continuous: bool,
}

impl Default for CalibrationConfig {
    fn default() -> Self {
        Self { enabled: false, blend: 1.0, continuous: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CorrectorsConfig {
    pub enabled: bool,
    /// `MouthClosed` cannot exceed `JawOpen`.
    pub mouth_closed_fix: bool,
    /// A lip cannot be sucked in while the same side is raised or lowered.
    pub lip_suck_fix: bool,
    /// 0..1: how much the eyelids follow each other (1 = fully linked).
    pub eye_lid_blend: f32,
    /// Both eyes look in the mean direction.
    pub eye_look_symmetrize: bool,
}

impl Default for CorrectorsConfig {
    fn default() -> Self {
        Self { enabled: true, mouth_closed_fix: true, lip_suck_fix: true, eye_lid_blend: 0.0, eye_look_symmetrize: false }
    }
}

/// Input range of one adjustment group. Missing fields are the identity (`0..1`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Range {
    pub min: f32,
    pub max: f32,
}

impl Default for Range {
    fn default() -> Self {
        Self { min: 0.0, max: 1.0 }
    }
}

impl Range {
    /// Both ends in 0..1 and `max >= min + 0.01`.
    pub fn sanitize(&mut self) {
        let min = finite_or(self.min, 0.0).clamp(0.0, 1.0);
        let max = finite_or(self.max, 1.0).clamp(0.0, 1.0);
        if max - min >= MIN_SPAN - SPAN_EPS {
            (self.min, self.max) = (min, max);
        } else {
            let max = (min + MIN_SPAN).min(1.0);
            (self.min, self.max) = (max - MIN_SPAN, max);
        }
    }

    /// True for `0..1`.
    pub fn is_identity(&self) -> bool {
        self.min == 0.0 && self.max == 1.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdjustmentConfig {
    pub enabled: bool,
    /// Missing group = `0..1`. Unknown group names in JSON are ignored.
    #[serde(deserialize_with = "deserialize_ranges")]
    pub ranges: BTreeMap<AdjustGroup, Range>,
}

impl Default for AdjustmentConfig {
    fn default() -> Self {
        Self { enabled: true, ranges: BTreeMap::new() }
    }
}

fn deserialize_ranges<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<AdjustGroup, Range>, D::Error> {
    let raw = BTreeMap::<String, Range>::deserialize(d)?;
    Ok(raw.into_iter().filter_map(|(name, r)| Some((AdjustGroup::from_name(&name)?, r))).collect())
}

impl AdjustmentConfig {
    /// The range of `group` (`0..1` when the group is not listed).
    pub fn range(&self, group: AdjustGroup) -> Range {
        self.ranges.get(&group).copied().unwrap_or_default()
    }
}

/// One One Euro filter category.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterChannel {
    pub enabled: bool,
    /// Hz, 0.01..30: lower = smoother at rest.
    pub min_cutoff: f32,
    /// 0..10: higher = less lag when moving.
    pub beta: f32,
    /// Hz, 0.01..30: cutoff of the low-pass on the speed estimate.
    pub derivative_cutoff: f32,
}

impl FilterChannel {
    const fn new(min_cutoff: f32, beta: f32) -> Self {
        Self { enabled: true, min_cutoff, beta, derivative_cutoff: 1.0 }
    }

    fn sanitize(&mut self, default: Self) {
        self.min_cutoff = finite_or(self.min_cutoff, default.min_cutoff).clamp(0.01, 30.0);
        self.beta = finite_or(self.beta, default.beta).clamp(0.0, 10.0);
        self.derivative_cutoff = finite_or(self.derivative_cutoff, default.derivative_cutoff).clamp(0.01, 30.0);
    }
}

impl Default for FilterChannel {
    fn default() -> Self {
        Self::new(1.0, 0.5)
    }
}

const EYE_GAZE: FilterChannel = FilterChannel::new(1.0, 0.5);
const EYE_OPENNESS: FilterChannel = FilterChannel::new(2.0, 2.0);
const PUPIL: FilterChannel = FilterChannel::new(0.5, 0.1);
const EXPRESSIONS: FilterChannel = FilterChannel::new(2.0, 1.0);
const HEAD: FilterChannel = FilterChannel::new(1.0, 0.05);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "RawFilterConfig")]
pub struct FilterConfig {
    pub enabled: bool,
    pub eye_gaze: FilterChannel,
    pub eye_openness: FilterChannel,
    pub pupil: FilterChannel,
    pub expressions: FilterChannel,
    pub head: FilterChannel,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            eye_gaze: EYE_GAZE,
            eye_openness: EYE_OPENNESS,
            pupil: PUPIL,
            expressions: EXPRESSIONS,
            head: HEAD,
        }
    }
}

impl FilterConfig {
    fn sanitize(&mut self) {
        self.eye_gaze.sanitize(EYE_GAZE);
        self.eye_openness.sanitize(EYE_OPENNESS);
        self.pupil.sanitize(PUPIL);
        self.expressions.sanitize(EXPRESSIONS);
        self.head.sanitize(HEAD);
    }
}

/// Deserialization helper: a category given partially keeps the defaults *of that category* for the rest.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawFilterConfig {
    enabled: Option<bool>,
    eye_gaze: RawChannel,
    eye_openness: RawChannel,
    pupil: RawChannel,
    expressions: RawChannel,
    head: RawChannel,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawChannel {
    enabled: Option<bool>,
    min_cutoff: Option<f32>,
    beta: Option<f32>,
    derivative_cutoff: Option<f32>,
}

impl RawChannel {
    fn over(self, base: FilterChannel) -> FilterChannel {
        FilterChannel {
            enabled: self.enabled.unwrap_or(base.enabled),
            min_cutoff: self.min_cutoff.unwrap_or(base.min_cutoff),
            beta: self.beta.unwrap_or(base.beta),
            derivative_cutoff: self.derivative_cutoff.unwrap_or(base.derivative_cutoff),
        }
    }
}

impl From<RawFilterConfig> for FilterConfig {
    fn from(raw: RawFilterConfig) -> Self {
        let d = FilterConfig::default();
        Self {
            enabled: raw.enabled.unwrap_or(d.enabled),
            eye_gaze: raw.eye_gaze.over(d.eye_gaze),
            eye_openness: raw.eye_openness.over(d.eye_openness),
            pupil: raw.pupil.over(d.pupil),
            expressions: raw.expressions.over(d.expressions),
            head: raw.head.over(d.head),
        }
    }
}
