//! One "Update selected" press. Admin items go to the elevated helper in one batch (one UAC prompt); the
//! user-level tools (Scoop, npm, pip, rustup, claude) run here as the user, so their files stay the user's.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::dev::{PythonSource, normalize};
use super::model::{ItemStatus, RunSummary, UpdateItem, UpdateSource};
use super::sources::ScoopSource;
use super::storage::{self, Settings, append};
use super::winget::{self, WingetOutcome};
use super::worker::{self, JobItem, ProgressReader, WorkerJob};
use super::{path_health, smoke};
use crate::proc::{self, ProcResult, minutes};
use crate::time;
use crate::tools::{self, contains_ic};

/// "Stop after this update": the installer that is running finishes (stopping one halfway can break the app),
/// and nothing new starts, here or in the admin helper.
#[derive(Clone, Default)]
pub struct StopHandle {
    flag: Arc<AtomicBool>,
    file: Arc<Mutex<Option<PathBuf>>>,
}

impl StopHandle {
    pub fn request(&self) {
        self.flag.store(true, Ordering::SeqCst);
        if let Some(f) = self.file.lock().unwrap().as_ref() {
            let _ = std::fs::write(f, "stop");
        }
    }
    pub fn requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

pub type Changed<'a> = &'a (dyn Fn(&UpdateItem) + Sync);
pub type Status<'a> = &'a (dyn Fn(&str) + Sync);

pub struct UpdateRun<'a> {
    pub settings: Settings,
    pub stop: StopHandle,
    changed: Changed<'a>,
    status: Status<'a>,
    summary: RunSummary,
}

fn pip_install(scope: Option<&str>, upgrade: bool) -> Vec<String> {
    let mut a: Vec<String> = ["-m", "pip", "install", "--disable-pip-version-check", "--no-warn-script-location"].map(String::from).to_vec();
    if scope == Some("user") {
        a.push("--user".into());
    }
    if upgrade {
        a.push("--upgrade".into());
    }
    a
}

fn keep(item: &mut UpdateItem, r: &ProcResult) {
    item.output = r.tail(12);
}

impl<'a> UpdateRun<'a> {
    pub fn new(settings: Settings, stop: StopHandle, changed: Changed<'a>, status: Status<'a>) -> Self {
        UpdateRun { settings, stop, changed, status, summary: RunSummary::default() }
    }

    fn set(&self, item: &mut UpdateItem, status: ItemStatus, message: Option<&str>) {
        item.status = status;
        if let Some(m) = message {
            item.result = Some(m.to_string());
        }
        (self.changed)(item);
    }

    /// True (and the item marked as kept back) when a stop was asked for before it started.
    fn stopped(&self, item: &mut UpdateItem) -> bool {
        if !self.stop.requested() {
            return false;
        }
        self.set(item, ItemStatus::Skipped, Some("Stopped before it started"));
        true
    }

    fn log(&self, text: &str) {
        append(std::path::Path::new(&self.summary.log_file), text);
    }

