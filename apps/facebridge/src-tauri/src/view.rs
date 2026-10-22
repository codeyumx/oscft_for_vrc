//! JSON shapes the UI consumes that are not already provided by `fb-core` snapshot types.

use std::net::SocketAddr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fb_osc::OscConfig;
use fb_tracking::{Expression, SHAPE_COUNT, SingleEye, TrackingFrame};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EyeView {
    pub gaze_x: f32,
    pub gaze_y: f32,
    pub openness: f32,
    pub pupil_diameter_mm: f32,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeadView {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FrameView {
    pub left: EyeView,
    pub right: EyeView,
    /// One weight per `Expression`, in index order (names from the `expression_names` command).
    pub shapes: Vec<f32>,
    pub head: HeadView,
    pub eye_claimed: bool,
    pub expression_claimed: bool,
}

/// Three decimals are far below what a progress bar can show and keep the per-frame JSON small.
fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn eye_view(e: &SingleEye) -> EyeView {
    EyeView {
        gaze_x: round3(e.gaze_x),
        gaze_y: round3(e.gaze_y),
        openness: round3(e.openness),
        pupil_diameter_mm: round3(e.pupil_diameter_mm),
    }
}

impl FrameView {
    pub fn new(frame: &TrackingFrame, eye_claimed: bool, expression_claimed: bool) -> Self {
        Self {
            left: eye_view(&frame.eye.left),
            right: eye_view(&frame.eye.right),
            shapes: frame.shapes.iter().map(|&w| round3(w)).collect(),
            head: HeadView { yaw: round3(frame.head.yaw), pitch: round3(frame.head.pitch), roll: round3(frame.head.roll) },
            eye_claimed,
            expression_claimed,
        }
    }
}

/// One parameter adjustment group as the UI lists it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdjustGroupView {
    pub key: &'static str,
    pub label: &'static str,
    /// Head groups are signed and unbounded; shape groups map into 0..1.
    pub is_head: bool,
}

impl AdjustGroupView {
    pub fn new(group: fb_core::fb_mutate::AdjustGroup) -> Self {
        let key = group.name();
        Self { key, label: group.label(), is_head: key.starts_with("head") }
    }
}

pub fn expression_names() -> Vec<&'static str> {
    (0..SHAPE_COUNT).filter_map(Expression::from_index).map(Expression::name).collect()
}

/// OSC settings as edited in the UI.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OscConfigInput {
    pub send_addr: String,
    pub listen_port: u16,
    pub rate_hz: u32,
    pub oscquery: bool,
}

impl OscConfigInput {
    /// Applies the edited fields onto `config`, leaving any fields the UI does not know about untouched.
    pub fn apply(&self, config: &mut OscConfig) -> Result<(), String> {
        let addr: SocketAddr = self
            .send_addr
            .trim()
            .parse()
            .map_err(|_| format!("'{}' is not a valid address; use host:port, e.g. 127.0.0.1:9000", self.send_addr.trim()))?;
        if self.rate_hz == 0 {
            return Err("The output rate must be at least 1 Hz.".into());
        }
        config.send_addr = addr;
        config.listen_port = self.listen_port;
        config.rate_hz = self.rate_hz;
        config.oscquery = self.oscquery;
        Ok(())
    }
}

/// Module host / .NET root paths as edited in the UI; empty or missing means "discover / not set".
#[derive(Deserialize, Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostPaths {
    pub module_host: Option<String>,
    pub dotnet_root: Option<String>,
}

impl HostPaths {
    pub fn normalized(self) -> Self {
        let clean = |v: Option<String>| v.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        Self { module_host: clean(self.module_host), dotnet_root: clean(self.dotnet_root) }
    }
}

/// Encodes an image as a `data:` URL, or `None` when the bytes are not a format a webview can show.
pub fn image_data_url(bytes: &[u8]) -> Option<String> {
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"BM") {
        "image/bmp"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return None;
    };
    Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_url_sniffs_format() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2];
        let url = image_data_url(&png).unwrap();
        assert!(url.starts_with("data:image/png;base64,iVBORw0KGgo"));
        assert!(image_data_url(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap().starts_with("data:image/jpeg;"));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        webp.push(0);
        assert!(image_data_url(&webp).unwrap().starts_with("data:image/webp;"));
        assert_eq!(image_data_url(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);
        assert_eq!(image_data_url(&[]), None);
    }

    #[test]
    fn frame_view_rounds_and_orders_shapes() {
        let mut frame = TrackingFrame::default();
        frame.set_shape(Expression::JawOpen, 0.123_456);
        frame.eye.left.openness = 0.9999;
        let view = FrameView::new(&frame, true, false);
        assert_eq!(view.shapes.len(), SHAPE_COUNT);
        assert_eq!(view.shapes[Expression::JawOpen.index()], 0.123);
        assert_eq!(view.left.openness, 1.0);
        assert!(view.eye_claimed && !view.expression_claimed);
        assert_eq!(expression_names()[Expression::JawOpen.index()], "JawOpen");
    }

    #[test]
    fn osc_input_rejects_bad_address_and_rate() {
        let mut config = OscConfig::default();
        let bad = OscConfigInput { send_addr: "localhost".into(), listen_port: 9001, rate_hz: 60, oscquery: true };
        assert!(bad.apply(&mut config).is_err());
        let zero = OscConfigInput { send_addr: "127.0.0.1:9000".into(), listen_port: 9001, rate_hz: 0, oscquery: true };
        assert!(zero.apply(&mut config).is_err());
        let ok = OscConfigInput { send_addr: " 192.168.1.5:9000 ".into(), listen_port: 0, rate_hz: 30, oscquery: false };
        ok.apply(&mut config).unwrap();
        assert_eq!(config.send_addr.to_string(), "192.168.1.5:9000");
        assert_eq!((config.listen_port, config.rate_hz, config.oscquery), (0, 30, false));
    }
}
