//! Commands for Home, Health, Repair, PATH doctor, Big files, App list, History and Settings, plus the window's
//! own: settings, accent, opening things, and closing while work runs.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use toolbox_core::proc::Cancel;
use toolbox_core::toolbox::{self, ToolboxSettings, admin_tasks, app_list, background, big_files, health, history, notify, path_doctor, repairs, schedule};
use toolbox_core::tools::is_elevated;

use crate::state::AppState;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ window

#[tauri::command]
pub fn app_info(st: State<AppState>) -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"), "isAdmin": is_elevated(), "exe": crate::exe_path(),
        "startPage": st.start_page.lock().unwrap().take(), "drive": toolbox_core::tools::system_drive(),
    })
}

fn windows_shows_accent() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\DWM")
        .and_then(|k| k.get_value::<u32, _>("ColorPrevalence"))
        .is_ok_and(|v| v == 1)
}

/// Windows' own seven accent shades, and whether Windows shows accent colour on title bars.
#[tauri::command]
pub fn accent() -> Value {
    use windows::UI::ViewManagement::{UIColorType, UISettings};
    let ui = UISettings::new().ok();
    let hex = |t: UIColorType| ui.as_ref().and_then(|u| u.GetColorValue(t).ok()).map(|c| format!("#{:02x}{:02x}{:02x}", c.R, c.G, c.B));
    json!({
        "windowsShows": windows_shows_accent(),
        "ramp": {
            "accent": hex(UIColorType::Accent), "accent-light-1": hex(UIColorType::AccentLight1), "accent-light-2": hex(UIColorType::AccentLight2),
            "accent-light-3": hex(UIColorType::AccentLight3), "accent-dark-1": hex(UIColorType::AccentDark1), "accent-dark-2": hex(UIColorType::AccentDark2),
            "accent-dark-3": hex(UIColorType::AccentDark3),
        }
    })
}

#[tauri::command]
pub fn get_settings(st: State<AppState>) -> ToolboxSettings {
    st.settings.lock().unwrap().clone()
}

/// Saves the settings, applies theme and backdrop, and creates, changes or removes the background task when its
/// schedule changed. Returns Task Scheduler's complaint, if any.
#[tauri::command]
pub async fn set_settings(app: AppHandle, settings: ToolboxSettings) -> Result<Option<String>, String> {
    let st = app.state::<AppState>();
    let old = {
        let mut s = st.settings.lock().unwrap();
        let old = s.clone();
        // window placement belongs to the window, not the page
        let mut next = settings.clone();
        next.window_bounds = s.window_bounds.clone();
        next.window_maximized = s.window_maximized;
        *s = next;
        s.save();
        old
    };
    if old.theme != settings.theme || old.backdrop != settings.backdrop {
        if let Some(w) = app.get_webview_window("main") {
            crate::apply_window(&w, &settings);
        }
    }
    if old.update_check != settings.update_check || old.cleanup_reminder != settings.cleanup_reminder {
        return blocking(move || schedule::sync(&settings, &crate::exe_path()).map(|e| format!("Task Scheduler didn't take the change: {e}"))).await;
    }
    Ok(None)
}

#[tauri::command]
pub fn force_close(app: AppHandle) {
    let st = app.state::<AppState>();
    st.force_close.store(true, Ordering::SeqCst);
    if let Some(s) = st.upd.lock().unwrap_or_else(|e| e.into_inner()).stop_handle() {
        s.request();
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.close();
    }
}

/// Files, folders, https links and the ms-settings: pages this app links to.
#[tauri::command]
pub fn open(target: String) -> Result<(), String> {
    let ok_scheme = ["https:", "ms-settings:", "windowsdefender:", "shell:"].iter().any(|p| target.starts_with(p));
    if !ok_scheme && !Path::new(&target).exists() {
        return Err(format!("{target} isn't there."));
    }
    if toolbox_core::shell::open(&target) { Ok(()) } else { Err("Windows couldn't open it.".into()) }
}

#[tauri::command]
pub fn reveal(path: String) {
    toolbox_core::shell::reveal(&path);
}