    pub fn run(&mut self, requested: Vec<UpdateItem>) -> (Vec<UpdateItem>, RunSummary) {
        // Hidden items are unsafe by policy; never act on one, whatever the caller passed. One item per key.
        let mut seen = HashSet::new();
        let mut items: Vec<UpdateItem> = requested.into_iter().filter(|i| i.selectable && !i.hidden && seen.insert(i.key())).collect();
        let stamp = time::now().format("%Y%m%d-%H%M%S").to_string();
        self.summary = RunSummary { log_file: storage::new_run_log(&stamp).to_string_lossy().into_owned(), ..Default::default() };
        *self.stop.file.lock().unwrap() = Some(storage::jobs().join(format!("stop-{stamp}.flag")));
        if self.stop.requested() {
            self.stop.request();
        }
        self.log(&format!("Safe Toolbox update run {}, {} items", time::now().format("%Y-%m-%d %H:%M"), items.len()));
        for i in items.iter_mut() {
            i.result = None;
            i.output.clear();
            i.retry_locally = false;
            i.reboot_needed = false;
            self.set(i, ItemStatus::Queued, Some("Waiting"));
        }

        let admin: Vec<usize> = (0..items.len()).filter(|&k| items[k].needs_admin).collect();
        if !admin.is_empty() && !self.stop.requested() {
            self.run_admin(&mut items, &admin, &stamp);
            for &k in &admin {
                if items[k].retry_locally && items[k].source == UpdateSource::Apps && !self.stopped(&mut items[k]) {
                    self.retry_locally(&mut items[k]);
                }
            }
        }
        for k in 0..items.len() {
            if items[k].source == UpdateSource::Scoop && !self.stopped(&mut items[k]) {
                self.run_scoop(&mut items[k]);
            }
        }
        let npm = tools::npm();
        let node: Vec<usize> = (0..items.len()).filter(|&k| items[k].source == UpdateSource::Node).collect();
        for k in node {
            let args = vec!["install".to_string(), "-g".into(), format!("{}@{}", items[k].id, items[k].available)];
            self.run_simple(&mut items[k], npm.as_deref(), &args, 15);
            if items[k].status == ItemStatus::Done && items[k].output.iter().any(|l| contains_ic(l, "install-scripts")) {
                items[k].result = Some("Updated. npm skipped a dependency's install script (its allowScripts setting); if the tool misbehaves, reinstall it with --allow-scripts.".into());
                (self.changed)(&items[k]);
            }
        }
        let py: Vec<usize> = (0..items.len()).filter(|&k| items[k].source == UpdateSource::Python).collect();
        self.run_python(&mut items, &py, &stamp);
        let rustup = tools::rustup();
        let mut rust: Vec<usize> = (0..items.len()).filter(|&k| items[k].source == UpdateSource::Rust).collect();
        rust.sort_by_key(|&k| if items[k].id == "rustup" { 0 } else { 1 });
        for k in rust {
            let args: Vec<String> = if items[k].id == "rustup" { vec!["self".into(), "update".into()] } else { vec!["update".into(), items[k].id.clone()] };
            self.run_simple(&mut items[k], rustup.as_deref(), &args, 20);
        }
        let claude = tools::claude();
        let claude_items: Vec<usize> = (0..items.len()).filter(|&k| items[k].source == UpdateSource::ClaudeCode).collect();
        for k in claude_items {
            self.run_simple(&mut items[k], claude.as_deref(), &["update".to_string()], 10);
        }

        let now = time::now();
        for i in items.iter_mut() {
            if i.status == ItemStatus::Queued && self.stop.requested() {
                self.set(i, ItemStatus::Skipped, Some("Stopped before it started"));
            }
            if matches!(i.status, ItemStatus::Queued | ItemStatus::Running) {
                let msg = i.result.clone().unwrap_or_else(|| "Didn't run".into());
                self.set(i, ItemStatus::Failed, Some(&msg));
            }
            storage::record(i, &now);
            match i.status {
                ItemStatus::Done => self.summary.updated += 1,
                ItemStatus::Failed => self.summary.failed += 1,
                _ => self.summary.skipped += 1,
            }
            self.summary.reboot_recommended |= i.reboot_needed;
        }
        self.settings.last_updated = Some(now);
        self.settings.save();
        (items, std::mem::take(&mut self.summary))
    }

