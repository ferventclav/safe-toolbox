//! The scheduled task's work, also used by "--check". It only looks: the same update rules and cleanup guard as
//! the window, no installs, no deletes. What it found goes to last-check.json for the Home page.

use std::sync::Mutex;

use chrono::TimeDelta;
use serde::{Deserialize, Serialize};

use super::{Frequency, ToolboxSettings, last_check_file, write_error};
use crate::proc::Cancel;
use crate::time::{Stamp, now};
use crate::updater::model::{SourceResult, UpdateSource};
use crate::updater::policy::{PolicyContext, apply_all};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateFinding {
    pub source: String,
    pub name: String,
    pub current: String,
    pub available: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CheckSummary {
    #[serde(with = "crate::time::opt")]
    pub when: Option<Stamp>,
    /// When each part last ran. Kept here, not in settings.json, so the background never races the window.
    #[serde(with = "crate::time::opt")]
    pub updates_when: Option<Stamp>,
    #[serde(with = "crate::time::opt")]
    pub cleanup_when: Option<Stamp>,
    /// The update set the last notification was about, so the same updates don't notify twice.
    pub notified_updates: Option<String>,
    pub updates_checked: bool,
    pub updates: Vec<UpdateFinding>,
    pub left_out: usize,
    pub failed_sources: Vec<String>,
    pub cleanup_checked: bool,
    pub cleanable: u64,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

impl CheckSummary {
    pub fn updates_key(&self) -> String {
        let mut keys: Vec<String> = self.updates.iter().map(|u| format!("{}:{}:{}", u.source, u.name, u.available)).collect();
        keys.sort();
        keys.join("|")
    }

    pub fn load() -> Option<CheckSummary> {
        let text = std::fs::read_to_string(last_check_file()).ok()?;
        serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
    }

    pub fn save(&self) {
        if let Err(e) = std::fs::write(last_check_file(), serde_json::to_string_pretty(self).unwrap_or_default()) {
            write_error(&format!("Could not save last-check.json: {e}"));
        }
    }
}

fn due(f: Frequency, last: Option<Stamp>, at: Stamp) -> bool {
    let gap = match f {
        Frequency::Daily => TimeDelta::hours(20),
        Frequency::Weekly => TimeDelta::hours(156),
        Frequency::Off => return false,
    };
    last.is_none_or(|l| at - l >= gap)
}

pub fn updates_due(s: &ToolboxSettings, last: Option<Stamp>, at: Stamp) -> bool {
    due(s.update_check, last, at)
}

pub fn cleanup_due(s: &ToolboxSettings, last: Option<Stamp>, at: Stamp) -> bool {
    due(s.cleanup_reminder, last, at)
}

pub fn run(updates: bool, cleanup: bool, toolbox: &ToolboxSettings, cancel: &Cancel) -> CheckSummary {
    let mut summary = CheckSummary::load().unwrap_or_default();
    summary.when = Some(now());
    if updates {
        check_updates(&mut summary, cancel);
        summary.updates_when = Some(now());
    }
    if cleanup {
        scan_cleanup(&mut summary, toolbox, cancel);
        summary.cleanup_when = Some(now());
    }
    if let Some((total, free)) = crate::disk::system_drive_usage() {
        summary.total_bytes = total;
        summary.free_bytes = free;
    }
    summary.save();
    summary
}

fn check_updates(summary: &mut CheckSummary, cancel: &Cancel) {
    let mut settings = crate::updater::storage::Settings::load();
    let results: Mutex<Vec<SourceResult>> = Mutex::new(Vec::new());
    crate::updater::checker::check_all(cancel, &|r| results.lock().unwrap().push(r));
    let results = results.into_inner().unwrap();
    let choco = results.iter().find(|r| r.source == Some(UpdateSource::Chocolatey)).map(|r| r.managed.clone()).unwrap_or_default();
    let mut items: Vec<_> = results.iter().flat_map(|r| r.items.iter().cloned()).collect();
    apply_all(&mut items, &PolicyContext::from(&settings, choco, crate::tools::rustup().is_some()));
    summary.updates_checked = true;
    summary.updates = items
        .iter()
        .filter(|i| !i.hidden && i.selectable && i.recommended)
        .map(|i| UpdateFinding {
            source: i.source.title().into(),
            name: i.display_name(),
            current: i.current.clone(),
            available: i.target_version.clone().unwrap_or_else(|| i.available.clone()),
        })
        .collect();
    summary.left_out = items.iter().filter(|i| i.hidden).count();
    summary.failed_sources = results.iter().filter(|r| r.failed).filter_map(|r| r.source.map(|s| s.title().to_string())).collect();
    settings.last_checked = Some(now());
    settings.save();
}

fn scan_cleanup(summary: &mut CheckSummary, toolbox: &ToolboxSettings, cancel: &Cancel) {
    let scan = crate::cleaner::engine::scan(crate::tools::is_elevated(), &|_, _, _| {}, Some(cancel));
    summary.cleanup_checked = true;
    summary.cleanable = scan.rules.iter().filter(|r| r.can_clean() && toolbox.cleanup_tick(&r.id, r.default_checked)).map(|r| r.total()).sum();
}

/// The notification to show, or None when there is nothing new worth one.
pub fn notification(s: &CheckSummary, toolbox: &ToolboxSettings, updates_ran: bool, cleanup_ran: bool) -> Option<(String, String)> {
    let mut lines = Vec::new();
    let mut title = None;
    let n = s.updates.len();
    if updates_ran && n > 0 && s.notified_updates.as_deref() != Some(s.updates_key().as_str()) {
        title = Some(if n == 1 { "1 safe update is ready".to_string() } else { format!("{n} safe updates are ready") });
        let names: Vec<&str> = s.updates.iter().take(3).map(|u| u.name.as_str()).collect();
        lines.push(format!("{}{}.", names.join(", "), if n > 3 { format!(" and {} more", n - 3) } else { String::new() }));
    }
    let threshold = toolbox.reminder_threshold_mb * 1024 * 1024;
    let low_space = s.total_bytes > 0 && s.free_bytes < s.total_bytes / 10;
    if cleanup_ran && (s.cleanable >= threshold || (low_space && s.cleanable > 0)) {
        let size = crate::disk::format_size(s.cleanable);
        lines.push(if low_space {
            format!("{size} can be freed safely, and C: has only {} free.", crate::disk::format_gb(s.free_bytes))
        } else {
            format!("{size} can be freed safely.")
        });
        title.get_or_insert_with(|| if low_space { "C: is getting full".into() } else { "Space to free".into() });
    }
    title.map(|t| (t, lines.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    #[test]
    fn jobs_run_when_due() {
        let at = Local.with_ymd_and_hms(2026, 11, 8, 12, 30, 0).unwrap();
        let s = |f| ToolboxSettings { update_check: f, ..Default::default() };
        let ago = |h: i64| Some(at - TimeDelta::hours(h));
        assert!(!updates_due(&s(Frequency::Off), None, at));
        assert!(updates_due(&s(Frequency::Daily), None, at));
        assert!(!updates_due(&s(Frequency::Daily), ago(19), at));
        assert!(updates_due(&s(Frequency::Daily), ago(21), at));
        assert!(!updates_due(&s(Frequency::Weekly), ago(24 * 6), at));
        assert!(updates_due(&s(Frequency::Weekly), ago(24 * 7), at));
    }

    #[test]
    fn notifications() {
        let s = ToolboxSettings::default();
        let finding = |n: &str| UpdateFinding { source: "Apps".into(), name: n.into(), available: "1".into(), ..Default::default() };
        let mut one = CheckSummary { updates: vec![finding("Git")], ..Default::default() };
        assert!(notification(&one, &s, true, false).is_some());
        one.notified_updates = Some(one.updates_key());
        assert!(notification(&one, &s, true, false).is_none());

        let five = CheckSummary { updates: ["Git", "Ollama", "Gemini", "Vortex", "Grammarly"].map(finding).to_vec(), ..Default::default() };
        assert_eq!(notification(&five, &s, true, false), Some(("5 safe updates are ready".into(), "Git, Ollama, Gemini and 2 more.".into())));

        let small = CheckSummary { cleanable: 300 << 20, total_bytes: 1000 << 30, free_bytes: 400 << 30, ..Default::default() };
        assert!(notification(&small, &s, false, true).is_none());
        let full = CheckSummary { free_bytes: 50 << 30, ..small };
        let (title, body) = notification(&full, &s, false, true).unwrap();
        assert_eq!(title, "C: is getting full");
        assert!(body.contains("can be freed safely"));
    }
}
