//! Snapshot / view / event types shared with the UI (all serialize as camelCase JSON).

use crate::config::ModuleOptions;
use fb_osc::{AvatarInfo, Claims, OscConfig, OscStatus};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleRunState {
    Stopped,
    Starting,
    Initializing,
    Running,
    Unclaimed,
    Failed,
    Exited,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleView {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub usage: Option<String>,
    pub page_url: Option<String>,
    /// GitHub `owner/repo` the module is released from (from `module.json` `DownloadUrl` / `ModulePageUrl`).
    pub source_repo: Option<String>,
    /// Options FaceBridge offers for every module (stored in `config.json`).
    pub options: ModuleOptions,
    pub enabled: bool,
    pub state: ModuleRunState,
    pub supports: Claims,
    pub claims: Claims,
    pub status: Option<i8>,
    pub last_error: Option<String>,
    pub has_settings: bool,
    /// Changes whenever the module's setting values change (user edit, module write, redeclaration).
    pub settings_rev: u64,
    pub image_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationView {
    pub id: u64,
    pub module_id: String,
    pub module_name: String,
    /// `"information"`, `"warning"`, `"error"`, ... (see `fb_protocol::LogLevel::as_str`).
    pub level: String,
    pub key: String,
    pub message: String,
    pub time_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub time_ms: u64,
    pub module_id: Option<String>,
    pub level: String,
    pub category: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineSnapshot {
    pub modules: Vec<ModuleView>,
    pub notifications: Vec<NotificationView>,
    pub avatar: Option<AvatarInfo>,
    pub osc: OscStatus,
    pub osc_config: OscConfig,
    /// Ring buffer, newest last, at most [`crate::LOG_CAPACITY`] entries.
    pub logs: Vec<LogEntry>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleSettingsView {
    pub schema: serde_json::Value,
    pub values: serde_json::Value,
}

#[derive(Clone, Debug)]
pub enum EngineEvent {
    ModulesChanged,
    NotificationsChanged,
    Log(LogEntry),
    AvatarChanged,
    OscStatusChanged,
}