    fn run_admin(&mut self, items: &mut [UpdateItem], admin: &[usize], stamp: &str) {
        let job = WorkerJob {
            stamp: stamp.to_string(),
            items: admin.iter().map(|&k| JobItem::from(&items[k])).collect(),
            path: Some(path_health::take()),
            progress_file: storage::jobs().join(format!("progress-{stamp}.jsonl")).to_string_lossy().into_owned(),
            log_file: self.summary.log_file.clone(),
            requesting_user_sid: tools::current_user_sid(),
            restore_point: self.settings.restore_point,
            stop_file: self.stop.file.lock().unwrap().as_ref().map(|p| p.to_string_lossy().into_owned()),
        };
        let job_path = storage::jobs().join(format!("job-{stamp}.json"));
        let _ = std::fs::write(&job_path, serde_json::to_string(&job).unwrap_or_default());

        (self.status)("Waiting for you to allow admin access…");
        let p = match worker::launch(&job_path) {
            Ok(p) => p,
            Err(e) => {
                let declined = e == "declined";
                for &k in admin {
                    self.set(&mut items[k], ItemStatus::Skipped, Some(if declined { "Skipped: admin access was declined" } else { "Skipped: the admin helper didn't start" }));
                }
                if !declined {
                    storage::write_error(&format!("Admin helper didn't start: {e}"));
                }
                self.summary.notes.push(if declined {
                    "App, Chocolatey and Windows updates were skipped because admin access was declined.".into()
                } else {
                    format!("App, Chocolatey and Windows updates were skipped because the admin helper didn't start: {e}")
                });
                return;
            }
        };

        (self.status)("Updating apps and Windows…");
        let mut reader = ProgressReader::new(&job.progress_file);
        let mut finished = false;
        while !finished {
            std::thread::sleep(worker::POLL);
            let exited = p.has_exited();
            for e in reader.read_new() {
                if e.finished {
                    finished = true;
                    break;
                }
                match (e.key.as_deref(), e.message.as_deref()) {
                    (Some("status"), Some(m)) => {
                        (self.status)(m);
                        continue;
                    }
                    (Some("path-health" | "restore-point"), Some(m)) => {
                        self.summary.notes.push(m.to_string());
                        continue;
                    }
                    _ => {}
                }
                let Some(k) = admin.iter().copied().find(|&k| Some(items[k].key()) == e.key) else { continue };
                let item = &mut items[k];
                item.retry_locally = e.retry_locally;
                item.reboot_needed |= e.reboot;
                if let Some(o) = e.output {
                    item.output = o;
                }
                self.set(item, e.status, e.message.as_deref());
                if e.status == ItemStatus::Running {
                    (self.status)(&format!("Installing {}…", items[k].name));
                }
            }
            if exited && !finished {
                // One last read for anything written just before it exited.
                finished = true;
                for e in reader.read_new() {
                    if let Some(k) = admin.iter().copied().find(|&k| Some(items[k].key()) == e.key) {
                        items[k].retry_locally = e.retry_locally;
                        items[k].reboot_needed |= e.reboot;
                        if let Some(o) = e.output {
                            items[k].output = o;
                        }
                        self.set(&mut items[k], e.status, e.message.as_deref());
                    }
                }
            }
        }
        for &k in admin {
            if matches!(items[k].status, ItemStatus::Queued | ItemStatus::Running) && !items[k].retry_locally {
                self.set(&mut items[k], ItemStatus::Failed, Some("The admin helper stopped before finishing this one. The log has details."));
            }
        }
    }

    fn retry_locally(&self, item: &mut UpdateItem) {
        self.set(item, ItemStatus::Running, Some("Installing as you…"));
        self.log(&format!("=== {} [{}] (as the user)", item.name, item.id));
        // The window itself may be elevated (a "run as administrator" setting on the exe), so this borrows
        // Explorer's token to really run as the user.
        let r = proc::run_as_user(&tools::winget(), &winget::upgrade_args(item, false), minutes(45));
        self.log(&format!("{}exit {}", r.output, r.exit_code));
        keep(item, &r);
        let o = winget::classify(r.exit_code, &r.output, false);
        item.reboot_needed |= o == WingetOutcome::OkReboot;
        let (status, msg) = match o {
            WingetOutcome::Ok | WingetOutcome::OkReboot => (ItemStatus::Done, "Updated (as you, without admin)".to_string()),
            WingetOutcome::AlreadyCurrent => (ItemStatus::Done, "Already up to date".to_string()),
            _ => (ItemStatus::Failed, format!("Didn't install as you either: {}", winget::failure_reason(&r))),
        };
        self.set(item, status, Some(msg.as_str()));
    }

