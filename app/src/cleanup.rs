//! The Clean up page: scan, the admin scan of the Windows group, ticks, and cleaning with a measured result.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use toolbox_core::cleaner::admin::{self, AdminJob, AdminResult};
use toolbox_core::cleaner::engine::{self, CleanResult};
use toolbox_core::cleaner::guard::GuardConfig;
use toolbox_core::cleaner::{KEPT, Rule, RuleGroup, protect};
use toolbox_core::proc::Cancel;
use toolbox_core::tools::is_elevated;

use crate::state::AppState;

const BUSY: &str = "Cleaning is still running and would stop partway, without a History entry.";

#[derive(Default)]
pub struct Cln {
    rules: Vec<Rule>,
    scanned: bool,
    scanning: bool,
    cleaning: bool,
    elapsed: f64,
    admin: bool,
    guard_folders: usize,
    stop: Option<Cancel>,
    stop_file: Option<String>,
}

fn group_id(g: RuleGroup) -> &'static str {
    match g {
        RuleGroup::Leftovers => "Leftovers",
        RuleGroup::Apps => "Apps",
        RuleGroup::YourCall => "YourCall",
        RuleGroup::Windows => "Windows",
    }
}

fn view(c: &Cln, st: &AppState) -> Value {
    let s = st.settings.lock().unwrap().clone();
    let rules: Vec<Value> = c
        .rules
        .iter()
        .filter(|r| r.visible() || r.needs_admin)
        .map(|r| {
            json!({
                "id": r.id, "group": group_id(r.group), "title": r.title, "why": r.why, "needsAdmin": r.needs_admin,
                "scannedElevated": r.scanned_elevated, "ticked": s.cleanup_tick(&r.id, r.default_checked), "defaultChecked": r.default_checked,
                "total": r.total(), "unknownSize": r.has_unknown_size(), "canClean": r.can_clean(), "blocked": r.blocked,
                "items": r.items.iter().take(60).map(|i| json!({ "label": i.label, "path": i.path, "size": i.size, "result": i.result })).collect::<Vec<_>>(),
                "itemCount": r.items.len(), "handlers": r.items.iter().any(|i| i.handler.is_some()),
                "skipped": r.skipped.iter().take(60).map(|k| json!({ "what": k.what, "why": k.why })).collect::<Vec<_>>(),
                "skippedCount": r.skipped.len(),
            })
        })
        .collect();
    let groups: Vec<Value> = RuleGroup::ORDER.iter().map(|&g| json!({ "id": group_id(g), "title": g.title(), "blurb": g.blurb(c.admin) })).collect();
    json!({
        "scanned": c.scanned, "scanning": c.scanning, "cleaning": c.cleaning, "elapsed": c.elapsed, "admin": c.admin,
        "rules": rules, "groups": groups, "guardFolders": c.guard_folders,
        "kept": KEPT.iter().map(|(a, b)| json!({ "what": a, "why": b })).collect::<Vec<_>>(),
        "drive": toolbox_core::disk::system_drive_usage().map(|(t, f)| json!({ "total": t, "free": f })),
        "lastCleaned": engine::read_last(None).map(|(t, f)| json!({ "when": t, "freed": f })),
        "remember": s.remember_cleanup_ticks,
    })
}

fn emit(app: &AppHandle) {
    let st = app.state::<AppState>();
    let v = view(&st.cln.lock().unwrap(), &st);
    let _ = app.emit("cln", v);
}

fn progress(app: &AppHandle, done: usize, total: usize, current: &str) {
    let _ = app.emit("cln-progress", json!({ "done": done, "total": total, "current": current }));
}

#[tauri::command]
pub fn cln_state(st: State<AppState>) -> Value {
    view(&st.cln.lock().unwrap(), &st)
}

