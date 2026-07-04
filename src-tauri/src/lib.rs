mod audio;
mod taskbar_wheel;

use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalSize, State,
    WebviewUrl, WebviewWindowBuilder,
};

const TRAY_ID: &str = "volume-scroller";
const STARTUP_REGISTRY_NAME: &str = "Volume Scroller";
const SETTINGS_WIDTH: f64 = 640.0;
const SETTINGS_HEIGHT: f64 = 640.0;

#[derive(Clone, Serialize)]
struct VolumePayload {
    volume: u8,
    muted: bool,
    direction: i32,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStatus {
    checked: bool,
    current_version: String,
    message: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    #[serde(default = "default_true")]
    scroller_enabled: bool,
    #[serde(default)]
    launch_at_startup: bool,
    #[serde(default = "default_true")]
    start_minimized_to_tray: bool,
    scroll_increment: f32,
    #[serde(default = "default_scroll_direction")]
    scroll_direction: String,
    #[serde(default = "default_true")]
    pause_while_hovering: bool,
    #[serde(default = "default_true")]
    pause_in_fullscreen_apps: bool,
    #[serde(default = "default_true")]
    show_tray_icon: bool,
    #[serde(default = "default_true")]
    check_for_updates_automatically: bool,
    overlay_width: f64,
    overlay_height: f64,
    horizontal_position: String,
    vertical_position: String,
    horizontal_offset: i32,
    vertical_offset: i32,
    #[serde(default = "default_theme")]
    theme: String,
}

struct AppState {
    preferences: Mutex<Preferences>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            scroller_enabled: true,
            launch_at_startup: false,
            start_minimized_to_tray: true,
            scroll_increment: 3.5,
            scroll_direction: default_scroll_direction(),
            pause_while_hovering: true,
            pause_in_fullscreen_apps: true,
            show_tray_icon: true,
            check_for_updates_automatically: true,
            overlay_width: 150.0,
            overlay_height: 40.0,
            horizontal_position: "center".into(),
            vertical_position: "bottom".into(),
            horizontal_offset: 0,
            vertical_offset: 76,
            theme: default_theme(),
        }
    }
}

impl VolumePayload {
    fn from_state(state: audio::VolumeState, direction: i32) -> Self {
        Self {
            volume: (state.scalar * 100.0).round().clamp(0.0, 100.0) as u8,
            muted: state.muted,
            direction,
        }
    }
}

#[tauri::command]
fn get_volume() -> Result<VolumePayload, String> {
    audio::get_volume().map(|state| VolumePayload::from_state(state, 0))
}

