//! Tracking mutation applied to every merged frame before it is sent: calibration, unified correctors, parameter
//! adjustment and One Euro data filtering. Pure: no I/O, no threads, no clocks (time is passed in). See
//! `spec/mutation.md`.

mod adjust;
mod calibration;
mod config;
mod correct;
mod filter;

#[cfg(test)]
mod tests;

pub use adjust::AdjustGroup;
pub use calibration::{CalibrationData, ShapeCalibration};
pub use config::{
    AdjustmentConfig, CalibrationConfig, CorrectorsConfig, FilterChannel, FilterConfig, MutationConfig, Range,
};

use adjust::ShapeRanges;
use calibration::Calibrator;
use fb_tracking::{Expression, TrackingFrame};
use filter::FilterBank;

/// A gap between frames longer than this (seconds) re-seeds the filters and skips calibration learning.
const MAX_GAP_S: f64 = 0.5;

/// Runs the four stages on frames and owns their state.
pub struct Mutator {
    config: MutationConfig,
    ranges: ShapeRanges,
    calibrator: Calibrator,
    filters: FilterBank,
    last_s: Option<f64>,
}

impl Mutator {
    /// `config` is sanitized; `calibration` is normalized (unknown shapes dropped, missing ones start at zero).
    pub fn new(config: MutationConfig, calibration: CalibrationData) -> Self {
        let config = config.sanitized();
        let mut m = Self {
            ranges: ShapeRanges::compile(&config.adjustment.ranges),
            config,
            calibrator: Calibrator::new(&calibration),
            filters: FilterBank::new(),
            last_s: None,
        };
        m.filters.reset_inactive(&m.config.filter, m.config.enabled);
        m
    }

    pub fn config(&self) -> &MutationConfig {
        &self.config
    }

    /// Validates (clamps out-of-range numbers, fixes ranges) and applies; resets filter state for categories
    /// that were switched off. Returns the sanitized config.
    pub fn set_config(&mut self, config: MutationConfig) -> MutationConfig {
        self.config = config.sanitized();
        self.ranges = ShapeRanges::compile(&self.config.adjustment.ranges);
        self.filters.reset_inactive(&self.config.filter, self.config.enabled);
        self.config.clone()
    }

    /// One entry per shape, in `Expression` order.
    pub fn calibration(&self) -> &CalibrationData {
        &self.calibrator.data
    }

    /// `None` resets every shape.
    pub fn reset_calibration(&mut self, shape: Option<Expression>) {
        self.calibrator.reset(shape);
    }

    /// Mutates `frame` in place. `now_s` is a monotonic time in seconds. The first call, or a gap longer than
    /// 0.5 s since the last call, re-seeds the filters with the input (and skips learning for that frame).
    pub fn apply(&mut self, frame: &mut TrackingFrame, now_s: f64) {
        let dt = self.last_s.map(|t| now_s - t);
        self.last_s = Some(now_s);
        let cfg = &self.config;
        if !cfg.enabled {
            return;
        }
        let seed = !dt.is_some_and(|d| d > 0.0 && d <= MAX_GAP_S);
        let dt = dt.unwrap_or(0.0);

        if cfg.calibration.enabled {
            if cfg.calibration.continuous && !seed {
                self.calibrator.learn(&frame.shapes, dt);
            }
            self.calibrator.apply(&mut frame.shapes, cfg.calibration.blend);
        }
        if cfg.correctors.enabled {
            correct::apply(&cfg.correctors, frame);
        }
        if cfg.adjustment.enabled {
            self.ranges.apply(&mut frame.shapes, &mut frame.head);
        }
        if cfg.filter.enabled {
            self.filters.apply(&cfg.filter, frame, dt, seed);
        }
    }

    /// True once calibration data changed since the last call (the caller persists it).
    pub fn take_calibration_dirty(&mut self) -> bool {
        self.calibrator.take_dirty()
    }
}
