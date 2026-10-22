//! Parameter adjustment groups and the per-group input range (spec/mutation.md, stage 3).

use fb_tracking::{Expression, Head, SHAPE_COUNT};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use Expression as E;

/// A set of parameters the user adjusts together with one `{min, max}` input range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AdjustGroup {
    EyeBrowRaiser,
    EyeBrowLower,
    EyeSquint,
    EyeWide,
    CheekPuffSuck,
    CheekSquint,
    JawOpen,
    MouthClosed,
    JawX,
    JawZ,
    LipFunnel,
    LipSuck,
    LipPucker,
    MouthOpener,
    MouthSmile,
    MouthFrown,
    MouthStretch,
    MouthTightener,
    MouthPress,
    MouthX,
    MouthRaiser,
    Nose,
    NoseSneer,
    Neck,
    TongueOut,
    TongueMove,
    TongueOther,
    HeadRotationYaw,
    HeadRotationPitch,
    HeadRotationRoll,
    HeadPositionX,
    HeadPositionY,
    HeadPositionZ,
}

impl AdjustGroup {
    /// Every group: shape groups first, then the head groups.
    pub const ALL: [AdjustGroup; 33] = [
        Self::EyeBrowRaiser,
        Self::EyeBrowLower,
        Self::EyeSquint,
        Self::EyeWide,
        Self::CheekPuffSuck,
        Self::CheekSquint,
        Self::JawOpen,
        Self::MouthClosed,
        Self::JawX,
        Self::JawZ,
        Self::LipFunnel,
        Self::LipSuck,
        Self::LipPucker,
        Self::MouthOpener,
        Self::MouthSmile,
        Self::MouthFrown,
        Self::MouthStretch,
        Self::MouthTightener,
        Self::MouthPress,
        Self::MouthX,
        Self::MouthRaiser,
        Self::Nose,
        Self::NoseSneer,
        Self::Neck,
        Self::TongueOut,
        Self::TongueMove,
        Self::TongueOther,
        Self::HeadRotationYaw,
        Self::HeadRotationPitch,
        Self::HeadRotationRoll,
        Self::HeadPositionX,
        Self::HeadPositionY,
        Self::HeadPositionZ,
    ];

