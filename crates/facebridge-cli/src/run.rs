//! `facebridge-cli run`: drive the engine and print what happens.

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, never, select};
use fb_core::{Engine, EngineEvent, EngineSnapshot, LogEntry, NotificationView};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

pub fn run(data_dir: PathBuf, exit_after: Option<f64>) -> Result<()> {
    let engine = Engine::start(data_dir.clone())?;
    let events = engine.subscribe();
    let (stop_tx, stop_rx) = bounded::<()>(1);
    ctrlc::set_handler(move || {
        let _ = stop_tx.try_send(());
    })
    .context("installing the Ctrl+C handler")?;

    println!("FaceBridge running (data: {}). Press Ctrl+C to stop.", data_dir.display());
    let mut printer = Printer::default();
    printer.initial(&engine.snapshot());

    let deadline = match exit_after {
        Some(s) => crossbeam_channel::after(Duration::from_secs_f64(s.max(0.0))),
        None => never(),
    };
    loop {
        select! {
            recv(events) -> event => match event {
                Ok(event) => printer.handle(&engine, event),
                Err(_) => break,
            },
            recv(stop_rx) -> _ => break,
            recv(deadline) -> _ => break,
        }
    }

    println!("Shutting down...");
    engine.shutdown();
    println!("Stopped.");
    Ok(())
}

#[derive(Default)]
struct Printer {
    /// Logs already printed from the startup snapshot, so events that raced with it are not shown twice.
    initial_logs: HashSet<(u64, String)>,
    modules: HashMap<String, String>,
    notifications: HashMap<u64, NotificationView>,
    osc: Option<(bool, bool, Option<String>)>,
    avatar: Option<String>,
}

impl Printer {
    fn initial(&mut self, snap: &EngineSnapshot) {
        for log in &snap.logs {
            self.initial_logs.insert((log.time_ms, log.message.clone()));
            print_log(log);
        }
        self.modules_changed(snap);
        self.notifications_changed(snap);
        self.osc_changed(snap, true);
        self.avatar_changed(snap);
    }

    fn handle(&mut self, engine: &Engine, event: EngineEvent) {
        match event {
            EngineEvent::Log(log) => {
                if !self.initial_logs.remove(&(log.time_ms, log.message.clone())) {
                    print_log(&log);
                }
            }
            EngineEvent::ModulesChanged => self.modules_changed(&engine.snapshot()),
            EngineEvent::NotificationsChanged => self.notifications_changed(&engine.snapshot()),
            EngineEvent::OscStatusChanged => self.osc_changed(&engine.snapshot(), false),
            EngineEvent::AvatarChanged => self.avatar_changed(&engine.snapshot()),
            // Nothing to print: settings changes are made through the CLI itself or the desktop app.
            EngineEvent::MutationChanged => {}
        }
    }

    fn modules_changed(&mut self, snap: &EngineSnapshot) {
        for m in &snap.modules {
            let mut text = format!("{:?}", m.state).to_lowercase();
            let claims: Vec<_> =
                [("eye", m.claims.eye), ("expression", m.claims.expression)].into_iter().filter(|c| c.1).map(|c| c.0).collect();
            if !claims.is_empty() {
                text.push_str(&format!(" ({})", claims.join(", ")));
            }
            if self.modules.get(&m.id) != Some(&text) {
                println!("MODULE {}: {text}", m.name);
                self.modules.insert(m.id.clone(), text);
            }
        }
        self.modules.retain(|id, _| snap.modules.iter().any(|m| &m.id == id));
    }

    fn notifications_changed(&mut self, snap: &EngineSnapshot) {
        for n in &snap.notifications {
            if let std::collections::hash_map::Entry::Vacant(slot) = self.notifications.entry(n.id) {
                println!("NOTIFY [{}] {}: {}", n.level, n.module_name, n.message);
                slot.insert(n.clone());
            }
        }
        let gone: Vec<u64> =
            self.notifications.keys().copied().filter(|id| !snap.notifications.iter().any(|n| n.id == *id)).collect();
        for id in gone {
            if let Some(n) = self.notifications.remove(&id) {
                println!("NOTIFY cleared: {}: {}", n.module_name, n.message);
            }
        }
    }

    fn osc_changed(&mut self, snap: &EngineSnapshot, first: bool) {
        let key = (snap.osc.sending, snap.osc.vrchat_detected, snap.osc.last_error.clone());
        if first || self.osc.as_ref() != Some(&key) {
            let error = snap.osc.last_error.as_deref().map_or(String::new(), |e| format!(", error: {e}"));
            println!(
                "OSC -> {}: sending={}, vrchat detected={}{error}",
                snap.osc.send_addr, snap.osc.sending, snap.osc.vrchat_detected
            );
            self.osc = Some(key);
        }
    }

    fn avatar_changed(&mut self, snap: &EngineSnapshot) {
        let text = snap.avatar.as_ref().map(|a| {
            format!(
                "{} ({}): {} of {} parameters matched",
                a.name.as_deref().unwrap_or("unnamed avatar"),
                a.id,
                a.matched_parameter_count,
                a.parameter_count
            )
        });
        if text != self.avatar {
            match &text {
                Some(t) => println!("AVATAR {t}"),
                None => println!("AVATAR none"),
            }
            self.avatar = text;
        }
    }
}

fn print_log(log: &LogEntry) {
    match &log.module_id {
        Some(id) => println!("LOG [{}] {} ({id}): {}", log.level, log.category, log.message),
        None => println!("LOG [{}] {}: {}", log.level, log.category, log.message),
    }
}