/// Windows tools that need a program and arguments, and this app's own folders and logs.
#[tauri::command]
pub fn open_tool(name: String) -> Result<(), String> {
    let sys = toolbox_core::tools::system_dir();
    let run = |exe: &str, args: &[&str]| std::process::Command::new(sys.join(exe)).args(args).spawn().map(|_| ()).map_err(|e| format!("Couldn't open it: {e}"));
    let open_path = |p: std::path::PathBuf| open(p.to_string_lossy().into_owned());
    match name.as_str() {
        "reliability" => run("perfmon.exe", &["/rel"]),
        "sysinfo" => run("msinfo32.exe", &[]),
        "env" => run("rundll32.exe", &["sysdm.cpl,EditEnvironmentVariables"]),
        "settings-folder" => open_path(toolbox::root()),
        "update-logs" => open_path(toolbox_core::updater::storage::logs()),
        "update-runs" => open_path(toolbox_core::updater::storage::runs_csv()),
        "clean-log" => open(toolbox_core::cleaner::engine::log_path()).map_err(|_| "No log yet. It is created the first time you clean.".into()),
        "protect" => open(toolbox_core::cleaner::protect::ensure_file(None)),
        "exports" => open_path(toolbox::exports()),
        _ => Err(format!("Unknown tool {name}")),
    }
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Option<String> {
    blocking(move || app.dialog().file().blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned())).await.ok().flatten()
}

// ------------------------------------------------------------------ home and settings

fn frequency_word(f: toolbox::Frequency) -> &'static str {
    match f {
        toolbox::Frequency::Daily => "daily",
        toolbox::Frequency::Weekly => "weekly",
        toolbox::Frequency::Off => "off",
    }
}

#[tauri::command]
pub async fn home_state(app: AppHandle) -> Result<Value, String> {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    let live = {
        let st = app.state::<AppState>();
        let u = st.upd.lock().unwrap();
        u.checked().then(|| json!({ "count": u.safe_count(), "names": u.safe_names(), "leftOut": u.left_out() }))
    };
    blocking(move || {
        let titles = rule_titles();
        let runs = history::read_all(&|id| titles.iter().find(|(i, _)| i == id).map_or(id.to_string(), |(_, t)| t.clone()));
        let problem = schedule::task_frequency(&settings).and_then(|_| schedule::last_problem());
        json!({
            "summary": background::CheckSummary::load(), "live": live,
            "drive": toolbox_core::disk::system_drive_usage().map(|(t, f)| json!({ "total": t, "free": f })),
            "recent": runs.into_iter().take(3).collect::<Vec<_>>(),
            "background": { "on": schedule::task_frequency(&settings).is_some(), "updates": frequency_word(settings.update_check), "cleanup": frequency_word(settings.cleanup_reminder), "problem": problem },
        })
    })
    .await
}

#[tauri::command]
pub async fn task_status(app: AppHandle) -> Result<Value, String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    blocking(move || {
        json!({
            "wanted": schedule::task_frequency(&s).map(frequency_word), "exists": schedule::exists(), "problem": schedule::last_problem(),
            "lastWhen": background::CheckSummary::load().and_then(|c| c.when), "taskName": schedule::TASK_NAME,
        })
    })
    .await
}

#[tauri::command]
pub async fn test_notification() -> Result<(), String> {
    blocking(|| {
        crate::register_notifications();
        notify::show("Safe Toolbox", "Notifications from Safe Toolbox are working.", "settings")
    })
    .await?
}

/// "Check now in the background": the scheduled task's work, run now; shows the notification it would show.
#[tauri::command]
pub async fn background_now(app: AppHandle) -> Result<Value, String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    blocking(move || {
        let summary = background::run(true, true, &s, &Cancel::new());
        let note = background::notification(&summary, &s, true, true);
        if let Some((t, b)) = &note {
            crate::register_notifications();
            let _ = notify::show(t, b, if summary.updates.is_empty() { "cleanup" } else { "updates" });
        }
        json!({ "summary": summary, "notification": note })
    })
    .await
}

#[tauri::command]
pub fn upd_settings(st: State<AppState>) -> Value {
    let mut u = st.upd.lock().unwrap();
    let s = u.settings();
    json!({ "testAfterUpdate": s.test_after_update, "restorePoint": s.restore_point, "ignored": s.ignored.len(), "skipped": s.skipped.len() })
}

#[tauri::command]
pub fn set_upd_setting(st: State<AppState>, name: String, on: bool) {
    let mut u = st.upd.lock().unwrap();
    let s = u.settings();
    match name.as_str() {
        "testAfterUpdate" => s.test_after_update = on,
        "restorePoint" => s.restore_point = on,
        _ => return,
    }
    s.save();
}

// ------------------------------------------------------------------ health

