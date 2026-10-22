//! FaceBridge desktop shell: owns the `fb_core::Engine`, exposes it to the webview, lives in the tray.

mod commands;
mod state;
mod updates;
mod view;

use std::sync::Arc;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

use state::Supervisor;

const MENU_SHOW: &str = "show";
const MENU_QUIT: &str = "quit";

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, MENU_SHOW, "Show FaceBridge", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("FaceBridge")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_SHOW => show_main_window(app),
            MENU_QUIT => {
                // Hide first: shutting modules down can take a few seconds and must not look like a hang.
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn run() {
    let supervisor = Supervisor::new(fb_core::default_data_dir());

    let app = tauri::Builder::default()
        // Must be the first plugin: a second launch just raises the existing window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main_window(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::clone(&supervisor))
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::expression_names_list,
            commands::install_module,
            commands::import_vrcft_modules,
            commands::uninstall_module,
            commands::set_module_enabled,
            commands::set_module_order,
            commands::restart_modules,
            commands::get_module_settings,
            commands::set_module_settings,
            commands::set_module_options,
            commands::press_module_button,
            commands::module_image,
            commands::dismiss_notification,
            commands::latest_frame,
            commands::set_live,
            commands::set_live_raw,
            commands::latest_raw_frame,
            commands::get_mutation_config,
            commands::set_mutation_config,
            commands::get_calibration,
            commands::reset_calibration,
            commands::get_adjust_groups,
            commands::set_osc_config,
            commands::set_check_module_updates,
            commands::set_host_paths,
            commands::restart_engine,
            commands::pick_path,
        ])
        .on_window_event(|window, event| {
            // Closing the window keeps tracking running in the tray; "Quit" in the tray menu really exits.
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            build_tray(app.handle())?;
            let sup = app.state::<Arc<Supervisor>>();
            sup.spawn_live_pusher(app.handle());
            sup.spawn_update_checker(app.handle());
            // The engine starts in the background so the window opens immediately, even if it fails.
            sup.start(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the FaceBridge application");

    app.run(move |_app, event| {
        if let RunEvent::Exit = event {
            supervisor.stop_background();
            supervisor.shutdown();
        }
    });
}
