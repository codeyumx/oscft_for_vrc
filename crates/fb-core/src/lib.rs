//! FaceBridge engine: installs VRCFT tracking modules, runs each in a `FaceBridge.ModuleHost` process, arbitrates
//! eye / expression claims, merges their frames and hands the result to `fb-osc`. See `spec/engine-api.md`.

mod config;
mod engine;
mod host;
mod merge;
mod notifications;
mod process;
mod run;
mod settings;
mod store;
mod types;

pub use fb_mutate;

pub use config::{EngineConfig, ModuleEntry, ModuleOptions, default_data_dir, load_config, save_config, write_json_atomic};
pub use engine::{Engine, LOG_CAPACITY, ModuleId};
pub use host::{HOST_ENV_VAR, discover_module_host};
pub use types::{
    EngineEvent, EngineSnapshot, LogEntry, ModuleRunState, ModuleSettingsView, ModuleView, NotificationView, SourceRepo,
};
