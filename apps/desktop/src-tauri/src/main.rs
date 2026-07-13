#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use llm_meter_daemon::{ipc, socket_path};
use serde_json::{Value, json};
use tauri::{
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

#[tauri::command]
async fn snapshot() -> Result<Value, String> {
    ipc::call(
        &socket_path().map_err(|e| e.to_string())?,
        "snapshot/get",
        json!({}),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn refresh_provider(connection_id: String) -> Result<Value, String> {
    ipc::call(
        &socket_path().map_err(|e| e.to_string())?,
        "connections/refresh",
        json!({"connection_id": connection_id}),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn remove_connection(connection_id: String) -> Result<Value, String> {
    ipc::call(
        &socket_path().map_err(|e| e.to_string())?,
        "connections/remove",
        json!({"connection_id": connection_id}),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn begin_auth(
    connection_type: String,
    display_name: String,
    auth_scheme: String,
) -> Result<Value, String> {
    ipc::call(
        &socket_path().map_err(|e| e.to_string())?,
        "connections/add",
        json!({
            "provider_id": "openai",
            "connection_type": connection_type,
            "display_name": display_name,
            "auth_scheme": auth_scheme,
        }),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn complete_auth(challenge_id: String, secret: Option<String>) -> Result<Value, String> {
    let result = ipc::call(
        &socket_path().map_err(|e| e.to_string())?,
        "connections/auth/complete",
        json!({"challenge_id": challenge_id, "secret": secret.as_deref()}),
    )
    .await
    .map_err(|e| e.to_string());
    drop(secret);
    result
}

#[tauri::command]
fn open_auth_url(auth_url: String) -> Result<(), String> {
    let parsed = url::Url::parse(&auth_url).map_err(|_| "认证地址无效".to_owned())?;
    if parsed.scheme() != "https" {
        return Err("只允许打开 HTTPS 认证地址".into());
    }
    std::process::Command::new("xdg-open")
        .arg(parsed.as_str())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法打开系统浏览器: {e}"))
}

#[tauri::command]
fn close_surface(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "popup" {
        window.hide().map_err(|e| e.to_string())
    } else {
        window.close().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn activate(app: AppHandle, action: String) -> Result<(), String> {
    apply_action(&app, &action)
}

fn popup(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    WebviewWindowBuilder::new(
        app,
        "popup",
        WebviewUrl::App("index.html?role=popup".into()),
    )
    .title("LLM Meter Popup")
    .inner_size(360.0, 440.0)
    .decorations(false)
    .resizable(false)
    .build()
    .map_err(|e| e.to_string())
}

fn main_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html?role=main".into()))
        .title("LLM Meter")
        .inner_size(900.0, 720.0)
        .min_inner_size(380.0, 500.0)
        .decorations(true)
        .resizable(true)
        .build()
        .map_err(|e| e.to_string())
}

fn focus(window: &tauri::WebviewWindow) -> Result<(), String> {
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

fn apply_action(app: &AppHandle, action: &str) -> Result<(), String> {
    match action {
        "hide" => {
            if let Some(window) = app.get_webview_window("popup") {
                window.hide().map_err(|e| e.to_string())?;
            }
        }
        "main" => {
            if let Some(window) = app.get_webview_window("popup") {
                let _ = window.hide();
            }
            match app.get_webview_window("main") {
                Some(window) => focus(&window)?,
                None => {
                    main_window(app)?;
                }
            }
        }
        "toggle" => match app.get_webview_window("popup") {
            Some(window) if window.is_focused().unwrap_or(false) => {
                window.hide().map_err(|e| e.to_string())?
            }
            Some(window) => focus(&window)?,
            None => {
                popup(app)?;
            }
        },
        _ => match app.get_webview_window("popup") {
            Some(window) => focus(&window)?,
            None => {
                popup(app)?;
            }
        },
    }
    Ok(())
}

fn requested_action(args: &[String]) -> &'static str {
    if args.iter().any(|v| v == "--main") {
        "main"
    } else if args.iter().any(|v| v == "--hide") {
        "hide"
    } else if args.iter().any(|v| v == "--toggle") {
        "toggle"
    } else {
        "show"
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let _ = apply_action(app, requested_action(&args));
        }))
        .on_window_event(|window, event| {
            if window.label() == "popup" && matches!(event, tauri::WindowEvent::Focused(false)) {
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            refresh_provider,
            remove_connection,
            begin_auth,
            complete_auth,
            open_auth_url,
            close_surface,
            activate
        ])
        .setup(|app| {
            let action = requested_action(&std::env::args().collect::<Vec<_>>());
            apply_action(app.handle(), action).map_err(std::io::Error::other)?;
            if std::env::var_os("LLM_METER_NO_TRAY").is_none() {
                TrayIconBuilder::new()
                    .icon(
                        app.default_window_icon()
                            .cloned()
                            .ok_or("application icon missing")?,
                    )
                    .tooltip("LLM Meter")
                    .show_menu_on_left_click(false)
                    .on_tray_icon_event(|tray, event| {
                        if matches!(
                            event,
                            TrayIconEvent::Click {
                                button: MouseButton::Left,
                                button_state: MouseButtonState::Up,
                                ..
                            }
                        ) {
                            let _ = apply_action(tray.app_handle(), "toggle");
                        }
                    })
                    .build(app)?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("LLM Meter desktop runtime failed");
}
