//! Claim arbitration and frame merging. Pure data structures: no processes, no I/O.

use fb_osc::Claims;
use fb_protocol::FrameFlags;
use fb_tracking::{Expression, TrackingFrame};

/// `EyeSquintRight/Left` and `EyeWideRight/Left` (indices `0..EYE_SHAPES`) belong to the eye claimer.
const EYE_SHAPES: usize = Expression::EyeWideLeft.index() + 1;

/// Identifies one module host process (unique per spawn, so late frames of a dead run are ignored).
pub type RunKey = u64;

struct Slot {
    key: RunKey,
    frame: TrackingFrame,
    head_valid: bool,
}

/// Which module owns eye / expression, plus the latest frame of each owner.
#[derive(Default)]
pub struct Merger {
    eye_owner: Option<RunKey>,
    expression_owner: Option<RunKey>,
    slots: Vec<Slot>,
}

impl Merger {
    /// Parts nobody has claimed yet.
    pub fn unclaimed(&self) -> Claims {
        Claims { eye: self.eye_owner.is_none(), expression: self.expression_owner.is_none() }
    }

    /// What to offer a module with the given `supports`: everything still unclaimed, or `None` when the module
    /// supports nothing that is still unclaimed (the module is skipped).
    pub fn offer(&self, supports: Claims) -> Option<Claims> {
        let free = self.unclaimed();
        ((supports.eye && free.eye) || (supports.expression && free.expression)).then_some(free)
    }

    /// Records what `key` claimed in `Init`. Claims for parts that were not offered (or are taken) are ignored.
    /// Returns the claims actually granted.
    pub fn claim(&mut self, key: RunKey, offered: Claims, wanted: Claims) -> Claims {
        let free = self.unclaimed();
        let granted = Claims {
            eye: wanted.eye && offered.eye && free.eye,
            expression: wanted.expression && offered.expression && free.expression,
        };
        if granted.eye {
            self.eye_owner = Some(key);
        }
        if granted.expression {
            self.expression_owner = Some(key);
        }
        if (granted.eye || granted.expression) && !self.slots.iter().any(|s| s.key == key) {
            self.slots.push(Slot { key, frame: TrackingFrame::default(), head_valid: false });
        }
        granted
    }

    /// Stores a frame from `key`. Returns the new merged frame, or `None` if `key` holds no claim.
    pub fn update(
        &mut self,
        key: RunKey,
        flags: FrameFlags,
        frame: &TrackingFrame,
    ) -> Option<(TrackingFrame, Claims)> {
        let slot = self.slots.iter_mut().find(|s| s.key == key)?;
        slot.frame = *frame;
        slot.head_valid = flags.contains(FrameFlags::HEAD);
        Some(self.merged())
    }

    /// Drops `key`'s claims and its last frame data. Returns the new merged frame if `key` held a claim.
    pub fn release(&mut self, key: RunKey) -> Option<(TrackingFrame, Claims)> {
        let before = self.slots.len();
        self.slots.retain(|s| s.key != key);
        if self.slots.len() == before {
            return None;
        }
        if self.eye_owner == Some(key) {
            self.eye_owner = None;
        }
        if self.expression_owner == Some(key) {
            self.expression_owner = None;
        }
        Some(self.merged())
    }

    /// Drops `key`'s claim on the given parts only. When that leaves `key` with no claim at all, its slot (and
    /// last frame) goes too. Returns the new merged frame if `key` held any of those parts.
    pub fn release_parts(&mut self, key: RunKey, parts: Claims) -> Option<(TrackingFrame, Claims)> {
        let mut changed = false;
        if parts.eye && self.eye_owner == Some(key) {
            self.eye_owner = None;
            changed = true;
        }
        if parts.expression && self.expression_owner == Some(key) {
            self.expression_owner = None;
            changed = true;
        }
        if !changed {
            return None;
        }
        if self.eye_owner != Some(key) && self.expression_owner != Some(key) {
            self.slots.retain(|s| s.key != key);
        }
        Some(self.merged())
    }

