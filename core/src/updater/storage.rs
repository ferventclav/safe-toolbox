//! Everything the update side writes lives in %LOCALAPPDATA%\SafeUpdater.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::model::UpdateItem;
use crate::time::{self, Stamp};
use crate::tools::local_app_data;

pub fn root() -> PathBuf {
    local_app_data().join("SafeUpdater")
}

fn ensure(p: PathBuf) -> PathBuf {
    let _ = fs::create_dir_all(&p);
    p
}

pub fn logs() -> PathBuf {
    ensure(root().join("logs"))
}
pub fn snapshots() -> PathBuf {
    ensure(root().join("snapshots"))
}
pub fn jobs() -> PathBuf {
    ensure(root().join("jobs"))
}
pub fn runs_csv() -> PathBuf {
    ensure(root()).join("runs.csv")
}
pub fn error_log() -> PathBuf {
    ensure(root()).join("error.log")
}
pub fn settings_file() -> PathBuf {
    ensure(root()).join("settings.json")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub include_majors: bool,
    pub test_after_update: bool,
    pub restore_point: bool,
    #[serde(with = "crate::time::opt")]
    pub last_checked: Option<Stamp>,
    #[serde(with = "crate::time::opt")]
    pub last_updated: Option<Stamp>,
    /// The user-site snapshot; the matching "-env" file, when present, holds the interpreter's own packages.
    pub last_pip_snapshot: Option<String>,
    /// Updates the user asked not to see again, as "Source|Id".
    pub ignored: BTreeSet<String>,
    /// "Skip this version": "Source|Id" to the version skipped. The next newer version is listed again.
    pub skipped: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            include_majors: false,
            test_after_update: true,
            restore_point: true,
            last_checked: None,
            last_updated: None,
            last_pip_snapshot: None,
            ignored: BTreeSet::new(),
            skipped: BTreeMap::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Settings {
        let f = settings_file();
        match fs::read_to_string(&f) {
            Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_else(|e| {
                write_error(&format!("settings.json unreadable, using defaults: {e}"));
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self) {
        match serde_json::to_string(self) {
            Ok(s) => {
                if let Err(e) = fs::write(settings_file(), s) {
                    write_error(&format!("Could not save settings: {e}"));
                }
            }
            Err(e) => write_error(&format!("Could not save settings: {e}")),
        }
    }

    pub fn is_ignored(&self, key: &str) -> bool {
        self.ignored.iter().any(|k| k.eq_ignore_ascii_case(key))
    }

    pub fn unignore(&mut self, key: &str) {
        self.ignored.retain(|k| !k.eq_ignore_ascii_case(key));
        self.skipped.retain(|k, _| !k.eq_ignore_ascii_case(key));
    }
}

static GATE: Mutex<()> = Mutex::new(());

pub fn new_run_log(stamp: &str) -> PathBuf {
    logs().join(format!("run-{stamp}.log"))
}

pub fn append(path: &std::path::Path, text: &str) {
    let _g = GATE.lock();
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(text.as_bytes());
        if !text.ends_with('\n') {
            let _ = f.write_all(b"\r\n");
        }
    }
}

pub fn write_error(text: &str) {
    append(&error_log(), &format!("{} {text}", time::now().format("%Y-%m-%d %H:%M:%S")));
}

fn csv(v: &str) -> String {
    if v.contains([',', '"', '\n']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.to_string() }
}

/// One line per item per run in runs.csv, so "what changed on 3 Nov" has an answer.
pub fn record(item: &UpdateItem, when: &Stamp) {
    let path = runs_csv();
    let mut s = String::new();
    if !path.exists() {
        s.push_str("when,source,id,name,from,to,status,result\r\n");
    }
    let fields = [
        when.format("%Y-%m-%d %H:%M:%S").to_string(),
        item.source.to_string(),
        item.id.clone(),
        item.name.clone(),
        item.current.clone(),
        item.target_version.clone().unwrap_or_else(|| item.available.clone()),
        item.status.name().to_string(),
        item.result.clone().unwrap_or_default(),
    ];
    s.push_str(&fields.iter().map(|f| csv(f)).collect::<Vec<_>>().join(","));
    s.push_str("\r\n");
    append(&path, &s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_settings_the_csharp_build_wrote() {
        let s: Settings = serde_json::from_str(
            r#"{"includeMajors":true,"testAfterUpdate":false,"restorePoint":true,"lastChecked":"2026-11-18T23:58:27.7793272+05:30","lastUpdated":null,"lastPipSnapshot":"C:\\x-user.txt","ignored":["Apps|Foo.Bar"],"skipped":{"Apps|Git.Git":"2.50.0"}}"#,
        )
        .unwrap();
        assert!(s.include_majors && !s.test_after_update);
        assert!(s.last_checked.is_some());
        assert!(s.is_ignored("apps|foo.bar"));
        assert_eq!(s.skipped["Apps|Git.Git"], "2.50.0");
        let empty: Settings = serde_json::from_str("{}").unwrap();
        assert!(empty.test_after_update && empty.restore_point);
    }

    #[test]
    fn csv_quotes_only_when_needed() {
        assert_eq!(csv("plain"), "plain");
        assert_eq!(csv("a,b"), "\"a,b\"");
        assert_eq!(csv("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
