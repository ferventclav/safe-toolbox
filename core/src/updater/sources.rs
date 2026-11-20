//! Chocolatey and Scoop, and the warnings worth reading before installing anything.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

use super::model::{SourceResult, UpdateItem, UpdateSource};
use super::policy::squash;
use super::versions;
use crate::proc::{self, Cancel, minutes};
use crate::tools::{self, contains_ic};

/// Chocolatey packages. Upgrades need admin, so they go to the elevated helper.
pub struct ChocolateySource;

static PKG_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.(install|portable|app)$").unwrap());

/// Popular Chocolatey packages whose names share nothing with the winget id (squashed choco id to squashed winget id start).
const KNOWN_PAIRS: [(&str, &str); 5] = [
    ("vscode", "microsoftvisualstudiocode"),
    ("notepadplusplus", "notepadnotepad"),
    ("nodejslts", "openjsnodejs"),
    ("adobereader", "adobeacrobatreader"),
    ("gh", "githubcli"),
];

impl ChocolateySource {
    pub fn check(cancel: &Cancel) -> SourceResult {
        let Some(choco) = tools::choco() else { return SourceResult::absent(UpdateSource::Chocolatey) };
        // Exit code 2 means "outdated packages found" when Chocolatey's enhanced exit codes are on.
        let r = proc::run(&choco, ["outdated", "-r", "--ignore-unfound"], minutes(6), None, Some(cancel));
        let items = Self::parse_outdated(&r.output);
        let managed = Self::managed_ids();
        if items.is_empty() && !matches!(r.exit_code, 0 | 2) {
            let mut res = SourceResult::failed(UpdateSource::Chocolatey, format!("Chocolatey could not check: {}", r.last_line().unwrap_or_default()));
            res.managed = managed;
            return res;
        }
        let mut res = SourceResult::items(UpdateSource::Chocolatey, items, "All Chocolatey packages are up to date");
        res.managed = managed;
        res
    }

    /// "name|current|available|pinned" lines from "choco outdated -r".
    pub fn parse_outdated(output: &str) -> Vec<UpdateItem> {
        let mut items = Vec::new();
        for line in output.split('\n').map(str::trim) {
            let p: Vec<&str> = line.split('|').collect();
            if p.len() < 4 || p[0].is_empty() || versions::parse(p[2]).is_none() {
                continue;
            }
            let mut i = UpdateItem::new(UpdateSource::Chocolatey, p[0], p[0]).versions(p[1], p[2]);
            i.pinned = p[3].eq_ignore_ascii_case("true");
            i.is_major = versions::is_major_jump(p[1], p[2]);
            i.needs_admin = true;
            items.push(i);
        }
        items
    }

    /// Every package Chocolatey has installed, read from its lib folder (works on Chocolatey 1 and 2).
    pub fn managed_ids() -> Vec<String> {
        std::fs::read_dir(tools::chocolatey_root().join("lib"))
            .map(|rd| rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    }

    /// The Chocolatey package a winget app belongs to, if any. Matches the winget id's last part ("Git.Git" to "git"),
    /// the whole id ("Google.Chrome" to "googlechrome"), a name that is the package id plus a version ("7-Zip 26.04"
    /// to "7zip"), or a known pair ("vscode"), but not "GitHub CLI" to "git".
    pub fn owner_of(winget_item: &UpdateItem, choco_ids: &[String]) -> Option<String> {
        let whole = squash(&winget_item.id);
        let last = squash(winget_item.id.split('.').next_back().unwrap_or(""));
        let name = squash(&winget_item.name);
        for raw in choco_ids {
            let id = squash(&PKG_SUFFIX.replace(raw, ""));
            if let Some((_, pair)) = KNOWN_PAIRS.iter().find(|(k, _)| *k == id) {
                if whole.starts_with(pair) {
                    return Some(raw.clone());
                }
            }
            if id.len() < 3 || id == "chocolatey" {
                continue;
            }
            let digit_after = name.starts_with(&id) && name.len() > id.len() && name.as_bytes()[id.len()].is_ascii_digit();
            if last == id || whole == id || name == id || digit_after {
                return Some(raw.clone());
            }
        }
        None
    }

    pub fn upgrade_args(item: &UpdateItem) -> Vec<String> {
        vec!["upgrade".into(), item.id.clone(), "-y".into(), "--no-progress".into(), "-r".into()]
    }
}

/// Scoop apps. They live in the user's profile, so they update without admin.
pub struct ScoopSource;

pub const SCOOP_STATUS_SCRIPT: &str = r#"
$ErrorActionPreference = 'Continue'
scoop update *> $null
$rows = foreach ($x in @(scoop status 6> $null)) {
    if ($x -is [string] -or -not $x.Name) { continue }
    [pscustomobject]@{ name = "$($x.Name)"; current = "$($x.'Installed Version')"; latest = "$($x.'Latest Version')"; info = "$($x.Info)" }
}
ConvertTo-Json -InputObject @($rows) -Compress
"#;

static SAFE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9._\-]+$").unwrap());

