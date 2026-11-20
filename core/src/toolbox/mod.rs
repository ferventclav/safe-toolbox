//! The toolbox's own parts: settings, the background check and its task, history, health, repairs, PATH,
//! big files and the app list. Data lives in %LOCALAPPDATA%\SafeToolbox, shared with the C# build.

pub mod admin_tasks;
pub mod app_list;
pub mod background;
pub mod big_files;
pub mod health;
pub mod history;
pub mod notify;
pub mod path_doctor;
pub mod repairs;
pub mod schedule;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

fn ensure(dir: PathBuf) -> PathBuf {
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn root() -> PathBuf {
    ensure(crate::tools::local_app_data().join("SafeToolbox"))
}
pub fn settings_file() -> PathBuf {
    root().join("settings.json")
}
pub fn error_log() -> PathBuf {
    root().join("error.log")
}
pub fn last_check_file() -> PathBuf {
    root().join("last-check.json")
}
pub fn jobs() -> PathBuf {
    ensure(root().join("jobs"))
}
pub fn exports() -> PathBuf {
    ensure(root().join("exports"))
}

pub fn write_error(text: &str) {
    let line = format!("{} {text}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    crate::updater::storage::append(&error_log(), &line);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccentMode {
    #[default]
    FollowWindows,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackdropMode {
    #[default]
    MicaAlt,
    Mica,
    Solid,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frequency {
    #[default]
    Off,
    Daily,
    Weekly,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolboxSettings {
    pub theme: ThemeMode,
    pub accent: AccentMode,
    pub backdrop: BackdropMode,
    pub update_check: Frequency,
    pub cleanup_reminder: Frequency,
    pub reminder_threshold_mb: u64,
    pub remember_cleanup_ticks: bool,
    /// Cleanup rule id to tick state, only for rules where you differ from the default.
    pub cleanup_choices: BTreeMap<String, bool>,
    pub last_page: Option<String>,
    pub check_updates_on_open: bool,
    pub notify: bool,
    /// x, y, width, height in screen pixels when the window last closed.
    pub window_bounds: Option<Vec<i32>>,
    pub window_maximized: bool,
}

impl Default for ToolboxSettings {
    fn default() -> Self {
        ToolboxSettings {
            theme: ThemeMode::System,
            accent: AccentMode::FollowWindows,
            backdrop: BackdropMode::MicaAlt,
            update_check: Frequency::Off,
            cleanup_reminder: Frequency::Off,
            reminder_threshold_mb: 1024,
            remember_cleanup_ticks: true,
            cleanup_choices: BTreeMap::new(),
            last_page: None,
            check_updates_on_open: true,
            notify: true,
            window_bounds: None,
            window_maximized: false,
        }
    }
}

impl ToolboxSettings {
    pub fn load_from(file: &Path) -> ToolboxSettings {
        let Ok(text) = std::fs::read_to_string(file) else { return ToolboxSettings::default() };
        serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_else(|e| {
            write_error(&format!("settings.json unreadable, using defaults: {e}"));
            ToolboxSettings::default()
        })
    }

    pub fn load() -> ToolboxSettings {
        Self::load_from(&settings_file())
    }

    /// Writes to a temp file and renames it over the old one, so a crash never leaves half a file.
    pub fn save_to(&self, file: &Path) {
        let tmp = file.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, file)) {
            write_error(&format!("Could not save settings: {e}"));
        }
    }

    pub fn save(&self) {
        self.save_to(&settings_file())
    }

    /// The tick for a cleanup rule: your remembered choice, else the rule's default.
    pub fn cleanup_tick(&self, rule_id: &str, default_checked: bool) -> bool {
        match self.cleanup_choices.get(rule_id) {
            Some(&on) if self.remember_cleanup_ticks => on,
            _ => default_checked,
        }
    }

    /// Remembers a tick, storing it only when it differs from the default.
    pub fn remember_tick(&mut self, rule_id: &str, on: bool, default_checked: bool) {
        if on == default_checked {
            self.cleanup_choices.remove(rule_id);
        } else {
            self.cleanup_choices.insert(rule_id.to_string(), on);
        }
    }
}

/// XML text escaping (as SecurityElement.Escape does) and its reverse.
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

pub fn xml_unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_store_only_differences_and_respect_remembering() {
        let mut s = ToolboxSettings::default();
        s.remember_tick("recycle", true, false);
        s.remember_tick("temp", true, true);
        assert_eq!(s.cleanup_choices.len(), 1);
        assert!(s.cleanup_tick("recycle", false));
        s.remember_tick("recycle", false, false);
        assert!(s.cleanup_choices.is_empty());
        s.remember_tick("temp", false, true);
        s.remember_cleanup_ticks = false;
        assert!(s.cleanup_tick("temp", true));
    }

    #[test]
    fn settings_round_trip_with_csharp_names() {
        let f = std::env::temp_dir().join(format!("st-settings-{}.json", std::process::id()));
        ToolboxSettings { accent: AccentMode::Never, theme: ThemeMode::Dark, update_check: Frequency::Daily, ..Default::default() }.save_to(&f);
        let back = ToolboxSettings::load_from(&f);
        assert_eq!((back.accent, back.theme, back.update_check), (AccentMode::Never, ThemeMode::Dark, Frequency::Daily));
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains("\"accent\": \"Never\"") && text.contains("\"reminderThresholdMb\": 1024"));
        let _ = std::fs::remove_file(&f);
        let old: ToolboxSettings = serde_json::from_str(r#"{"theme":"Light","windowBounds":[1,2,3,4],"cleanupChoices":{"temp":false}}"#).unwrap();
        assert_eq!(old.window_bounds, Some(vec![1, 2, 3, 4]));
        assert!(old.notify && !old.cleanup_tick("temp", true));
    }
}
