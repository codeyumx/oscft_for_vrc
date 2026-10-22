# Tracking mutation: calibration, correctors, parameter adjustment, data filter

FaceBridge post-processes the merged tracking frame before it goes to OSC. Four stages, applied in this order to
every merged frame (`Inner::submit` in fb-core):

1. **Calibration**: learns how far the user actually moves each shape and rescales it to the full 0..1 range.
2. **Unified correctors**: fixes physically impossible combinations.
3. **Parameter adjustment**: user-set input range per parameter group (dead zone and gain).
4. **Data filter**: One Euro smoothing.

Calibration and correctors produce normalized values. Adjustment is the user's final say on ranges. Filtering
comes last so that it smooths everything the earlier stages do.

There is a master switch, and each stage has its own switch. With the master switch off, or with every stage off
(or at identity settings), the output equals the input bit for bit.

All shape values are clamped to 0..1 after each stage. Eye openness is clamped to 0..1. Gaze, pupil and head are
not clamped.

The algorithms here are FaceBridge's own. They are not derived from any other implementation.

## Crate `fb-mutate`

The crate is pure: no I/O, no threads, no clocks. Time is passed in.

```rust
pub struct Mutator { /* config, calibration state, filter state */ }

impl Mutator {
    pub fn new(config: MutationConfig, calibration: CalibrationData) -> Self;
    pub fn config(&self) -> &MutationConfig;
    /// Validates (clamps out-of-range numbers, fixes min >= max) and applies; resets filter state for stages that
    /// were switched off. Returns the sanitized config.
    pub fn set_config(&mut self, config: MutationConfig) -> MutationConfig;
    pub fn calibration(&self) -> &CalibrationData;
    /// `None` resets every shape.
    pub fn reset_calibration(&mut self, shape: Option<Expression>);
    /// Mutates `frame` in place. `now_s` is a monotonic time in seconds (used for filter dt and calibration
    /// timing). The first call, or a gap > 0.5 s since the last call, re-seeds the filters with the input.
    pub fn apply(&mut self, frame: &mut TrackingFrame, now_s: f64);
    /// True once calibration data changed since the last call (the caller persists it).
    pub fn take_calibration_dirty(&mut self) -> bool;
}
```

`MutationConfig` and `CalibrationData` are `Serialize + Deserialize + Clone + Debug + PartialEq + Default`. Field
names serialize as camelCase, and missing fields take their defaults (`#[serde(default)]`).

### Config (`config.json` → `"mutation"`)

```json
{
  "enabled": true,
  "calibration": { "enabled": false, "blend": 1.0, "continuous": true },
  "correctors": {
    "enabled": true,
    "mouthClosedFix": true,
    "lipSuckFix": true,
    "eyeLidBlend": 0.0,
    "eyeLookSymmetrize": false
  },
  "adjustment": {
    "enabled": true,
    "ranges": { "jawOpen": { "min": 0.0, "max": 1.0 } }
  },
  "filter": {
    "enabled": false,
    "eyeGaze":     { "enabled": true, "minCutoff": 1.0, "beta": 0.5,  "derivativeCutoff": 1.0 },
    "eyeOpenness": { "enabled": true, "minCutoff": 2.0, "beta": 2.0,  "derivativeCutoff": 1.0 },
    "pupil":       { "enabled": true, "minCutoff": 0.5, "beta": 0.1,  "derivativeCutoff": 1.0 },
    "expressions": { "enabled": true, "minCutoff": 2.0, "beta": 1.0,  "derivativeCutoff": 1.0 },
    "head":        { "enabled": true, "minCutoff": 1.0, "beta": 0.05, "derivativeCutoff": 1.0 }
  }
}
```

The JSON above shows the defaults. `ranges` may omit groups; a missing group means `{min: 0, max: 1}`, which is
the identity. Unknown group keys are ignored.

Sanitizing:

| Setting | Rule |
|---|---|
| `blend` | 0..1 |
| `eyeLidBlend` | 0..1 |
| Range `min`, `max` | 0..1, with `max >= min + 0.01`; if not, `max = min(min + 0.01, 1)`, then `min = max - 0.01` |
| `minCutoff` | 0.01..30 Hz |
| `beta` | 0..10 |
| `derivativeCutoff` | 0.01..30 Hz |

Non-finite numbers revert to the default.

### 1. Calibration

Per shape `i`, the stored state is `peak` (0..1) and `progress` (0..1). The runtime-only state is `hold`.

Learning happens only while `calibration.enabled && continuous`:

- `hold` is an exponential moving average of the input with τ = 50 ms: `hold += (v - hold) * (1 - exp(-dt/0.05))`.
  This ignores single-frame spikes.
