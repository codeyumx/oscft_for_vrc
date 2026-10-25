//! Example native OSCFT module: synthetic tracking (blinking eyes, jaw / smile animation) with a settings form.
//!
//! Build with `cargo build --release -p fb-module-sdk --example demo_module`, then put the binary next to a
//! `module.json` such as
//!
//! ```json
//! { "ModuleId": "5d1c0f4e-2a7b-4c1e-9a43-6f0d2b8e7c11", "ModuleName": "Demo (native)", "Version": "1.0",
//!   "Executable": "demo_module" }
//! ```
//!
//! and install the folder (or a zip of it) in OSCFT.

use fb_module_sdk::{Claims, Context, Event, Expression, Init, LogLevel, Map, Module, Status, TrackingFrame, Value};
use serde_json::json;
use std::time::{Duration, Instant};

struct Demo;

/// The user's choices from the settings form.
struct Options {
    speed: f32,
    blink: bool,
    jaw: bool,
    smile: bool,
}

impl Options {
    fn read(ctx: &Context) -> Self {
        let settings = ctx.settings();
        let motion = settings.get("motion").and_then(Value::as_str).unwrap_or("jaw");
        Self {
            speed: settings.get("speed").and_then(Value::as_f64).unwrap_or(1.0) as f32,
            blink: settings.get("blink").and_then(Value::as_bool).unwrap_or(true),
            jaw: matches!(motion, "jaw" | "both"),
            smile: matches!(motion, "smile" | "both"),
        }
    }
}

impl Module for Demo {
    fn supported(&self) -> Claims {
        Claims { eye: true, expression: true }
    }

    fn initialize(&mut self, ctx: &Context, available: Claims) -> Init {
        ctx.declare_settings(&json!({ "fields": [
            { "key": "speed", "label": "Speed", "type": "float", "default": 1.0, "min": 0.1, "max": 5.0, "step": 0.1 },
            { "key": "blink", "label": "Blink", "type": "bool", "default": true,
              "description": "Close both eyes briefly every few seconds." },
            { "key": "motion", "label": "Mouth animation", "type": "choice", "default": "jaw", "options": [
                { "value": "jaw", "label": "Jaw open" },
                { "value": "smile", "label": "Smile" },
                { "value": "both", "label": "Jaw open and smile" },
            ] },
            { "key": "restart", "label": "Restart cycle", "type": "button" },
            { "key": "status", "label": "Status", "type": "text", "default": "Starting" },
        ] }));
        ctx.log(LogLevel::Information, format!("Demo module offered eye={} expression={}", available.eye, available.expression));
        Init { name: "Demo (native)".into(), ..Init::new(available) }
    }

    fn run(&mut self, ctx: &Context) {
        ctx.set_status(Status::Active);
        let mut options = Options::read(ctx);
        let mut start = Instant::now();
        let mut status_written = Instant::now();
        let mut frame = TrackingFrame::default();
        frame.eye.min_dilation = 2.0;
        frame.eye.max_dilation = 6.0;
        while ctx.sleep(Duration::from_millis(10)) {
            while let Some(event) = ctx.try_event() {
                match event {
                    Event::SettingsChanged => options = Options::read(ctx),
                    Event::Action(key) if key == "restart" => start = Instant::now(),
                    Event::Action(_) => {}
                }
            }
            let t = start.elapsed().as_secs_f32() * options.speed;
            let wave = (t.sin() + 1.0) / 2.0;
            let openness = if options.blink && t % 4.0 < 0.15 { 0.0 } else { 1.0 };
            for eye in [&mut frame.eye.left, &mut frame.eye.right] {
                eye.openness = openness;
                eye.gaze_x = (t * 0.5).sin() * 0.3;
                eye.pupil_diameter_mm = 4.0;
            }
            frame.set_shape(Expression::JawOpen, if options.jaw { wave } else { 0.0 });
            let smile = if options.smile { wave } else { 0.0 };
            frame.set_shape(Expression::MouthCornerPullLeft, smile);
            frame.set_shape(Expression::MouthCornerPullRight, smile);
            ctx.send_frame(&frame);
            if status_written.elapsed() >= Duration::from_secs(1) {
                status_written = Instant::now();
                let mut values = Map::new();
                values.insert("status".into(), Value::String(format!("Running, mouth {wave:.2}")));
                ctx.write_settings(values);
            }
        }
    }

    fn teardown(&mut self, ctx: &Context) {
        ctx.log(LogLevel::Information, "Demo module stopped");
    }
}

fn main() {
    fb_module_sdk::run(|_data_dir| Ok(Demo));
}