    fn run_scoop(&self, item: &mut UpdateItem) {
        let Some(script) = ScoopSource::update_script(item) else {
            self.set(item, ItemStatus::Failed, Some("Unexpected Scoop app name"));
            return;
        };
        self.set(item, ItemStatus::Running, Some("Installing…"));
        (self.status)(&format!("Updating {}…", item.name));
        self.log(&format!("=== {} (Scoop)", item.name));
        let r = proc::powershell(&script, minutes(20), None);
        self.log(&format!("{}exit {}", r.output, r.exit_code));
        keep(item, &r);
        let msg = if r.ok() { "Updated".to_string() } else { format!("Didn't install: {}", r.last_line().unwrap_or_else(|| format!("exit {}", r.exit_code))) };
        self.set(item, if r.ok() { ItemStatus::Done } else { ItemStatus::Failed }, Some(&msg));
    }

    fn run_simple(&self, item: &mut UpdateItem, exe: Option<&str>, args: &[String], timeout_min: u64) {
        if self.stopped(item) {
            return;
        }
        let Some(exe) = exe else {
            self.set(item, ItemStatus::Failed, Some("The tool was not found"));
            return;
        };
        self.set(item, ItemStatus::Running, Some("Installing…"));
        (self.status)(&format!("Updating {}…", item.name));
        let file = std::path::Path::new(exe).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.log(&format!("=== {}: {file} {}", item.name, args.join(" ")));
        let r = proc::run_simple(exe, args, minutes(timeout_min));
        self.log(&format!("{}exit {}", r.output, r.exit_code));
        keep(item, &r);
        let msg = if r.ok() { "Updated".to_string() } else { format!("Didn't install: {}", r.last_line().unwrap_or_else(|| format!("exit {}", r.exit_code))) };
        self.set(item, if r.ok() { ItemStatus::Done } else { ItemStatus::Failed }, Some(&msg));
    }

    fn py(&self, py: &str, args: &[String], minutes_: u64) -> ProcResult {
        self.log(&format!("=== python {}", args.join(" ")));
        let r = proc::run_simple(py, args, Duration::from_secs(minutes_ * 60));
        self.log(&format!("{}exit {}", r.output, r.exit_code));
        r
    }

