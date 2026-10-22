use super::*;
use fb_tracking::{Expression as E, SHAPE_COUNT};
use serde_json::json;

const DT: f64 = 1.0 / 60.0;

fn off() -> MutationConfig {
    let mut c = MutationConfig::default();
    c.calibration.enabled = false;
    c.correctors.enabled = false;
    c.adjustment.enabled = false;
    c.filter.enabled = false;
    c
}

fn calibrating() -> MutationConfig {
    let mut c = off();
    c.calibration.enabled = true;
    c
}

fn with_shape(e: E, v: f32) -> TrackingFrame {
    let mut f = TrackingFrame::default();
    f.set_shape(e, v);
    f
}

fn run(m: &mut Mutator, frame: &TrackingFrame, seconds: f64, t: &mut f64) -> TrackingFrame {
    let mut out = *frame;
    for _ in 0..(seconds / DT).round() as usize {
        *t += DT;
        out = *frame;
        m.apply(&mut out, *t);
    }
    out
}

fn busy_frame() -> TrackingFrame {
    let mut f = TrackingFrame::default();
    for (i, v) in f.shapes.iter_mut().enumerate() {
        *v = (i as f32 * 0.37).fract();
    }
    f.set_shape(E::JawOpen, 0.9);
    f.set_shape(E::MouthClosed, 0.3);
    f.set_shape(E::MouthUpperUpRight, 0.0);
    f.set_shape(E::MouthUpperUpLeft, 0.0);
    f.set_shape(E::MouthLowerDownRight, 0.0);
    f.set_shape(E::MouthLowerDownLeft, 0.0);
    f.eye.left.gaze_x = 0.2;
    f.eye.left.openness = 0.7;
    f.eye.right.openness = 0.9;
    f.eye.left.pupil_diameter_mm = 4.0;
    f.head.yaw = 12.5;
    f.head.pos_z = -0.3;
    f
}

#[test]
fn disabled_and_identity_settings_are_bit_exact() {
    let input = busy_frame();
    let mut t = 0.0;

    let mut master_off = MutationConfig { enabled: false, ..MutationConfig::default() };
    master_off.filter.enabled = true;
    master_off.calibration.enabled = true;
    let mut m = Mutator::new(master_off, CalibrationData::default());
    assert_eq!(run(&mut m, &input, 1.0, &mut t), input);

    // Everything on but at identity: no learned data, no corrections needed, default ranges, no filtering.
    let mut identity = MutationConfig::default();
    identity.calibration.enabled = true;
    let mut input = input;
    // Shapes that the correctors would touch are in a consistent state already.
    input.set_shape(E::LipSuckUpperRight, 0.4);
    let mut m = Mutator::new(identity, CalibrationData::default());
    let mut out = input;
    m.apply(&mut out, 1.0);
    assert_eq!(out, input, "first frame: progress is 0, nothing to correct");

    let mut m = Mutator::new(off(), CalibrationData::default());
    assert_eq!(run(&mut m, &input, 1.0, &mut t), input);
}

#[test]
fn calibration_learns_a_sustained_peak_but_not_a_one_frame_spike() {
    let mut m = Mutator::new(calibrating(), CalibrationData::default());
    let mut t = 0.0;
    let spike = with_shape(E::JawOpen, 0.8);
    run(&mut m, &TrackingFrame::default(), 0.2, &mut t);
    run(&mut m, &spike, DT, &mut t);
    run(&mut m, &TrackingFrame::default(), 2.0, &mut t);
    let jaw = m.calibration().get(E::JawOpen).unwrap().clone();
    assert!(jaw.peak < 0.3, "spike peak {}", jaw.peak);
    assert!(jaw.progress < 0.02, "spike progress {}", jaw.progress);

    m.reset_calibration(None);
    assert_eq!(m.calibration().get(E::JawOpen).unwrap().peak, 0.0);
    run(&mut m, &with_shape(E::JawOpen, 0.8), 1.0, &mut t);
    let jaw = m.calibration().get(E::JawOpen).unwrap().clone();
    assert!((jaw.peak - 0.8).abs() < 0.01, "sustained peak {}", jaw.peak);
}

