//! The elevated half of the cleaner: "safe-toolbox.exe --clean-worker job.json". It only touches rules marked admin
//! (the Windows group and the memory dump), which are machine-wide, so whichever admin account approves the
//! prompt, nothing in a user profile is affected. Every path still goes through the guard twice.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::engine::{self, write_error};
use super::guard::Guard;
use super::{Action, Item, ProcessSnapshot, Rule, ScanContext, Skip, rules};
use crate::proc::Cancel;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdminJob {
    /// "scan" to size the admin rules, "clean" to clean the listed ones.
    pub mode: String,
    pub rule_ids: Vec<String>,
    pub result_file: String,
    pub progress_file: String,
    /// The requesting user's runs.csv, so one log holds the whole clean.
    pub log_file: Option<String>,
    pub stop_file: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdminItem {
    pub label: String,
    pub path: Option<String>,
    pub size: Option<u64>,
    pub handler: bool,
    pub action: bool,
    pub result: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdminRule {
    pub id: String,
    pub blocked: Option<String>,
    pub items: Vec<AdminItem>,
    pub skipped: Vec<Skip>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdminResult {
    pub rules: Vec<AdminRule>,
    pub freed: i64,
    pub items: usize,
    pub problems: usize,
    pub stopped: usize,
    pub error: Option<String>,
}

pub fn run_file(job_path: &str) -> i32 {
    let job: AdminJob = match std::fs::read_to_string(job_path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())) {
        Ok(j) => j,
        Err(e) => {
            write_error(&format!("Admin worker failed: {e}"));
            return 1;
        }
    };
    let result = run(&job);
    let _ = std::fs::write(&job.result_file, serde_json::to_string(&result).unwrap_or_default());
    i32::from(result.error.is_some())
}

pub fn run(job: &AdminJob) -> AdminResult {
    let wanted: HashSet<&str> = job.rule_ids.iter().map(String::as_str).collect();
    let mut rules: Vec<Rule> = rules::build().into_iter().filter(|r| r.needs_admin && wanted.contains(r.id.as_str())).collect();
    let ctx = ScanContext::new(ProcessSnapshot::take());
    let guard = Guard::create_default();
    for r in rules.iter_mut() {
        engine::scan_rule(r, &ctx, &guard, true);
    }
    let mut result = AdminResult::default();
    if job.mode == "clean" {
        let stop = Cancel::new();
        let stop_file = job.stop_file.clone();
        let watcher_stop = stop.clone();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        let watcher = std::thread::spawn(move || {
            while !done2.load(std::sync::atomic::Ordering::SeqCst) {
                if stop_file.as_deref().is_some_and(|f| Path::new(f).exists()) {
                    watcher_stop.cancel();
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
        let progress_file = job.progress_file.clone();
        let progress = move |done: usize, total: usize, current: &str| crate::updater::storage::append(Path::new(&progress_file), &format!("{done}/{total}\t{current}"));
        let cr = engine::clean(&mut rules, &progress, false, true, job.log_file.as_deref(), Some(&stop));
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = watcher.join();
        result.freed = cr.freed;
        result.items = cr.items;
        result.problems = cr.problems;
        result.stopped = cr.stopped;
    }
    result.rules = rules
        .iter()
        .map(|r| AdminRule {
            id: r.id.clone(),
            blocked: r.blocked.clone(),
            skipped: r.skipped.clone(),
            items: r
                .items
                .iter()
                .map(|i| AdminItem { label: i.label.clone(), path: i.path.clone(), size: i.size, handler: i.handler.is_some(), action: i.action.is_some(), result: i.result.clone() })
                .collect(),
        })
        .collect();
    result
}

/// Copies the helper's findings into the window's rules, so the Windows group shows sizes and paths. The copies
/// are display-only: the window never deletes these itself, it hands the rule ids back to the helper.
pub fn apply(result: &AdminResult, rules: &mut [Rule]) {
    for r in rules.iter_mut().filter(|r| r.needs_admin) {
        let Some(a) = result.rules.iter().find(|a| a.id == r.id) else { continue };
        r.items.clear();
        r.skipped = a.skipped.clone();
        r.blocked = a.blocked.clone();
        for i in &a.items {
            let mut it = if i.handler {
                Item::handler(&i.label, "elevated", i.size)
            } else if let Some(p) = &i.path {
                Item { label: i.label.clone(), size: i.size, ..Item::path(p) }
            } else {
                Item::action(&i.label, Action::Elevated, i.size)
            };
            it.result = i.result.clone();
            r.items.push(it);
        }
        r.scanned_elevated = true;
    }
}

pub fn read_result(file: &str) -> Option<AdminResult> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

/// Starts this exe elevated in clean-worker mode. Err("declined") when the prompt was declined.
pub fn launch(job_path: &Path) -> Result<crate::shell::ElevatedProcess, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    crate::shell::run_as_admin(&exe.to_string_lossy(), &format!("--clean-worker \"{}\"", job_path.to_string_lossy()), &dir)
}