#[tauri::command]
fn get_preferences(state: State<'_, AppState>) -> Result<Preferences, String> {
    state
        .preferences
        .lock()
        .map(|preferences| preferences.clone())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_preferences(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    preferences: Preferences,
) -> Result<Preferences, String> {
    let preferences = normalize_preferences(preferences);

    {
        let mut stored_preferences = state
            .preferences
            .lock()
            .map_err(|error| error.to_string())?;
        *stored_preferences = preferences.clone();
    }

    save_preferences_file(&app_handle, &preferences)?;
    position_overlay(&app_handle, &preferences);
    sync_launch_at_startup(&app_handle, &preferences)?;
    sync_tray_icon(&app_handle, &preferences).map_err(|error| error.to_string())?;
    let _ = app_handle.emit("preferences-changed", preferences.clone());

    Ok(preferences)
}

#[tauri::command]
fn preview_preferences(
    app_handle: AppHandle,
    preferences: Preferences,
) -> Result<Preferences, String> {
    let preferences = normalize_preferences(preferences);

    position_overlay(&app_handle, &preferences);
    let _ = app_handle.emit("preferences-preview", preferences.clone());

    let preview_volume = get_volume().unwrap_or(VolumePayload {
        volume: 100,
        muted: false,
        direction: 0,
    });
    let _ = app_handle.emit("volume-changed", preview_volume);

    Ok(preferences)
}

#[tauri::command]
fn reset_preferences(
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<Preferences, String> {
    save_preferences(app_handle, state, Preferences::default())
}

#[tauri::command]
fn open_settings(app_handle: AppHandle) -> Result<(), String> {
    show_settings_window(&app_handle)
}

#[tauri::command]
fn check_for_updates(app_handle: AppHandle) -> Result<UpdateStatus, String> {
    emit_update_status(&app_handle)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            get_volume,
            get_preferences,
            save_preferences,
            preview_preferences,
            reset_preferences,
            open_settings,
            check_for_updates
        ])
        .setup(|app| {
            let preferences = load_preferences_file(app.handle());
            app.manage(AppState {
                preferences: Mutex::new(preferences.clone()),
            });

            position_overlay(app.handle(), &preferences);
            if let Err(error) = sync_launch_at_startup(app.handle(), &preferences) {
                eprintln!("startup preference sync failed: {error}");
            }
            sync_tray_icon(app.handle(), &preferences)?;

            if !preferences.start_minimized_to_tray || !preferences.show_tray_icon {
                let _ = show_settings_window(app.handle());
            }

            if preferences.check_for_updates_automatically {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = emit_update_status(&app_handle);
                });
            }

            taskbar_wheel::start(app.handle().clone(), |app_handle, scroll| {
                let preferences = current_preferences(&app_handle);

                if !preferences.scroller_enabled {
                    return;
                }

                if preferences.pause_in_fullscreen_apps
                    && fullscreen_app_active_on_point(scroll.point_x, scroll.point_y)
                {
                    return;
                }

                let direction = scroll_direction(&preferences, scroll.direction);
                let step = preferences.scroll_increment;

                match audio::change_volume(direction, step) {
                    Ok(state) => {
                        position_overlay_at_point(
                            &app_handle,
                            &preferences,
                            scroll.point_x,
                            scroll.point_y,
                        );
                        let _ = app_handle.emit(
                            "volume-changed",
                            VolumePayload::from_state(state, direction),
                        );
                    }
                    Err(error) => eprintln!("volume change failed: {error}"),
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run volume scroller");
}

fn current_preferences(app_handle: &AppHandle) -> Preferences {
    app_handle
        .try_state::<AppState>()
        .and_then(|state| {
            state
                .preferences
                .lock()
                .ok()
                .map(|preferences| preferences.clone())
        })
        .unwrap_or_default()
}

fn sync_tray_icon(app_handle: &AppHandle, preferences: &Preferences) -> tauri::Result<()> {
    if preferences.show_tray_icon {
        if app_handle.tray_by_id(TRAY_ID).is_none() {
            create_tray(app_handle)?;
        }
    } else {
        let _ = app_handle.remove_tray_by_id(TRAY_ID);
    }

    Ok(())
}

fn create_tray(app_handle: &AppHandle) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app_handle, "settings", "Settings", true, None::<&str>)?;
    let check_updates =
        MenuItem::with_id(app_handle, "check-updates", "Check for Updates", true, None::<&str>)?;
    let quit = MenuItem::with_id(app_handle, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app_handle, &[&settings, &check_updates, &quit])?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Volume Scroller")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if event.id() == "settings" {
                let _ = show_settings_window(app);
            } else if event.id() == "check-updates" {
                let _ = emit_update_status(app);
            } else if event.id() == "quit" {
                app.exit(0);
            }
        })
        .on_tray_icon_event(|tray, event| {
            let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            else {
                return;
            };

            if button == MouseButton::Left && button_state == MouseButtonState::Up {
                let _ = show_settings_window(tray.app_handle());
            }
        });

    if let Some(icon) = app_handle.default_window_icon() {
        tray = tray.icon(icon.clone());
    }

    tray.build(app_handle)?;
    Ok(())
}