- When `hold > peak`, set `peak = hold`.
- While `hold >= 0.5 * peak` and `hold >= 0.1`, `progress += dt / 5.0`, capped at 1. In other words, progress
  reaches full after about 5 s of actually using the shape at no less than half its peak. A peak set by a stray
  spike that is never reached again leaves progress low, so it has little effect.
- Any change to `peak` or `progress` sets the dirty flag.

Applying happens while `calibration.enabled`, whether or not `continuous` is on:

- `effective_peak = max(peak, 0.2)`. Amplification is limited to 5×, so noise is not blown up.
- `calibrated = clamp(v / effective_peak, 0, 1)`.
- `out = v + (calibrated - v) * blend * progress`.

`CalibrationData` serializes as follows:

```json
{ "shapes": [ { "name": "EyeWideRight", "peak": 0.99, "progress": 1.0 } ] }
```

There is one entry per shape, by `Expression` name. Unknown names are ignored, and missing shapes start at
`peak 0, progress 0`.

### 2. Unified correctors

Only active while `correctors.enabled`.

- **`mouthClosedFix`**: `MouthClosed = min(MouthClosed, JawOpen)`. Lips cannot be pressed shut further than the
  jaw is open.
- **`lipSuckFix`**: a lip cannot be sucked in while the same side is raised or lowered.
  - `LipSuckUpper{R,L} *= 1 - MouthUpperUp{R,L}`
  - `LipSuckLower{R,L} *= 1 - MouthLowerDown{R,L}`
- **`eyeLidBlend`** `b`: `avg = (left.openness + right.openness) / 2`. Each eye's openness becomes
  `lerp(own, avg, b)`. At 1 the eyelids are fully linked.
- **`eyeLookSymmetrize`**: both eyes get the mean vertical gaze (`gaze_y = mean`). Horizontal gaze is left alone,
  so vergence survives.

### 3. Parameter adjustment

Only active while `adjustment.enabled`.

For shapes, each group's `{min, max}` maps the input range `[min, max]` to `[0, 1]`:
`out = clamp((v - min) / (max - min), 0, 1)`. `min` acts as a dead zone; `max < 1` acts as a gain.

For head groups, which are signed and unbounded: `out = sign(v) * max(|v| - min, 0) / (max - min)`, unclamped.

Each group serializes under its camelCase name:

| Group | Shapes / fields |
|---|---|
| `eyeBrowRaiser` | BrowInnerUp R/L, BrowOuterUp R/L |
| `eyeBrowLower` | BrowLowerer R/L, BrowPinch R/L |
| `eyeSquint` | EyeSquint R/L |
| `eyeWide` | EyeWide R/L |
| `cheekPuffSuck` | CheekPuff R/L, CheekSuck R/L |
| `cheekSquint` | CheekSquint R/L |
| `jawOpen` | JawOpen |
| `mouthClosed` | MouthClosed |
| `jawX` | JawRight, JawLeft |
| `jawZ` | JawForward, JawBackward |
| `lipFunnel` | LipFunnel Upper/Lower R/L |
| `lipSuck` | LipSuck Upper/Lower/Corner R/L |
| `lipPucker` | LipPucker Upper/Lower R/L |
| `mouthOpener` | MouthUpperUp R/L, MouthLowerDown R/L, MouthUpperDeepen R/L |
| `mouthSmile` | MouthCornerPull R/L, MouthCornerSlant R/L, MouthDimple R/L |
| `mouthFrown` | MouthFrown R/L |
| `mouthStretch` | MouthStretch R/L |
| `mouthTightener` | MouthTightener R/L |
| `mouthPress` | MouthPress R/L |
| `mouthX` | MouthUpperRight/Left, MouthLowerRight/Left |
| `mouthRaiser` | MouthRaiserUpper, MouthRaiserLower |
| `nose` | NasalDilation R/L, NasalConstrict R/L |
| `noseSneer` | NoseSneer R/L |
| `neck` | NeckFlex R/L |
| `tongueOut` | TongueOut |
| `tongueMove` | TongueUp, TongueDown, TongueRight, TongueLeft |
| `tongueOther` | TongueRoll, TongueBendDown, TongueCurlUp, TongueSquish, TongueFlat, TongueTwistRight/Left |
| `headRotationYaw` / `headRotationPitch` / `headRotationRoll` | head.yaw / pitch / roll |
| `headPositionX` / `headPositionY` / `headPositionZ` | head.pos_x / pos_y / pos_z |

These shapes belong to no group and are left alone: JawClench, JawMandibleRaise, SoftPalateClose, ThroatSwallow.