fn crash_json(c: &health::Crash) -> Value {
    let code = c.stop_code;
    json!({
        "when": c.when, "kind": c.kind, "source": c.source, "stopCodeHex": c.stop_code_hex(), "stopName": c.stop_name(),
        "hint": code.and_then(health::stop_codes::hint), "link": code.map(health::stop_codes::link),
    })
}

#[tauri::command]
pub async fn health_snapshot() -> Result<Value, String> {
    blocking(|| {
        let (crashes, apps, (drives, antivirus), batteries) = std::thread::scope(|s| {
            let a = s.spawn(|| health::crashes(90));
            let b = s.spawn(|| health::app_crashes(14));
            let c = s.spawn(health::drives_and_antivirus);
            let d = s.spawn(health::batteries);
            (a.join().unwrap_or_default(), b.join().unwrap_or_default(), c.join().unwrap_or_default(), d.join().unwrap_or_default())
        });
        json!({
            "taken": toolbox_core::time::now(), "crashes": crashes.iter().map(crash_json).collect::<Vec<_>>(), "appCrashes": apps,
            "drives": drives, "antivirus": antivirus,
            "batteries": batteries.iter().map(|b| json!({ "name": b.name, "percent": b.percent(), "cycles": b.cycle_count, "design": b.design_capacity, "full": b.full_charge_capacity })).collect::<Vec<_>>(),
            "uptimeSecs": health::uptime().as_secs(), "restartPending": health::restart_pending(), "isAdmin": is_elevated(),
        })
    })
    .await
}

#[tauri::command]
pub async fn health_wear() -> Result<Vec<health::DriveWear>, String> {
    let out = blocking(|| admin_tasks::run_elevated("drive-wear")).await??;
    Ok(health::parse_drive_wear(toolbox_core::proc::last_json_line(&out, true).unwrap_or(out.trim())))
}

#[tauri::command]
pub async fn health_battery_report() -> Result<(), String> {
    let path = blocking(health::battery_report).await?.ok_or("Windows couldn't make the report.")?;
    open(path)
}

#[tauri::command]
pub async fn health_restore_point() -> Result<String, String> {
    let out = blocking(|| admin_tasks::run_elevated("restore-point")).await??;
    Ok(toolbox_core::updater::sources::preflight::describe_restore_point(&out))
}

// ------------------------------------------------------------------ repair

#[tauri::command]
pub fn repair_list() -> Value {
    let all = repairs::all();
    json!(all.iter().map(|r| json!({
        "id": r.id, "title": r.title, "what": r.what, "heads": r.heads, "needsAdmin": r.needs_admin, "restartAfter": r.restart_after,
        "steps": r.steps.iter().map(|s| json!({ "label": s.label, "command": repairs::describe(s) })).collect::<Vec<_>>(),
    })).collect::<Vec<_>>())
}

