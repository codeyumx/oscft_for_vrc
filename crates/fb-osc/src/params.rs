//! VRChat avatar parameter catalog, mapping math and per-avatar send plans. See `spec/osc-output.md`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use fb_tracking::{Expression as E, SingleEye, TrackingFrame};

use crate::Claims;
use crate::osc::Value;

/// Prefix of every avatar parameter address.
pub const PARAMETER_PREFIX: &str = "/avatar/parameters/";

/// Which claim must be held for a parameter to be sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Gaze, eyelid openness, pupils: the module that claimed eye tracking.
    Eye,
    /// Everything computed from the 88 expression shapes: the module that claimed expression tracking.
    Expression,
}

impl Group {
    fn claimed(self, claims: Claims) -> bool {
        match self {
            Group::Eye => claims.eye,
            Group::Expression => claims.expression,
        }
    }
}

/// Value range of a float parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    /// `0.0 ..= 1.0`
    Unit,
    /// `-1.0 ..= 1.0`
    Signed,
}

impl Range {
    fn clamp(self, v: f32) -> f32 {
        if v.is_nan() {
            return 0.0;
        }
        match self {
            Range::Unit => v.clamp(0.0, 1.0),
            Range::Signed => v.clamp(-1.0, 1.0),
        }
    }
}

/// Inputs of one evaluation.
struct Ctx<'a> {
    frame: &'a TrackingFrame,
    claims: Claims,
}

impl Ctx<'_> {
    fn s(&self, e: E) -> f32 {
        unit(self.frame.shape(e))
    }

    fn mean(&self, shapes: &[E]) -> f32 {
        shapes.iter().map(|&e| self.s(e)).sum::<f32>() / shapes.len() as f32
    }
}

fn unit(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

fn signed(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) }
}

