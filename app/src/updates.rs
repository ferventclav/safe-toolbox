//! The Updates page: check every source at once, apply the safety rules, and run the selected updates.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use toolbox_core::proc::Cancel;
use toolbox_core::updater::model::{ItemStatus, SourceResult, UpdateItem, UpdateSource};
use toolbox_core::updater::run::{StopHandle, UpdateRun, restore_snapshot};
use toolbox_core::updater::sources::preflight;
use toolbox_core::updater::storage::Settings;
use toolbox_core::updater::{checker, nvidia, policy, release_notes};

use crate::state::AppState;

const BUSY: &str = "Updates are still installing. Anything not started yet won't run, and this run won't appear in History.";

#[derive(Default)]
pub struct Upd {
    settings: Option<Settings>,
    results: BTreeMap<UpdateSource, SourceResult>,
    pending: usize,
    checking: bool,
    items: Vec<UpdateItem>,
    /// Ticks you changed by hand, kept across re-applying the rules.
    manual: HashMap<String, bool>,
    running: bool,
    stop: Option<StopHandle>,
    warnings: Vec<String>,
    rustup: Option<bool>,
}

impl Upd {
    pub fn settings(&mut self) -> &mut Settings {
        self.settings.get_or_insert_with(Settings::load)
    }

    fn reapply(&mut self) {
        let choco = self.results.get(&UpdateSource::Chocolatey).map(|r| r.managed.clone()).unwrap_or_default();
        let rustup = *self.rustup.get_or_insert_with(|| toolbox_core::tools::rustup().is_some());
        let ctx = policy::PolicyContext::from(self.settings(), choco, rustup);
        policy::apply_all(&mut self.items, &ctx);
        for i in self.items.iter_mut() {
            if let Some(&on) = self.manual.get(&i.key()) {
                if i.selectable && !i.hidden {
                    i.selected = on;
                }
            }
        }
    }

    pub fn safe_count(&self) -> usize {
        self.items.iter().filter(|i| i.selectable && !i.hidden && i.status != ItemStatus::Done).count()
    }

    pub fn checked(&self) -> bool {
        !self.results.is_empty() && !self.checking
    }

    pub fn safe_names(&self) -> Vec<String> {
        self.items.iter().filter(|i| i.selectable && !i.hidden && i.status != ItemStatus::Done).map(UpdateItem::display_name).collect()
    }

    pub fn left_out(&self) -> usize {
        self.items.iter().filter(|i| i.hidden).count()
    }

    pub fn stop_handle(&self) -> Option<&StopHandle> {
        self.stop.as_ref()
    }
}

fn item_json(i: &UpdateItem) -> Value {
    let mut v = serde_json::to_value(i).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        m.insert("key".into(), i.key().into());
        m.insert("displayName".into(), i.display_name().into());
        m.insert("command".into(), checker::describe_command(i).into());
    }
    v
}

fn view(u: &mut Upd) -> Value {
    let s = u.settings().clone();
    let sources: Vec<Value> = UpdateSource::ALL
        .iter()
        .map(|&src| {
            let r = u.results.get(&src);
            json!({
                "source": src, "title": src.title(), "describe": src.describe(), "done": r.is_some(),
                "absent": r.is_some_and(|r| r.absent), "failed": r.is_some_and(|r| r.failed), "message": r.and_then(|r| r.message.clone()),
            })
        })
        .collect();
    json!({
        "checking": u.checking, "pending": u.pending, "total": UpdateSource::ALL.len(), "checked": u.checked(), "running": u.running,
        "sources": sources, "items": u.items.iter().map(item_json).collect::<Vec<_>>(), "warnings": u.warnings,
        "includeMajors": s.include_majors, "lastChecked": s.last_checked, "lastUpdated": s.last_updated,
        "canRollback": s.last_pip_snapshot.as_deref().is_some_and(|p| std::path::Path::new(p).exists()),
        "isAdmin": toolbox_core::tools::is_elevated(),
    })
}