#[test]
fn calibration_progress_reaches_full_after_about_five_seconds() {
    let mut m = Mutator::new(calibrating(), CalibrationData::default());
    let mut t = 0.0;
    let f = with_shape(E::JawOpen, 0.6);
    run(&mut m, &f, 2.5, &mut t);
    let half = m.calibration().get(E::JawOpen).unwrap().progress;
    assert!((half - 0.5).abs() < 0.03, "progress after 2.5 s = {half}");
    run(&mut m, &f, 2.6, &mut t);
    assert_eq!(m.calibration().get(E::JawOpen).unwrap().progress, 1.0);
    // Shapes that were never used did not progress.
    assert_eq!(m.calibration().get(E::TongueOut).unwrap().progress, 0.0);
}

#[test]
fn calibration_blend_progress_formula_and_peak_floor() {
    let data = CalibrationData {
        shapes: vec![
            ShapeCalibration { name: "JawOpen".into(), peak: 0.5, progress: 0.5 },
            ShapeCalibration { name: "TongueOut".into(), peak: 0.05, progress: 1.0 },
            ShapeCalibration { name: "CheekPuffRight".into(), peak: 0.5, progress: 0.0 },
        ],
    };
    let mut cfg = calibrating();
    cfg.calibration.continuous = false;
    cfg.calibration.blend = 0.8;
    let mut m = Mutator::new(cfg, data);
    let mut f = TrackingFrame::default();
    f.set_shape(E::JawOpen, 0.25);
    f.set_shape(E::TongueOut, 0.1);
    f.set_shape(E::CheekPuffRight, 0.25);
    m.apply(&mut f, 1.0);
    // calibrated = 0.25 / 0.5 = 0.5; out = 0.25 + (0.5 - 0.25) * 0.8 * 0.5
    assert!((f.shape(E::JawOpen) - 0.35).abs() < 1e-6);
    // effective peak 0.2 (not 0.05): calibrated = 0.5, progress 1, blend 0.8
    assert!((f.shape(E::TongueOut) - (0.1 + 0.4 * 0.8)).abs() < 1e-6);
    // progress 0: untouched, exactly
    assert_eq!(f.shape(E::CheekPuffRight), 0.25);
    // not learning while `continuous` is off
    assert!(!m.take_calibration_dirty());
}

#[test]
fn calibration_dirty_flag_follows_changes() {
    let mut m = Mutator::new(calibrating(), CalibrationData::default());
    let mut t = 0.0;
    assert!(!m.take_calibration_dirty());
    run(&mut m, &with_shape(E::JawOpen, 0.5), 0.5, &mut t);
    assert!(m.take_calibration_dirty());
    assert!(!m.take_calibration_dirty());
    m.reset_calibration(Some(E::JawOpen));
    assert!(m.take_calibration_dirty());
    m.reset_calibration(Some(E::JawOpen));
    assert!(!m.take_calibration_dirty(), "nothing left to reset");
}

#[test]
fn calibration_data_is_normalized_on_load() {
    let data = CalibrationData {
        shapes: vec![
            ShapeCalibration { name: "Bogus".into(), peak: 1.0, progress: 1.0 },
            ShapeCalibration { name: "JawOpen".into(), peak: 7.0, progress: f32::NAN },
        ],
    };
    let m = Mutator::new(off(), data);
    assert_eq!(m.calibration().shapes.len(), SHAPE_COUNT);
    assert_eq!(m.calibration().shapes[22].name, "JawOpen");
    assert_eq!(m.calibration().get(E::JawOpen).unwrap().peak, 1.0);
    assert_eq!(m.calibration().get(E::JawOpen).unwrap().progress, 0.0);
    assert_eq!(m.calibration().mean_progress(), 0.0);
}

fn correctors() -> MutationConfig {
    let mut c = off();
    c.correctors.enabled = true;
    c
}

#[test]
fn corrector_mouth_closed_cannot_exceed_jaw_open() {
    let mut m = Mutator::new(correctors(), CalibrationData::default());
    let mut f = with_shape(E::MouthClosed, 0.8);
    f.set_shape(E::JawOpen, 0.3);
    m.apply(&mut f, 0.0);
    assert_eq!(f.shape(E::MouthClosed), 0.3);

    let mut cfg = correctors();
    cfg.correctors.mouth_closed_fix = false;
    m.set_config(cfg);
    let mut f = with_shape(E::MouthClosed, 0.8);
    m.apply(&mut f, 1.0);
    assert_eq!(f.shape(E::MouthClosed), 0.8);
}

