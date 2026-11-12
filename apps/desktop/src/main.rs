//! EdgeSlicerRS on the desktop: one window holding several projects, each
//! prepared, sliced with the engine, looked through layer by layer and
//! exported as G-code, in the tabs of the C++ application.
//!
//! `edge-desktop model.3mf` opens that model. `edge-desktop --serve [port]`
//! opens no window and serves the same page to a browser on this machine
//! instead. With `EDGE_DESKTOP_TRACE` set, each thing the page asks for is
//! noted on standard error.

// No console window beside the app on Windows, except in a debug build.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod hosts;
mod serve;

use std::sync::Arc;

use api::Session;
use serde_json::{Value, json};
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;

/// Everything the page asks of the engine.
#[tauri::command(async)]
fn call(
    session: tauri::State<'_, Arc<Session>>,
    name: String,
    args: Value,
) -> Result<Value, String> {
    session.call(&name, &args)
}

/// The same for what is answered in bytes (a mesh, a print's moves): the
/// page is handed them as they are, not as JSON.
#[tauri::command(async)]
fn call_bytes(
    session: tauri::State<'_, Arc<Session>>,
    name: String,
    args: Value,
) -> Result<tauri::ipc::Response, String> {
    session
        .call_bytes(&name, &args)
        .map(tauri::ipc::Response::new)
}

/// Asks the person for a model to open, a folder of profiles, or where to
/// save G-code or a project. None when they change their mind.
#[tauri::command(async)]
fn pick(app: tauri::AppHandle, what: String, suggested: Option<String>) -> Option<String> {
    let dialog = app.dialog().file();
    let picked = match what.as_str() {
        "model" => dialog
            .add_filter("Models", &["3mf", "stl"])
            .blocking_pick_file(),
        "folder" => dialog.blocking_pick_folder(),
        "gcode" => dialog
            .add_filter("G-code", &["gcode"])
            .set_file_name(suggested.unwrap_or_else(|| "print.gcode".into()))
            .blocking_save_file(),
        // "Save Project as...": the reference's title and filter
        // (Plater.cpp `get_export_file`, GUI_App.cpp `file_wildcards`).
        "project" => {
            let suggested =
                std::path::PathBuf::from(suggested.unwrap_or_else(|| "Untitled.3mf".into()));
            let mut dialog = dialog
                .set_title("Save file as:")
                .add_filter("3MF files", &["3mf"]);
            if let Some(dir) = suggested.parent().filter(|dir| dir.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            if let Some(name) = suggested.file_name() {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
            dialog.blocking_save_file()
        }
        _ => None,
    };
    let path = picked?.into_path().ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// Closes the window once the page has asked about what is unsaved.
#[tauri::command]
fn close_window(window: tauri::Window) {
    let _ = window.destroy();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let serving = args.iter().position(|arg| arg == "--serve");
    // `edge-desktop model.3mf` opens that model.
    let model = match serving {
        None => args.get(1).filter(|arg| !arg.starts_with('-')),
        Some(_) => None,
    };
    let session = Arc::new(Session::new(model.map(std::path::PathBuf::from)));
    if let Some(at) = serving {
        let port = args.get(at + 1).and_then(|port| port.parse().ok());
        if let Err(error) = serve::run(session, port.unwrap_or(4710)) {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
        return;
    }
    let asking = session.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(session)
        // With a project unsaved, the window is not closed but the page
        // asks first (save.js), and closes it itself.
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let unsaved = asking
                    .call("unsaved", &json!({}))
                    .ok()
                    .and_then(|answer| answer["unsaved"].as_array().map(Vec::len))
                    .unwrap_or(0);
                if unsaved > 0 {
                    api.prevent_close();
                    let _ = window.emit("close-requested", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            call,
            call_bytes,
            pick,
            close_window
        ])
        .run(tauri::generate_context!())
        .expect("the window could not be opened");
}