fn emit(app: &AppHandle) {
    let st = app.state::<AppState>();
    let v = view(&mut st.upd.lock().unwrap());
    let _ = app.emit("upd", v);
}

#[tauri::command]
pub fn upd_state(st: State<AppState>) -> Value {
    view(&mut st.upd.lock().unwrap())
}

#[tauri::command]
pub fn upd_check(app: AppHandle) {
    {
        let st = app.state::<AppState>();
        let mut u = st.upd.lock().unwrap();
        if u.checking || u.running {
            return;
        }
        u.checking = true;
        u.pending = UpdateSource::ALL.len();
        u.results.clear();
        u.items.clear();
        u.warnings.clear();
        u.settings = Some(Settings::load());
    }
    emit(&app);
    std::thread::spawn(move || {
        let a = app.clone();
        checker::check_all(&Cancel::new(), &move |mut r: SourceResult| {
            let st = a.state::<AppState>();
            {
                let mut u = st.upd.lock().unwrap();
                let items = std::mem::take(&mut r.items);
                u.items.extend(items);
                if let Some(src) = r.source {
                    u.results.insert(src, r);
                }
                u.pending = u.pending.saturating_sub(1);
                u.reapply();
            }
            emit(&a);
        });
        let warnings = preflight::warnings();
        {
            let st = app.state::<AppState>();
            let mut u = st.upd.lock().unwrap();
            u.checking = false;
            u.warnings = warnings;
            let s = u.settings();
            s.last_checked = Some(toolbox_core::time::now());
            s.save();
        }
        emit(&app);
    });
}

fn with_item(app: &AppHandle, key: &str, f: impl FnOnce(&mut Upd, UpdateItem)) {
    let st = app.state::<AppState>();
    {
        let mut u = st.upd.lock().unwrap();
        let Some(item) = u.items.iter().find(|i| i.key() == key).cloned() else { return };
        f(&mut u, item);
        u.reapply();
    }
    emit(app);
}

#[tauri::command]
pub fn upd_select(app: AppHandle, keys: Vec<String>, on: bool) {
    let st = app.state::<AppState>();
    {
        let mut u = st.upd.lock().unwrap();
        for k in keys {
            u.manual.insert(k.clone(), on);
            if let Some(i) = u.items.iter_mut().find(|i| i.key() == k && i.selectable && !i.hidden) {
                i.selected = on;
            }
        }
    }
    emit(&app);
}

/// "Don't show this update" (every version) or "Show again".
#[tauri::command]
pub fn upd_ignore(app: AppHandle, key: String, ignored: bool) {
    with_item(&app, &key, |u, i| {
        let s = u.settings();
        if ignored {
            s.ignored.insert(i.ignore_key());
        } else {
            s.unignore(&i.ignore_key());
        }
        s.save();
    });
}

/// "Skip this version": hides this version only; the next one is listed again.
#[tauri::command]
pub fn upd_skip(app: AppHandle, key: String) {
    with_item(&app, &key, |u, i| {
        let s = u.settings();
        s.skipped.insert(i.ignore_key(), i.target_version.clone().unwrap_or(i.available.clone()));
        s.save();
    });
}

#[tauri::command]
pub fn upd_unhide_all(app: AppHandle) -> usize {
    let st = app.state::<AppState>();
    let n = {
        let mut u = st.upd.lock().unwrap();
        let s = u.settings();
        let n = s.ignored.len() + s.skipped.len();
        s.ignored.clear();
        s.skipped.clear();
        s.save();
        u.reapply();
        n
    };
    emit(&app);
    n
}

#[tauri::command]
pub fn upd_majors(app: AppHandle, on: bool) {
    let st = app.state::<AppState>();
    {
        let mut u = st.upd.lock().unwrap();
        u.settings().include_majors = on;
        u.settings().save();
        u.reapply();
    }
    emit(&app);
}