fn emit_update_status(app_handle: &AppHandle) -> Result<UpdateStatus, String> {
    let status = UpdateStatus {
        checked: true,
        current_version: app_handle.package_info().version.to_string(),
        message: "No update provider is configured for this build.".into(),
    };

    let _ = app_handle.emit("update-check-completed", status.clone());
    Ok(status)
}

fn show_settings_window(app_handle: &AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("settings") {
        let _ = window.set_min_size(Some(LogicalSize::new(SETTINGS_WIDTH, SETTINGS_HEIGHT)));
        let _ = window.set_size(LogicalSize::new(SETTINGS_WIDTH, SETTINGS_HEIGHT));
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        app_handle,
        "settings",
        WebviewUrl::App("index.html".into()),
    )
    .title("Volume Scroller Settings")
    .inner_size(SETTINGS_WIDTH, SETTINGS_HEIGHT)
    .min_inner_size(SETTINGS_WIDTH, SETTINGS_HEIGHT)
    .center()
    .resizable(true)
    .decorations(true)
    .skip_taskbar(false)
    .always_on_top(false)
    .visible(true)
    .build()
    .map_err(|error| error.to_string())?;

    if let Some(icon) = app_handle.default_window_icon() {
        let _ = window.set_icon(icon.clone());
    }

    Ok(())
}

fn load_preferences_file(app_handle: &AppHandle) -> Preferences {
    let Ok(path) = preferences_path(app_handle) else {
        return Preferences::default();
    };

    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Preferences>(&contents).ok())
        .map(normalize_preferences)
        .unwrap_or_default()
}

fn save_preferences_file(app_handle: &AppHandle, preferences: &Preferences) -> Result<(), String> {
    let path = preferences_path(app_handle)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let contents = serde_json::to_string_pretty(preferences).map_err(|error| error.to_string())?;
    fs::write(path, contents).map_err(|error| error.to_string())
}

fn preferences_path(app_handle: &AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_config_dir()
        .map(|path| path.join("preferences.json"))
        .map_err(|error| error.to_string())
}

fn normalize_preferences(mut preferences: Preferences) -> Preferences {
    if !matches!(preferences.scroll_direction.as_str(), "upIncreases" | "downIncreases") {
        preferences.scroll_direction = default_scroll_direction();
    }

    preferences.scroll_increment = preferences.scroll_increment.clamp(0.5, 25.0);
    preferences.overlay_width = preferences.overlay_width.clamp(96.0, 320.0);
    preferences.overlay_height = preferences.overlay_height.clamp(30.0, 120.0);
    preferences.horizontal_offset = preferences.horizontal_offset.clamp(-400, 400);
    preferences.vertical_offset = preferences.vertical_offset.clamp(-400, 400);

    if !matches!(
        preferences.horizontal_position.as_str(),
        "left" | "center" | "right"
    ) {
        preferences.horizontal_position = "center".into();
    }

    if !matches!(
        preferences.vertical_position.as_str(),
        "top" | "center" | "bottom"
    ) {
        preferences.vertical_position = "bottom".into();
    }

    if !matches!(
        preferences.theme.as_str(),
        "monochrome" | "windows11" | "ubuntu" | "solarized"
    ) {
        preferences.theme = default_theme();
    }

    preferences
}

fn default_theme() -> String {
    "monochrome".into()
}

fn default_scroll_direction() -> String {
    "upIncreases".into()
}

fn default_true() -> bool {
    true
}

fn scroll_direction(preferences: &Preferences, raw_direction: i32) -> i32 {
    if preferences.scroll_direction == "downIncreases" {
        -raw_direction
    } else {
        raw_direction
    }
}

#[cfg(windows)]
fn sync_launch_at_startup(
    _app_handle: &AppHandle,
    preferences: &Preferences,
) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run_key, _) = hkcu
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|error| error.to_string())?;

    if preferences.launch_at_startup {
        let exe_path = env::current_exe().map_err(|error| error.to_string())?;
        let mut command = format!("\"{}\"", exe_path.display());

        if preferences.start_minimized_to_tray {
            command.push_str(" --minimized");
        }

        run_key
            .set_value(STARTUP_REGISTRY_NAME, &command)
            .map_err(|error| error.to_string())
    } else {
        match run_key.delete_value(STARTUP_REGISTRY_NAME) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(not(windows))]
