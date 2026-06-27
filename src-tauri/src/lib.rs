mod mouse_hook;
mod gesture_recognizer;
mod config;
mod command_executor;
mod trajectory_renderer;

use config::{Action, ConfigManager};
use once_cell::sync::OnceCell;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIcon},
    AppHandle, Emitter, Manager, WebviewWindow,
};

#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{HWND, COLORREF},
    Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR},
};

static APP_HANDLE: OnceCell<AppHandle> = OnceCell::new();
static GESTURE_ENABLED: Mutex<bool> = Mutex::new(true);
static TRAJECTORY_ENABLED: Mutex<bool> = Mutex::new(true);
static TRAY_ICON: OnceCell<Mutex<Option<TrayIcon>>> = OnceCell::new();

pub fn is_gesture_enabled_internal() -> bool {
    *GESTURE_ENABLED.lock().unwrap()
}

pub fn is_trajectory_enabled_internal() -> bool {
    *TRAJECTORY_ENABLED.lock().unwrap()
}

pub fn set_trajectory_enabled_internal(enabled: bool) {
    let mut trajectory_enabled = TRAJECTORY_ENABLED.lock().unwrap();
    *trajectory_enabled = enabled;
}

fn load_icon_from_bytes(bytes: &[u8]) -> Option<Image<'static>> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    let rgba_data = rgba.into_raw();
    Some(Image::new_owned(rgba_data, width, height))
}

fn update_tray_icon(enabled: bool) {
    if let Some(tray_mutex) = TRAY_ICON.get() {
        if let Ok(mut tray_opt) = tray_mutex.lock() {
            if let Some(tray) = tray_opt.as_mut() {
                let icon_bytes: &[u8] = if enabled {
                    include_bytes!("../icons/128x128.png")
                } else {
                    include_bytes!("../icons/256x256_disabled.png")
                };
                
                if let Some(icon) = load_icon_from_bytes(icon_bytes) {
                    let _ = tray.set_icon(Some(icon));
                }
            }
        }
    }
}

#[derive(Clone, Serialize)]
pub struct GestureEvent {
    pub name: String,
    pub action_type: Option<String>,
}

pub fn emit_gesture_recognized(name: &str, action_type: Option<&str>) {
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit(
            "gesture-recognized",
            GestureEvent {
                name: name.to_string(),
                action_type: action_type.map(|s| s.to_string()),
            },
        );
    }
}

pub fn emit_trajectory_update(points: &[(i32, i32)], is_drawing: bool) {
    if !is_trajectory_enabled_internal() {
        return;
    }
    let points = points.to_vec();
    std::thread::spawn(move || {
        trajectory_renderer::update_trajectory(&points, is_drawing);
    });
}

pub fn append_trajectory_point(x: i32, y: i32) {
    if !is_trajectory_enabled_internal() {
        return;
    }
    trajectory_renderer::append_trajectory_point(x, y);
}

