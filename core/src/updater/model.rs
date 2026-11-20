use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::tools::ends_ic;

/// The groups shown in the window, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum UpdateSource {
    Apps,
    Chocolatey,
    Scoop,
    Windows,
    Nvidia,
    Python,
    Node,
    Rust,
    ClaudeCode,
}

impl UpdateSource {
    pub const ALL: [UpdateSource; 9] = [
        UpdateSource::Apps,
        UpdateSource::Chocolatey,
        UpdateSource::Scoop,
        UpdateSource::Windows,
        UpdateSource::Nvidia,
        UpdateSource::Python,
        UpdateSource::Node,
        UpdateSource::Rust,
        UpdateSource::ClaudeCode,
    ];

    pub fn name(self) -> &'static str {
        match self {
            UpdateSource::Apps => "Apps",
            UpdateSource::Chocolatey => "Chocolatey",
            UpdateSource::Scoop => "Scoop",
            UpdateSource::Windows => "Windows",
            UpdateSource::Nvidia => "Nvidia",
            UpdateSource::Python => "Python",
            UpdateSource::Node => "Node",
            UpdateSource::Rust => "Rust",
            UpdateSource::ClaudeCode => "ClaudeCode",
        }
    }

    pub fn parse(s: &str) -> Option<UpdateSource> {
        UpdateSource::ALL.into_iter().find(|x| x.name().eq_ignore_ascii_case(s))
    }

    pub fn title(self) -> &'static str {
        match self {
            UpdateSource::Apps => "Apps",
            UpdateSource::Chocolatey => "Chocolatey packages",
            UpdateSource::Scoop => "Scoop apps",
            UpdateSource::Windows => "Windows and drivers",
            UpdateSource::Nvidia => "NVIDIA graphics driver",
            UpdateSource::Python => "Python packages",
            UpdateSource::Node => "Node packages",
            UpdateSource::Rust => "Rust",
            UpdateSource::ClaudeCode => "Claude Code",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            UpdateSource::Apps => "Installed through winget, the Windows package manager.",
            UpdateSource::Chocolatey => "Installed with Chocolatey. Updating needs admin, so it is part of the one admin step.",
            UpdateSource::Scoop => "Installed with Scoop, in your user folder.",
            UpdateSource::Windows => "Checked against what you have. Windows Update installs them.",
            UpdateSource::Nvidia => "Read from the NVIDIA app. Drivers are installed there.",
            UpdateSource::Python => "pip packages, updated where each one is installed. A snapshot is saved first, so you can roll back.",
            UpdateSource::Node => "Global npm packages.",
            UpdateSource::Rust => "rustup and its toolchains.",
            UpdateSource::ClaudeCode => "The claude command-line tool. The desktop app updates itself.",
        }
    }
}

impl std::fmt::Display for UpdateSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ItemKind {
    #[default]
    Package,
    WindowsUpdate,
    Driver,
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ItemStatus {
    #[default]
    Idle,
    Queued,
    Running,
    Done,
    Failed,
    Skipped,
}

impl ItemStatus {
    pub fn name(self) -> &'static str {
        match self {
            ItemStatus::Idle => "Idle",
            ItemStatus::Queued => "Queued",
            ItemStatus::Running => "Running",
            ItemStatus::Done => "Done",
            ItemStatus::Failed => "Failed",
            ItemStatus::Skipped => "Skipped",
        }
    }
}

/// One thing that can be updated: a winget app, a Windows update, a pip package and so on.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateItem {
    pub source: UpdateSource,
    #[serde(default)]
    pub kind: ItemKind,
    /// winget or Chocolatey id, pip or npm name, Windows Update id, rustup toolchain.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub current: String,
    #[serde(default)]
    pub available: String,
    /// An exact version to install instead of the newest (the "stay on 24.x" Node entry).
    #[serde(default)]
    pub target_version: Option<String>,
    #[serde(default)]
    pub unknown_version: bool,
    #[serde(default)]
    pub is_major: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub needs_admin: bool,
    #[serde(default = "yes")]
    pub selectable: bool,
    #[serde(default, with = "crate::time::opt_date")]
    pub driver_date: Option<NaiveDate>,
    /// For a Windows driver: the date of the driver already on that device, when it could be found.
    #[serde(default, with = "crate::time::opt_date")]
    pub installed_driver_date: Option<NaiveDate>,
    /// For a Windows update: optional (previews, extras) rather than one Windows would install itself.
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub detail: Option<String>,
    /// pip: "user" for the user site, "env" for the interpreter's own site-packages.
    #[serde(default)]
    pub scope: Option<String>,
    /// Other installed packages whose version rules exclude the newest version.
    #[serde(default)]
    pub blockers: Vec<String>,
    /// Set by a source that knows updating would be unsafe on this PC (needs admin, conda, held in Scoop).
    #[serde(default)]
    pub unsafe_reason: Option<String>,
    #[serde(default)]
    pub recommended: bool,
    #[serde(default)]
    pub selected: bool,
    /// Unsafe to update: left out of the list (so it can't be ticked by accident), shown only with its reason.
    #[serde(default)]
    pub hidden: bool,
    /// Hidden because the user asked not to see it again.
    #[serde(default)]
    pub ignored_by_user: bool,
    #[serde(default)]
    pub badge: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub status: ItemStatus,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub reboot_needed: bool,
    /// Set by the admin worker when the item should be tried again as the signed-in user.
    #[serde(default)]
    pub retry_locally: bool,
    #[serde(default)]
    pub output: Vec<String>,
}