pub fn str_of(v: &Value, name: &str) -> String {
    v.get(name).and_then(Value::as_str).unwrap_or("").to_string()
}

/// A JSON array, or a single object as a one-element list (PowerShell's ConvertTo-Json shape).
pub fn rows_of(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(a) => a.iter().collect(),
        other => vec![other],
    }
}

impl ScoopSource {
    pub fn check(cancel: &Cancel) -> SourceResult {
        if tools::scoop().is_none() {
            return SourceResult::absent(UpdateSource::Scoop);
        }
        let r = proc::powershell(SCOOP_STATUS_SCRIPT, minutes(6), Some(cancel));
        match Self::parse_status(&r.output) {
            None => SourceResult::failed(UpdateSource::Scoop, format!("Scoop could not check: {}", r.last_line().unwrap_or_default())),
            Some(items) => SourceResult::items(UpdateSource::Scoop, items, "All Scoop apps are up to date"),
        }
    }

    pub fn parse_status(output: &str) -> Option<Vec<UpdateItem>> {
        let json = proc::last_json_line(output, true)?;
        let doc: Value = serde_json::from_str(json).ok()?;
        let mut items = Vec::new();
        for e in rows_of(&doc).into_iter().filter(|e| e.is_object()) {
            let (name, current, latest, info) = (str_of(e, "name"), str_of(e, "current"), str_of(e, "latest"), str_of(e, "info"));
            if name.is_empty() || latest.is_empty() || latest == current || !SAFE_NAME.is_match(&name) {
                continue;
            }
            let mut i = UpdateItem::new(UpdateSource::Scoop, &name, &name).versions(&current, &latest);
            i.is_major = versions::is_major_jump(&current, &latest);
            i.unsafe_reason = if contains_ic(&info, "Held") {
                Some("Held in Scoop (scoop hold), so it stays at this version until you run scoop unhold.".into())
            } else if contains_ic(&info, "Global") {
                Some("Installed for all users (scoop install -g), which needs an admin terminal to update.".into())
            } else if contains_ic(&info, "fail") {
                Some("Its last install failed. Reinstall it with Scoop before updating it.".into())
            } else {
                None
            };
            items.push(i);
        }
        Some(items)
    }

    pub fn update_script(item: &UpdateItem) -> Option<String> {
        SAFE_NAME.is_match(&item.id).then(|| format!("scoop update '{}'", item.id))
    }
}

/// Warnings worth reading before installing anything (the same checks Windows Update makes).
pub mod preflight {
    use super::*;

    pub fn warnings() -> Vec<String> {
        let mut w = Vec::new();
        if reboot_pending() {
            w.push("Windows is waiting for a restart. Installers can fail until then, so restart first if you can.".into());
        }
        if let Some((on_battery, pct)) = battery() {
            if on_battery {
                let pct = pct.filter(|p| *p <= 100).map(|p| format!(" ({p}%)")).unwrap_or_default();
                w.push(format!("This PC is on battery{pct}. Plug it in: an installer cut off by a flat battery can leave an app broken."));
            }
        }
        if let Some((_, free)) = crate::disk::system_drive_usage() {
            let free_gb = free as f64 / 1024.0 / 1024.0 / 1024.0;
            if free_gb < 10.0 {
                w.push(format!("Only {} GB is free on {}. Large updates may fail to install.", crate::disk::one_decimal(free_gb), tools::system_drive()));
            }
        }
        w
    }

    /// (on battery, charge percent) from GetSystemPowerStatus; None on a PC without a battery.
    pub fn battery() -> Option<(bool, Option<u8>)> {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut p = SYSTEM_POWER_STATUS::default();
        unsafe { GetSystemPowerStatus(&mut p).ok()? };
        // 128 = no system battery.
        if p.BatteryFlag & 128 != 0 {
            return None;
        }
        Some((p.ACLineStatus == 0, Some(p.BatteryLifePercent)))
    }

