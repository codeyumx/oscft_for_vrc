//! Tauri commands: thin mappings from the UI onto `fb_core::Engine`. Everything that can block (file I/O,
//! child processes, sockets, dialogs) runs on the blocking pool so the main/UI thread never stalls.

use std::path::PathBuf;
use std::sync::Arc;

use fb_core::fb_mutate::{AdjustGroup, CalibrationData, MutationConfig, Mutator};
use fb_core::{Engine, EngineSnapshot, ModuleOptions, ModuleSettingsView};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::state::{Phase, Supervisor};
use crate::updates::{AppUpdate, ModuleUpdate};
use crate::view::{AdjustGroupView, FrameView, HostPaths, OscConfigInput, expression_names, image_data_url};

type Sup<'a> = State<'a, Arc<Supervisor>>;
type CmdResult<T> = Result<T, String>;

async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CmdResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| format!("background task failed: {e}"))?
}

/// Runs `f` against the running engine on the blocking pool.
async fn with_engine<T, F>(sup: &Sup<'_>, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Engine) -> anyhow::Result<T> + Send + 'static,
{
    let engine = sup.engine()?;
    blocking(move || f(&engine).map_err(|e| format!("{e:#}"))).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateView {
    #[serde(flatten)]
    phase: Phase,
    snapshot: Option<EngineSnapshot>,
    data_dir: String,
    version: &'static str,
    host_paths: HostPaths,
    /// OSC settings from `config.json` (what the Settings page edits; available even when the engine is down).
    osc_config: Option<fb_osc::OscConfig>,
    /// Why `config.json` could not be read, if it could not.
    config_error: Option<String>,
    /// Whether module sources are checked for new releases (`app-settings.json`).
    check_module_updates: bool,
    /// Whether OSCFT for VRC's own repository is checked for new releases (`app-settings.json`).
    check_app_updates: bool,
    /// A newer OSCFT for VRC release than this build (`None` while the app check is off).
    app_update: Option<AppUpdate>,
    /// Installed modules with a newer release at their source (empty while the module check is off).
    module_updates: Vec<ModuleUpdate>,
    /// Whether closing the window hides it to the tray (`app-settings.json`).
    close_to_tray: bool,
}

/// Reads the persisted config for the Settings page.
fn persisted_settings(sup: &Supervisor) -> (HostPaths, Option<fb_osc::OscConfig>, Option<String>) {
    match fb_core::load_config(&sup.data_dir) {
        Ok(cfg) => (
            HostPaths {
                module_host: cfg.module_host.map(|p| p.display().to_string()),
                dotnet_root: cfg.dotnet_root.map(|p| p.display().to_string()),
            },
            Some(cfg.osc),
            None,
        ),
        Err(e) => (HostPaths::default(), None, Some(format!("{e:#}"))),
    }
}

#[tauri::command]
pub async fn get_state(sup: Sup<'_>) -> CmdResult<AppStateView> {
    let sup = Arc::clone(sup.inner());
    blocking(move || {
        let (host_paths, osc_config, config_error) = persisted_settings(&sup);
        let snapshot = sup.engine().ok().map(|e| e.snapshot());
        let module_updates = snapshot.as_ref().map(|s| sup.updates.updates(&s.modules)).unwrap_or_default();
        Ok(AppStateView {
            phase: sup.phase(),
            snapshot,
            data_dir: sup.data_dir.display().to_string(),
            version: env!("CARGO_PKG_VERSION"),
            host_paths,
            osc_config,
            config_error,
            check_module_updates: sup.updates.modules_enabled(),
            check_app_updates: sup.updates.app_enabled(),
            app_update: sup.updates.app_update(),
            module_updates,
            close_to_tray: sup.settings().close_to_tray,
        })
    })
    .await
}

#[tauri::command]
pub fn expression_names_list() -> Vec<&'static str> {
    expression_names()
}

/// `kind` is `"zip"` (pick a module archive) or `"folder"` (pick an unpacked module folder).
/// Returns the new module id, or `None` when the dialog was cancelled.
#[tauri::command]
pub async fn install_module(app: AppHandle, sup: Sup<'_>, kind: String) -> CmdResult<Option<String>> {
    let engine = sup.engine()?;
    blocking(move || {
        let dialog = app.dialog().file().set_title("Install a tracking module");
        let picked = match kind.as_str() {
            "zip" => dialog.add_filter("Module archive (*.zip)", &["zip"]).blocking_pick_file(),
            "folder" => dialog.blocking_pick_folder(),
            other => return Err(format!("unknown install kind '{other}'")),
        };
        let Some(picked) = picked else { return Ok(None) };
        let path: PathBuf = picked.into_path().map_err(|e| e.to_string())?;
        engine.install_module(&path).map(Some).map_err(|e| format!("{e:#}"))
    })
    .await
}

#[tauri::command]
pub async fn import_vrcft_modules(sup: Sup<'_>) -> CmdResult<Vec<String>> {
    with_engine(&sup, |e| e.import_vrcft_modules()).await
}

#[tauri::command]
pub async fn uninstall_module(sup: Sup<'_>, id: String) -> CmdResult<()> {
    with_engine(&sup, move |e| e.uninstall_module(&id)).await
}

#[tauri::command]
pub async fn set_module_enabled(sup: Sup<'_>, id: String, enabled: bool) -> CmdResult<()> {
    with_engine(&sup, move |e| e.set_module_enabled(&id, enabled)).await
}

#[tauri::command]
pub async fn set_module_order(sup: Sup<'_>, ids: Vec<String>) -> CmdResult<()> {
    with_engine(&sup, move |e| e.set_module_order(&ids)).await
}

#[tauri::command]
pub async fn restart_modules(sup: Sup<'_>) -> CmdResult<()> {
    with_engine(&sup, |e| e.restart_modules()).await
}

#[tauri::command]
pub async fn get_module_settings(sup: Sup<'_>, id: String) -> CmdResult<Option<ModuleSettingsView>> {
    with_engine(&sup, move |e| Ok(e.module_settings(&id))).await
}

#[tauri::command]
pub async fn set_module_settings(sup: Sup<'_>, id: String, values: serde_json::Value) -> CmdResult<()> {
    with_engine(&sup, move |e| e.set_module_settings(&id, values)).await
}

#[tauri::command]
pub async fn set_module_options(sup: Sup<'_>, id: String, options: ModuleOptions) -> CmdResult<()> {
    with_engine(&sup, move |e| e.set_module_options(&id, options)).await
}

/// The user pressed a button in the module's settings form.
#[tauri::command]
pub async fn press_module_button(sup: Sup<'_>, id: String, key: String) -> CmdResult<()> {
    with_engine(&sup, move |e| e.press_module_button(&id, &key)).await
}

/// The module's n-th image as a `data:` URL (`None` when absent or not a displayable format).
#[tauri::command]
pub async fn module_image(sup: Sup<'_>, id: String, index: usize) -> CmdResult<Option<String>> {
    with_engine(&sup, move |e| Ok(e.module_image(&id, index).and_then(|b| image_data_url(&b)))).await
}

#[tauri::command]
pub async fn dismiss_notification(sup: Sup<'_>, id: u64) -> CmdResult<()> {
    let engine = sup.engine()?;
    engine.dismiss_notification(id);
    Ok(())
}

#[tauri::command]
pub async fn latest_frame(sup: Sup<'_>) -> CmdResult<FrameView> {
    let engine = sup.engine()?;
    let (frame, claims) = engine.latest_frame();
    Ok(FrameView::new(&frame, claims.eye, claims.expression))
}

#[tauri::command]
pub async fn latest_raw_frame(sup: Sup<'_>) -> CmdResult<FrameView> {
    let engine = sup.engine()?;
    let (frame, claims) = engine.latest_raw_frame();
    Ok(FrameView::new(&frame, claims.eye, claims.expression))
}

/// Chooses whether pushed `frame` events carry the merged input (`raw`) or the mutated output.
#[tauri::command]
pub fn set_live_raw(sup: Sup<'_>, raw: bool) {
    sup.set_live_raw(raw);
}

/// The mutation settings. Works while the engine is down too (reads `config.json`).
#[tauri::command]
pub async fn get_mutation_config(sup: Sup<'_>) -> CmdResult<MutationConfig> {
    let sup = Arc::clone(sup.inner());
    blocking(move || match sup.engine() {
        Ok(engine) => Ok(engine.mutation_config()),
        Err(_) => fb_core::load_config(&sup.data_dir).map(|c| c.mutation).map_err(|e| format!("{e:#}")),
    })
    .await
}

/// Applies mutation settings and returns the sanitized result. While the engine is down this edits `config.json`.
#[tauri::command]
pub async fn set_mutation_config(sup: Sup<'_>, config: MutationConfig) -> CmdResult<MutationConfig> {
    let sup = Arc::clone(sup.inner());
    blocking(move || match sup.engine() {
        Ok(engine) => engine.set_mutation_config(config).map_err(|e| format!("{e:#}")),
        Err(_) => {
            let sanitized = Mutator::new(MutationConfig::default(), CalibrationData::default()).set_config(config);
            let mut persisted = fb_core::load_config(&sup.data_dir).map_err(|e| format!("{e:#}"))?;
            persisted.mutation = sanitized.clone();
            fb_core::save_config(&sup.data_dir, &persisted).map_err(|e| format!("{e:#}"))?;
            Ok(sanitized)
        }
    })
    .await
}

/// Learned calibration (empty while the engine is down).
#[tauri::command]
pub async fn get_calibration(sup: Sup<'_>) -> CmdResult<CalibrationData> {
    let engine = sup.engine().ok();
    blocking(move || Ok(engine.map(|e| e.calibration()).unwrap_or_default())).await
}

/// Resets calibration of one shape (by `Expression` name) or of all shapes.
#[tauri::command]
pub async fn reset_calibration(sup: Sup<'_>, shape: Option<String>) -> CmdResult<()> {
    with_engine(&sup, move |e| e.reset_calibration(shape.as_deref())).await
}

/// Parameter adjustment groups with their UI labels, in display order.
#[tauri::command]
pub fn get_adjust_groups() -> Vec<AdjustGroupView> {
    AdjustGroup::ALL.iter().map(|g| AdjustGroupView::new(*g)).collect()
}

/// Tells the backend whether the Live view is open (frames are only pushed while it is).
#[tauri::command]
pub fn set_live(sup: Sup<'_>, live: bool) {
    sup.set_live(live);
}

/// Applies OSC settings. Works while the engine is down too (edits `config.json` for the next start).
#[tauri::command]
pub async fn set_osc_config(sup: Sup<'_>, osc: OscConfigInput) -> CmdResult<()> {
    let sup = Arc::clone(sup.inner());
    blocking(move || match sup.engine() {
        Ok(engine) => {
            let mut config = engine.snapshot().osc_config;
            osc.apply(&mut config)?;
            engine.set_osc_config(config).map_err(|e| format!("{e:#}"))
        }
        Err(_) => {
            let mut config = fb_core::load_config(&sup.data_dir).map_err(|e| format!("{e:#}"))?;
            osc.apply(&mut config.osc)?;
            fb_core::save_config(&sup.data_dir, &config).map_err(|e| format!("{e:#}"))
        }
    })
    .await
}

/// Switches the module update check on or off and saves it to `app-settings.json`.
#[tauri::command]
pub async fn set_check_module_updates(sup: Sup<'_>, enabled: bool) -> CmdResult<()> {
    let sup = Arc::clone(sup.inner());
    blocking(move || {
        sup.update_settings(|s| s.check_module_updates = enabled).map_err(|e| format!("{e:#}"))?;
        sup.updates.set_modules_enabled(enabled);
        sup.wake_update_checker();
        Ok(())
    })
    .await
}

/// Switches the OSCFT for VRC update check on or off and saves it to `app-settings.json`.
#[tauri::command]
pub async fn set_check_app_updates(sup: Sup<'_>, enabled: bool) -> CmdResult<()> {
    let sup = Arc::clone(sup.inner());
    blocking(move || {
        sup.update_settings(|s| s.check_app_updates = enabled).map_err(|e| format!("{e:#}"))?;
        sup.updates.set_app_enabled(enabled);
        sup.wake_update_checker();
        Ok(())
    })
    .await
}

/// Chooses whether closing the window hides it to a tray icon or quits, and shows or removes the tray icon to match.
#[tauri::command]
pub async fn set_close_to_tray(app: AppHandle, sup: Sup<'_>, enabled: bool) -> CmdResult<()> {
    let sup = Arc::clone(sup.inner());
    blocking(move || {
        sup.update_settings(|s| s.close_to_tray = enabled).map_err(|e| format!("{e:#}"))?;
        crate::set_tray_visible(&app, enabled).map_err(|e| format!("tray icon: {e}"))
    })
    .await
}

/// Saves the module host / .NET root paths and restarts the engine so they take effect.
#[tauri::command]
pub async fn set_host_paths(app: AppHandle, sup: Sup<'_>, paths: HostPaths) -> CmdResult<()> {
    let sup = Arc::clone(sup.inner());
    let paths = paths.normalized();
    blocking(move || {
        // Shut down first: the engine persists config.json itself and must not overwrite our edit.
        sup.shutdown();
        let result = (|| {
            let mut config = fb_core::load_config(&sup.data_dir)?;
            config.module_host = paths.module_host.map(PathBuf::from);
            config.dotnet_root = paths.dotnet_root.map(PathBuf::from);
            fb_core::save_config(&sup.data_dir, &config)
        })()
        .map_err(|e| format!("{e:#}"));
        sup.start(&app);
        result
    })
    .await
}

/// Stops and starts the engine again (also the retry action after a failed start).
#[tauri::command]
pub async fn restart_engine(app: AppHandle, sup: Sup<'_>) -> CmdResult<()> {
    let sup = Arc::clone(sup.inner());
    blocking(move || {
        sup.restart(&app);
        Ok(())
    })
    .await
}

/// File picker for the Settings page (executable or folder), so users need not type paths.
#[tauri::command]
pub async fn pick_path(app: AppHandle, folder: bool, title: String) -> CmdResult<Option<String>> {
    blocking(move || {
        let dialog = app.dialog().file().set_title(title);
        let picked = if folder { dialog.blocking_pick_folder() } else { dialog.blocking_pick_file() };
        match picked {
            Some(p) => Ok(Some(p.into_path().map_err(|e| e.to_string())?.display().to_string())),
            None => Ok(None),
        }
    })
    .await
}
