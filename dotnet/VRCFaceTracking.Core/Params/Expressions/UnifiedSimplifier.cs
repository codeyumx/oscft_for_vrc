using System;
using System.Collections.Generic;
using VRCFaceTracking.Core.Params.Data;

namespace VRCFaceTracking.Core.Params.Expressions;

/// <summary>FaceBridge's own reduction of the unified shapes to a handful of simple expressions (all 0..1).</summary>
public static class UnifiedSimplifier
{
    public static Dictionary<UnifiedSimpleExpressions, Func<UnifiedTrackingData, float>> ExpressionMap = new()
    {
        [UnifiedSimpleExpressions.BrowUpRight] = d => Mean(d, UnifiedExpressions.BrowInnerUpRight, UnifiedExpressions.BrowOuterUpRight),
        [UnifiedSimpleExpressions.BrowUpLeft] = d => Mean(d, UnifiedExpressions.BrowInnerUpLeft, UnifiedExpressions.BrowOuterUpLeft),
        [UnifiedSimpleExpressions.BrowDownRight] = d => Max(d, UnifiedExpressions.BrowLowererRight, UnifiedExpressions.BrowPinchRight),
        [UnifiedSimpleExpressions.BrowDownLeft] = d => Max(d, UnifiedExpressions.BrowLowererLeft, UnifiedExpressions.BrowPinchLeft),
        [UnifiedSimpleExpressions.MouthSmileRight] = d => Max(d, UnifiedExpressions.MouthCornerPullRight, UnifiedExpressions.MouthCornerSlantRight),
        [UnifiedSimpleExpressions.MouthSmileLeft] = d => Max(d, UnifiedExpressions.MouthCornerPullLeft, UnifiedExpressions.MouthCornerSlantLeft),
        [UnifiedSimpleExpressions.MouthSadRight] = d => Weight(d, UnifiedExpressions.MouthFrownRight),
        [UnifiedSimpleExpressions.MouthSadLeft] = d => Weight(d, UnifiedExpressions.MouthFrownLeft),
    };

    private static float Weight(UnifiedTrackingData d, UnifiedExpressions e) => Math.Clamp(d.Shapes[(int)e].Weight, 0f, 1f);

    private static float Mean(UnifiedTrackingData d, UnifiedExpressions a, UnifiedExpressions b) =>
        (Weight(d, a) + Weight(d, b)) * 0.5f;

    private static float Max(UnifiedTrackingData d, UnifiedExpressions a, UnifiedExpressions b) =>
        Math.Max(Weight(d, a), Weight(d, b));
}