#[derive(Clone, Copy)]
enum Src {
    /// One shape weight.
    Shape(E),
    /// `a - b`
    Diff(E, E),
    /// Arithmetic mean of shape weights.
    Mean(&'static [E]),
    /// Mean of two shape weights.
    Pair(E, E),
    Custom(fn(&Ctx) -> f32),
}

/// One float parameter FaceBridge can send (`<prefix>/v2/<name>`).
pub struct ParamDef {
    pub name: Cow<'static, str>,
    pub range: Range,
    pub group: Group,
    src: Src,
}

impl ParamDef {
    fn eval(&self, c: &Ctx) -> f32 {
        let v = match self.src {
            Src::Shape(e) => c.s(e),
            Src::Diff(a, b) => c.s(a) - c.s(b),
            Src::Pair(a, b) => (c.s(a) + c.s(b)) * 0.5,
            Src::Mean(shapes) => c.mean(shapes),
            Src::Custom(f) => f(c),
        };
        self.range.clamp(v)
    }
}

// ---- Eye (FaceBridge definitions where the docs give no formula) -------------------------------------------

fn gaze(v: f32) -> f32 {
    signed(v)
}

/// Eyelid: `0..0.75` is openness, `0.75..1` is widening (docs). FaceBridge definition: `0.75*open + 0.25*wide`,
/// where `wide` is the `EyeWide*` shape (0 unless expression tracking is claimed).
fn lid(c: &Ctx, eye: &SingleEye, wide: E) -> f32 {
    let w = if c.claims.expression { c.s(wide) } else { 0.0 };
    0.75 * unit(eye.openness) + 0.25 * w
}

fn lid_left(c: &Ctx) -> f32 {
    lid(c, &c.frame.eye.left, E::EyeWideLeft)
}

fn lid_right(c: &Ctx) -> f32 {
    lid(c, &c.frame.eye.right, E::EyeWideRight)
}

/// Normalized pupil dilation: mean diameter placed between the smallest and largest diameter seen.
fn pupil_dilation(c: &Ctx) -> f32 {
    let e = &c.frame.eye;
    let d = (e.left.pupil_diameter_mm + e.right.pupil_diameter_mm) * 0.5;
    let span = e.max_dilation - e.min_dilation;
    if span > 0.0 { (d - e.min_dilation) / span } else { 0.0 }
}

/// Pupil diameter in centimetres (the modules report millimetres), `0..1` covering 0 to 10 mm.
fn pupil_cm(mm: f32) -> f32 {
    mm / 10.0
}

// ---- Simplified mouth ----------------------------------------------------------------------------------------

fn smile_r(c: &Ctx) -> f32 {
    c.mean(&[E::MouthCornerPullRight, E::MouthCornerSlantRight])
}

fn smile_l(c: &Ctx) -> f32 {
    c.mean(&[E::MouthCornerPullLeft, E::MouthCornerSlantLeft])
}

fn sad_r(c: &Ctx) -> f32 {
    c.mean(&[E::MouthFrownRight, E::MouthStretchRight])
}

fn sad_l(c: &Ctx) -> f32 {
    c.mean(&[E::MouthFrownLeft, E::MouthStretchLeft])
}

fn smile_frown_r(c: &Ctx) -> f32 {
    smile_r(c) - c.s(E::MouthFrownRight)
}

fn smile_frown_l(c: &Ctx) -> f32 {
    smile_l(c) - c.s(E::MouthFrownLeft)
}

fn smile_sad_r(c: &Ctx) -> f32 {
    smile_r(c) - sad_r(c)
}

fn smile_sad_l(c: &Ctx) -> f32 {
    smile_l(c) - sad_l(c)
}

fn brow_up_r(c: &Ctx) -> f32 {
    c.mean(&[E::BrowInnerUpRight, E::BrowOuterUpRight])
}

fn brow_up_l(c: &Ctx) -> f32 {
    c.mean(&[E::BrowInnerUpLeft, E::BrowOuterUpLeft])
}

fn brow_down_r(c: &Ctx) -> f32 {
    c.mean(&[E::BrowLowererRight, E::BrowPinchRight])
}

fn brow_down_l(c: &Ctx) -> f32 {
    c.mean(&[E::BrowLowererLeft, E::BrowPinchLeft])
}

fn mouth_upper_x(c: &Ctx) -> f32 {
    c.s(E::MouthUpperRight) - c.s(E::MouthUpperLeft)
}

fn mouth_lower_x(c: &Ctx) -> f32 {
    c.s(E::MouthLowerRight) - c.s(E::MouthLowerLeft)
}

const fn def(name: &'static str, range: Range, group: Group, src: Src) -> ParamDef {
    ParamDef { name: Cow::Borrowed(name), range, group, src }
}

use Group::{Expression as Ex, Eye};
use Range::{Signed, Unit};

/// Parameters that are not a plain shape: eye data, signed pairs and simplified/combined parameters.
static DERIVED: &[ParamDef] = &[
    // eye gaze (Eye)
    def("EyeLeftX", Signed, Eye, Src::Custom(|c| gaze(c.frame.eye.left.gaze_x))),
    def("EyeLeftY", Signed, Eye, Src::Custom(|c| gaze(c.frame.eye.left.gaze_y))),
    def("EyeRightX", Signed, Eye, Src::Custom(|c| gaze(c.frame.eye.right.gaze_x))),
    def("EyeRightY", Signed, Eye, Src::Custom(|c| gaze(c.frame.eye.right.gaze_y))),
    def("EyeX", Signed, Eye, Src::Custom(|c| (gaze(c.frame.eye.left.gaze_x) + gaze(c.frame.eye.right.gaze_x)) * 0.5)),
    def("EyeY", Signed, Eye, Src::Custom(|c| (gaze(c.frame.eye.left.gaze_y) + gaze(c.frame.eye.right.gaze_y)) * 0.5)),
    // eyelids and pupils (Eye)
    def("EyeLidLeft", Unit, Eye, Src::Custom(lid_left)),
    def("EyeLidRight", Unit, Eye, Src::Custom(lid_right)),
    def("EyeLid", Unit, Eye, Src::Custom(|c| (lid_left(c) + lid_right(c)) * 0.5)),
    def("PupilDilation", Unit, Eye, Src::Custom(pupil_dilation)),
    def("PupilDiameterLeft", Unit, Eye, Src::Custom(|c| pupil_cm(c.frame.eye.left.pupil_diameter_mm))),
    def("PupilDiameterRight", Unit, Eye, Src::Custom(|c| pupil_cm(c.frame.eye.right.pupil_diameter_mm))),
    def(
        "PupilDiameter",
        Unit,
        Eye,
        Src::Custom(|c| pupil_cm((c.frame.eye.left.pupil_diameter_mm + c.frame.eye.right.pupil_diameter_mm) * 0.5)),
    ),
    // eye squint (shapes)
    def("EyeSquint", Unit, Ex, Src::Mean(&[E::EyeSquintRight, E::EyeSquintLeft])),
    // cheeks
    def("CheekPuffSuckRight", Signed, Ex, Src::Diff(E::CheekPuffRight, E::CheekSuckRight)),
    def("CheekPuffSuckLeft", Signed, Ex, Src::Diff(E::CheekPuffLeft, E::CheekSuckLeft)),
    def(
        "CheekPuffSuck",
        Signed,
        Ex,
        Src::Custom(|c| (c.s(E::CheekPuffRight) - c.s(E::CheekSuckRight) + c.s(E::CheekPuffLeft) - c.s(E::CheekSuckLeft)) * 0.5),
    ),
    def("CheekSquint", Unit, Ex, Src::Mean(&[E::CheekSquintRight, E::CheekSquintLeft])),
    def("NoseSneer", Unit, Ex, Src::Mean(&[E::NoseSneerRight, E::NoseSneerLeft])),
    // jaw
    def("JawX", Signed, Ex, Src::Diff(E::JawRight, E::JawLeft)),
    def("JawZ", Signed, Ex, Src::Diff(E::JawForward, E::JawBackward)),
    // mouth
    def("MouthUpperX", Signed, Ex, Src::Diff(E::MouthUpperRight, E::MouthUpperLeft)),
    def("MouthLowerX", Signed, Ex, Src::Diff(E::MouthLowerRight, E::MouthLowerLeft)),
    def("MouthX", Signed, Ex, Src::Custom(|c| (mouth_upper_x(c) + mouth_lower_x(c)) * 0.5)),
    def("MouthUpperUp", Unit, Ex, Src::Mean(&[E::MouthUpperUpRight, E::MouthUpperUpLeft])),
    def("MouthLowerDown", Unit, Ex, Src::Mean(&[E::MouthLowerDownRight, E::MouthLowerDownLeft])),
    def(
        "MouthOpen",
        Unit,
        Ex,
        Src::Mean(&[E::MouthUpperUpRight, E::MouthUpperUpLeft, E::MouthLowerDownRight, E::MouthLowerDownLeft]),
    ),
    def("MouthSmileRight", Unit, Ex, Src::Custom(smile_r)),
    def("MouthSmileLeft", Unit, Ex, Src::Custom(smile_l)),
    def("MouthSadRight", Unit, Ex, Src::Custom(sad_r)),
    def("MouthSadLeft", Unit, Ex, Src::Custom(sad_l)),
    def("SmileFrownRight", Signed, Ex, Src::Custom(smile_frown_r)),
    def("SmileFrownLeft", Signed, Ex, Src::Custom(smile_frown_l)),
    def("SmileFrown", Signed, Ex, Src::Custom(|c| (smile_frown_r(c) + smile_frown_l(c)) * 0.5)),
    def("SmileSadRight", Signed, Ex, Src::Custom(smile_sad_r)),
    def("SmileSadLeft", Signed, Ex, Src::Custom(smile_sad_l)),
    def("SmileSad", Signed, Ex, Src::Custom(|c| (smile_sad_r(c) + smile_sad_l(c)) * 0.5)),
    // brows
    def("BrowDownRight", Unit, Ex, Src::Custom(brow_down_r)),
    def("BrowDownLeft", Unit, Ex, Src::Custom(brow_down_l)),
    def("BrowOuterUp", Unit, Ex, Src::Mean(&[E::BrowOuterUpRight, E::BrowOuterUpLeft])),
    def("BrowInnerUp", Unit, Ex, Src::Mean(&[E::BrowInnerUpRight, E::BrowInnerUpLeft])),
    def(
        "BrowUp",
        Unit,
        Ex,
        Src::Mean(&[E::BrowInnerUpRight, E::BrowInnerUpLeft, E::BrowOuterUpRight, E::BrowOuterUpLeft]),
    ),
    def("BrowExpressionRight", Signed, Ex, Src::Custom(|c| brow_up_r(c) - brow_down_r(c))),
    def("BrowExpressionLeft", Signed, Ex, Src::Custom(|c| brow_up_l(c) - brow_down_l(c))),
    def(
        "BrowExpression",
        Signed,
        Ex,
        Src::Custom(|c| (brow_up_r(c) - brow_down_r(c) + brow_up_l(c) - brow_down_l(c)) * 0.5),
    ),
    // lips
    def("LipSuckUpper", Unit, Ex, Src::Mean(&[E::LipSuckUpperRight, E::LipSuckUpperLeft])),
    def("LipSuckLower", Unit, Ex, Src::Mean(&[E::LipSuckLowerRight, E::LipSuckLowerLeft])),
    def(
        "LipSuck",
        Unit,
        Ex,
        Src::Mean(&[E::LipSuckUpperRight, E::LipSuckUpperLeft, E::LipSuckLowerRight, E::LipSuckLowerLeft]),
    ),
    def("LipFunnelUpper", Unit, Ex, Src::Mean(&[E::LipFunnelUpperRight, E::LipFunnelUpperLeft])),
    def("LipFunnelLower", Unit, Ex, Src::Mean(&[E::LipFunnelLowerRight, E::LipFunnelLowerLeft])),
    def(
        "LipFunnel",
        Unit,
        Ex,
        Src::Mean(&[E::LipFunnelUpperRight, E::LipFunnelUpperLeft, E::LipFunnelLowerRight, E::LipFunnelLowerLeft]),
    ),
    def("LipPuckerUpper", Unit, Ex, Src::Mean(&[E::LipPuckerUpperRight, E::LipPuckerUpperLeft])),
    def("LipPuckerLower", Unit, Ex, Src::Mean(&[E::LipPuckerLowerRight, E::LipPuckerLowerLeft])),
    def(
        "LipPucker",
        Unit,
        Ex,
        Src::Mean(&[E::LipPuckerUpperRight, E::LipPuckerUpperLeft, E::LipPuckerLowerRight, E::LipPuckerLowerLeft]),
    ),
    // tongue
    def("TongueX", Signed, Ex, Src::Diff(E::TongueRight, E::TongueLeft)),
    def("TongueY", Signed, Ex, Src::Diff(E::TongueUp, E::TongueDown)),
    def("TongueArchY", Signed, Ex, Src::Diff(E::TongueBendDown, E::TongueCurlUp)),
    def("TongueShape", Signed, Ex, Src::Diff(E::TongueSquish, E::TongueFlat)),
];

/// `<X>Right`/`<X>Left` shape pairs that are axes (signed `JawX`, `MouthUpperX`, `MouthLowerX`) rather than
/// left/right halves of one expression: no combined `<X>` parameter.
const NOT_COMBINED: [&str; 3] = ["Jaw", "MouthUpper", "MouthLower"];

/// Float names the VRCFT docs list with a stray binary-style suffix; accepted as aliases for float parameters.
const FLOAT_ALIASES: [(&str, &str); 2] = [("JawClench1", "JawClench"), ("JawMandibleRaise1", "JawMandibleRaise")];

pub struct Catalog {
    defs: Vec<ParamDef>,
    by_name: HashMap<String, usize>,
}

impl Catalog {
    pub fn defs(&self) -> &[ParamDef] {
        &self.defs
    }

