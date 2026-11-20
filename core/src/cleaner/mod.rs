//! The disk cleaner: rules that find caches and leftovers, a guard that checks every path twice, and a delete
//! that never follows links or touches cloud files.

pub mod admin;
pub mod engine;
pub mod guard;
pub mod paths;
pub mod protect;
pub mod rules;
pub mod system;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// The cards in the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleGroup {
    Leftovers,
    Apps,
    YourCall,
    Windows,
}

impl RuleGroup {
    pub const ORDER: [RuleGroup; 4] = [RuleGroup::Leftovers, RuleGroup::Apps, RuleGroup::YourCall, RuleGroup::Windows];

    pub fn title(self) -> &'static str {
        match self {
            RuleGroup::Leftovers => "Leftovers",
            RuleGroup::Apps => "App and browser caches",
            RuleGroup::YourCall => "Your call",
            RuleGroup::Windows => "Windows",
        }
    }

    pub fn blurb(self, admin: bool) -> &'static str {
        match self {
            RuleGroup::Leftovers => "Nothing uses these any more",
            RuleGroup::Apps => "Logins and settings stay. An open app is skipped.",
            RuleGroup::YourCall => "Unchecked until you tick them",
            RuleGroup::Windows if admin => "What Disk Cleanup's system-file mode clears",
            RuleGroup::Windows => "Restart as admin to scan these",
        }
    }
}

/// Things to delete that are not plain paths.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    EmptyRecycleBin,
    DeliveryOptimization,
    DeleteDriver(String),
    /// A display copy of an item the admin helper owns; never run in the window.
    Elevated,
}

/// One thing to delete: a path (checked by the guard), an action such as emptying the Recycle Bin, or a Disk
/// Cleanup category that Windows' own handler removes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub path: Option<String>,
    pub label: String,
    pub action: Option<Action>,
    /// A Disk Cleanup category (a VolumeCaches subkey name) run by cleanmgr in one pass at the end of Clean.
    pub handler: Option<String>,
    /// Bytes, or None when unknown.
    pub size: Option<u64>,
    /// What happened when cleaning: "deleted", "refused: ...", "partly in use, ..." and so on.
    pub result: Option<String>,
}

impl Item {
    pub fn path(p: impl Into<String>) -> Item {
        let p = p.into();
        Item { label: p.clone(), path: Some(p), action: None, handler: None, size: None, result: None }
    }
    pub fn labelled(p: impl Into<String>, label: impl Into<String>) -> Item {
        Item { label: label.into(), ..Item::path(p) }
    }
    pub fn sized(p: impl Into<String>, size: u64) -> Item {
        Item { size: Some(size), ..Item::path(p) }
    }
    pub fn action(label: impl Into<String>, action: Action, size: Option<u64>) -> Item {
        Item { path: None, label: label.into(), action: Some(action), handler: None, size, result: None }
    }
    pub fn handler(label: impl Into<String>, handler: impl Into<String>, size: Option<u64>) -> Item {
        Item { path: None, label: label.into(), action: None, handler: Some(handler.into()), size, result: None }
    }
}

/// Something a rule found but will not delete, with the reason.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skip {
    pub what: String,
    pub why: String,
}

pub fn skip(what: impl Into<String>, why: impl Into<String>) -> Skip {
    Skip { what: what.into(), why: why.into() }
}

/// Process names and install folders that mean "the app is open", with the message to show.
#[derive(Clone, Debug)]
pub struct Busy {
    pub names: Vec<String>,
    pub dirs: Vec<String>,
    pub message: String,
}

pub type Finder = Arc<dyn Fn(&ScanContext, &mut Vec<Skip>) -> Vec<Item> + Send + Sync>;

#[derive(Clone)]
pub struct Rule {
    pub id: String,
    pub group: RuleGroup,
    pub title: String,
    pub why: String,
    pub find: Finder,
    pub default_checked: bool,
    pub needs_admin: bool,
    pub busy: Option<Busy>,
    pub items: Vec<Item>,
    pub skipped: Vec<Skip>,
    /// Why the whole rule cannot be cleaned right now (app open, needs admin, scan error), else None.
    pub blocked: Option<String>,
    /// Its items came from the admin helper; cleaning them goes back to the helper.
    pub scanned_elevated: bool,
}