fn sync_launch_at_startup(
    _app_handle: &AppHandle,
    _preferences: &Preferences,
) -> Result<(), String> {
    Ok(())
}

fn position_overlay(app_handle: &AppHandle, preferences: &Preferences) {
    let Some(window) = app_handle.get_webview_window("main") else {
        return;
    };

    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };

    position_overlay_on_monitor(&window, &monitor, preferences);
}

fn position_overlay_at_point(
    app_handle: &AppHandle,
    preferences: &Preferences,
    point_x: i32,
    point_y: i32,
) {
    let Some(window) = app_handle.get_webview_window("main") else {
        return;
    };

    let Ok(Some(monitor)) = app_handle.monitor_from_point(f64::from(point_x), f64::from(point_y))
    else {
        position_overlay(app_handle, preferences);
        return;
    };

    position_overlay_on_monitor(&window, &monitor, preferences);
}

fn position_overlay_on_monitor(
    window: &tauri::WebviewWindow,
    monitor: &Monitor,
    preferences: &Preferences,
) {
    let monitor_size = monitor.size();
    let monitor_pos = monitor.position();
    let size = PhysicalSize::from_logical(
        LogicalSize::new(preferences.overlay_width, preferences.overlay_height),
        monitor.scale_factor(),
    );

    let _ = window.set_size(size);

    let x = anchored_position(
        monitor_pos.x,
        monitor_size.width,
        size.width,
        &preferences.horizontal_position,
        preferences.horizontal_offset,
    );
    let y = anchored_position(
        monitor_pos.y,
        monitor_size.height,
        size.height,
        &preferences.vertical_position,
        preferences.vertical_offset,
    );

    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn anchored_position(
    monitor_origin: i32,
    monitor_length: u32,
    window_length: u32,
    anchor: &str,
    offset: i32,
) -> i32 {
    let origin = i64::from(monitor_origin);
    let monitor_length = i64::from(monitor_length);
    let window_length = i64::from(window_length);
    let offset = i64::from(offset);
    let position = match anchor {
        "left" | "top" => origin + offset,
        "right" | "bottom" => origin + monitor_length - window_length - offset,
        _ => origin + ((monitor_length - window_length) / 2) + offset,
    };

    position.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(windows)]
fn fullscreen_app_active_on_point(point_x: i32, point_y: i32) -> bool {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        MONITOR_DEFAULTTONULL,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowRect, IsWindowVisible,
    };

    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() || !IsWindowVisible(foreground).as_bool() {
            return false;
        }

        let point_monitor = MonitorFromPoint(
            POINT {
                x: point_x,
                y: point_y,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let foreground_monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONULL);

        if point_monitor.0.is_null()
            || foreground_monitor.0.is_null()
            || point_monitor != foreground_monitor
        {
            return false;
        }

        let mut monitor_info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };

        if !GetMonitorInfoW(point_monitor, &mut monitor_info).as_bool() {
            return false;
        }

        let mut window_rect = RECT::default();
        if GetWindowRect(foreground, &mut window_rect).is_err() {
            return false;
        }

        rect_covers(&window_rect, &monitor_info.rcMonitor, 2)
    }
}

#[cfg(windows)]
fn rect_covers(
    window: &windows::Win32::Foundation::RECT,
    monitor: &windows::Win32::Foundation::RECT,
    tolerance: i32,
) -> bool {
    window.left <= monitor.left + tolerance
        && window.top <= monitor.top + tolerance
        && window.right >= monitor.right - tolerance
        && window.bottom >= monitor.bottom - tolerance
}

#[cfg(not(windows))]
fn fullscreen_app_active_on_point(_point_x: i32, _point_y: i32) -> bool {
    false
}