pub fn clear_trajectory_display() {
    trajectory_renderer::clear_trajectory_display();
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_gestures() -> Result<Vec<config::GestureTemplate>, String> {
    let manager = ConfigManager::new()?;
    manager.load_gestures()
}

#[tauri::command]
fn save_gesture(name: String, points: Vec<(f64, f64)>) -> Result<(), String> {
    let manager = ConfigManager::new()?;
    let mut gestures = manager.load_gestures()?;

    if gestures.iter().any(|g| g.name == name) {
        return Err(format!("Gesture '{}' already exists", name));
    }

    gestures.push(config::GestureTemplate { name, points });

    manager.save_gestures(&gestures)?;
    Ok(())
}

#[tauri::command]
fn update_gesture(old_name: String, new_name: String, points: Vec<(f64, f64)>) -> Result<(), String> {
    let manager = ConfigManager::new()?;
    let mut gestures = manager.load_gestures()?;

    if old_name != new_name && gestures.iter().any(|g| g.name == new_name) {
        return Err(format!("Gesture '{}' already exists", new_name));
    }

    if let Some(gesture) = gestures.iter_mut().find(|g| g.name == old_name) {
        gesture.name = new_name;
        gesture.points = points;
        manager.save_gestures(&gestures)?;
        Ok(())
    } else {
        Err(format!("Gesture '{}' not found", old_name))
    }
}

#[tauri::command]
fn delete_gesture(name: String) -> Result<(), String> {
    let manager = ConfigManager::new()?;
    let mut gestures = manager.load_gestures()?;

    gestures.retain(|g| g.name != name);

    manager.save_gestures(&gestures)?;
    Ok(())
}

#[tauri::command]
fn get_license_info() -> Result<String, String> {
    let license_html = include_str!("../license.html");
    Ok(license_html.to_string())
}

#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
fn get_icon_bytes() -> Vec<u8> {
    include_bytes!("../icons/128x128@2x.png").to_vec()
}

#[tauri::command]
fn get_config() -> Result<config::Config, String> {
    let manager = ConfigManager::new()?;
    manager.load_config()
}

#[tauri::command]
fn save_config(config: config::Config) -> Result<(), String> {
    let manager = ConfigManager::new()?;
    set_trajectory_enabled_internal(config.trajectory);
    trajectory_renderer::set_line_color_from_hex(&config.trajectory_color);
    manager.save_config(&config)
}

#[tauri::command]
fn get_actions() -> Result<Vec<Action>, String> {
    let manager = ConfigManager::new()?;
    let config = manager.load_config()?;
    Ok(config.actions)
}

#[tauri::command]
fn add_action(action: Action) -> Result<(), String> {
    let manager = ConfigManager::new()?;
    let mut config = manager.load_config()?;

    let duplicate = config.actions.iter().any(|a| {
        if action.trigger_type == "wheel" && a.trigger_type == "wheel" {
            a.wheel_trigger == action.wheel_trigger
        } else if action.trigger_type == "gesture" && a.trigger_type == "gesture" {
            a.gesture == action.gesture
        } else {
            false
        }
    });

    if duplicate {
        if action.trigger_type == "wheel" {
            return Err(format!(
                "Action for wheel trigger '{:?}' already exists",
                action.wheel_trigger
            ));
        } else {
            return Err(format!(
                "Action for gesture '{}' already exists",
                action.gesture
            ));
        }
    }

    config.actions.push(action);
    manager.save_config(&config)?;
    Ok(())
}

#[tauri::command]
fn update_action(gesture: String, action: Action) -> Result<(), String> {
    let manager = ConfigManager::new()?;
    let mut config = manager.load_config()?;

    // ジェスチャーキーが"wheel_"で始まる場合はホイールトリガーで一致させる
    if let Some(wheel_key) = gesture.strip_prefix("wheel_") {
        if let Some(existing) = config
            .actions
            .iter_mut()
            .find(|a| a.trigger_type == "wheel" && a.wheel_trigger.as_ref().map_or(false, |wt| wt == wheel_key))
        {
            *existing = action;
            manager.save_config(&config)?;
            return Ok(());
        }
        return Err(format!("Wheel action '{}' not found", wheel_key));
    }

    // 通常のジェスチャー更新
    if let Some(existing) = config.actions.iter_mut().find(|a| a.gesture == gesture) {
        *existing = action;
        manager.save_config(&config)?;
        Ok(())
    } else {
        Err(format!("Action for gesture '{}' not found", gesture))
    }
}

#[tauri::command]
fn delete_action(gesture: String) -> Result<(), String> {
    let manager = ConfigManager::new()?;
    let mut config = manager.load_config()?;

    // ジェスチャーキーが"wheel_"の場合はホイールトリガーで削除
    if let Some(wheel_key) = gesture.strip_prefix("wheel_") {
        config
            .actions
            .retain(|a| !(a.trigger_type == "wheel" && a.wheel_trigger.as_ref().map_or(false, |wt| wt == wheel_key)));
        manager.save_config(&config)?;
        return Ok(());
    }

    // 通常のジェスチャー削除
    config.actions.retain(|a| a.gesture != gesture);
    manager.save_config(&config)?;
    Ok(())
}

#[tauri::command]
fn set_gesture_enabled(enabled: bool) {
    let mut gesture_enabled = GESTURE_ENABLED.lock().unwrap();
    *gesture_enabled = enabled;
}

#[tauri::command]
fn is_gesture_enabled() -> bool {
    let gesture_enabled = GESTURE_ENABLED.lock().unwrap();
    *gesture_enabled
}

#[tauri::command]
fn get_config_file_path() -> Result<String, String> {
    let manager = ConfigManager::new()?;
    let path = manager.config_dir().join("config.json");
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn get_gestures_file_path() -> Result<String, String> {
    let manager = ConfigManager::new()?;
    let path = manager.config_dir().join("gestures.json");
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn reset_config_to_default() -> Result<(), String> {
    let manager = ConfigManager::new()?;
    let default_config = include_str!("../../config/default-config.json");
    let config: config::Config = serde_json::from_str(default_config)
        .map_err(|e| format!("Failed to parse default config: {}", e))?;
    manager.save_config(&config)?;
    set_trajectory_enabled_internal(config.trajectory);
    trajectory_renderer::set_line_color_from_hex(&config.trajectory_color);
    Ok(())
}

#[tauri::command]
fn reset_gestures_to_default() -> Result<(), String> {
    let manager = ConfigManager::new()?;
    let default_gestures = include_str!("../../config/default-gestures.json");
    let gestures: Vec<config::GestureTemplate> = serde_json::from_str(default_gestures)
        .map_err(|e| format!("Failed to parse default gestures: {}", e))?;
    manager.save_gestures(&gestures)?;
    Ok(())
}

#[tauri::command]
fn validate_config_file() -> Result<bool, String> {
    let manager = ConfigManager::new()?;
    match manager.load_config() {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
fn validate_gestures_file() -> Result<bool, String> {
    let manager = ConfigManager::new()?;
    match manager.load_gestures() {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
fn get_config_validation_error() -> Result<Option<String>, String> {
    let manager = ConfigManager::new()?;
    match manager.load_config() {
        Ok(_) => Ok(None),
        Err(e) => Ok(Some(e)),
    }
}

#[tauri::command]
fn get_gestures_validation_error() -> Result<Option<String>, String> {
    let manager = ConfigManager::new()?;
    match manager.load_gestures() {
        Ok(_) => Ok(None),
        Err(e) => Ok(Some(e)),
    }
}

fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "設定を開く", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show, &quit])?;

    let icon = load_icon_from_bytes(include_bytes!("../icons/128x128.png"))
        .or_else(|| {
            app.default_window_icon()
                .cloned()
        })
        .unwrap_or_else(|| Image::new_owned(vec![0; 32 * 32 * 4], 32, 32));

    let tray = TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("OpenMouseGesture")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => {
                app.exit(0);
            }
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|_tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                let mut enabled = GESTURE_ENABLED.lock().unwrap();
                *enabled = !*enabled;
                let new_state = *enabled;
                drop(enabled);
                
                if new_state {
                    let _ = mouse_hook::install_hook();
                } else {
                    let _ = mouse_hook::uninstall_hook();
                }
                
                update_tray_icon(new_state);
            }
        })
        .build(app)?;

    let _ = TRAY_ICON.set(Mutex::new(Some(tray)));

    Ok(())
}

#[cfg(target_os = "windows")]
fn set_titlebar_color(window: &WebviewWindow) -> Result<(), Box<dyn std::error::Error>> {
    // #3B4953 をBGR形式に変換 (Windows APIはBGRを使用)
    // RGB: #3B4953 -> R=0x3B, G=0x49, B=0x53
    // BGR: 0x53493B
    let color: COLORREF = COLORREF(0x0053493B);
    
    let hwnd = HWND(window.hwnd()?.0 as *mut _);
    
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &color as *const _ as *const _,
            std::mem::size_of::<COLORREF>() as u32,
        )?;
    }
    
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn set_titlebar_color(_window: &WebviewWindow) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let _ = APP_HANDLE.set(app.handle().clone());
            setup_tray(app.handle())?;
            
            // ウィンドウのタイトルバー色を設定
            if let Some(window) = app.get_webview_window("main") {
                let _ = set_titlebar_color(&window);
            }
            
            // 設定から軌跡表示の有効無効を読み込む
            if let Ok(manager) = ConfigManager::new() {
                match manager.load_config() {
                    Ok(config) => {
                        set_trajectory_enabled_internal(config.trajectory);
                        trajectory_renderer::set_line_color_from_hex(&config.trajectory_color);
                    }
                    Err(e) => {
                        eprintln!("[ERROR] config.json validation failed: {}", e);
                        // エラーの場合はデフォルト値を使用（ファイルは上書きしない）
                        set_trajectory_enabled_internal(true);
                    }
                }
            }
            
            let _ = trajectory_renderer::init_renderer();
            let _ = mouse_hook::install_hook();

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                window.hide().unwrap();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_gestures,
            save_gesture,
            update_gesture,
            delete_gesture,
            get_license_info,
            get_config,
            save_config,
            get_actions,
            add_action,
            update_action,
            delete_action,
            set_gesture_enabled,
            is_gesture_enabled,
            get_config_file_path,
            get_gestures_file_path,
            reset_config_to_default,
            reset_gestures_to_default,
            validate_config_file,
            validate_gestures_file,
            get_config_validation_error,
            get_gestures_validation_error,
            get_version,
            get_icon_bytes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
