//! Windows notification-area (system tray) icon with quick actions.
//!
//! The menu is rebuilt whenever the pointer enters the icon, so the Midgard state and
//! the pinned apps (apps with BIFRÖST Desktop/Start Menu shortcuts) are always current.
//! All actions call the same `service` functions as the main window.

use bifrost_bridge::{dispatch_windows_toast, AndroidNotification};
use bifrost_core::RuntimeState;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::service;

const TRAY_ID: &str = "bifrost-tray";
const APP_ITEM_PREFIX: &str = "app:";

/// Creates the tray icon. Called once from `setup` in dashboard mode.
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("BIFRÖST")
        .menu(&build_menu(app)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => show_main_window(tray.app_handle()),
            TrayIconEvent::Enter { .. } => refresh_menu(tray.app_handle()),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Shows, un-minimizes and focuses the main window.
pub fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn refresh_menu(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), build_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = service::local_runtime_state().unwrap_or(RuntimeState::Stopped);
    let active = state.is_active();

    let status = MenuItem::with_id(
        app,
        "status",
        format!("Midgard: {state}"),
        false,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, "open", "Open BIFRÖST", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "Start Midgard", !active, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop Midgard", active, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit BIFRÖST", true, None::<&str>)?;

    let shortcuts = service::list_app_shortcuts().unwrap_or_default();
    let apps = Submenu::with_id(app, "apps", "Apps", !shortcuts.is_empty())?;
    for shortcut in &shortcuts {
        let item = MenuItem::with_id(
            app,
            format!("{APP_ITEM_PREFIX}{}", shortcut.package_name),
            &shortcut.display_name,
            true,
            None::<&str>,
        )?;
        apps.append(&item)?;
    }

    Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &start,
            &stop,
            &apps,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )
}

fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "open" => show_main_window(app),
        "quit" => app.exit(0),
        "start" => {
            notify(
                "Starting Midgard",
                "Booting the active Android image in the background.",
            );
            tauri::async_runtime::spawn(async {
                if let Err(err) = service::start_active_image(true).await {
                    notify("BIFRÖST could not start Midgard", &err.to_string());
                }
            });
        }
        "stop" => {
            tauri::async_runtime::spawn(async {
                match service::stop_runtime(false).await {
                    Ok(_) => notify("Midgard stopped", "The Android VM shut down cleanly."),
                    Err(err) => notify("BIFRÖST could not stop Midgard", &err.to_string()),
                }
            });
        }
        other => {
            if let Some(package) = other.strip_prefix(APP_ITEM_PREFIX) {
                let package = package.to_string();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) =
                        service::launch_app_window(package.clone(), None, None, None).await
                    {
                        notify(
                            &format!("BIFRÖST could not launch {package}"),
                            &err.to_string(),
                        );
                    }
                });
            }
        }
    }
}

fn notify(title: &str, text: &str) {
    let _ = dispatch_windows_toast(&AndroidNotification {
        key: format!("bifrost-tray-{title}"),
        package_name: "BIFRÖST".to_string(),
        title: Some(title.to_string()),
        text: Some(text.to_string()),
    });
}
