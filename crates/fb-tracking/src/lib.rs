//! Unified tracking data model shared by every FaceBridge crate.
//!
//! Mirrors the data a VRCFT module writes into `UnifiedTracking.Data` (see `spec/module-api.md`): two eyes,
//! [`SHAPE_COUNT`] expression weights indexed by [`Expression`], and head pose.

macro_rules! expressions {
    ($($name:ident = $index:literal,)*) => {
        /// Unified Expressions shape index. Numeric values equal VRCFT's `UnifiedExpressions` enum values.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum Expression {
            $($name = $index,)*
        }

        impl Expression {
            /// Every shape, in index order (`ALL[i] as usize == i`).
            pub const ALL: [Expression; SHAPE_COUNT] = [$(Expression::$name,)*];

            /// Shape name as used in VRCFT (`"JawOpen"`), which is also the base of its OSC parameter names.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Expression::$name => stringify!($name),)*
                }
            }
        }
    };
}

/// Number of Unified Expressions shapes (`UnifiedExpressions.Max`).
pub const SHAPE_COUNT: usize = 88;

expressions! {
    EyeSquintRight = 0,
    EyeSquintLeft = 1,
    EyeWideRight = 2,
    EyeWideLeft = 3,
    BrowPinchRight = 4,
    BrowPinchLeft = 5,
    BrowLowererRight = 6,
    BrowLowererLeft = 7,
    BrowInnerUpRight = 8,
    BrowInnerUpLeft = 9,
    BrowOuterUpRight = 10,
    BrowOuterUpLeft = 11,
    NasalDilationRight = 12,
    NasalDilationLeft = 13,
    NasalConstrictRight = 14,
    NasalConstrictLeft = 15,
    CheekSquintRight = 16,
    CheekSquintLeft = 17,
    CheekPuffRight = 18,
    CheekPuffLeft = 19,
    CheekSuckRight = 20,
    CheekSuckLeft = 21,
    JawOpen = 22,
    JawRight = 23,
    JawLeft = 24,
    JawForward = 25,
    JawBackward = 26,
    JawClench = 27,
    JawMandibleRaise = 28,
    MouthClosed = 29,
    LipSuckUpperRight = 30,
    LipSuckUpperLeft = 31,
    LipSuckLowerRight = 32,
    LipSuckLowerLeft = 33,
    LipSuckCornerRight = 34,
    LipSuckCornerLeft = 35,
    LipFunnelUpperRight = 36,
    LipFunnelUpperLeft = 37,
    LipFunnelLowerRight = 38,
    LipFunnelLowerLeft = 39,
    LipPuckerUpperRight = 40,
    LipPuckerUpperLeft = 41,
    LipPuckerLowerRight = 42,
    LipPuckerLowerLeft = 43,
    MouthUpperUpRight = 44,
    MouthUpperUpLeft = 45,
    MouthUpperDeepenRight = 46,
    MouthUpperDeepenLeft = 47,
    NoseSneerRight = 48,
    NoseSneerLeft = 49,
    MouthLowerDownRight = 50,
    MouthLowerDownLeft = 51,
    MouthUpperRight = 52,
    MouthUpperLeft = 53,
    MouthLowerRight = 54,
    MouthLowerLeft = 55,
    MouthCornerPullRight = 56,
    MouthCornerPullLeft = 57,
    MouthCornerSlantRight = 58,
    MouthCornerSlantLeft = 59,
    MouthFrownRight = 60,
    MouthFrownLeft = 61,
    MouthStretchRight = 62,
    MouthStretchLeft = 63,
    MouthDimpleRight = 64,
    MouthDimpleLeft = 65,
    MouthRaiserUpper = 66,
    MouthRaiserLower = 67,
    MouthPressRight = 68,
    MouthPressLeft = 69,
    MouthTightenerRight = 70,
    MouthTightenerLeft = 71,
    TongueOut = 72,
    TongueUp = 73,
    TongueDown = 74,
    TongueRight = 75,
    TongueLeft = 76,
    TongueRoll = 77,
    TongueBendDown = 78,
    TongueCurlUp = 79,
    TongueSquish = 80,
    TongueFlat = 81,
    TongueTwistRight = 82,
    TongueTwistLeft = 83,
    SoftPalateClose = 84,
    ThroatSwallow = 85,
    NeckFlexRight = 86,
    NeckFlexLeft = 87,
}

impl Expression {
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.name() == name)
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// One eye. Gaze is a normalized direction (x right, y up); openness 0..1; pupil diameter in millimetres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SingleEye {
    pub gaze_x: f32,
    pub gaze_y: f32,
    pub openness: f32,
    pub pupil_diameter_mm: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Eyes {
    pub left: SingleEye,
    pub right: SingleEye,
    /// Largest pupil diameter seen (mm); with `min_dilation` normalizes pupil dilation.
    pub max_dilation: f32,
    /// Smallest pupil diameter seen (mm).
    pub min_dilation: f32,
    pub left_diameter: f32,
    pub right_diameter: f32,
}

/// Head rotation (yaw/pitch/roll) and position as written by the module.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Head {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
}

/// Complete tracking state: what a module reports per frame, and what the host merges and sends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackingFrame {
    pub eye: Eyes,
    pub shapes: [f32; SHAPE_COUNT],
    pub head: Head,
}

impl Default for TrackingFrame {
    fn default() -> Self {
        Self { eye: Eyes::default(), shapes: [0.0; SHAPE_COUNT], head: Head::default() }
    }
}

impl TrackingFrame {
    pub fn shape(&self, expression: Expression) -> f32 {
        self.shapes[expression.index()]
    }

    pub fn set_shape(&mut self, expression: Expression, weight: f32) {
        self.shapes[expression.index()] = weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_are_dense_and_names_round_trip() {
        for (i, e) in Expression::ALL.into_iter().enumerate() {
            assert_eq!(e.index(), i);
            assert_eq!(Expression::from_name(e.name()), Some(e));
        }
        assert_eq!(Expression::from_index(SHAPE_COUNT), None);
        assert_eq!(Expression::JawOpen.index(), 22);
        assert_eq!(Expression::NeckFlexLeft.index(), 87);
    }
}