/// Runs a repair, streaming its output as "repair-line" and ending with "repair-done".
#[tauri::command]
pub fn repair_run(app: AppHandle, id: String) -> Result<(), String> {
    let r = repairs::find(&id).ok_or("Unknown repair")?;
    let st = app.state::<AppState>();
    if st.repair_running.swap(true, Ordering::SeqCst) {
        return Err("A repair is already running.".into());
    }
    st.set_busy(Some("A repair is still running, and its output would be lost."));
    std::thread::spawn(move || {
        let line = |l: &str| {
            let _ = app.emit("repair-line", l.to_string());
        };
        let (ok, declined) = if r.needs_admin && !is_elevated() {
            let _ = app.emit("repair-phase", "Waiting for you to allow admin access…");
            match repairs::launch_elevated(&r.id) {
                Err(e) if e == "declined" => (false, true),
                Err(e) => {
                    line(&format!("!! {e}"));
                    (false, false)
                }
                Ok((p, log)) => {
                    let _ = app.emit("repair-phase", "Running…");
                    let mut seen = 0;
                    let mut ok = false;
                    loop {
                        let exited = p.has_exited();
                        let text = std::fs::read_to_string(&log).unwrap_or_default();
                        let lines: Vec<&str> = text.lines().collect();
                        for l in lines.iter().skip(seen) {
                            match *l {
                                "##EXIT ok" => ok = true,
                                "##EXIT fail" => {}
                                l => line(l),
                            }
                        }
                        seen = lines.len();
                        if exited {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    (ok, false)
                }
            }
        } else {
            let _ = app.emit("repair-phase", "Running…");
            (repairs::run(&r, &line, None), false)
        };
        let st = app.state::<AppState>();
        st.repair_running.store(false, Ordering::SeqCst);
        st.set_busy(None);
        let _ = app.emit("repair-done", json!({ "id": r.id, "ok": ok, "declined": declined, "restart": ok && r.restart_after }));
    });
    Ok(())
}

// ------------------------------------------------------------------ PATH doctor

#[tauri::command]
pub async fn path_state() -> Result<Value, String> {
    blocking(|| {
        let entries = path_doctor::analyze_system();
        let commands: Vec<Value> = path_doctor::commands(&entries)
            .iter()
            .map(|h| json!({ "command": h.command, "runs": h.locations[0], "others": &h.locations[1..], "storeAlias": path_doctor::is_store_alias(&h.locations[0]) }))
            .collect();
        json!({
            "entries": entries, "commands": commands, "broken": path_doctor::broken_winget_links(),
            "backup": path_doctor::latest_backup().map(|b| Path::new(&b).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(b)),
        })
    })
    .await
}

#[tauri::command]
pub async fn path_fix() -> Result<String, String> {
    blocking(|| path_doctor::fix_user_path(&path_doctor::analyze_system())).await?
}

#[tauri::command]
pub async fn path_undo() -> Result<String, String> {
    blocking(|| {
        let b = path_doctor::latest_backup().ok_or("No PATH fix has been made from here yet.")?;
        path_doctor::restore(&b)?;
        let _ = std::fs::rename(&b, format!("{b}.restored"));
        Ok(b)
    })
    .await?
}

// ------------------------------------------------------------------ big files

#[tauri::command]
pub async fn big_find(app: AppHandle, min_mb: u64) -> Result<Value, String> {
    blocking(move || {
        let last = std::sync::Mutex::new(std::time::Instant::now());
        let report = |dir: &str| {
            let mut l = last.lock().unwrap();
            if l.elapsed() > Duration::from_millis(150) {
                *l = std::time::Instant::now();
                let _ = app.emit("big-progress", dir.to_string());
            }
        };
        let files = big_files::find(&big_files::default_roots(), min_mb * 1024 * 1024, 300, None, Some(&report));
        json!({ "files": files, "downloads": toolbox_core::cleaner::paths::K.downloads(), "taken": toolbox_core::time::now() })
    })
    .await
}

#[tauri::command]
pub async fn big_recycle(path: String) -> Result<Option<String>, String> {
    blocking(move || big_files::recycle(&path, &toolbox_core::cleaner::guard::GuardConfig::from_system().in_use)).await
}

// ------------------------------------------------------------------ app list

#[tauri::command]
pub async fn applist_save(include_versions: bool) -> Result<app_list::ExportResult, String> {
    blocking(move || {
        let folder = toolbox_core::tools::home().join("Documents").join("Safe Toolbox").join("App lists").join(chrono::Local::now().format("%Y-%m-%d %H%M").to_string());
        app_list::export(&folder.to_string_lossy(), include_versions, None)
    })
    .await?
}

#[tauri::command]
pub async fn applist_pick(app: AppHandle) -> Result<Option<Value>, String> {
    blocking(move || {
        let Some(p) = app.dialog().file().add_filter("winget app list", &["json"]).blocking_pick_file().and_then(|p| p.into_path().ok()) else { return Ok(None) };
        let text = std::fs::read_to_string(&p).map_err(|_| "That file couldn't be read.".to_string())?;
        let ids = app_list::winget_ids(&text);
        if ids.is_empty() {
            return Err("That file has no winget packages in it.".into());
        }
        Ok(Some(json!({ "file": p.to_string_lossy(), "name": p.file_name().map(|n| n.to_string_lossy().into_owned()), "ids": ids })))
    })
    .await?
}

#[tauri::command]
pub fn applist_import(file: String) -> bool {
    app_list::start_import(&file)
}

// ------------------------------------------------------------------ history

fn rule_titles() -> Vec<(String, String)> {
    toolbox_core::cleaner::rules::build().into_iter().map(|r| (r.id, r.title)).collect()
}

#[tauri::command]
pub async fn history() -> Result<Vec<history::HistoryRun>, String> {
    blocking(|| {
        let titles = rule_titles();
        history::read_all(&|id| titles.iter().find(|(i, _)| i == id).map_or(id.to_string(), |(_, t)| t.clone()))
    })
    .await
}