impl Rule {
    pub fn new(id: &str, group: RuleGroup, title: &str, why: &str, find: Finder) -> Rule {
        Rule {
            id: id.into(),
            group,
            title: title.into(),
            why: why.into(),
            find,
            default_checked: group != RuleGroup::YourCall,
            needs_admin: false,
            busy: None,
            items: Vec::new(),
            skipped: Vec::new(),
            blocked: None,
            scanned_elevated: false,
        }
    }
    pub fn admin(mut self) -> Rule {
        self.needs_admin = true;
        self
    }
    pub fn unchecked(mut self) -> Rule {
        self.default_checked = false;
        self
    }
    pub fn busy_when(mut self, names: &[&str], dirs: Vec<String>, message: String) -> Rule {
        self.busy = Some(Busy { names: names.iter().map(|s| s.to_string()).collect(), dirs, message });
        self
    }
    pub fn total(&self) -> u64 {
        self.items.iter().map(|i| i.size.unwrap_or(0)).sum()
    }
    pub fn has_unknown_size(&self) -> bool {
        self.items.iter().any(|i| i.size.is_none())
    }
    pub fn can_clean(&self) -> bool {
        self.blocked.is_none() && !self.items.is_empty()
    }
    /// Worth a row in the window: it found something, or has a reason to show.
    pub fn visible(&self) -> bool {
        !self.items.is_empty() || !self.skipped.is_empty()
    }
}

/// Running processes, by exe name and full image path.
#[derive(Clone, Debug, Default)]
pub struct ProcessSnapshot {
    pub names: Vec<String>,
    pub images: Vec<String>,
}

impl ProcessSnapshot {
    pub fn take() -> ProcessSnapshot {
        let list = crate::processes::list();
        ProcessSnapshot {
            names: list.iter().map(|p| p.exe.to_lowercase()).collect(),
            images: list.iter().filter_map(|p| crate::processes::image_path(p.pid)).map(|i| paths::norm(&i).unwrap_or(i)).collect(),
        }
    }

    /// True when a process with one of these exe names runs, or one runs from inside one of these folders.
    pub fn running(&self, names: &[String], dirs: &[String]) -> bool {
        if names.iter().any(|n| self.names.contains(&n.to_lowercase())) {
            return true;
        }
        dirs.iter().filter(|d| paths::is_dir(d)).filter_map(|d| paths::norm(d)).any(|nd| self.images.iter().any(|img| paths::under_normalized(img, &nd)))
    }

    pub fn busy(&self, b: &Option<Busy>) -> Option<String> {
        b.as_ref().filter(|b| self.running(&b.names, &b.dirs)).map(|b| b.message.clone())
    }
}

/// What each finder may consult: running processes and, lazily, the installed NVIDIA driver.
pub struct ScanContext {
    pub processes: ProcessSnapshot,
    nvidia: std::sync::OnceLock<Option<paths::VersionKey>>,
}

impl ScanContext {
    pub fn new(processes: ProcessSnapshot) -> ScanContext {
        ScanContext { processes, nvidia: std::sync::OnceLock::new() }
    }

    pub fn nvidia_driver(&self) -> Option<paths::VersionKey> {
        self.nvidia.get_or_init(system::nvidia_driver_version).clone()
    }

    pub fn running(&self, names: &[&str], dirs: &[String]) -> bool {
        self.processes.running(&names.iter().map(|s| s.to_string()).collect::<Vec<_>>(), dirs)
    }
}

/// What is deliberately left alone, shown so people can see it.
pub const KEPT: [(&str, &str); 12] = [
    ("Claude desktop VM (vm_bundles)", "Claude's local VM runs on it"),
    ("Ollama, LM Studio and Hugging Face models", "Re-downloading costs hours"),
    ("WinGet tools, Programs, PATH folders", "Installed tools such as ffmpeg and uv"),
    ("NVIDIA and AMD shader caches", "Games stutter while they rebuild. The DirectX one is under Your call"),
    ("Chrome on-device AI model", "Chrome features use it"),
    ("Windows minidumps", "Needed to diagnose blue screens"),
    ("Your Downloads folder", "Windows Disk Cleanup offers to empty it. This app never will"),
    ("File History versions", "Your backups. Disk Cleanup lists them; this app leaves them"),
    ("OneDrive, CrossDevice, .ssh", "Synced or personal files"),
    ("Browser logins, cookies and history", "Personal data; clearing them signs you out"),
    ("Pictures, Music, Videos, game saves", "Your files"),
    ("Folders listed in protect.txt", "You asked for these to be left alone"),
];