#[test]
fn corrector_lip_suck_is_suppressed_by_the_same_side_raise_or_lower() {
    let mut m = Mutator::new(correctors(), CalibrationData::default());
    let mut f = TrackingFrame::default();
    f.set_shape(E::LipSuckUpperRight, 0.8);
    f.set_shape(E::MouthUpperUpRight, 0.75);
    f.set_shape(E::LipSuckUpperLeft, 0.8);
    f.set_shape(E::LipSuckLowerLeft, 0.6);
    f.set_shape(E::MouthLowerDownLeft, 0.5);
    f.set_shape(E::LipSuckCornerRight, 0.9);
    f.set_shape(E::MouthUpperUpRight, 0.75);
    m.apply(&mut f, 0.0);
    assert!((f.shape(E::LipSuckUpperRight) - 0.2).abs() < 1e-6);
    assert_eq!(f.shape(E::LipSuckUpperLeft), 0.8, "other side untouched");
    assert!((f.shape(E::LipSuckLowerLeft) - 0.3).abs() < 1e-6);
    assert_eq!(f.shape(E::LipSuckCornerRight), 0.9, "corner suck is not corrected");
}

#[test]
fn corrector_eyelid_blend_and_look_symmetrize() {
    let mut cfg = correctors();
    cfg.correctors.eye_lid_blend = 0.5;
    cfg.correctors.eye_look_symmetrize = true;
    let mut m = Mutator::new(cfg, CalibrationData::default());
    let mut f = TrackingFrame::default();
    f.eye.left.openness = 1.0;
    f.eye.right.openness = 0.0;
    f.eye.left.gaze_x = -0.4;
    f.eye.right.gaze_x = 0.2;
    f.eye.left.gaze_y = 0.5;
    f.eye.right.gaze_y = -0.1;
    m.apply(&mut f, 0.0);
    assert_eq!((f.eye.left.openness, f.eye.right.openness), (0.75, 0.25));
    assert!((f.eye.left.gaze_y - 0.2).abs() < 1e-6 && (f.eye.right.gaze_y - 0.2).abs() < 1e-6);
    assert_eq!((f.eye.left.gaze_x, f.eye.right.gaze_x), (-0.4, 0.2), "vergence survives");

    let mut cfg = correctors();
    cfg.correctors.eye_lid_blend = 1.0;
    m.set_config(cfg);
    let mut f = TrackingFrame::default();
    f.eye.left.openness = 1.0;
    f.eye.right.openness = 0.0;
    m.apply(&mut f, 1.0);
    assert_eq!((f.eye.left.openness, f.eye.right.openness), (0.5, 0.5));
}

fn adjusting(group: AdjustGroup, min: f32, max: f32) -> MutationConfig {
    let mut c = off();
    c.adjustment.enabled = true;
    c.adjustment.ranges.insert(group, Range { min, max });
    c
}

#[test]
fn adjustment_maps_the_input_range_to_unit_and_clamps() {
    let mut m = Mutator::new(adjusting(AdjustGroup::JawOpen, 0.2, 0.6), CalibrationData::default());
    for (input, expected) in [(0.1, 0.0), (0.2, 0.0), (0.4, 0.5), (0.6, 1.0), (0.9, 1.0)] {
        let mut f = with_shape(E::JawOpen, input);
        f.set_shape(E::MouthClosed, 0.5);
        m.apply(&mut f, 0.0);
        assert!((f.shape(E::JawOpen) - expected).abs() < 1e-6, "{input} -> {}", f.shape(E::JawOpen));
        assert_eq!(f.shape(E::MouthClosed), 0.5, "other groups are left alone");
    }
}