    /// Releases everything.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Eye fields and the eye-region shapes (`EyeSquint*`, `EyeWide*`, indices `0..EYE_SHAPES`) from the eye owner
    /// when there is one; all other shapes, and the eye-region shapes without an eye owner, from the expression
    /// owner. Head from whichever of those frames has its head flag set (expression owner first). Unowned parts
    /// are zero.
    pub fn merged(&self) -> (TrackingFrame, Claims) {
        let find = |owner: Option<RunKey>| owner.and_then(|k| self.slots.iter().find(|s| s.key == k));
        let eye = find(self.eye_owner);
        let expression = find(self.expression_owner);
        let mut out = TrackingFrame::default();
        if let Some(s) = expression {
            out.shapes = s.frame.shapes;
        }
        if let Some(s) = eye {
            out.eye = s.frame.eye;
            out.shapes[..EYE_SHAPES].copy_from_slice(&s.frame.shapes[..EYE_SHAPES]);
        }
        if let Some(s) = [expression, eye].into_iter().flatten().find(|s| s.head_valid) {
            out.head = s.frame.head;
        }
        (out, Claims { eye: self.eye_owner.is_some(), expression: self.expression_owner.is_some() })
    }
}

/// Startup data check of one module: which of the watched parts has shown changing data, i.e. two frames that
/// differ in a field of that part. The eye part is the eye fields plus the eye-region shapes (`EyeSquint*`,
/// `EyeWide*`), the expression part is every other shape; head data does not count.
pub struct LiveCheck {
    watch: Claims,
    live: Claims,
    last: Option<TrackingFrame>,
}

impl LiveCheck {
    pub fn new(watch: Claims) -> Self {
        Self { watch, live: Claims::default(), last: None }
    }

    /// Compares `frame` with the previous one.
    pub fn feed(&mut self, frame: &TrackingFrame) {
        if let Some(prev) = &self.last {
            self.live.eye |= self.watch.eye && (prev.eye != frame.eye || prev.shapes[..EYE_SHAPES] != frame.shapes[..EYE_SHAPES]);
            self.live.expression |= self.watch.expression && prev.shapes[EYE_SHAPES..] != frame.shapes[EYE_SHAPES..];
        }
        self.last = Some(*frame);
    }

