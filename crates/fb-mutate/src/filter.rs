//! One Euro filter (spec/mutation.md, stage 4).

use crate::config::{FilterChannel, FilterConfig};
use fb_tracking::{SHAPE_COUNT, TrackingFrame};
use std::f64::consts::TAU;

fn alpha(cutoff: f64, dt: f64) -> f64 {
    1.0 / (1.0 + 1.0 / (TAU * cutoff * dt))
}

/// One channel. `dt <= 0` or a gap is handled by the caller passing `seed = true`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OneEuro {
    seeded: bool,
    x_prev: f64,
    dx_hat: f64,
    x_hat: f64,
}

impl OneEuro {
    pub fn reset(&mut self) {
        self.seeded = false;
    }

    /// Filters one sample. `seed` (first sample after a gap) makes the output equal the input.
    pub fn step(&mut self, x: f32, dt: f64, seed: bool, ch: &FilterChannel) -> f32 {
        let x = f64::from(x);
        if seed || !self.seeded || !x.is_finite() {
            *self = Self { seeded: x.is_finite(), x_prev: x, dx_hat: 0.0, x_hat: x };
            return x as f32;
        }
        let dx = (x - self.x_prev) / dt;
        self.dx_hat += alpha(f64::from(ch.derivative_cutoff), dt) * (dx - self.dx_hat);
        let cutoff = f64::from(ch.min_cutoff) + f64::from(ch.beta) * self.dx_hat.abs();
        self.x_hat += alpha(cutoff, dt) * (x - self.x_hat);
        self.x_prev = x;
        self.x_hat as f32
    }
}

/// All filter states.
pub(crate) struct FilterBank {
    gaze: [OneEuro; 4],
    openness: [OneEuro; 2],
    pupil: [OneEuro; 4],
    expressions: Vec<OneEuro>,
    head: [OneEuro; 6],
}

fn reset_all(states: &mut [OneEuro]) {
    states.iter_mut().for_each(OneEuro::reset);
}

impl FilterBank {
    pub fn new() -> Self {
        Self {
            gaze: [OneEuro::default(); 4],
            openness: [OneEuro::default(); 2],
            pupil: [OneEuro::default(); 4],
            expressions: vec![OneEuro::default(); SHAPE_COUNT],
            head: [OneEuro::default(); 6],
        }
    }

    /// Forgets the state of every category that is not active, so it re-seeds when it is switched on again.
    pub fn reset_inactive(&mut self, cfg: &FilterConfig, master_and_filter_on: bool) {
        let on = |ch: &FilterChannel| master_and_filter_on && cfg.enabled && ch.enabled;
        if !on(&cfg.eye_gaze) {
            reset_all(&mut self.gaze);
        }
        if !on(&cfg.eye_openness) {
            reset_all(&mut self.openness);
        }
        if !on(&cfg.pupil) {
            reset_all(&mut self.pupil);
        }
        if !on(&cfg.expressions) {
            reset_all(&mut self.expressions);
        }
        if !on(&cfg.head) {
            reset_all(&mut self.head);
        }
    }

    /// Filters every enabled category of `frame`. `dt` is the time since the previous frame; `seed` forces a
    /// re-seed (first frame, gap, non-positive `dt`).
    pub fn apply(&mut self, cfg: &FilterConfig, frame: &mut TrackingFrame, dt: f64, seed: bool) {
        let eye = &mut frame.eye;
        if cfg.eye_gaze.enabled {
            let ch = &cfg.eye_gaze;
            let [lx, ly, rx, ry] = &mut self.gaze;
            eye.left.gaze_x = lx.step(eye.left.gaze_x, dt, seed, ch);
            eye.left.gaze_y = ly.step(eye.left.gaze_y, dt, seed, ch);
            eye.right.gaze_x = rx.step(eye.right.gaze_x, dt, seed, ch);
            eye.right.gaze_y = ry.step(eye.right.gaze_y, dt, seed, ch);
        }
        if cfg.eye_openness.enabled {
            let ch = &cfg.eye_openness;
            let [l, r] = &mut self.openness;
            eye.left.openness = l.step(eye.left.openness, dt, seed, ch).clamp(0.0, 1.0);
            eye.right.openness = r.step(eye.right.openness, dt, seed, ch).clamp(0.0, 1.0);
        }
        if cfg.pupil.enabled {
            let ch = &cfg.pupil;
            let [lm, rm, ld, rd] = &mut self.pupil;
            eye.left.pupil_diameter_mm = lm.step(eye.left.pupil_diameter_mm, dt, seed, ch);
            eye.right.pupil_diameter_mm = rm.step(eye.right.pupil_diameter_mm, dt, seed, ch);
            eye.left_diameter = ld.step(eye.left_diameter, dt, seed, ch);
            eye.right_diameter = rd.step(eye.right_diameter, dt, seed, ch);
        }
        if cfg.expressions.enabled {
            for (v, state) in frame.shapes.iter_mut().zip(&mut self.expressions) {
                *v = state.step(*v, dt, seed, &cfg.expressions).clamp(0.0, 1.0);
            }
        }
        if cfg.head.enabled {
            let h = &mut frame.head;
            let [yaw, pitch, roll, x, y, z] = &mut self.head;
            let ch = &cfg.head;
            h.yaw = yaw.step(h.yaw, dt, seed, ch);
            h.pitch = pitch.step(h.pitch, dt, seed, ch);
            h.roll = roll.step(h.roll, dt, seed, ch);
            h.pos_x = x.step(h.pos_x, dt, seed, ch);
            h.pos_y = y.step(h.pos_y, dt, seed, ch);
            h.pos_z = z.step(h.pos_z, dt, seed, ch);
        }
    }
}