    /// The camelCase key used in JSON (`"jawOpen"`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::EyeBrowRaiser => "eyeBrowRaiser",
            Self::EyeBrowLower => "eyeBrowLower",
            Self::EyeSquint => "eyeSquint",
            Self::EyeWide => "eyeWide",
            Self::CheekPuffSuck => "cheekPuffSuck",
            Self::CheekSquint => "cheekSquint",
            Self::JawOpen => "jawOpen",
            Self::MouthClosed => "mouthClosed",
            Self::JawX => "jawX",
            Self::JawZ => "jawZ",
            Self::LipFunnel => "lipFunnel",
            Self::LipSuck => "lipSuck",
            Self::LipPucker => "lipPucker",
            Self::MouthOpener => "mouthOpener",
            Self::MouthSmile => "mouthSmile",
            Self::MouthFrown => "mouthFrown",
            Self::MouthStretch => "mouthStretch",
            Self::MouthTightener => "mouthTightener",
            Self::MouthPress => "mouthPress",
            Self::MouthX => "mouthX",
            Self::MouthRaiser => "mouthRaiser",
            Self::Nose => "nose",
            Self::NoseSneer => "noseSneer",
            Self::Neck => "neck",
            Self::TongueOut => "tongueOut",
            Self::TongueMove => "tongueMove",
            Self::TongueOther => "tongueOther",
            Self::HeadRotationYaw => "headRotationYaw",
            Self::HeadRotationPitch => "headRotationPitch",
            Self::HeadRotationRoll => "headRotationRoll",
            Self::HeadPositionX => "headPositionX",
            Self::HeadPositionY => "headPositionY",
            Self::HeadPositionZ => "headPositionZ",
        }
    }

    /// Human readable text for the UI.
    pub const fn label(self) -> &'static str {
        match self {
            Self::EyeBrowRaiser => "Eyebrow Raiser",
            Self::EyeBrowLower => "Eyebrow Lowerer",
            Self::EyeSquint => "Eye Squint",
            Self::EyeWide => "Eye Wide",
            Self::CheekPuffSuck => "Cheek Puff/Suck",
            Self::CheekSquint => "Cheek Squint",
            Self::JawOpen => "Jaw Open",
            Self::MouthClosed => "Mouth Closed",
            Self::JawX => "Jaw Sideways",
            Self::JawZ => "Jaw Forward/Backward",
            Self::LipFunnel => "Lip Funnel",
            Self::LipSuck => "Lip Suck",
            Self::LipPucker => "Lip Pucker",
            Self::MouthOpener => "Mouth Open",
            Self::MouthSmile => "Mouth Smile",
            Self::MouthFrown => "Mouth Frown",
            Self::MouthStretch => "Mouth Stretch",
            Self::MouthTightener => "Mouth Tightener",
            Self::MouthPress => "Mouth Press",
            Self::MouthX => "Mouth Sideways",
            Self::MouthRaiser => "Mouth Raiser",
            Self::Nose => "Nose",
            Self::NoseSneer => "Nose Sneer",
            Self::Neck => "Neck",
            Self::TongueOut => "Tongue Out",
            Self::TongueMove => "Tongue Directions",
            Self::TongueOther => "Tongue Other",
            Self::HeadRotationYaw => "Head Yaw",
            Self::HeadRotationPitch => "Head Pitch",
            Self::HeadRotationRoll => "Head Roll",
            Self::HeadPositionX => "Head Position X",
            Self::HeadPositionY => "Head Position Y",
            Self::HeadPositionZ => "Head Position Z",
        }
    }

    /// The group with this camelCase key.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.name() == name)
    }

    /// The expression shapes the group covers (empty for head groups).
    pub const fn shapes(self) -> &'static [Expression] {
        match self {
            Self::EyeBrowRaiser => &[E::BrowInnerUpRight, E::BrowInnerUpLeft, E::BrowOuterUpRight, E::BrowOuterUpLeft],
            Self::EyeBrowLower => &[E::BrowLowererRight, E::BrowLowererLeft, E::BrowPinchRight, E::BrowPinchLeft],
            Self::EyeSquint => &[E::EyeSquintRight, E::EyeSquintLeft],
            Self::EyeWide => &[E::EyeWideRight, E::EyeWideLeft],
            Self::CheekPuffSuck => &[E::CheekPuffRight, E::CheekPuffLeft, E::CheekSuckRight, E::CheekSuckLeft],
            Self::CheekSquint => &[E::CheekSquintRight, E::CheekSquintLeft],
            Self::JawOpen => &[E::JawOpen],
            Self::MouthClosed => &[E::MouthClosed],
            Self::JawX => &[E::JawRight, E::JawLeft],
            Self::JawZ => &[E::JawForward, E::JawBackward],
            Self::LipFunnel => {
                &[E::LipFunnelUpperRight, E::LipFunnelUpperLeft, E::LipFunnelLowerRight, E::LipFunnelLowerLeft]
            }
            Self::LipSuck => &[
                E::LipSuckUpperRight,
                E::LipSuckUpperLeft,
                E::LipSuckLowerRight,
                E::LipSuckLowerLeft,
                E::LipSuckCornerRight,
                E::LipSuckCornerLeft,
            ],
            Self::LipPucker => {
                &[E::LipPuckerUpperRight, E::LipPuckerUpperLeft, E::LipPuckerLowerRight, E::LipPuckerLowerLeft]
            }
            Self::MouthOpener => &[
                E::MouthUpperUpRight,
                E::MouthUpperUpLeft,
                E::MouthLowerDownRight,
                E::MouthLowerDownLeft,
                E::MouthUpperDeepenRight,
                E::MouthUpperDeepenLeft,
            ],
            Self::MouthSmile => &[
                E::MouthCornerPullRight,
                E::MouthCornerPullLeft,
                E::MouthCornerSlantRight,
                E::MouthCornerSlantLeft,
                E::MouthDimpleRight,
                E::MouthDimpleLeft,
            ],
            Self::MouthFrown => &[E::MouthFrownRight, E::MouthFrownLeft],
            Self::MouthStretch => &[E::MouthStretchRight, E::MouthStretchLeft],
            Self::MouthTightener => &[E::MouthTightenerRight, E::MouthTightenerLeft],
            Self::MouthPress => &[E::MouthPressRight, E::MouthPressLeft],
            Self::MouthX => &[E::MouthUpperRight, E::MouthUpperLeft, E::MouthLowerRight, E::MouthLowerLeft],
            Self::MouthRaiser => &[E::MouthRaiserUpper, E::MouthRaiserLower],
            Self::Nose => &[E::NasalDilationRight, E::NasalDilationLeft, E::NasalConstrictRight, E::NasalConstrictLeft],
            Self::NoseSneer => &[E::NoseSneerRight, E::NoseSneerLeft],
            Self::Neck => &[E::NeckFlexRight, E::NeckFlexLeft],
            Self::TongueOut => &[E::TongueOut],
            Self::TongueMove => &[E::TongueUp, E::TongueDown, E::TongueRight, E::TongueLeft],
            Self::TongueOther => &[
                E::TongueRoll,
                E::TongueBendDown,
                E::TongueCurlUp,
                E::TongueSquish,
                E::TongueFlat,
                E::TongueTwistRight,
                E::TongueTwistLeft,
            ],
            Self::HeadRotationYaw
            | Self::HeadRotationPitch
            | Self::HeadRotationRoll
            | Self::HeadPositionX
            | Self::HeadPositionY
            | Self::HeadPositionZ => &[],
        }
    }

    /// True for the six signed, unbounded head groups.
    pub const fn is_head(self) -> bool {
        matches!(
            self,
            Self::HeadRotationYaw
                | Self::HeadRotationPitch
                | Self::HeadRotationRoll
                | Self::HeadPositionX
                | Self::HeadPositionY
                | Self::HeadPositionZ
        )
    }

    /// The head field a head group adjusts.
    pub(crate) fn head_field(self, head: &mut Head) -> Option<&mut f32> {
        match self {
            Self::HeadRotationYaw => Some(&mut head.yaw),
            Self::HeadRotationPitch => Some(&mut head.pitch),
            Self::HeadRotationRoll => Some(&mut head.roll),
            Self::HeadPositionX => Some(&mut head.pos_x),
            Self::HeadPositionY => Some(&mut head.pos_y),
            Self::HeadPositionZ => Some(&mut head.pos_z),
            _ => None,
        }
    }
}

