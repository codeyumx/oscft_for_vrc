# OSC output (fb-osc)

What FaceBridge sends to VRChat, how it learns what an avatar accepts, and how it finds VRChat. Implemented in
`crates/fb-osc`. "FaceBridge definition" marks anything the public docs do not define.

## Sources

| Topic | URL |
|---|---|
| VRCFT parameters (names, ranges, prefixes, tracking-active flags) | https://docs.vrcft.io/docs/tutorial-avatars/tutorial-avatars-extras/parameters |
| VRCFT binary parameters | https://docs.vrcft.io/docs/tutorial-avatars/tutorial-avatars-extras/parameters/types/binary |
| Unified Expressions shapes and blended shapes | https://docs.vrcft.io/docs/tutorial-avatars/tutorial-avatars-extras/unified-blendshapes |
| VRChat OSC overview (ports 9000/9001) | https://docs.vrchat.com/docs/osc-overview |
| VRChat avatar parameters + per-avatar config files | https://docs.vrchat.com/docs/osc-avatar-parameters |
| VRChat OSCQuery | https://docs.vrchat.com/docs/oscquery , https://github.com/vrchat-community/osc/wiki/OSCQuery |
| vrc-oscquery-lib (service types, behaviour) | https://github.com/vrchat-community/vrc-oscquery-lib |
| OSCQuery protocol | https://github.com/Vidvox/OSCQueryProposal |
| OSC 1.0 | https://opensoundcontrol.stanford.edu/spec-1_0.html |

## Wire format