    fn run_python(&mut self, items: &mut [UpdateItem], idx: &[usize], stamp: &str) {
        if idx.is_empty() {
            return;
        }
        if self.stop.requested() {
            for &k in idx {
                self.stopped(&mut items[k]);
            }
            return;
        }
        let Some(py) = tools::python() else {
            for &k in idx {
                self.set(&mut items[k], ItemStatus::Failed, Some("Python was not found"));
            }
            return;
        };

        // Snapshot both locations so any package can go back exactly where it was.
        (self.status)("Saving a snapshot of your Python packages…");
        let snap_script = storage::root().join("snapshot.py");
        let _ = std::fs::write(&snap_script, super::dev::SNAPSHOT_SCRIPT);
        let mut before: BTreeMap<String, (String, String)> = BTreeMap::new();
        if let Some((user, env)) = PythonSource::parse_snapshot(&self.py(&py, &[snap_script.to_string_lossy().into_owned()], 3).output) {
            let user_file = storage::snapshots().join(format!("pip-{stamp}-user.txt"));
            let _ = std::fs::write(&user_file, user.join("\r\n"));
            let _ = std::fs::write(storage::snapshots().join(format!("pip-{stamp}-env.txt")), env.join("\r\n"));
            self.settings.last_pip_snapshot = Some(user_file.to_string_lossy().into_owned());
            self.settings.save();
            for line in env.iter().chain(user.iter()) {
                before.extend(PythonSource::parse_freeze(line));
            }
        }

        let baseline = self.settings.test_after_update.then(|| {
            (self.status)("Testing your tools before updating…");
            let b = smoke::run(&py, None);
            self.log(&format!("=== baseline test\n{}", b.raw));
            b
        });

        let mut groups: Vec<(Option<String>, Vec<usize>)> = Vec::new();
        for &k in idx {
            match groups.iter_mut().find(|(s, _)| *s == items[k].scope) {
                Some((_, v)) => v.push(k),
                None => groups.push((items[k].scope.clone(), vec![k])),
            }
        }
        for (scope, batch) in &groups {
            if self.stop.requested() {
                for &k in batch {
                    self.stopped(&mut items[k]);
                }
                continue;
            }
            let specs: Vec<String> = batch.iter().map(|&k| format!("{}=={}", items[k].id, items[k].available)).collect();
            // Look at the whole plan first: a dependency pulled along must not jump a major version or go backwards.
            (self.status)("Checking what the Python update would change…");
            let mut dry = pip_install(scope.as_deref(), true);
            dry.extend(["--dry-run", "--quiet", "--report", "-"].map(String::from));
            dry.extend(specs.iter().cloned());
            if let Some(planned) = PythonSource::parse_plan(&self.py(&py, &dry, 10).output) {
                let picked: HashSet<String> = batch.iter().map(|&k| normalize(&items[k].id)).collect();
                let risky = PythonSource::risky_plan_changes(&planned, &picked, &before);
                if !risky.is_empty() {
                    let list = risky.join(", ");
                    for &k in batch {
                        self.set(&mut items[k], ItemStatus::Skipped, Some(&format!("Not installed: together these would also change {list}, which nobody picked (a major version or a downgrade).")));
                    }
                    self.summary.notes.push(format!("Some Python updates were skipped because pip would also have changed {list}."));
                    continue;
                }
            }
            (self.status)("Updating Python packages…");
            for &k in batch {
                self.set(&mut items[k], ItemStatus::Running, Some("Installing…"));
            }
            let mut args = pip_install(scope.as_deref(), true);
            args.extend(specs.iter().cloned());
            if self.py(&py, &args, 40).ok() {
                for &k in batch {
                    self.set(&mut items[k], ItemStatus::Done, Some("Updated"));
                }
            } else {
                // One bad package should not hold back the rest.
                for &k in batch {
                    let mut one = pip_install(scope.as_deref(), true);
                    one.push(format!("{}=={}", items[k].id, items[k].available));
                    let r = self.py(&py, &one, 20);
                    keep(&mut items[k], &r);
                    let msg = if r.ok() { "Updated".to_string() } else { format!("Didn't install: {}", r.last_line().unwrap_or_default()) };
                    self.set(&mut items[k], if r.ok() { ItemStatus::Done } else { ItemStatus::Failed }, Some(&msg));
                }
            }
        }

        let mut changed: BTreeMap<String, usize> = idx.iter().filter(|&&k| items[k].status == ItemStatus::Done).map(|&k| (normalize(&items[k].id), k)).collect();
        if changed.is_empty() {
            return;
        }
        // If an update broke another package's version rule, put that package back.
        let check = self.py(&py, &["-m", "pip", "check", "--disable-pip-version-check"].map(String::from), 3);
        for c in PythonSource::parse_pip_check(&check.output) {
            let key = normalize(&c.have);
            if let (Some(&k), Some((old_name, old_version))) = (changed.get(&key), before.get(&key)) {
                let mut args = pip_install(items[k].scope.as_deref(), false);
                args.push(format!("{old_name}=={old_version}"));
                self.py(&py, &args, 10);
                self.set(&mut items[k], ItemStatus::Skipped, Some(&format!("Kept at {old_version}: {} needs {}", c.package, c.requirement)));
                changed.remove(&key);
            }
        }
        // Playwright needs a matching browser build, but only fetch one if this PC already used it.
        if changed.contains_key("playwright") && playwright_chromium_installed() {
            (self.status)("Updating Playwright's browser…");
            self.py(&py, &["-m", "playwright", "install", "chromium"].map(String::from), 15);
        }
        if !self.settings.test_after_update || changed.is_empty() {
            return;
        }
        (self.status)("Testing your tools after updating…");
        let after = smoke::run(&py, None);
        self.log(&format!("=== test after updating\n{}", after.raw));
        let broke = smoke::new_failures(baseline.as_ref(), &after);
        if broke.is_empty() {
            let total = after.passed.len() + after.failed.len();
            self.summary.notes.push(if after.ran && total > 0 {
                format!("Your tools passed {} of {total} checks after the Python update.", after.passed.len())
            } else {
                "No tool checks applied to this PC.".into()
            });
            return;
        }
        // Roll back the major versions first (the usual culprit); if that isn't enough, everything.
        let broke_text = broke.join(", ");
        let majors: Vec<usize> = changed.values().copied().filter(|&k| items[k].is_major).collect();
        if !majors.is_empty() && majors.len() < changed.len() {
            (self.status)("A check failed. Rolling back the major-version packages…");
            self.roll_back(&py, items, &majors, &before);
            let retest = smoke::run(&py, None);
            if smoke::new_failures(baseline.as_ref(), &retest).is_empty() {
                let names: Vec<String> = majors.iter().map(|&k| items[k].name.clone()).collect();
                for &k in &majors {
                    self.set(&mut items[k], ItemStatus::Failed, Some(&format!("Rolled back: the {broke_text} check failed after this update")));
                }
                self.summary.notes.push(format!("The {broke_text} check failed after updating, so {} went back to the old version. Everything passes again.", names.join(", ")));
                return;
            }
        }
        (self.status)("A check failed. Rolling back the Python update…");
        let all: Vec<usize> = changed.values().copied().collect();
        self.roll_back(&py, items, &all, &before);
        for &k in &all {
            self.set(&mut items[k], ItemStatus::Failed, Some(&format!("Rolled back: the {broke_text} check failed after updating")));
        }
        let fin = smoke::run(&py, None);
        self.summary.notes.push(if smoke::new_failures(baseline.as_ref(), &fin).is_empty() {
            format!("The {broke_text} check failed after the Python update, so it was rolled back. Everything passes again.")
        } else {
            format!("The {broke_text} check still fails after rolling back. Use \"Roll back the last Python update\" or check the log.")
        });
    }