    /// Watched parts that have not shown live data yet.
    pub fn pending(&self) -> Claims {
        Claims { eye: self.watch.eye && !self.live.eye, expression: self.watch.expression && !self.live.expression }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fb_tracking::SingleEye;

    const EYE: Claims = Claims { eye: true, expression: false };
    const EXPR: Claims = Claims { eye: false, expression: true };
    const BOTH: Claims = Claims { eye: true, expression: true };
    const NONE: Claims = Claims { eye: false, expression: false };

    fn eye_frame(gaze: f32) -> TrackingFrame {
        let mut f = TrackingFrame::default();
        f.eye.left = SingleEye { gaze_x: gaze, ..SingleEye::default() };
        f.set_shape(Expression::JawOpen, 0.9); // present but must be ignored for an eye-only claimer
        f.head.yaw = 11.0;
        f
    }

    fn expr_frame(jaw: f32) -> TrackingFrame {
        let mut f = TrackingFrame::default();
        f.set_shape(Expression::JawOpen, jaw);
        f.eye.left.gaze_x = 0.77; // must be ignored for an expression-only claimer
        f.head.yaw = 22.0;
        f
    }

    #[test]
    fn offer_is_everything_unclaimed_and_skips_modules_with_nothing_to_claim() {
        let mut m = Merger::default();
        assert_eq!(m.offer(EYE), Some(BOTH));
        assert_eq!(m.claim(1, BOTH, EYE), EYE);
        // Eye is taken: an eye-only module is skipped, a module supporting expression is offered the rest.
        assert_eq!(m.offer(EYE), None);
        assert_eq!(m.offer(BOTH), Some(EXPR));
        assert_eq!(m.offer(NONE), None);
        assert_eq!(m.claim(2, EXPR, BOTH), EXPR, "claiming what was not offered is ignored");
        assert_eq!(m.offer(BOTH), None);
        assert_eq!(m.unclaimed(), NONE);
    }

    #[test]
    fn taken_parts_cannot_be_claimed_again() {
        let mut m = Merger::default();
        m.claim(1, BOTH, EYE);
        assert_eq!(m.claim(2, BOTH, BOTH), EXPR);
    }

    #[test]
    fn merges_eye_and_expression_from_their_owners_only() {
        let mut m = Merger::default();
        m.claim(1, BOTH, EYE);
        m.claim(2, EXPR, EXPR);
        let (merged, claims) = m.update(1, FrameFlags::EYE, &eye_frame(0.3)).unwrap();
        assert_eq!(claims, BOTH);
        assert_eq!(merged.eye.left.gaze_x, 0.3);
        assert_eq!(merged.shape(Expression::JawOpen), 0.0);
        let (merged, _) = m.update(2, FrameFlags::EXPRESSION, &expr_frame(0.6)).unwrap();
        assert_eq!(merged.eye.left.gaze_x, 0.3, "expression owner does not overwrite eye data");
        assert_eq!(merged.shape(Expression::JawOpen), 0.6);
        assert_eq!(merged.head.yaw, 0.0, "no head flag, no head data");
    }

    #[test]
    fn head_comes_from_a_frame_with_the_head_flag_expression_owner_first() {
        let mut m = Merger::default();
        m.claim(1, BOTH, EYE);
        m.claim(2, EXPR, EXPR);
        m.update(1, FrameFlags::EYE | FrameFlags::HEAD, &eye_frame(0.0));
        assert_eq!(m.merged().0.head.yaw, 11.0);
        let (merged, _) = m.update(2, FrameFlags::EXPRESSION | FrameFlags::HEAD, &expr_frame(0.0)).unwrap();
        assert_eq!(merged.head.yaw, 22.0);
        let (merged, _) = m.update(2, FrameFlags::EXPRESSION, &expr_frame(0.0)).unwrap();
        assert_eq!(merged.head.yaw, 11.0, "falls back to the eye owner when the expression owner has none");
    }

    #[test]
    fn eye_region_shapes_come_from_the_eye_owner_and_the_rest_from_the_expression_owner() {
        let mut m = Merger::default();
        m.claim(1, BOTH, EYE);
        m.claim(2, EXPR, EXPR);
        let mut eye = eye_frame(0.3);
        eye.set_shape(Expression::EyeSquintRight, 0.1);
        eye.set_shape(Expression::EyeSquintLeft, 0.2);
        eye.set_shape(Expression::EyeWideRight, 0.3);
        eye.set_shape(Expression::EyeWideLeft, 0.4);
        let mut expr = expr_frame(0.6);
        for e in [Expression::EyeSquintRight, Expression::EyeSquintLeft, Expression::EyeWideRight, Expression::EyeWideLeft] {
            expr.set_shape(e, 0.99); // must lose against the eye owner
        }
        m.update(2, FrameFlags::EXPRESSION, &expr);
        let (merged, _) = m.update(1, FrameFlags::EYE, &eye).unwrap();
        assert_eq!(
            [
                merged.shape(Expression::EyeSquintRight),
                merged.shape(Expression::EyeSquintLeft),
                merged.shape(Expression::EyeWideRight),
                merged.shape(Expression::EyeWideLeft),
            ],
            [0.1, 0.2, 0.3, 0.4]
        );
        assert_eq!(merged.shape(Expression::JawOpen), 0.6, "other shapes stay with the expression owner");
    }

    #[test]
    fn eye_owner_alone_provides_only_the_eye_region_shapes() {
        let mut m = Merger::default();
        m.claim(1, BOTH, EYE);
        let mut eye = eye_frame(0.3);
        eye.set_shape(Expression::EyeWideLeft, 0.4);
        let (merged, claims) = m.update(1, FrameFlags::EYE, &eye).unwrap();
        assert_eq!(claims, EYE);
        assert_eq!(merged.shape(Expression::EyeWideLeft), 0.4);
        assert_eq!(merged.shape(Expression::JawOpen), 0.0, "non-eye shapes of an eye-only module are ignored");
        assert_eq!(merged.shapes.iter().filter(|s| **s != 0.0).count(), 1);
    }

    #[test]
    fn expression_owner_alone_keeps_its_eye_region_shapes() {
        let mut m = Merger::default();
        m.claim(2, EXPR, EXPR);
        let mut expr = expr_frame(0.6);
        expr.set_shape(Expression::EyeSquintLeft, 0.5);
        let (merged, _) = m.update(2, FrameFlags::EXPRESSION, &expr).unwrap();
        assert_eq!(merged.shape(Expression::EyeSquintLeft), 0.5);
    }

    #[test]
    fn single_module_claiming_both_uses_one_frame() {
        let mut m = Merger::default();
        assert_eq!(m.claim(5, BOTH, BOTH), BOTH);
        let mut f = expr_frame(0.4);
        f.eye.right.openness = 0.5;
        let (merged, claims) = m.update(5, FrameFlags::EYE | FrameFlags::EXPRESSION | FrameFlags::HEAD, &f).unwrap();
        assert_eq!(claims, BOTH);
        assert_eq!(merged, f);
    }

    #[test]
    fn frames_from_non_claimers_are_ignored() {
        let mut m = Merger::default();
        m.claim(1, BOTH, EYE);
        assert!(m.update(9, FrameFlags::EYE, &eye_frame(0.5)).is_none());
        assert_eq!(m.merged().0, TrackingFrame::default());
    }

    #[test]
    fn release_zeroes_that_modules_data_and_frees_the_claim_without_reassigning() {
        let mut m = Merger::default();
        m.claim(1, BOTH, EYE);
        m.claim(2, EXPR, EXPR);
        m.update(1, FrameFlags::EYE, &eye_frame(0.3));
        m.update(2, FrameFlags::EXPRESSION, &expr_frame(0.6));
        let (merged, claims) = m.release(1).unwrap();
        assert_eq!(claims, EXPR);
        assert_eq!(merged.eye.left.gaze_x, 0.0);
        assert_eq!(merged.shape(Expression::JawOpen), 0.6);
        // The expression owner is untouched and the freed eye claim is open again (but nobody is auto-assigned).
        assert_eq!(m.unclaimed(), EYE);
        assert!(m.release(1).is_none());
        let (merged, claims) = m.release(2).unwrap();
        assert_eq!(claims, NONE);
        assert_eq!(merged, TrackingFrame::default());
    }

    #[test]
    fn releasing_one_part_keeps_the_other_and_the_slot_until_nothing_is_left() {
        let mut m = Merger::default();
        m.claim(1, BOTH, BOTH);
        let mut f = expr_frame(0.4);
        f.eye.left.gaze_x = 0.3;
        m.update(1, FrameFlags::EYE | FrameFlags::EXPRESSION, &f);
        let (merged, claims) = m.release_parts(1, EYE).unwrap();
        assert_eq!(claims, EXPR);
        assert_eq!(merged.eye.left.gaze_x, 0.0);
        assert_eq!(merged.shape(Expression::JawOpen), 0.4, "the kept part still flows");
        assert_eq!(m.unclaimed(), EYE);
        assert!(m.release_parts(1, EYE).is_none(), "already released");
        assert!(m.release_parts(2, EXPR).is_none(), "not the owner");
        assert!(m.update(1, FrameFlags::EXPRESSION, &f).is_some(), "frames for the kept part are still accepted");
        let (merged, claims) = m.release_parts(1, EXPR).unwrap();
        assert_eq!((claims, merged), (NONE, TrackingFrame::default()));
        assert!(m.update(1, FrameFlags::EXPRESSION, &f).is_none(), "no claim left, no slot");
        assert_eq!(m.claim(2, BOTH, BOTH), BOTH);
    }

    #[test]
    fn live_check_needs_two_differing_frames_per_part_and_ignores_head() {
        let mut c = LiveCheck::new(BOTH);
        assert_eq!(c.pending(), BOTH);
        c.feed(&eye_frame(0.3));
        assert_eq!(c.pending(), BOTH, "one frame proves nothing");
        c.feed(&eye_frame(0.3));
        assert_eq!(c.pending(), BOTH, "identical frames");
        let mut moved_head = eye_frame(0.3);
        moved_head.head.yaw = 50.0;
        c.feed(&moved_head);
        assert_eq!(c.pending(), BOTH, "head data does not count");
        c.feed(&eye_frame(0.4));
        assert_eq!(c.pending(), EXPR, "eye gaze changed");
        let mut f = eye_frame(0.4);
        f.set_shape(Expression::JawOpen, 0.1);
        c.feed(&f);
        assert_eq!(c.pending(), NONE);
    }

    #[test]
    fn live_check_assigns_eye_region_shapes_to_the_eye_and_ignores_unwatched_parts() {
        let mut c = LiveCheck::new(BOTH);
        let mut a = TrackingFrame::default();
        let mut b = a;
        b.set_shape(Expression::EyeWideLeft, 0.5);
        c.feed(&a);
        c.feed(&b);
        assert_eq!(c.pending(), EXPR, "EyeWide* belongs to the eye part");
        a.set_shape(Expression::JawOpen, 0.2);
        let mut only_eye = LiveCheck::new(EYE);
        only_eye.feed(&TrackingFrame::default());
        only_eye.feed(&a);
        assert_eq!(only_eye.pending(), EYE, "expression changes do not make the eye live");
    }
}