fn yes() -> bool {
    true
}

impl UpdateItem {
    pub fn new(source: UpdateSource, id: impl Into<String>, name: impl Into<String>) -> Self {
        UpdateItem {
            source,
            kind: ItemKind::Package,
            id: id.into(),
            name: name.into(),
            current: String::new(),
            available: String::new(),
            target_version: None,
            unknown_version: false,
            is_major: false,
            pinned: false,
            needs_admin: false,
            selectable: true,
            driver_date: None,
            installed_driver_date: None,
            optional: false,
            detail: None,
            scope: None,
            blockers: Vec::new(),
            unsafe_reason: None,
            recommended: false,
            selected: false,
            hidden: false,
            ignored_by_user: false,
            badge: None,
            note: None,
            status: ItemStatus::Idle,
            result: None,
            reboot_needed: false,
            retry_locally: false,
            output: Vec::new(),
        }
    }

    pub fn versions(mut self, current: impl Into<String>, available: impl Into<String>) -> Self {
        self.current = current.into();
        self.available = available.into();
        self
    }

    /// The name without the installed version some installers bake into it ("Ollama version 0.40.14"), since
    /// the row shows the versions beside it anyway.
    pub fn display_name(&self) -> String {
        if self.current.is_empty() || !ends_ic(&self.name, &self.current) {
            return self.name.clone();
        }
        let mut rest = self.name[..self.name.len() - self.current.len()].trim_end().to_string();
        for tail in [" version", " ver.", " v", "-v", " -"] {
            if ends_ic(&rest, tail) {
                rest = rest[..rest.len() - tail.len()].trim_end().to_string();
                break;
            }
        }
        if rest.chars().count() >= 2 { rest } else { self.name.clone() }
    }

    pub fn key(&self) -> String {
        match &self.target_version {
            None => format!("{}|{}", self.source, self.id),
            Some(t) => format!("{}|{}|{}", self.source, self.id, t),
        }
    }

    /// What "Don't show this update" remembers: the package, not the version.
    pub fn ignore_key(&self) -> String {
        format!("{}|{}", self.source, self.id)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceResult {
    pub source: Option<UpdateSource>,
    pub items: Vec<UpdateItem>,
    /// A line for the group header: "Driver 621.01 is the newest", "Python was not found".
    pub message: Option<String>,
    pub failed: bool,
    /// The tool isn't on this PC, so the group is not shown at all.
    pub absent: bool,
    /// Chocolatey: every package it manages, so winget doesn't update the same apps behind its back.
    pub managed: Vec<String>,
}

impl SourceResult {
    pub fn of(source: UpdateSource) -> Self {
        SourceResult { source: Some(source), ..Default::default() }
    }
    pub fn absent(source: UpdateSource) -> Self {
        SourceResult { source: Some(source), absent: true, ..Default::default() }
    }
    pub fn failed(source: UpdateSource, message: impl Into<String>) -> Self {
        SourceResult { source: Some(source), failed: true, message: Some(message.into()), ..Default::default() }
    }
    pub fn message(source: UpdateSource, message: impl Into<String>) -> Self {
        SourceResult { source: Some(source), message: Some(message.into()), ..Default::default() }
    }
    pub fn items(source: UpdateSource, items: Vec<UpdateItem>, when_empty: &str) -> Self {
        let message = items.is_empty().then(|| when_empty.to_string());
        SourceResult { source: Some(source), items, message, ..Default::default() }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub updated: usize,
    pub failed: usize,
    pub skipped: usize,
    pub reboot_recommended: bool,
    pub notes: Vec<String>,
    pub log_file: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_drops_a_baked_in_version() {
        let i = UpdateItem::new(UpdateSource::Apps, "Ollama.Ollama", "Ollama version 0.40.14").versions("0.40.14", "0.41.0");
        assert_eq!(i.display_name(), "Ollama");
        let j = UpdateItem::new(UpdateSource::Apps, "x", "7-Zip 26.04").versions("26.04", "26.05");
        assert_eq!(j.display_name(), "7-Zip");
        let k = UpdateItem::new(UpdateSource::Apps, "x", "26.04").versions("26.04", "26.05");
        assert_eq!(k.display_name(), "26.04");
    }

    #[test]
    fn keys_include_a_target_version() {
        let mut i = UpdateItem::new(UpdateSource::Apps, "OpenJS.NodeJS", "Node");
        assert_eq!(i.key(), "Apps|OpenJS.NodeJS");
        i.target_version = Some("24.15.0".into());
        assert_eq!(i.key(), "Apps|OpenJS.NodeJS|24.15.0");
        assert_eq!(i.ignore_key(), "Apps|OpenJS.NodeJS");
    }
}