    fn roll_back(&self, py: &str, items: &[UpdateItem], idx: &[usize], before: &BTreeMap<String, (String, String)>) {
        let mut scopes: Vec<Option<String>> = idx.iter().map(|&k| items[k].scope.clone()).collect();
        scopes.dedup();
        for scope in scopes {
            let pins: Vec<String> = idx
                .iter()
                .filter(|&&k| items[k].scope == scope)
                .filter_map(|&k| before.get(&normalize(&items[k].id)))
                .map(|(n, v)| format!("{n}=={v}"))
                .collect();
            if !pins.is_empty() {
                let mut args = pip_install(scope.as_deref(), false);
                args.extend(pins);
                self.py(py, &args, 30);
            }
        }
    }
}

fn playwright_chromium_installed() -> bool {
    let v = tools::env("PLAYWRIGHT_BROWSERS_PATH");
    let root = if v.is_empty() { tools::local_app_data().join("ms-playwright") } else { PathBuf::from(v) };
    std::fs::read_dir(root).map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with("chromium"))).unwrap_or(false)
}

/// Puts packages back to a snapshot: the user-site file with --user, its "-env" twin without.
pub fn restore_snapshot(user_snapshot: &str) -> ProcResult {
    let Some(py) = tools::python() else { return ProcResult::new(-1, "Python was not found", false) };
    let env_snapshot = user_snapshot.replace("-user.txt", "-env.txt");
    let mut last = ProcResult::new(0, "Nothing to restore", false);
    for (file, scope) in [(user_snapshot.to_string(), Some("user")), (env_snapshot.clone(), None)] {
        let empty = std::fs::metadata(&file).map(|m| m.len() == 0).unwrap_or(true);
        if empty || (scope.is_none() && file == user_snapshot) {
            continue;
        }
        let mut args = pip_install(scope, false);
        args.extend(["-r".to_string(), file]);
        last = proc::run_simple(&py, &args, minutes(40));
        if !last.ok() {
            return last;
        }
    }
    last
}