impl Serialize for AdjustGroup {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for AdjustGroup {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = AdjustGroup;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an adjustment group name such as \"jawOpen\"")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<AdjustGroup, E> {
                AdjustGroup::from_name(v).ok_or_else(|| E::unknown_variant(v, &[]))
            }
        }
        d.deserialize_str(V)
    }
}

/// Per-shape lookup of the active (non-identity) ranges, compiled from the config.
pub(crate) struct ShapeRanges {
    /// `(min, max)` per shape; `None` = leave alone.
    pub shapes: [Option<(f32, f32)>; SHAPE_COUNT],
    /// Head groups with a non-identity range.
    pub head: Vec<(AdjustGroup, f32, f32)>,
}

impl ShapeRanges {
    pub fn compile(ranges: &std::collections::BTreeMap<AdjustGroup, crate::Range>) -> Self {
        let mut out = Self { shapes: [None; SHAPE_COUNT], head: Vec::new() };
        for (&group, r) in ranges {
            if r.is_identity() {
                continue;
            }
            if group.is_head() {
                out.head.push((group, r.min, r.max));
            }
            for shape in group.shapes() {
                out.shapes[shape.index()] = Some((r.min, r.max));
            }
        }
        out
    }

    pub fn apply(&self, shapes: &mut [f32; SHAPE_COUNT], head: &mut Head) {
        for (v, range) in shapes.iter_mut().zip(&self.shapes) {
            if let Some((min, max)) = *range {
                *v = ((*v - min) / (max - min)).clamp(0.0, 1.0);
            }
        }
        for &(group, min, max) in &self.head {
            if let Some(v) = group.head_field(head) {
                *v = v.signum() * (v.abs() - min).max(0.0) / (max - min);
                if *v == 0.0 {
                    *v = 0.0;
                }
            }
        }
    }
}
