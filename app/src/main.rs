#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! One exe, several roles. Without arguments it is the window. The others run without any UI:
//!   --worker job.json         elevated helper for app, Chocolatey and Windows updates
//!   --clean-worker job.json   elevated helper for the Windows cleanup rules
//!   --repair id log           elevated helper for a repair
//!   --admin-task name out     elevated helper for a single admin read (drive wear, restore point)
//!   --background              the scheduled check: looks, then notifies; never installs or deletes
//!   --check [--json]          the same check on demand, for scripts and agents (pipe the output)
//!   --open safe-toolbox:page  a notification click: opens the window on that page

mod cleanup;
mod pages;
mod state;
mod updates;

use std::sync::atomic::Ordering;

use tauri::window::{Effect, EffectsBuilder};
use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window, WindowEvent};
use toolbox_core::proc::Cancel;
use toolbox_core::toolbox::{self, BackdropMode, ThemeMode, ToolboxSettings, background, notify, schedule};

use state::AppState;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli(&args) {
        std::process::exit(code);
    }
    run_window(open_page(&args));
}

fn cli(args: &[String]) -> Option<i32> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    Some(match a.as_slice() {
        ["--worker", job, ..] => toolbox_core::updater::worker::run_file(job),
        ["--clean-worker", job, ..] => toolbox_core::cleaner::admin::run_file(job),
        ["--repair", id, log, ..] => toolbox::repairs::run_elevated(id, log),
        ["--admin-task", name, out, ..] => toolbox::admin_tasks::run_task(name, out),
        ["--background", ..] => background_mode(true, false),
        ["--check", rest @ ..] => background_mode(false, rest.contains(&"--json")),
        _ => return None,
    })
}

fn open_page(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "--open")?;
    notify::page_from_link(args.get(i + 1)?)
}

pub fn exe_path() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The notification identity and the click-through link, with the icon written next to the settings.
pub fn register_notifications() {
    let icon = toolbox::root().join("icon.png");
    if !icon.exists() {
        let _ = std::fs::write(&icon, include_bytes!("../icons/icon.png"));
    }
    if let Err(e) = notify::register(&exe_path(), Some(&icon.to_string_lossy())) {
        toolbox::write_error(&format!("Could not register notifications: {e}"));
    }
}

fn background_mode(scheduled: bool, json: bool) -> i32 {
    let r = std::panic::catch_unwind(|| {
        let settings = ToolboxSettings::load();
        let last = background::CheckSummary::load();
        let now = toolbox_core::time::now();
        let updates = !scheduled || background::updates_due(&settings, last.as_ref().and_then(|l| l.updates_when), now);
        let cleanup = !scheduled || background::cleanup_due(&settings, last.as_ref().and_then(|l| l.cleanup_when), now);
        if !updates && !cleanup {
            return 0;
        }
        let mut summary = background::run(updates, cleanup, &settings, &Cancel::new());
        if scheduled && settings.notify {
            if let Some((title, body)) = background::notification(&summary, &settings, updates, cleanup) {
                let fresh = updates && !summary.updates.is_empty() && summary.notified_updates.as_deref() != Some(summary.updates_key().as_str());
                register_notifications();
                match notify::show(&title, &body, if fresh { "updates" } else { "cleanup" }) {
                    Ok(()) if updates => {
                        summary.notified_updates = Some(summary.updates_key());
                        summary.save();
                    }
                    Ok(()) => {}
                    Err(e) => toolbox::write_error(&format!("Notification failed: {e}")),
                }
            }
        }
        if json {
            println!("{}", serde_json::to_string_pretty(&summary).unwrap_or_default());
        }
        0
    });
    r.unwrap_or_else(|_| {
        toolbox::write_error("Background check failed with an internal error.");
        1
    })
}

/// Theme for the title bar and Mica, and the backdrop itself. The page paints the rest from the same settings.
pub fn apply_window(w: &WebviewWindow, s: &ToolboxSettings) {
    let _ = w.set_theme(match s.theme {
        ThemeMode::Light => Some(tauri::Theme::Light),
        ThemeMode::Dark => Some(tauri::Theme::Dark),
        ThemeMode::System => None,
    });
    let effect = match s.backdrop {
        BackdropMode::MicaAlt => Some(Effect::Tabbed),
        BackdropMode::Mica => Some(Effect::Mica),
        BackdropMode::Solid => None,
    };
    let _ = w.set_effects(effect.map(|e| EffectsBuilder::new().effect(e).build()));
}