#[tauri::command]
pub fn cln_scan(app: AppHandle) {
    {
        let st = app.state::<AppState>();
        let mut c = st.cln.lock().unwrap();
        if c.scanning || c.cleaning {
            return;
        }
        c.scanning = true;
    }
    emit(&app);
    std::thread::spawn(move || {
        let a = app.clone();
        let last = std::sync::Mutex::new(Instant::now() - Duration::from_secs(1));
        let report = move |d: usize, t: usize, cur: &str| {
            let mut l = last.lock().unwrap();
            if l.elapsed() > Duration::from_millis(120) || d == t {
                *l = Instant::now();
                progress(&a, d, t, cur);
            }
        };
        let admin = is_elevated();
        let scan = engine::scan(admin, &report, None);
        let folders = GuardConfig::from_system().in_use.len();
        {
            let st = app.state::<AppState>();
            let mut c = st.cln.lock().unwrap();
            c.rules = scan.rules;
            c.elapsed = scan.elapsed.as_secs_f64();
            c.admin = admin;
            c.guard_folders = folders;
            c.scanned = true;
            c.scanning = false;
        }
        emit(&app);
    });
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Runs the elevated helper and waits, following its progress file. Err("declined") when admin was refused.
fn run_admin_job(app: &AppHandle, mode: &str, rule_ids: Vec<String>, stop_file: Option<String>) -> Result<AdminResult, String> {
    let jobs = toolbox_core::toolbox::jobs();
    let s = stamp();
    let file = |n: &str| jobs.join(format!("clean-{n}-{s}.{}", if n == "job" || n == "result" { "json" } else { "txt" })).to_string_lossy().into_owned();
    let job = AdminJob { mode: mode.into(), rule_ids, result_file: file("result"), progress_file: file("progress"), log_file: Some(engine::log_path()), stop_file };
    let job_path = jobs.join(format!("clean-job-{s}.json"));
    std::fs::write(&job_path, serde_json::to_string(&job).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let p = admin::launch(&job_path)?;
    let mut read = 0usize;
    loop {
        let exited = p.has_exited();
        if let Ok(text) = std::fs::read_to_string(&job.progress_file) {
            let lines: Vec<&str> = text.lines().collect();
            if let Some(last) = lines.get(read..).and_then(|l| l.last()) {
                let (n, cur) = last.split_once('\t').unwrap_or(("", last));
                let (d, t) = n.split_once('/').unwrap_or(("0", "0"));
                progress(app, d.parse().unwrap_or(0), t.parse().unwrap_or(0), cur);
            }
            read = lines.len();
        }
        if exited {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    for f in [&job.progress_file, &job_path.to_string_lossy().into_owned()] {
        let _ = std::fs::remove_file(f);
    }
    let r = admin::read_result(&job.result_file).ok_or("The admin helper didn't report back. Details may be in error.log.")?;
    let _ = std::fs::remove_file(&job.result_file);
    match &r.error {
        Some(e) => Err(e.clone()),
        None => Ok(r),
    }
}

/// Sizes the Windows group with admin. Ok(message) or Err(reason); "declined" when the prompt was refused.
#[tauri::command]
pub async fn cln_scan_admin(app: AppHandle) -> Result<String, String> {
    let ids: Vec<String> = app.state::<AppState>().cln.lock().unwrap().rules.iter().filter(|r| r.needs_admin).map(|r| r.id.clone()).collect();
    let a = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || run_admin_job(&a, "scan", ids, None)).await.map_err(|e| e.to_string())??;
    let total = {
        let st = app.state::<AppState>();
        let mut c = st.cln.lock().unwrap();
        admin::apply(&r, &mut c.rules);
        c.rules.iter().filter(|r| r.needs_admin && r.can_clean()).map(Rule::total).sum::<u64>()
    };
    emit(&app);
    Ok(format!("Windows group scanned: {} found.", toolbox_core::disk::format_size(total)))
}

#[tauri::command]
pub fn cln_tick(app: AppHandle, ids: Vec<String>, on: bool) {
    let st = app.state::<AppState>();
    {
        let c = st.cln.lock().unwrap();
        let mut s = st.settings.lock().unwrap();
        for id in &ids {
            if let Some(r) = c.rules.iter().find(|r| &r.id == id) {
                s.remember_tick(id, on, r.default_checked);
            }
        }
        s.save();
    }
    emit(&app);
}

#[tauri::command]
pub fn cln_forget_ticks(app: AppHandle) {
    let st = app.state::<AppState>();
    {
        let mut s = st.settings.lock().unwrap();
        s.cleanup_choices.clear();
        s.save();
    }
    emit(&app);
}

#[tauri::command]
pub fn cln_clean(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    let st = app.state::<AppState>();
    let elevated = is_elevated();
    let (mut local, helper, stop) = {
        let mut c = st.cln.lock().unwrap();
        if c.scanning || c.cleaning {
            return Err("Wait for the scan to finish.".into());
        }
        let set: HashSet<&String> = ids.iter().collect();
        let chosen: Vec<Rule> = c.rules.iter().filter(|r| set.contains(&r.id) && r.can_clean()).cloned().collect();
        if chosen.is_empty() {
            return Err("Nothing selected.".into());
        }
        let (helper, local): (Vec<Rule>, Vec<Rule>) = chosen.into_iter().partition(|r| r.needs_admin && !elevated);
        let stop = Cancel::new();
        c.cleaning = true;
        c.stop = Some(stop.clone());
        c.stop_file = (!helper.is_empty()).then(|| toolbox_core::toolbox::jobs().join(format!("clean-stop-{}.flag", stamp())).to_string_lossy().into_owned());
        (local, helper, stop)
    };
    st.set_busy(Some(BUSY));
    emit(&app);
    std::thread::spawn(move || {
        let st = app.state::<AppState>();
        let mut declined = false;
        let mut error = None;
        let mut result = CleanResult::default();
        if !helper.is_empty() {
            let _ = app.emit("cln-phase", "Cleaning Windows items…");
            let stop_file = st.cln.lock().unwrap().stop_file.clone();
            match run_admin_job(&app, "clean", helper.iter().map(|r| r.id.clone()).collect(), stop_file) {
                Ok(r) => {
                    result = CleanResult { freed: r.freed, items: r.items, problems: r.problems, stopped: r.stopped, logged: true };
                    admin::apply(&r, &mut st.cln.lock().unwrap().rules);
                }
                Err(e) if e == "declined" => declined = true,
                Err(e) => error = Some(e),
            }
        }
        if !local.is_empty() {
            let _ = app.emit("cln-phase", "Cleaning…");
            let a = app.clone();
            let report = move |d: usize, t: usize, cur: &str| progress(&a, d, t, cur);
            let r = engine::clean(&mut local, &report, false, elevated, None, Some(&stop));
            result.freed += r.freed;
            result.items += r.items;
            result.problems += r.problems;
            result.stopped += r.stopped;
            let mut c = st.cln.lock().unwrap();
            for done in local {
                if let Some(x) = c.rules.iter_mut().find(|x| x.id == done.id) {
                    x.items = done.items;
                }
            }
        }
        {
            let mut c = st.cln.lock().unwrap();
            c.cleaning = false;
            c.stop = None;
            if let Some(f) = c.stop_file.take() {
                let _ = std::fs::remove_file(f);
            }
        }
        st.set_busy(None);
        let _ = app.emit("cln-done", json!({ "freed": result.freed, "items": result.items, "problems": result.problems, "stopped": result.stopped, "declined": declined, "error": error, "helper": !helper.is_empty() }));
        emit(&app);
    });
    Ok(())
}

/// Stops after the item in progress, here and in the admin helper.
#[tauri::command]
pub fn cln_stop(st: State<AppState>) {
    let c = st.cln.lock().unwrap();
    if let Some(s) = &c.stop {
        s.cancel();
    }
    if let Some(f) = &c.stop_file {
        let _ = std::fs::write(f, "stop");
    }
}

/// Adds a folder to protect.txt. Err with a reason when the folder isn't there.
#[tauri::command]
pub fn cln_protect(folder: String) -> Result<String, String> {
    let f = toolbox_core::tools::expand(folder.trim().trim_matches('"'));
    if !Path::new(&f).is_dir() {
        return Err(format!("{f} isn't a folder on this PC."));
    }
    protect::add(&f, None)?;
    Ok(f)
}