    pub fn reboot_pending() -> bool {
        let lm = RegKey::predef(HKEY_LOCAL_MACHINE);
        [
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
        ]
        .iter()
        .any(|k| lm.open_subkey(k).is_ok())
    }

    /// Makes a restore point when System Restore is on. It never turns System Restore on.
    pub const RESTORE_POINT_SCRIPT: &str = r#"
$w = $null
try {
    Checkpoint-Computer -Description 'Before Safe Toolbox updates' -RestorePointType MODIFY_SETTINGS -ErrorAction Stop -WarningVariable w -WarningAction SilentlyContinue
    if ($w) { "SKIPPED: $w" } else { "CREATED" }
} catch { "FAILED: $($_.Exception.Message)" }
"#;

    pub fn describe_restore_point(output: &str) -> String {
        // The script's own result line, wherever it is: PowerShell can add progress records (CLIXML) after it.
        let line = output
            .split('\n')
            .map(str::trim)
            .filter(|l| l.starts_with("CREATED") || l.starts_with("SKIPPED") || l.starts_with("FAILED"))
            .last()
            .unwrap_or("");
        if line.starts_with("CREATED") {
            "Made a restore point before updating.".into()
        } else if line.starts_with("SKIPPED") {
            "Windows already made a restore point in the last 24 hours, so it didn't make another.".into()
        } else if let Some(rest) = line.strip_prefix("FAILED: ") {
            format!("No restore point was made: {}. Safe Toolbox never turns System Restore on.", rest.trim_end_matches('.'))
        } else {
            "No restore point was made, and Windows didn't say why. Safe Toolbox never turns System Restore on.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chocolatey_outdated_lines() {
        let items = ChocolateySource::parse_outdated("Chocolatey v2.4.0\ngit|2.49.0|2.50.0|false\nvcredist140|14.51|14.52.37029|true\nbad line\n");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "git");
        assert!(items[1].pinned);
        assert!(items.iter().all(|i| i.needs_admin));
    }

    #[test]
    fn chocolatey_owner_matching() {
        let ids: Vec<String> = ["git.install", "7zip", "vscode", "googlechrome", "chocolatey"].iter().map(|s| s.to_string()).collect();
        let w = |id: &str, name: &str| UpdateItem::new(UpdateSource::Apps, id, name);
        assert_eq!(ChocolateySource::owner_of(&w("Git.Git", "Git"), &ids).as_deref(), Some("git.install"));
        assert_eq!(ChocolateySource::owner_of(&w("7zip.7zip", "7-Zip 26.04"), &ids).as_deref(), Some("7zip"));
        assert_eq!(ChocolateySource::owner_of(&w("Microsoft.VisualStudioCode", "VS Code"), &ids).as_deref(), Some("vscode"));
        assert_eq!(ChocolateySource::owner_of(&w("Google.Chrome", "Google Chrome"), &ids).as_deref(), Some("googlechrome"));
        assert_eq!(ChocolateySource::owner_of(&w("GitHub.cli", "GitHub CLI"), &ids), None);
    }

    #[test]
    fn scoop_status() {
        let out = r#"noise
[{"name":"ffmpeg","current":"7.0","latest":"8.0","info":""},{"name":"held","current":"1","latest":"2","info":"Held package"},{"name":"same","current":"1","latest":"1","info":""},{"name":"bad name;","current":"1","latest":"2","info":""}]"#;
        let items = ScoopSource::parse_status(out).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].is_major);
        assert!(items[1].unsafe_reason.as_deref().unwrap().contains("Held"));
        assert!(ScoopSource::parse_status("no json").is_none());
        let single = ScoopSource::parse_status(r#"{"name":"git","current":"1.0","latest":"1.1","info":""}"#).unwrap();
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn restore_point_lines() {
        assert_eq!(preflight::describe_restore_point("x\nCREATED\n#< CLIXML"), "Made a restore point before updating.");
        assert!(preflight::describe_restore_point("SKIPPED: 24h").starts_with("Windows already"));
        assert_eq!(
            preflight::describe_restore_point("FAILED: System Restore is off."),
            "No restore point was made: System Restore is off. Safe Toolbox never turns System Restore on."
        );
        assert!(preflight::describe_restore_point("").contains("didn't say why"));
    }
}