/// Open apps whose installer is about to run, so you can save and close them first.
#[tauri::command]
pub fn upd_running_apps(st: State<AppState>, keys: Vec<String>) -> Vec<String> {
    let u = st.upd.lock().unwrap();
    let set: HashSet<&String> = keys.iter().collect();
    checker::running_apps(u.items.iter().filter(|i| set.contains(&i.key())), &toolbox_core::processes::names())
}

#[tauri::command]
pub fn upd_run(app: AppHandle, keys: Vec<String>) -> Result<(), String> {
    let st = app.state::<AppState>();
    let (items, settings, stop) = {
        let mut u = st.upd.lock().unwrap();
        if u.running || u.checking {
            return Err("Wait for the current check or run to finish.".into());
        }
        let set: HashSet<&String> = keys.iter().collect();
        let items: Vec<UpdateItem> = u.items.iter().filter(|i| set.contains(&i.key()) && i.selectable && !i.hidden && i.status != ItemStatus::Done).cloned().collect();
        if items.is_empty() {
            return Err("Nothing selected.".into());
        }
        u.running = true;
        let stop = StopHandle::default();
        u.stop = Some(stop.clone());
        (items, u.settings().clone(), stop)
    };
    st.set_busy(Some(BUSY));
    emit(&app);
    std::thread::spawn(move || {
        let a = app.clone();
        let changed = move |i: &UpdateItem| {
            let st = a.state::<AppState>();
            if let Some(x) = st.upd.lock().unwrap().items.iter_mut().find(|x| x.key() == i.key()) {
                *x = i.clone();
            }
            emit(&a);
        };
        let b = app.clone();
        let status = move |s: &str| {
            let _ = b.emit("upd-status", s.to_string());
        };
        let mut run = UpdateRun::new(settings, stop, &changed, &status);
        let (done, summary) = run.run(items);
        run.settings.save();
        let st = app.state::<AppState>();
        {
            let mut u = st.upd.lock().unwrap();
            for d in done {
                if let Some(x) = u.items.iter_mut().find(|x| x.key() == d.key()) {
                    *x = d;
                }
            }
            u.settings = Some(run.settings.clone());
            u.running = false;
            u.stop = None;
        }
        st.set_busy(None);
        let _ = app.emit("upd-done", &summary);
        emit(&app);
    });
    Ok(())
}

/// "Stop after this update": the installer that is running finishes, nothing new starts.
#[tauri::command]
pub fn upd_stop(st: State<AppState>) {
    if let Some(s) = st.upd.lock().unwrap().stop.as_ref() {
        s.request();
    }
}

#[tauri::command]
pub async fn upd_notes(app: AppHandle, key: String) -> Vec<Value> {
    let item = app.state::<AppState>().upd.lock().unwrap().items.iter().find(|i| i.key() == key).cloned();
    let Some(i) = item else { return Vec::new() };
    tauri::async_runtime::spawn_blocking(move || {
        let mut links = if i.source == UpdateSource::Apps { release_notes::winget(&i.id) } else { Vec::new() };
        links.extend(release_notes::for_item(&i));
        let mut seen = HashSet::new();
        links.into_iter().filter(|l| seen.insert(l.url.clone())).map(|l| json!({ "label": l.label, "url": l.url })).collect()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
pub async fn upd_rollback(app: AppHandle) -> Result<String, String> {
    let snap = app.state::<AppState>().upd.lock().unwrap().settings().last_pip_snapshot.clone();
    let Some(snap) = snap.filter(|p| std::path::Path::new(p).exists()) else { return Err("No Python update has run from here yet, so there is no snapshot.".into()) };
    app.state::<AppState>().set_busy(Some("Python packages are being rolled back."));
    let r = tauri::async_runtime::spawn_blocking(move || restore_snapshot(&snap)).await.map_err(|e| e.to_string())?;
    app.state::<AppState>().set_busy(None);
    if r.ok() { Ok("Python packages rolled back".into()) } else { Err(r.tail(6).join("\n")) }
}

#[tauri::command]
pub fn upd_open_nvidia() -> bool {
    nvidia::open_app()
}