/// Puts the window back where it was, if that spot is still on a screen.
fn restore_bounds(w: &WebviewWindow, s: &ToolboxSettings) {
    if let Some(b) = s.window_bounds.as_deref().filter(|b| b.len() == 4 && b[2] >= 480 && b[3] >= 360) {
        let on_screen = w.available_monitors().unwrap_or_default().iter().any(|m| {
            let (p, z) = (m.position(), m.size());
            b[0] + 120 > p.x && b[0] < p.x + z.width as i32 - 120 && b[1] >= p.y - 8 && b[1] < p.y + z.height as i32 - 60
        });
        if on_screen {
            let _ = w.set_size(PhysicalSize::new(b[2] as u32, b[3] as u32));
            let _ = w.set_position(PhysicalPosition::new(b[0], b[1]));
        }
    }
    if s.window_maximized {
        let _ = w.maximize();
    }
}

fn save_bounds(w: &Window, st: &AppState) {
    let mut s = st.settings.lock().unwrap();
    let max = w.is_maximized().unwrap_or(false);
    if !max && !w.is_minimized().unwrap_or(false) {
        if let (Ok(p), Ok(z)) = (w.outer_position(), w.inner_size()) {
            s.window_bounds = Some(vec![p.x, p.y, z.width as i32, z.height as i32]);
        }
    }
    s.window_maximized = max;
    s.save();
}

fn run_window(page: Option<String>) {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
            if let Some(p) = open_page(argv.get(1..).unwrap_or_default()) {
                let _ = app.emit("navigate", p);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(page))
        .invoke_handler(tauri::generate_handler![
            pages::app_info,
            pages::accent,
            pages::get_settings,
            pages::set_settings,
            pages::force_close,
            pages::open,
            pages::reveal,
            pages::open_tool,
            pages::pick_folder,
            pages::home_state,
            pages::task_status,
            pages::test_notification,
            pages::background_now,
            pages::upd_settings,
            pages::set_upd_setting,
            pages::health_snapshot,
            pages::health_wear,
            pages::health_battery_report,
            pages::health_restore_point,
            pages::repair_list,
            pages::repair_run,
            pages::path_state,
            pages::path_fix,
            pages::path_undo,
            pages::big_find,
            pages::big_recycle,
            pages::applist_save,
            pages::applist_pick,
            pages::applist_import,
            pages::history,
            updates::upd_state,
            updates::upd_check,
            updates::upd_select,
            updates::upd_ignore,
            updates::upd_skip,
            updates::upd_unhide_all,
            updates::upd_majors,
            updates::upd_running_apps,
            updates::upd_run,
            updates::upd_stop,
            updates::upd_notes,
            updates::upd_rollback,
            updates::upd_open_nvidia,
            cleanup::cln_state,
            cleanup::cln_scan,
            cleanup::cln_scan_admin,
            cleanup::cln_tick,
            cleanup::cln_forget_ticks,
            cleanup::cln_clean,
            cleanup::cln_stop,
            cleanup::cln_protect,
        ])
        .setup(|app| {
            let s = app.state::<AppState>().settings.lock().unwrap().clone();
            let w = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Safe Toolbox")
                .inner_size(1280.0, 860.0)
                .min_inner_size(960.0, 640.0)
                .transparent(true)
                .visible(false)
                .build()?;
            apply_window(&w, &s);
            restore_bounds(&w, &s);
            w.show()?;
            std::thread::spawn(move || {
                register_notifications();
                if let Some(e) = schedule::repair(&s, &exe_path()) {
                    toolbox::write_error(&format!("Could not update the background task: {e}"));
                }
            });
            Ok(())
        })
        .on_window_event(|w, e| {
            if let WindowEvent::CloseRequested { api, .. } = e {
                let st = w.state::<AppState>();
                let reason = st.busy.lock().unwrap().clone();
                if let Some(reason) = reason.filter(|_| !st.force_close.load(Ordering::SeqCst)) {
                    api.prevent_close();
                    let _ = w.emit("close-blocked", reason);
                    return;
                }
                save_bounds(w, &st);
            }
        })
        .run(tauri::generate_context!())
        .expect("Safe Toolbox could not start");
}