#[test]
fn adjustment_covers_every_shape_of_a_group_and_groups_do_not_overlap() {
    let mut seen = [false; SHAPE_COUNT];
    for g in AdjustGroup::ALL {
        assert_eq!(g.is_head(), g.shapes().is_empty());
        for s in g.shapes() {
            assert!(!seen[s.index()], "{} is in two groups", s.name());
            seen[s.index()] = true;
        }
    }
    let ungrouped: Vec<_> = Expression::ALL.into_iter().filter(|e| !seen[e.index()]).map(E::name).collect();
    assert_eq!(ungrouped, ["JawClench", "JawMandibleRaise", "SoftPalateClose", "ThroatSwallow"]);

    let mut m = Mutator::new(adjusting(AdjustGroup::MouthSmile, 0.0, 0.5), CalibrationData::default());
    let mut f = TrackingFrame::default();
    for s in AdjustGroup::MouthSmile.shapes() {
        f.set_shape(*s, 0.25);
    }
    m.apply(&mut f, 0.0);
    for s in AdjustGroup::MouthSmile.shapes() {
        assert_eq!(f.shape(*s), 0.5);
    }
}

#[test]
fn adjustment_head_is_signed_and_unclamped() {
    let mut m = Mutator::new(adjusting(AdjustGroup::HeadRotationYaw, 0.2, 0.7), CalibrationData::default());
    for (input, expected) in [(0.1, 0.0), (-0.1, 0.0), (0.45, 0.5), (-0.45, -0.5), (1.7, 3.0), (-1.2, -2.0)] {
        let mut f = TrackingFrame::default();
        f.head.yaw = input;
        f.head.pitch = 0.45;
        m.apply(&mut f, 0.0);
        assert!((f.head.yaw - expected).abs() < 1e-5, "{input} -> {}", f.head.yaw);
        assert_eq!(f.head.pitch, 0.45);
    }
}

#[test]
fn sanitize_follows_the_spec_table() {
    let mut cfg = MutationConfig::default();
    cfg.calibration.blend = 3.0;
    cfg.correctors.eye_lid_blend = -1.0;
    cfg.filter.eye_gaze.min_cutoff = 0.0;
    cfg.filter.eye_gaze.beta = 99.0;
    cfg.filter.eye_gaze.derivative_cutoff = 100.0;
    cfg.filter.head.min_cutoff = f32::NAN;
    cfg.filter.head.beta = f32::INFINITY;
    cfg.filter.pupil.derivative_cutoff = f32::NAN;
    cfg.adjustment.ranges.insert(AdjustGroup::JawOpen, Range { min: 0.6, max: 0.3 });
    cfg.adjustment.ranges.insert(AdjustGroup::EyeWide, Range { min: 0.995, max: 1.0 });
    cfg.adjustment.ranges.insert(AdjustGroup::JawX, Range { min: -2.0, max: 5.0 });
    cfg.adjustment.ranges.insert(AdjustGroup::Neck, Range { min: f32::NAN, max: f32::NAN });
    let mut m = Mutator::new(MutationConfig::default(), CalibrationData::default());
    let out = m.set_config(cfg);
    assert_eq!(m.config(), &out);

    assert_eq!(out.calibration.blend, 1.0);
    assert_eq!(out.correctors.eye_lid_blend, 0.0);
    assert_eq!(out.filter.eye_gaze.min_cutoff, 0.01);
    assert_eq!(out.filter.eye_gaze.beta, 10.0);
    assert_eq!(out.filter.eye_gaze.derivative_cutoff, 30.0);
    assert_eq!(out.filter.head.min_cutoff, FilterConfig::default().head.min_cutoff);
    assert_eq!(out.filter.head.beta, FilterConfig::default().head.beta);
    assert_eq!(out.filter.pupil.derivative_cutoff, 1.0);

    let jaw = out.adjustment.range(AdjustGroup::JawOpen);
    assert!((jaw.min - 0.6).abs() < 1e-6, "{jaw:?}");
    assert!((jaw.max - 0.61).abs() < 1e-6, "max = min + 0.01: {jaw:?}");
    let wide = out.adjustment.range(AdjustGroup::EyeWide);
    assert_eq!(wide.max, 1.0);
    assert!((wide.min - 0.99).abs() < 1e-6, "{wide:?}");
    assert_eq!(out.adjustment.range(AdjustGroup::JawX), Range { min: 0.0, max: 1.0 });
    assert!(out.adjustment.range(AdjustGroup::Neck).is_identity());
    assert!(out.adjustment.range(AdjustGroup::TongueOut).is_identity(), "missing group is the identity");

    // Sanitizing is idempotent.
    assert_eq!(out.clone().sanitized(), out);
}