`AdjustGroup` is a public enum with `ALL`, `name()` (the camelCase key), `label()` (UI text) and `shapes()`.
Labels: Eyebrow Raiser, Eyebrow Lowerer, Eye Squint, Eye Wide, Cheek Puff/Suck, Cheek Squint, Jaw Open,
Mouth Closed, Jaw Sideways, Jaw Forward/Backward, Lip Funnel, Lip Suck, Lip Pucker, Mouth Open, Mouth Smile,
Mouth Frown, Mouth Stretch, Mouth Tightener, Mouth Press, Mouth Sideways, Mouth Raiser, Nose, Nose Sneer, Neck,
Tongue Out, Tongue Directions, Tongue Other, Head Yaw, Head Pitch, Head Roll, Head Position X/Y/Z.

### 4. Data filter

Only active while `filter.enabled`.

This is a One Euro filter, one per channel. The channels per category are:

| Category | Channels |
|---|---|
| `eyeGaze` | left/right gaze x/y |
| `eyeOpenness` | left/right openness |
| `pupil` | left/right `pupil_diameter_mm`, `left_diameter`, `right_diameter` |
| `expressions` | the 88 shapes |
| `head` | 6 channels |

A category that is disabled passes its channels through, and its filter state is reset.

The One Euro filter, with `dt` taken from successive `now_s` values:

- The derivative estimate is `dx = (x - x_prev) / dt`, smoothed with the category's `derivativeCutoff`.
- `cutoff = minCutoff + beta * |dx_hat|`.
- `alpha(c) = 1 / (1 + 1 / (2π c dt))`.
- `x_hat = x_hat_prev + alpha(cutoff) * (x - x_hat_prev)`.

The filter re-seeds (output = input) on the first sample, when `dt > 0.5 s`, and when `dt <= 0`.

## Engine integration (fb-core)

- `EngineConfig` gets `pub mutation: MutationConfig`, persisted in `config.json`.
- Calibration state is stored in `<data_dir>/calibration.json`, written atomically with `write_json_atomic`.
  - It is loaded at engine start; a missing or corrupt file means empty data, and a corrupt file is logged.
  - It is saved when dirty, at most once every 10 s, by the writer.
  - It is also saved on reset and in `shutdown`.
  - Disk writes never happen under the merger lock: the data is cloned out and written on a background saver
    thread, or by the caller after releasing the lock.
- `Inner` gets `mutator: Mutex<Mutator>` and `last_output: Mutex<(TrackingFrame, Claims)>`.
  - `submit` runs `mutator.apply` on a copy of the merged frame with a monotonic clock, stores the result in
    `last_output`, and sends it to OSC.
  - Lock order is merger → mutator.
- Public `Engine` API:
  - `latest_frame()` returns the **mutated** output.
  - `latest_raw_frame() -> (TrackingFrame, Claims)` returns the merged input.
  - `mutation_config() -> MutationConfig`.
  - `set_mutation_config(cfg) -> Result<MutationConfig>` sanitizes, applies live, persists `config.json`, emits
    `EngineEvent::MutationChanged`, and returns the sanitized config. It works in offline mode too, where it only
    stores.
  - `calibration() -> CalibrationData`.
  - `reset_calibration(shape: Option<&str>) -> Result<()>` takes an `Expression` name; an unknown name is an error.
    It persists immediately and emits `MutationChanged`.
- CLI:
  - `facebridge-cli mutation show` prints the config JSON.
  - `facebridge-cli mutation set <json>` deep-merges a partial JSON object onto the current config, then prints the
    sanitized result.
  - `facebridge-cli calibration show` prints one table row per shape with nonzero progress or peak, plus the mean
    progress.
  - `facebridge-cli calibration reset [Shape]`.

## UI (Tauri)

A new **Tracking** page sits between Live and Output in the nav. From top to bottom:

- The master switch.
- **Calibration** card:
  - switches for enable and continuous learning;
  - a blend slider;
  - overall progress: the mean of `progress` over the shapes with `peak > 0`;
  - a "Reset calibration" button with confirmation;
  - an expandable per-shape list (name, peak, progress bar, per-shape reset). It refreshes at 1 Hz while the page
    is open.
- **Correctors** card: the 4 controls.
- **Parameter adjustment** card:
  - one row per group: label, min slider, max slider, numeric readouts, reset-row button; a "Reset all" button
    for the card;
  - shape groups and head groups in separate sub-lists.
- **Data filter** card:
  - the enable switch;
  - per category: enable, `minCutoff` slider (log scale, 0.01–30 Hz), `beta` slider (0–10), and a short hint
    ("lower min cutoff = smoother at rest, higher beta = less lag when moving").
- **Live** page: a "Raw / Output" toggle that switches between `latest_raw_frame` and `latest_frame`.

Edits are debounced at about 150 ms and sent with `set_mutation_config`. The returned sanitized config is
re-rendered.