OSC 1.0 over UDP to `OscConfig.send_addr` (default `127.0.0.1:9000`, VRChat's input port). Address
`/avatar/parameters/<name>` where `<name>` is the avatar's own spelling (prefix included), exactly as found in the
avatar's parameter list. Arguments: float `f`, bool `T`/`F`, int `i` (big-endian 32-bit, strings NUL-padded to 4).

Messages are packed into OSC bundles (`#bundle`, time tag 1 = "immediately", elements size-prefixed) of at most
1400 bytes; a datagram with a single message is a plain message. VRChat's OscCore parses bundles.

## Parameter matching

FaceBridge sends only to parameters the loaded avatar has (see "Avatar discovery"). For each avatar parameter
`/avatar/parameters/<path>` with VRChat's type:

1. **Tracking-active flags**: last path segment `EyeTrackingActive`, `ExpressionTrackingActive` or
   `LipTrackingActive`, any prefix, any type (bool `T`/`F`, int `1`/`0`, float `1.0`/`0.0`).
2. **VRCFT v2 parameters**: the segment directly before the last must be `v2` (`v2/X`, `FT/v2/X`,
   `A/B/v2/X`; docs: "`.../v2/JawOpen`, `...ExamplePrefix/v2/JawOpen`, `...Example/Nest/v2/JawOpen`"). With `X`:
   - type **Float** and `X` a name from the tables below: float parameter;
   - type **Bool** and `X` = `<Name>Negative` or `<Name><2^k>` (k = 0..15, e.g. `JawOpen1`, `JawOpen2`,
     `JawOpen4`): binary parameter of `<Name>` (see "Binary parameters");
   - everything else (other types, unknown names, unprefixed legacy v1 names such as `JawOpen`) is ignored.
   Several avatar parameters may map to the same name under different prefixes; each gets the value.
3. The VRCFT docs spell two float names `JawClench1` and `JawMandibleRaise1`. Those spellings are accepted as
   **float** aliases of `JawClench` / `JawMandibleRaise` (as **bool** they are binary bit 1 of those names).

Duplicate addresses are matched once.

## Claims

`submit(frame, Claims { eye, expression })`. Parameters are grouped by the data they need:

- **Eye group** (needs `claims.eye`): gaze, eyelid, pupil parameters.
- **Expression group** (needs `claims.expression`): every parameter computed from the 88 shapes (including
  `EyeSquint*`; `EyeWide*` only enters `EyeLid*` when expression is claimed).
- **Tracking-active flags**: always sent (value = the claim), so an avatar falls back to its own animation when
  nothing is tracked.

Parameters of an unclaimed group are never sent. When a claim is lost, only the flags change; the avatar keeps the
last values (it is expected to gate on the flags). Head pose is not part of the VRCFT parameter set and is not sent.

## Float parameters

All values are clamped to their range; NaN becomes 0. Shape weights are clamped to `0..1` before use.
`R`/`L` = Right/Left. "mean" = arithmetic mean (keeps the `0..1` / `-1..1` range). `s(X)` = shape weight.

### Eye (claims.eye) — ranges per docs, formulas are FaceBridge definitions

| Parameter | Range | Formula |
|---|---|---|
| `EyeLeftX`, `EyeLeftY`, `EyeRightX`, `EyeRightY` | -1..1 | gaze x (+ right) / y (+ up) of that eye, as reported by the module |
| `EyeX`, `EyeY` | -1..1 | mean of left and right gaze component |
| `EyeLidLeft` / `EyeLidRight` | 0..1 | `0.75*openness + 0.25*s(EyeWide<side>)`. Docs: 0..0.75 openness, 0.75..1 widen. Openness is clamped to 0..1; the widen term is 0 without an expression claim |
| `EyeLid` | 0..1 | mean of `EyeLidLeft`, `EyeLidRight` |
| `PupilDilation` | 0..1 | `(d - min) / (max - min)`, `d` = mean pupil diameter (mm) of both eyes, `min`/`max` = `eye.min_dilation`/`max_dilation`; 0 when `max <= min` |
| `PupilDiameterLeft` / `PupilDiameterRight` | 0..1 | diameter in cm = `pupil_diameter_mm / 10` |
| `PupilDiameter` | 0..1 | mean of both, in cm |

### Expression (claims.expression)

**Direct**, `0..1`, value = the shape of the same name. The documented ones are `EyeSquintRight/Left`,
`BrowPinch*`, `BrowLowerer*`, `BrowInnerUp{Right,Left}`, `BrowOuterUp{Right,Left}`, `NasalDilation*`,
`NasalConstrict*`, `CheekSquint{Right,Left}`, `NoseSneer{Right,Left}`, `JawOpen`, `MouthClosed`, `JawClench`,
`JawMandibleRaise`, `LipSuck{Upper,Lower,Corner}{Right,Left}`, `LipFunnel{Upper,Lower}{Right,Left}`,
`LipPucker{Upper,Lower}{Right,Left}`, `MouthUpperUp*`, `MouthLowerDown*`, `MouthUpperDeepen*`, `MouthCornerPull*`,
`MouthCornerSlant*`, `MouthFrown*`, `MouthStretch*`, `MouthDimple*`, `MouthRaiserUpper/Lower`, `MouthPress*`,
`MouthTightener*`, `TongueOut`, `TongueRoll`, `TongueTwist{Right,Left}`, `SoftPalateClose`, `ThroatSwallow`,
`NeckFlex{Right,Left}` (`*` = Right and Left). FaceBridge additionally accepts **all 88 shapes** by their own name
(`CheekPuffLeft`, `JawForward`, `EyeWideRight`, `TongueUp`, ...) and a **combined `<X>`** (mean of `<X>Right` and
`<X>Left`, `0..1`) for every Right/Left shape pair that has no explicit definition below (e.g. `MouthPress`,
`MouthTightener`, `MouthDimple`, `NeckFlex`, `EyeWide`, `CheekPuff`, `CheekSuck`, `TongueTwist`); `Jaw`, `MouthUpper`
and `MouthLower` are excluded because those pairs are the signed `JawX`, `MouthUpperX`, `MouthLowerX` axes.

**Signed pairs** (docs give the sign convention; formula `a - b` is a FaceBridge definition), range `-1..1`:

| Parameter | Formula |
|---|---|
| `JawX` | `s(JawRight) - s(JawLeft)` |
| `JawZ` | `s(JawForward) - s(JawBackward)` |
| `MouthUpperX` / `MouthLowerX` | `s(MouthUpperRight) - s(MouthUpperLeft)` / `s(MouthLowerRight) - s(MouthLowerLeft)` |
| `CheekPuffSuckRight` / `CheekPuffSuckLeft` | `s(CheekPuff<side>) - s(CheekSuck<side>)` |
| `TongueX` | `s(TongueRight) - s(TongueLeft)` |
| `TongueY` | `s(TongueUp) - s(TongueDown)` |
| `TongueArchY` | `s(TongueBendDown) - s(TongueCurlUp)` |
| `TongueShape` | `s(TongueSquish) - s(TongueFlat)` |

**Simplified / combined** (names and ranges per docs, formulas are FaceBridge definitions; the docs' "A + B"
descriptions are read as the mean so the range is kept):

| Parameter | Range | Formula |
|---|---|---|
| `EyeSquint` | 0..1 | mean `EyeSquintRight`, `EyeSquintLeft` |
| `CheekSquint`, `NoseSneer` | 0..1 | mean of Right, Left |
| `CheekPuffSuck` | -1..1 | mean of `CheekPuffSuckRight`, `CheekPuffSuckLeft` |
| `MouthX` | -1..1 | mean of `MouthUpperX`, `MouthLowerX` |
| `MouthUpperUp`, `MouthLowerDown` | 0..1 | mean of Right, Left |
| `MouthOpen` | 0..1 | mean of the four `MouthUpperUp*`, `MouthLowerDown*` |
| `MouthSmileRight/Left` | 0..1 | mean(`MouthCornerPull<side>`, `MouthCornerSlant<side>`) |
| `MouthSadRight/Left` | 0..1 | mean(`MouthFrown<side>`, `MouthStretch<side>`) |
| `SmileFrownRight/Left` | -1..1 | `MouthSmile<side> - s(MouthFrown<side>)` |
| `SmileFrown` | -1..1 | mean of Right, Left |
| `SmileSadRight/Left` | -1..1 | `MouthSmile<side> - MouthSad<side>` |
| `SmileSad` | -1..1 | mean of Right, Left |
| `BrowDownRight/Left` | 0..1 | mean(`BrowLowerer<side>`, `BrowPinch<side>`) (docs: blended from those two) |
| `BrowInnerUp`, `BrowOuterUp` | 0..1 | mean of Right, Left |
| `BrowUp` | 0..1 | mean of the four `BrowInnerUp*`, `BrowOuterUp*` |
| `BrowExpressionRight/Left` | -1..1 | `mean(BrowInnerUp<side>, BrowOuterUp<side>) - BrowDown<side>` |
| `BrowExpression` | -1..1 | mean of Right, Left |
| `LipSuckUpper`, `LipSuckLower` (same for `LipFunnel*`, `LipPucker*`) | 0..1 | mean of Right, Left of that row |
| `LipSuck`, `LipFunnel`, `LipPucker` | 0..1 | mean of the four Upper/Lower x Right/Left shapes (corner shapes of LipSuck excluded) |

### Tracking-active flags (docs: "only sent upon load or when the status changes")

| Parameter | Value |
|---|---|
| `EyeTrackingActive` | `claims.eye` |
| `ExpressionTrackingActive`, `LipTrackingActive` | `claims.expression` |

They go through the same change detection as everything else (so: on avatar load, on change, and in the periodic
resend).

## Binary parameters

Docs: a float is represented by Bool parameters `<Name>1`, `<Name>2`, `<Name>4`, `<Name>8`, ... (value 2^k)
and `<Name>Negative` (sign); the avatar normalizes the integer value `0..2^N-1` back to `0..1`.

FaceBridge encoding (resolution = the bits the avatar actually has; docs do not fix the rounding):

- `max_bit` = largest `k` present for that `(prefix, name)` set; `steps = 2^(max_bit+1) - 1`.
- `q = round(|value| * steps)` clamped to `0..steps`; bit `k` is `T` when `(q >> k) & 1 == 1`.
- `<Name>Negative` is `T` when `value < 0` (so for 0..1 parameters it is always `F`).
- Examples with bits 1,2,4 (`steps = 7`): `0.0 -> 000`, `1.0 -> 111`, `5/7 -> 101`, `-2/7 -> 010` + Negative `T`.

Float and binary variants of the same name on one avatar are all sent.

## What is sent, and when

- A **send plan** is built per avatar: only parameters of the avatar that match (above) are ever sent. No avatar
  known (none announced yet, or its parameters could not be read) = no plan and no packets go out.
- A sender thread runs at `rate_hz` (clamped 1..240, default 60) and takes the **latest** submitted frame
  (`submit` only stores it; it never blocks on the network).
- **Change detection**: a value is sent when it differs from the last *sent* value: floats by more than `0.0005`
  (far below VRChat's 8-bit float sync), or when they move to exactly `0`, `1` or `-1`; bools/ints on any change.
- **Full resend** of every parameter: when a new plan is installed (avatar change) and every 3 seconds.
- Send errors (e.g. ICMP port unreachable on Windows = VRChat not listening) are reported in `OscStatus.last_error`
  and cleared by the next successful send; sending continues.
- `OscStatus`: `messages_per_second` = OSC messages (not datagrams) actually sent, averaged over the last 3 seconds;
  `sending` = a plan with matches exists, a frame was submitted, and the last send did not fail;
  `vrchat_detected` = VRChat's OSCQuery service is visible on mDNS or a datagram arrived on our receive port in the
  last 15 s; `send_addr` = effective target (see below).

## Avatar discovery

VRChat tells apps about avatar changes by sending `/avatar/change <avtr_id>` to the receiver it knows about:
the legacy port 9001, and any OSCQuery-advertised receiver. FaceBridge listens for it and then resolves the avatar's
parameters (retrying for ~20 s, because the config file and OSCQuery tree appear shortly after the avatar loads):

1. **OSCQuery tree** (`AvatarSource::OscQuery`): `GET /avatar` (fallback `/`) on VRChat's OSCQuery HTTP server; the
   avatar id is `/avatar/change` `VALUE[0]` and must match the id announced; parameters are every leaf under
   `/avatar/parameters` with `TYPE` `f`, `i`, `T`/`F` and (when present) writable `ACCESS` (bit 2). Nested containers
   (`FT/v2/JawOpen`) are walked; the node's `FULL_PATH` is the address.
2. **Config file** (`AvatarSource::ConfigFile`): `%USERPROFILE%\AppData\LocalLow\VRChat\VRChat\OSC\<usr_...>\Avatars\<avtr_...>.json`
   (override the OSC directory with env `FACEBRIDGE_VRCHAT_OSC_DIR`); the most recently written user directory
   wins. Entries with an `input` block whose address is under `/avatar/parameters/` and type `Int`/`Float`/`Bool`
   (case-insensitive) are usable; the `input.address` is what is sent to (config files can rename addresses).
   Not written for Build & Test avatars. A UTF-8 BOM is tolerated.

When both are unavailable after the retries `last_error` explains it and nothing is sent. While VRChat's OSCQuery
service is known, `/avatar/change` is also polled every 5 s so a missed change message (FaceBridge started
after VRChat) still resolves the avatar. A new `/avatar/change` for a different avatar clears the old plan at once.

`AvatarInfo.parameter_count` = parameters the avatar declares (config file: all entries; OSCQuery: all leaves);
`matched_parameter_count` = avatar parameters that receive FaceBridge data.

## Ports and OSCQuery

- Legacy listener: UDP `listen_port` (default 9001; bound to `127.0.0.1` when `send_addr` is loopback, else all interfaces). If it is taken
  (another OSC app) FaceBridge reports it in `last_error` and, with OSCQuery on, receives on an ephemeral port that
  only OSCQuery advertises. `listen_port = 0` = OSCQuery-only (ephemeral port).
- With `oscquery = true` (docs: VRChat OSCQuery, vrc-oscquery-lib):
  - HTTP server on an ephemeral port (127.0.0.1 when `send_addr` is loopback, else all interfaces) serving:
    `/` root (`FULL_PATH`, `ACCESS`, `CONTENTS`), `/avatar` container, `/avatar/change` (`TYPE "s"`, `ACCESS 2`) and
    `?HOST_INFO` (`NAME "FaceBridge"`, `OSC_IP`, `OSC_PORT`, `OSC_TRANSPORT "UDP"`, `EXTENSIONS`). Per-attribute queries
    (`?TYPE`) answer 200/204; unknown paths 404; unknown attributes 400.
  - mDNS advertisement of service name `FaceBridge`: `_osc._udp` (UDP receive port) and `_oscjson._tcp` (HTTP port),
    on `127.0.0.1` when `send_addr` is loopback, else on every host address. VRChat finds `/avatar` in the tree and
    starts sending avatar changes (and parameters) to FaceBridge, showing a HUD notice naming "FaceBridge".
  - mDNS browse for `_oscjson._tcp`; instances named `VRChat-Client-*` are VRChat. VRChat on Windows serves OSCQuery
    to the local machine only, so loopback is tried first when `send_addr` is loopback.
- **Send target follows VRChat**: when `send_addr` is exactly the default `127.0.0.1:9000` and VRChat's
  `HOST_INFO.OSC_PORT` is a different port (VRChat picks another one when 9000 is busy), the target port becomes
  VRChat's. Any other configured `send_addr` is never overridden. `OscStatus.send_addr` shows the effective target.
- OSCQuery services are withdrawn (goodbye packets) when `OscOutput` is dropped (drop blocks up to ~1 s).

### Network exposure / Windows Firewall

When `send_addr` is loopback (the default) the send socket, the legacy UDP listener and the OSCQuery HTTP server
are all bound to `127.0.0.1` and nothing is reachable from the LAN. The mDNS part is the exception: the mDNS library
(`mdns-sd`) must bind `0.0.0.0:5353` and join the multicast group, which can make Windows Firewall show its
"allow this app" prompt on first run. Setting `oscquery = false` creates no mDNS socket (and no HTTP server), at the
price of relying on legacy port 9001 only. The advertised records themselves only carry `127.0.0.1`.

## Not implemented / not sourced

- VRChat's native eye tracking addresses (`/tracking/eye/*`) and head/chatbox are not sent.
- Head pose has no VRCFT parameter.
- Docs define no formulas for combined/simplified parameters, pupil/eyelid mapping or the binary rounding: all of
  those are FaceBridge definitions above. The `v2/` prefix rule is generalized from the three documented examples.
- Not in the docs' parameter list but present on a real avatar inspected through VRChat's OSCQuery
  (`FT/v2/CheekPuffLeft`, `FT/v2/JawForward`, `FT/v2/MouthPress`, each with `1/2/4` bit variants): every Unified
  Expressions shape is accepted by its own name, and the combined `<X>` of any `<X>Right`/`<X>Left` pair
  (`MouthPress`, `MouthTightener`, `NeckFlex`, `EyeWide`, `CheekPuff`, `TongueTwist`, ...) is the mean of both. The docs
  confirm the idea ("any shape ... can be controlled using any Unified Expressions parameter", "some [parameters]
  undocumented") but give no list; the exact set of undocumented names VRCFT itself sends is not sourced.