    fn lookup(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    fn lookup_float(&self, name: &str) -> Option<usize> {
        self.lookup(name).or_else(|| FLOAT_ALIASES.iter().find(|(a, _)| *a == name).and_then(|(_, base)| self.lookup(base)))
    }
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let mut defs: Vec<ParamDef> = E::ALL.into_iter().map(|e| def(e.name(), Unit, Ex, Src::Shape(e))).collect();
    defs.extend(DERIVED.iter().map(|d| ParamDef { name: d.name.clone(), range: d.range, group: d.group, src: d.src }));
    // `<X>Right` + `<X>Left` shapes without an explicit parameter `<X>`: combined parameter, mean of both.
    for right in E::ALL {
        let Some(base) = right.name().strip_suffix("Right") else { continue };
        let Some(left) = E::from_name(&format!("{base}Left")) else { continue };
        if NOT_COMBINED.contains(&base) || defs.iter().any(|d| d.name == base) {
            continue;
        }
        defs.push(ParamDef { name: Cow::Owned(base.to_owned()), range: Unit, group: Ex, src: Src::Pair(right, left) });
    }
    let by_name = defs.iter().enumerate().map(|(i, d)| (d.name.to_string(), i)).collect();
    Catalog { defs, by_name }
});

pub fn catalog() -> &'static Catalog {
    &CATALOG
}