fn filtering() -> MutationConfig {
    let mut c = off();
    c.filter.enabled = true;
    c
}

#[test]
fn filter_smooths_jitter_and_reseeds_after_a_gap() {
    let mut m = Mutator::new(filtering(), CalibrationData::default());
    let mut t = 0.0;
    let mut raw_dev = 0.0f32;
    let mut out_dev = 0.0f32;
    for i in 0..300 {
        t += DT;
        let noisy = if i % 2 == 0 { 0.6 } else { 0.4 };
        let mut f = with_shape(E::CheekPuffLeft, noisy);
        f.eye.left.gaze_x = if i % 2 == 0 { 0.1 } else { -0.1 };
        m.apply(&mut f, t);
        if i == 0 {
            assert_eq!(f.shape(E::CheekPuffLeft), 0.6, "first sample seeds the filter");
        }
        if i > 100 {
            raw_dev = raw_dev.max((noisy - 0.5).abs());
            out_dev = out_dev.max((f.shape(E::CheekPuffLeft) - 0.5).abs());
            assert!(f.eye.left.gaze_x.abs() < 0.05, "gaze jitter {}", f.eye.left.gaze_x);
        }
    }
    assert!(out_dev < raw_dev * 0.5, "output deviation {out_dev} vs input {raw_dev}");

    // After a gap the filter starts over from the input.
    t += 2.0;
    let mut f = with_shape(E::CheekPuffLeft, 0.9);
    m.apply(&mut f, t);
    assert_eq!(f.shape(E::CheekPuffLeft), 0.9);
    // A non-positive dt re-seeds too.
    let mut f = with_shape(E::CheekPuffLeft, 0.1);
    m.apply(&mut f, t);
    assert_eq!(f.shape(E::CheekPuffLeft), 0.1);
}

#[test]
fn filter_follows_fast_movement_better_with_higher_beta() {
    let lag_after_step = |beta: f32| {
        let mut cfg = filtering();
        cfg.filter.expressions.beta = beta;
        cfg.filter.expressions.min_cutoff = 0.5;
        let mut m = Mutator::new(cfg, CalibrationData::default());
        let mut t = 0.0;
        run(&mut m, &with_shape(E::JawOpen, 0.0), 0.5, &mut t);
        run(&mut m, &with_shape(E::JawOpen, 1.0), 0.1, &mut t).shape(E::JawOpen)
    };
    assert!(lag_after_step(10.0) > lag_after_step(0.0) + 0.2);
}

#[test]
fn filter_categories_are_independent_and_reset_when_disabled() {
    let mut cfg = filtering();
    cfg.filter.expressions.enabled = false;
    let mut m = Mutator::new(cfg.clone(), CalibrationData::default());
    let mut t = 0.0;
    run(&mut m, &with_shape(E::JawOpen, 0.0), 0.2, &mut t);
    let mut f = with_shape(E::JawOpen, 1.0);
    f.head.yaw = 10.0;
    t += DT;
    m.apply(&mut f, t);
    assert_eq!(f.shape(E::JawOpen), 1.0, "disabled category passes through");
    assert!(f.head.yaw < 10.0, "head is filtered");

    // Switched off and on again, the filter must start from the input, not from stale state.
    cfg.filter.expressions.enabled = true;
    m.set_config(cfg.clone());
    let mut f = with_shape(E::JawOpen, 0.0);
    t += DT;
    m.apply(&mut f, t);
    cfg.filter.expressions.enabled = false;
    m.set_config(cfg.clone());
    cfg.filter.expressions.enabled = true;
    m.set_config(cfg);
    let mut f = with_shape(E::JawOpen, 0.7);
    t += DT;
    m.apply(&mut f, t);
    assert_eq!(f.shape(E::JawOpen), 0.7);
}

