//! Unified correctors: fix physically impossible combinations (spec/mutation.md, stage 2).

use crate::config::CorrectorsConfig;
use fb_tracking::{Expression as E, TrackingFrame};

pub(crate) fn apply(cfg: &CorrectorsConfig, frame: &mut TrackingFrame) {
    if cfg.mouth_closed_fix {
        let closed = frame.shape(E::MouthClosed).min(frame.shape(E::JawOpen));
        frame.set_shape(E::MouthClosed, closed.clamp(0.0, 1.0));
    }
    if cfg.lip_suck_fix {
        for (suck, lip) in [
            (E::LipSuckUpperRight, E::MouthUpperUpRight),
            (E::LipSuckUpperLeft, E::MouthUpperUpLeft),
            (E::LipSuckLowerRight, E::MouthLowerDownRight),
            (E::LipSuckLowerLeft, E::MouthLowerDownLeft),
        ] {
            let v = frame.shape(suck) * (1.0 - frame.shape(lip));
            frame.set_shape(suck, v.clamp(0.0, 1.0));
        }
    }
    let eye = &mut frame.eye;
    if cfg.eye_lid_blend > 0.0 {
        let b = cfg.eye_lid_blend;
        let avg = (eye.left.openness + eye.right.openness) / 2.0;
        eye.left.openness = (eye.left.openness + (avg - eye.left.openness) * b).clamp(0.0, 1.0);
        eye.right.openness = (eye.right.openness + (avg - eye.right.openness) * b).clamp(0.0, 1.0);
    }
    if cfg.eye_look_symmetrize {
        let y = (eye.left.gaze_y + eye.right.gaze_y) / 2.0;
        eye.left.gaze_y = y;
        eye.right.gaze_y = y;
    }
}