// ---- Binary parameters -------------------------------------------------------------------------------------------

/// Quantize a magnitude in `0..=1` to `0..=steps`.
pub fn quantize(magnitude: f32, steps: u32) -> u32 {
    let m = if magnitude.is_nan() { 0.0 } else { magnitude.clamp(0.0, 1.0) };
    ((m * steps as f32).round() as u32).min(steps)
}

/// Value of binary bit `bit` (weight `2^bit`) for a float `value` in a set whose largest bit is `max_bit`:
/// the magnitude is quantized to `2^(max_bit+1) - 1` steps.
pub fn binary_bit(value: f32, bit: u8, max_bit: u8) -> bool {
    (quantize(value.abs(), binary_steps(max_bit)) >> bit) & 1 == 1
}

fn binary_steps(max_bit: u8) -> u32 {
    (1u32 << (max_bit as u32 + 1)) - 1
}

// ---- Send plan --------------------------------------------------------------------------------------------------

/// Type of an avatar parameter as reported by VRChat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamType {
    Float,
    Int,
    Bool,
}

/// An avatar parameter VRChat accepts input for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvatarParam {
    /// Full OSC address, e.g. `/avatar/parameters/FT/v2/JawOpen`.
    pub address: String,
    pub ty: ParamType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Active {
    Eye,
    Expression,
    Lip,
}

const ACTIVE_PARAMS: [(&str, Active); 3] = [
    ("EyeTrackingActive", Active::Eye),
    ("ExpressionTrackingActive", Active::Expression),
    ("LipTrackingActive", Active::Lip),
];

#[derive(Clone, Copy, Debug)]
enum Out {
    Float,
    Bit { bit: u8, max_bit: u8 },
    Negative,
    Active(Active),
}

struct Entry {
    address: Box<[u8]>,
    ty: ParamType,
    out: Out,
    /// Claim needed to send this entry (`None` for the tracking-active flags).
    group: Option<Group>,
    /// Index into `SendPlan::values`.
    slot: usize,
    last: Option<Value>,
}

/// What to send to one avatar: the avatar parameters that carry FaceBridge data, with their encoding and change
/// detection state.
pub struct SendPlan {
    entries: Vec<Entry>,
    /// Catalog indices of the base values the entries need.
    slots: Vec<usize>,
    values: Vec<f32>,
}

/// Floats that moved less than this since the last *sent* value are not resent (VRChat quantizes synced
/// floats to 8 bits, so this is far below what is observable).
pub const FLOAT_EPSILON: f32 = 0.0005;

/// Change detection for one parameter. Exact bool/int comparison; floats use [`FLOAT_EPSILON`], but a move to
/// exactly `0`, `1` or `-1` is always sent so rest and end positions are never left slightly off.
pub fn changed(last: Option<Value>, new: Value) -> bool {
    match (last, new) {
        (Some(Value::Float(a)), Value::Float(b)) => {
            let snap = b == 0.0 || b == 1.0 || b == -1.0;
            (a - b).abs() > FLOAT_EPSILON || (snap && a != b)
        }
        (Some(a), b) => a != b,
        (None, _) => true,
    }
}

fn active_value(ty: ParamType, on: bool) -> Value {
    match ty {
        ParamType::Bool => Value::Bool(on),
        ParamType::Int => Value::Int(on as i32),
        ParamType::Float => Value::Float(if on { 1.0 } else { 0.0 }),
    }
}