#[test]
fn stages_run_in_order_calibration_correctors_adjustment() {
    let data = CalibrationData {
        shapes: vec![ShapeCalibration { name: "JawOpen".into(), peak: 0.5, progress: 1.0 }],
    };
    let mut cfg = MutationConfig::default();
    cfg.calibration.enabled = true;
    cfg.calibration.continuous = false;
    cfg.adjustment.ranges.insert(AdjustGroup::MouthClosed, Range { min: 0.0, max: 0.5 });
    let mut m = Mutator::new(cfg, data);
    let mut f = with_shape(E::JawOpen, 0.25); // calibrated to 0.5
    f.set_shape(E::MouthClosed, 0.4);
    m.apply(&mut f, 0.0);
    assert_eq!(f.shape(E::JawOpen), 0.5);
    // corrector sees the calibrated jaw (0.5, no change); adjustment then maps 0.4 with max 0.5 -> 0.8
    assert!((f.shape(E::MouthClosed) - 0.8).abs() < 1e-6);
}

#[test]
fn json_defaults_round_trip_and_unknown_groups_are_ignored() {
    let empty: MutationConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, MutationConfig::default());
    assert!(empty.enabled && !empty.calibration.enabled && empty.calibration.continuous);
    assert!(!empty.filter.enabled && empty.adjustment.enabled);
    let data: CalibrationData = serde_json::from_str("{}").unwrap();
    assert_eq!(data, CalibrationData::default());

    let mut cfg = MutationConfig::default();
    cfg.calibration.blend = 0.5;
    cfg.correctors.eye_look_symmetrize = true;
    cfg.adjustment.ranges.insert(AdjustGroup::HeadPositionZ, Range { min: 0.25, max: 0.75 });
    cfg.filter.enabled = true;
    cfg.filter.head.derivative_cutoff = 2.5;
    let text = serde_json::to_string(&cfg).unwrap();
    assert_eq!(serde_json::from_str::<MutationConfig>(&text).unwrap(), cfg);

    let v = serde_json::to_value(&cfg).unwrap();
    assert_eq!(v["correctors"]["eyeLookSymmetrize"], json!(true));
    assert_eq!(v["adjustment"]["ranges"]["headPositionZ"], json!({ "min": 0.25, "max": 0.75 }));
    assert_eq!(v["filter"]["eyeOpenness"]["minCutoff"], json!(2.0));
    assert_eq!(v["filter"]["head"]["derivativeCutoff"], json!(2.5));

    let parsed: MutationConfig = serde_json::from_value(json!({
        "adjustment": { "ranges": { "jawOpen": { "min": 0.25 }, "noSuchGroup": { "min": 0.5, "max": 0.9 } } },
        "filter": { "eyeGaze": { "beta": 3.0 }, "pupil": { "enabled": false } },
    }))
    .unwrap();
    assert_eq!(parsed.adjustment.ranges.len(), 1);
    assert_eq!(parsed.adjustment.range(AdjustGroup::JawOpen), Range { min: 0.25, max: 1.0 });
    // A partial category keeps that category's own defaults.
    assert_eq!(parsed.filter.eye_gaze, FilterChannel { beta: 3.0, ..FilterConfig::default().eye_gaze });
    assert_eq!(parsed.filter.eye_gaze.min_cutoff, 1.0);
    assert!(!parsed.filter.pupil.enabled && parsed.filter.pupil.min_cutoff == 0.5);
    assert_eq!(parsed.filter.expressions, FilterConfig::default().expressions);

    let cal = CalibrationData { shapes: vec![ShapeCalibration { name: "EyeWideRight".into(), peak: 0.99, progress: 1.0 }] };
    let text = serde_json::to_string(&cal).unwrap();
    assert_eq!(text, r#"{"shapes":[{"name":"EyeWideRight","peak":0.99,"progress":1.0}]}"#);
    assert_eq!(serde_json::from_str::<CalibrationData>(&text).unwrap(), cal);
}

#[test]
fn group_names_labels_and_lookup() {
    for g in AdjustGroup::ALL {
        assert_eq!(AdjustGroup::from_name(g.name()), Some(g));
        assert!(!g.label().is_empty());
    }
    assert_eq!(AdjustGroup::JawX.name(), "jawX");
    assert_eq!(AdjustGroup::JawX.label(), "Jaw Sideways");
    assert_eq!(AdjustGroup::TongueMove.label(), "Tongue Directions");
    assert_eq!(AdjustGroup::HeadRotationYaw.label(), "Head Yaw");
    assert_eq!(AdjustGroup::from_name("nope"), None);
}