enum Binary {
    Bit(u8),
    Negative,
}

/// `JawOpen4` -> (`JawOpen`, bit 2); `JawXNegative` -> (`JawX`, sign). Only exact powers of two up to `2^15`.
fn split_binary(name: &str) -> Option<(&str, Binary)> {
    if let Some(base) = name.strip_suffix("Negative") {
        return (!base.is_empty()).then_some((base, Binary::Negative));
    }
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &name[base.len()..];
    if base.is_empty() || digits.is_empty() || digits.starts_with('0') || digits.len() > 5 {
        return None;
    }
    let n: u32 = digits.parse().ok()?;
    (n.is_power_of_two() && n <= 1 << 15).then(|| (base, Binary::Bit(n.trailing_zeros() as u8)))
}

/// Split `FT/v2/JawOpen` into its prefix (`FT/v2`) and name (`JawOpen`); `None` unless the parameter sits directly
/// under a `v2` segment.
fn split_v2(name: &str) -> Option<(&str, &str)> {
    let (head, last) = name.rsplit_once('/')?;
    (head == "v2" || head.ends_with("/v2")).then_some((head, last))
}

/// Build the send plan for an avatar: float parameters, binary bit/sign parameters and tracking-active flags
/// found among `params`. Unmatched parameters are ignored. Addresses keep VRChat's exact spelling.
pub fn build_plan(params: &[AvatarParam]) -> SendPlan {
    let cat = catalog();
    let mut seen = HashSet::new();
    let mut slot_of: HashMap<usize, usize> = HashMap::new();
    let mut slots = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    // (prefix, catalog index) -> largest bit present
    let mut max_bits: HashMap<(&str, usize), u8> = HashMap::new();
    let mut bits: Vec<(&AvatarParam, &str, usize, u8)> = Vec::new();

    let mut slot = |def: usize| -> usize {
        *slot_of.entry(def).or_insert_with(|| {
            slots.push(def);
            slots.len() - 1
        })
    };

    for p in params {
        let Some(name) = p.address.strip_prefix(PARAMETER_PREFIX) else { continue };
        if !seen.insert(p.address.as_str()) {
            continue;
        }
        let last = name.rsplit_once('/').map_or(name, |(_, l)| l);
        if let Some(&(_, active)) = ACTIVE_PARAMS.iter().find(|(n, _)| *n == last) {
            entries.push(Entry {
                address: crate::osc::encode_address(&p.address),
                ty: p.ty,
                out: Out::Active(active),
                group: None,
                slot: 0,
                last: None,
            });
            continue;
        }
        let Some((prefix, pname)) = split_v2(name) else { continue };
        match p.ty {
            ParamType::Float => {
                if let Some(def) = cat.lookup_float(pname) {
                    entries.push(Entry {
                        address: crate::osc::encode_address(&p.address),
                        ty: p.ty,
                        out: Out::Float,
                        group: Some(cat.defs[def].group),
                        slot: slot(def),
                        last: None,
                    });
                }
            }
            ParamType::Bool => {
                let Some((base, part)) = split_binary(pname) else { continue };
                let Some(def) = cat.lookup(base) else { continue };
                match part {
                    Binary::Negative => entries.push(Entry {
                        address: crate::osc::encode_address(&p.address),
                        ty: p.ty,
                        out: Out::Negative,
                        group: Some(cat.defs[def].group),
                        slot: slot(def),
                        last: None,
                    }),
                    Binary::Bit(bit) => {
                        let m = max_bits.entry((prefix, def)).or_insert(0);
                        *m = (*m).max(bit);
                        bits.push((p, prefix, def, bit));
                    }
                }
            }
            ParamType::Int => {}
        }
    }

    for (p, prefix, def, bit) in bits {
        entries.push(Entry {
            address: crate::osc::encode_address(&p.address),
            ty: p.ty,
            out: Out::Bit { bit, max_bit: max_bits[&(prefix, def)] },
            group: Some(cat.defs[def].group),
            slot: slot(def),
            last: None,
        });
    }

    let values = vec![0.0; slots.len()];
    SendPlan { entries, slots, values }
}

impl SendPlan {
    /// Number of avatar parameters that will receive data.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Compute every parameter of the plan from `frame` and hand the ones that need sending to `sink`
    /// (padded address, value). Unclaimed groups are skipped entirely; `full` sends everything regardless of
    /// change detection (periodic resend, new avatar).
    pub fn collect(&mut self, frame: &TrackingFrame, claims: Claims, full: bool, sink: &mut impl FnMut(&[u8], Value)) {
        let cat = catalog();
        let ctx = Ctx { frame, claims };
        let Self { entries, slots, values } = self;
        for (v, &def) in values.iter_mut().zip(slots.iter()) {
            let d = &cat.defs[def];
            *v = if d.group.claimed(claims) { d.eval(&ctx) } else { 0.0 };
        }
        for e in entries.iter_mut() {
            let value = match e.out {
                Out::Active(a) => active_value(
                    e.ty,
                    match a {
                        Active::Eye => claims.eye,
                        Active::Expression | Active::Lip => claims.expression,
                    },
                ),
                out => {
                    if !e.group.is_some_and(|g| g.claimed(claims)) {
                        continue;
                    }
                    let v = values[e.slot];
                    match out {
                        Out::Float => Value::Float(v),
                        Out::Bit { bit, max_bit } => Value::Bool(binary_bit(v, bit, max_bit)),
                        Out::Negative => Value::Bool(v < 0.0),
                        Out::Active(_) => unreachable!(),
                    }
                }
            };
            if full || changed(e.last, value) {
                sink(&e.address, value);
                e.last = Some(value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osc::{self, Arg};

    fn p(address: &str, ty: ParamType) -> AvatarParam {
        AvatarParam { address: format!("{PARAMETER_PREFIX}{address}"), ty }
    }

    fn both() -> Claims {
        Claims { eye: true, expression: true }
    }

    fn eval(name: &str, frame: &TrackingFrame, claims: Claims) -> f32 {
        let cat = catalog();
        cat.defs[cat.lookup(name).unwrap()].eval(&Ctx { frame, claims })
    }

    fn sent(plan: &mut SendPlan, frame: &TrackingFrame, claims: Claims, full: bool) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        plan.collect(frame, claims, full, &mut |addr, v| {
            let (a, args) = {
                let mut m = addr.to_vec();
                m.extend_from_slice(match v {
                    Value::Float(_) => b",f\0\0",
                    Value::Int(_) => b",i\0\0",
                    Value::Bool(true) => b",T\0\0",
                    Value::Bool(false) => b",F\0\0",
                });
                match v {
                    Value::Float(f) => m.extend_from_slice(&f.to_be_bytes()),
                    Value::Int(i) => m.extend_from_slice(&i.to_be_bytes()),
                    Value::Bool(_) => {}
                }
                osc::decode_message(&m).unwrap()
            };
            assert_eq!(args.len(), 1);
            assert!(matches!(args[0], Arg::Float(_) | Arg::Int(_) | Arg::Bool(_)));
            out.push((a, v));
        });
        out
    }

    #[test]
    fn catalog_is_consistent() {
        let cat = catalog();
        assert_eq!(cat.defs().len(), cat.by_name.len(), "parameter names must be unique");
        for e in E::ALL {
            assert!(cat.lookup(e.name()).is_some(), "{}", e.name());
        }
        // generated combined (Right+Left) parameters seen on real avatars but absent from the docs
        for name in ["MouthPress", "MouthTightener", "NeckFlex", "EyeWide", "CheekPuff", "CheekSuck", "TongueTwist"] {
            assert!(cat.lookup(name).is_some(), "{name}");
        }
        // axis pairs are signed parameters, not "Jaw"/"MouthUpper" means
        for name in ["Jaw", "MouthUpper", "MouthLower"] {
            assert!(cat.lookup(name).is_none(), "{name}");
        }
        // explicitly defined simplified parameters keep their definition
        assert!(matches!(cat.defs[cat.lookup("NoseSneer").unwrap()].src, Src::Mean(_)));
        for (_, base) in FLOAT_ALIASES {
            assert!(cat.lookup(base).is_some());
        }
    }

    #[test]
    fn eye_gaze_and_lids() {
        let mut f = TrackingFrame::default();
        f.eye.left.gaze_x = 0.5;
        f.eye.right.gaze_x = -0.25;
        f.eye.left.gaze_y = -2.0; // out of range input is clamped
        f.eye.left.openness = 1.0;
        f.eye.right.openness = 0.4;
        let c = both();
        assert_eq!(eval("EyeLeftX", &f, c), 0.5);
        assert_eq!(eval("EyeRightX", &f, c), -0.25);
        assert_eq!(eval("EyeX", &f, c), 0.125);
        assert_eq!(eval("EyeLeftY", &f, c), -1.0);
        // fully open, not widened = 0.75; openness 0.4 = 0.3
        assert!((eval("EyeLidLeft", &f, c) - 0.75).abs() < 1e-6);
        assert!((eval("EyeLidRight", &f, c) - 0.3).abs() < 1e-6);
        assert!((eval("EyeLid", &f, c) - 0.525).abs() < 1e-6);
        f.set_shape(E::EyeWideLeft, 1.0);
        assert!((eval("EyeLidLeft", &f, c) - 1.0).abs() < 1e-6);
        // widening needs the expression claim
        assert!((eval("EyeLidLeft", &f, Claims { eye: true, expression: false }) - 0.75).abs() < 1e-6);
        f.eye.left.openness = 0.0;
        f.set_shape(E::EyeWideLeft, 0.0);
        assert_eq!(eval("EyeLidLeft", &f, c), 0.0);
    }

    #[test]
    fn pupils() {
        let mut f = TrackingFrame::default();
        f.eye.left.pupil_diameter_mm = 4.0;
        f.eye.right.pupil_diameter_mm = 6.0;
        f.eye.min_dilation = 2.0;
        f.eye.max_dilation = 8.0;
        let c = both();
        assert!((eval("PupilDilation", &f, c) - 0.5).abs() < 1e-6); // mean 5 in [2, 8]
        assert!((eval("PupilDiameterLeft", &f, c) - 0.4).abs() < 1e-6);
        assert!((eval("PupilDiameter", &f, c) - 0.5).abs() < 1e-6);
        f.eye.max_dilation = f.eye.min_dilation; // no range seen yet
        assert_eq!(eval("PupilDilation", &f, c), 0.0);
        f.eye.left.pupil_diameter_mm = 40.0; // clamped
        assert_eq!(eval("PupilDiameterLeft", &f, c), 1.0);
    }

    #[test]
    fn signed_and_simplified_shapes() {
        let mut f = TrackingFrame::default();
        f.set_shape(E::JawRight, 0.75);
        f.set_shape(E::JawLeft, 0.25);
        f.set_shape(E::CheekPuffRight, 0.2);
        f.set_shape(E::CheekSuckRight, 0.8);
        f.set_shape(E::MouthCornerPullRight, 0.6);
        f.set_shape(E::MouthCornerSlantRight, 0.2);
        f.set_shape(E::MouthFrownRight, 0.1);
        f.set_shape(E::MouthStretchRight, 0.3);
        f.set_shape(E::BrowInnerUpRight, 0.4);
        f.set_shape(E::BrowOuterUpRight, 0.2);
        f.set_shape(E::BrowLowererRight, 0.1);
        f.set_shape(E::BrowPinchRight, 0.1);
        f.set_shape(E::TongueBendDown, 0.1);
        f.set_shape(E::TongueCurlUp, 0.6);
        f.set_shape(E::JawOpen, 1.7); // shape weights are clamped to 0..1
        let c = both();
        assert_eq!(eval("JawX", &f, c), 0.5);
        assert!((eval("CheekPuffSuckRight", &f, c) + 0.6).abs() < 1e-6);
        assert!((eval("CheekPuffSuck", &f, c) + 0.3).abs() < 1e-6);
        assert!((eval("MouthSmileRight", &f, c) - 0.4).abs() < 1e-6);
        assert!((eval("SmileFrownRight", &f, c) - 0.3).abs() < 1e-6);
        assert!((eval("SmileSadRight", &f, c) - 0.2).abs() < 1e-6); // 0.4 - mean(0.1, 0.3)
        assert!((eval("BrowExpressionRight", &f, c) - 0.2).abs() < 1e-6); // 0.3 - 0.1
        assert!((eval("TongueArchY", &f, c) + 0.5).abs() < 1e-6);
        assert_eq!(eval("JawOpen", &f, c), 1.0);
        assert!((eval("BrowUp", &f, c) - 0.15).abs() < 1e-6); // (0.4 + 0.2 + 0 + 0) / 4
    }

    #[test]
    fn binary_encoding_matches_documented_scheme() {
        // 3 bits (1, 2, 4): steps = 7, each bit is worth 1/8 of the range in the docs' arithmetic.
        let bits = |v: f32| (0..3).map(|b| binary_bit(v, b, 2)).collect::<Vec<_>>();
        assert_eq!(bits(0.0), [false, false, false]);
        assert_eq!(bits(1.0), [true, true, true]);
        assert_eq!(bits(5.0 / 7.0), [true, false, true]);
        assert_eq!(bits(0.5), [false, false, true]); // round(3.5) = 4
        // sign is carried by the Negative parameter, the bits encode the magnitude
        assert_eq!(bits(-1.0), [true, true, true]);
        assert_eq!(bits(-2.0 / 7.0), [false, true, false]);
        assert_eq!(quantize(f32::NAN, 7), 0);
        assert_eq!(quantize(3.0, 7), 7);
    }

    #[test]
    fn split_names() {
        assert_eq!(split_v2("v2/JawOpen"), Some(("v2", "JawOpen")));
        assert_eq!(split_v2("FT/v2/JawOpen"), Some(("FT/v2", "JawOpen")));
        assert_eq!(split_v2("A/B/v2/X"), Some(("A/B/v2", "X")));
        assert_eq!(split_v2("JawOpen"), None);
        assert_eq!(split_v2("FT/v3/JawOpen"), None);
        assert_eq!(split_v2("FT/xv2/JawOpen"), None);
        assert!(matches!(split_binary("JawOpen4"), Some(("JawOpen", Binary::Bit(2)))));
        assert!(matches!(split_binary("JawOpen1"), Some(("JawOpen", Binary::Bit(0)))));
        assert!(matches!(split_binary("JawXNegative"), Some(("JawX", Binary::Negative))));
        assert!(split_binary("JawOpen3").is_none());
        assert!(split_binary("JawOpen08").is_none());
        assert!(split_binary("JawOpen").is_none());
        assert!(split_binary("16").is_none());
    }

    fn fixture_params() -> Vec<AvatarParam> {
        use ParamType::*;
        vec![
            p("FT/v2/JawOpen", Float),
            p("FT/v2/JawX", Float),
            p("FT/v2/JawXNegative", Bool),
            p("FT/v2/JawX1", Bool),
            p("FT/v2/JawX2", Bool),
            p("FT/v2/JawX4", Bool),
            p("FT/v2/EyeLeftX", Float),
            p("v2/EyeLidRight", Float),
            p("FT/v2/JawClench1", Float),
            p("FT/v2/JawClench4", Bool),
            p("FT/v2/NotAParameter", Float),
            p("FT/v2/TongueOut", Int), // wrong type: ignored
            p("JawOpen", Float),       // legacy unprefixed: ignored
            p("EyeTrackingActive", Bool),
            p("Prefix/LipTrackingActive", Bool),
            p("VRCEmote", Int),
            p("FT/v2/JawOpen", Float), // duplicate address
        ]
    }

    #[test]
    fn plan_matches_prefixed_params_and_binary_variants() {
        let plan = build_plan(&fixture_params());
        // JawOpen, JawX, Negative, 3 bits, EyeLeftX, EyeLidRight, JawClench float alias, JawClench4 bit, 2 active
        assert_eq!(plan.len(), 12);
        assert_eq!(plan.slots.len(), 5); // JawOpen, JawX, EyeLeftX, EyeLidRight, JawClench (shared by float and bit)
    }

    #[test]
    fn plan_sends_only_claimed_groups_and_changes() {
        let mut plan = build_plan(&fixture_params());
        let mut f = TrackingFrame::default();
        f.set_shape(E::JawOpen, 0.5);
        f.set_shape(E::JawLeft, 5.0 / 7.0); // JawX = -5/7
        f.eye.left.gaze_x = 0.25;
        f.eye.right.openness = 1.0;

        // expression only: no eye parameters; active flags always sent
        let first = sent(&mut plan, &f, Claims { eye: false, expression: true }, false);
        let names: Vec<&str> = first.iter().map(|(a, _)| a.as_str()).collect();
        assert!(!names.contains(&"/avatar/parameters/FT/v2/EyeLeftX"));
        assert!(!names.contains(&"/avatar/parameters/v2/EyeLidRight"));
        assert!(names.contains(&"/avatar/parameters/FT/v2/JawOpen"));
        assert!(names.contains(&"/avatar/parameters/EyeTrackingActive"));
        let get = |n: &str| first.iter().find(|(a, _)| a == &format!("{PARAMETER_PREFIX}{n}")).map(|(_, v)| *v);
        assert_eq!(get("EyeTrackingActive"), Some(Value::Bool(false)));
        assert_eq!(get("Prefix/LipTrackingActive"), Some(Value::Bool(true)));
        assert_eq!(get("FT/v2/JawOpen"), Some(Value::Float(0.5)));
        assert_eq!(get("FT/v2/JawX"), Some(Value::Float(-5.0 / 7.0)));
        assert_eq!(get("FT/v2/JawXNegative"), Some(Value::Bool(true)));
        // 3 bits, magnitude 5/7 -> q = 5 = 0b101
        assert_eq!(get("FT/v2/JawX1"), Some(Value::Bool(true)));
        assert_eq!(get("FT/v2/JawX2"), Some(Value::Bool(false)));
        assert_eq!(get("FT/v2/JawX4"), Some(Value::Bool(true)));
        // JawClench4 is the only bit of its set -> 1 step: magnitude 0 -> false
        assert_eq!(get("FT/v2/JawClench4"), Some(Value::Bool(false)));
        assert_eq!(get("FT/v2/JawClench1"), Some(Value::Float(0.0)));

        // nothing changed -> nothing sent
        assert!(sent(&mut plan, &f, Claims { eye: false, expression: true }, false).is_empty());
        // full resend sends everything claimed again
        assert_eq!(sent(&mut plan, &f, Claims { eye: false, expression: true }, true).len(), first.len());

        // gaining the eye claim sends the eye parameters and flips the active flag
        let eye = sent(&mut plan, &f, both(), false);
        let en: Vec<&str> = eye.iter().map(|(a, _)| a.as_str()).collect();
        assert!(en.contains(&"/avatar/parameters/FT/v2/EyeLeftX"));
        assert!(en.contains(&"/avatar/parameters/EyeTrackingActive"));
        assert_eq!(eye.len(), 3);
    }

    #[test]
    fn change_detection_thresholds() {
        let f = |v| Value::Float(v);
        assert!(changed(None, f(0.3)));
        assert!(!changed(Some(f(0.3)), f(0.3002)));
        assert!(changed(Some(f(0.3)), f(0.302)));
        // rest/end positions are never left slightly off
        assert!(changed(Some(f(0.0003)), f(0.0)));
        assert!(changed(Some(f(0.9997)), f(1.0)));
        assert!(changed(Some(f(-0.9997)), f(-1.0)));
        assert!(!changed(Some(f(0.0)), f(0.0)));
        assert!(!changed(Some(Value::Bool(true)), Value::Bool(true)));
        assert!(changed(Some(Value::Bool(true)), Value::Bool(false)));
        assert!(changed(Some(Value::Int(1)), Value::Int(2)));
    }

    #[test]
    fn active_flags_adapt_to_avatar_type() {
        let params = vec![
            p("EyeTrackingActive", ParamType::Float),
            p("ExpressionTrackingActive", ParamType::Int),
        ];
        let mut plan = build_plan(&params);
        let out = sent(&mut plan, &TrackingFrame::default(), both(), true);
        assert_eq!(out[0].1, Value::Float(1.0));
        assert_eq!(out[1].1, Value::Int(1));
    }
}
