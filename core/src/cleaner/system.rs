//! What only Windows can remove: the Recycle Bin, Disk Cleanup's own handlers, old driver packages and the
//! Delivery Optimization cache. Plus the installed NVIDIA driver version.

use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::Value;
use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, KEY_READ, KEY_WOW64_64KEY};

use super::paths::{K, VersionKey, join, tree_size};
use crate::proc::{self, minutes};
use crate::updater::sources::{rows_of, str_of};

// ------------------------------------------------------------------ Recycle Bin

/// Recycle Bin size and item count across all drives.
pub fn query_recycle_bin() -> Option<(u64, u64)> {
    use windows::Win32::UI::Shell::{SHQUERYRBINFO, SHQueryRecycleBinW};
    use windows::core::PCWSTR;
    let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info).ok()? };
    Some((info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64))
}

/// Empties the Recycle Bin on all drives without confirmation, progress UI or sound.
pub fn empty_recycle_bin() -> Result<(), String> {
    use windows::Win32::UI::Shell::{SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW};
    use windows::core::PCWSTR;
    match unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND) } {
        Ok(()) => Ok(()),
        // E_UNEXPECTED is what the shell returns when the bin is already empty.
        Err(e) if e.code().0 as u32 == 0x8000_FFFF => Ok(()),
        Err(e) => Err(e.message().to_string()),
    }
}

// ------------------------------------------------------------------ Disk Cleanup handlers

/// The cleanmgr /sagerun slot this app uses. Its flags are removed before and after every run.
pub const SAGE: u32 = 777;
const FLAG: &str = "StateFlags0777";
const VOLUME_CACHES: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VolumeCaches";

/// Handler categories behind the "Windows Update cleanup" row: category name, what it is.
pub const UPDATE_HANDLERS: [(&str, &str); 11] = [
    ("Update Cleanup", "Superseded Windows Update files"),
    ("Windows Upgrade Log Files", "Logs from past Windows upgrades"),
    ("Temporary Setup Files", "Files Windows setup left behind"),
    ("Upgrade Discarded Files", "Files a past upgrade set aside"),
    ("Windows Reset Log Files", "Logs from past resets"),
    ("Language Pack", "Language resources no account here uses"),
    ("Downloaded Program Files", "Old ActiveX controls and Java applets"),
    ("Temporary Sync Files", "Leftover sync scratch files"),
    ("Offline Pages Files", "Old offline web pages"),
    ("Active Setup Temp Folders", "Old component setup scratch"),
    ("Content Indexer Cleaner", "Old search indexer files"),
];
pub const PREVIOUS_INSTALLATIONS: &str = "Previous Installations";
pub const ESD_FILES: &str = "Windows ESD installation files";

fn caches(write: bool) -> Option<RegKey> {
    let flags = if write { KEY_ALL_ACCESS | KEY_WOW64_64KEY } else { KEY_READ | KEY_WOW64_64KEY };
    RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(VOLUME_CACHES, flags).ok()
}

pub fn handler_exists(category: &str) -> bool {
    caches(false).is_some_and(|k| k.open_subkey(category).is_ok())
}

/// Runs Disk Cleanup for exactly these categories. First every category's flag in this app's slot is deleted,
/// so a crashed earlier run cannot widen this one (the Downloads folder is a category too); then only the ticked
/// categories that exist are flagged; and the flags are deleted again afterwards, even if cleanmgr fails.
pub fn run_handlers(categories: &[String]) -> Result<Vec<String>, String> {
    let root = caches(true).ok_or("Disk Cleanup's settings need admin")?;
    let all: Vec<String> = root.enum_keys().flatten().collect();
    let clear = |names: &[String]| {
        for c in names {
            if let Ok(k) = root.open_subkey_with_flags(c, KEY_ALL_ACCESS | KEY_WOW64_64KEY) {
                let _ = k.delete_value(FLAG);
            }
        }
    };
    clear(&all);
    let mut wanted: Vec<String> = Vec::new();
    for c in categories {
        if all.iter().any(|a| a.eq_ignore_ascii_case(c)) && !wanted.iter().any(|w| w.eq_ignore_ascii_case(c)) {
            wanted.push(c.clone());
        }
    }
    if wanted.is_empty() {
        return Ok(wanted);
    }
    for c in &wanted {
        if let Ok(k) = root.open_subkey_with_flags(c, KEY_ALL_ACCESS | KEY_WOW64_64KEY) {
            let _ = k.set_value(FLAG, &2u32);
        }
    }
    // Like the reference, the exit code is not treated as failure: cleanmgr does not document it.
    let r = proc::run_simple(&join(&K.windows, r"System32\cleanmgr.exe"), [format!("/sagerun:{SAGE}")], Duration::from_secs(2 * 3600));
    clear(&all);
    if r.timed_out { Err("Disk Cleanup didn't finish in 2 hours".into()) } else { Ok(wanted) }
}

// ------------------------------------------------------------------ drivers

/// A third-party driver package in the driver store.
#[derive(Clone, Debug, PartialEq)]
pub struct DriverPackage {
    /// oemNN.inf
    pub published_name: String,
    /// The INF's own name, for example netwtw10.inf.
    pub original_name: String,
    pub provider: String,
    pub version: VersionKey,
}

pub const KEPT_BY_WINDOWS: &str = "Windows kept it (still in use)";

type OldList = Vec<(DriverPackage, VersionKey)>;
type SkipList = Vec<(DriverPackage, String)>;

/// Groups packages by original INF name plus provider and picks every package older than the newest one in its
/// group. The newest, and any package that ties with it, stay. Packages a device uses now are never picked.
pub fn select_old(packages: &[DriverPackage], infs_in_use: &HashSet<String>) -> (OldList, SkipList) {
    let mut old = Vec::new();
    let mut skipped = Vec::new();
    let mut keys: Vec<(String, String)> = Vec::new();
    for p in packages {
        let k = (p.original_name.to_lowercase(), p.provider.clone());
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    for (name, provider) in keys {
        let mut group: Vec<&DriverPackage> = packages.iter().filter(|p| p.original_name.to_lowercase() == name && p.provider == provider).collect();
        let newest = group.iter().map(|p| p.version.clone()).max().unwrap_or(VersionKey(vec![0]));
        group.sort_by_key(|p| p.published_name.to_lowercase());
        for p in group {
            if p.version >= newest {
                continue;
            }
            if infs_in_use.contains(&p.published_name.to_lowercase()) {
                skipped.push((p.clone(), "a device still uses it".to_string()));
            } else {
                old.push((p.clone(), newest.clone()));
            }
        }
    }
    (old, skipped)
}

/// Third-party driver packages, listed language-neutrally through the DISM PowerShell module. Needs admin.
pub fn list_third_party_drivers() -> Result<Vec<DriverPackage>, String> {
    let script = "Get-WindowsDriver -Online | Where-Object { -not $_.InBox -and $_.Driver -like 'oem*' } | \
                  ForEach-Object { [pscustomobject]@{ published = $_.Driver; original = (Split-Path $_.OriginalFileName -Leaf); provider = \"$($_.ProviderName)\"; version = \"$($_.Version)\" } } | \
                  ConvertTo-Json -Compress";
    let r = proc::powershell(script, minutes(5), None);
    let Some(json) = proc::last_json_line(&r.output, true) else {
        return if r.ok() && r.output.trim().is_empty() { Ok(Vec::new()) } else { Err(r.last_line().unwrap_or_else(|| "no answer".into())) };
    };
    let doc: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Ok(rows_of(&doc)
        .into_iter()
        .map(|e| DriverPackage {
            published_name: str_of(e, "published"),
            original_name: str_of(e, "original"),
            provider: str_of(e, "provider"),
            version: VersionKey::parse(&str_of(e, "version")),
        })
        .filter(|p| !p.published_name.is_empty())
        .collect())
}

/// oemNN.inf names a device is installed with right now (InfPath under each device class), lower-case.
pub fn infs_in_use() -> HashSet<String> {
    let mut used = HashSet::new();
    let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(r"SYSTEM\CurrentControlSet\Control\Class", KEY_READ | KEY_WOW64_64KEY) else {
        return used;
    };
    for cls in k.enum_keys().flatten() {
        let Ok(ck) = k.open_subkey(&cls) else { continue };
        for sub in ck.enum_keys().flatten() {
            if let Ok(inf) = ck.open_subkey(&sub).and_then(|d| d.get_value::<String, _>("InfPath")) {
                if !inf.is_empty() {
                    used.insert(inf.to_lowercase());
                }
            }
        }
    }
    used
}

/// Bytes the package takes in DriverStore\FileRepository, or None if unknown. The folder name is the "Active"
/// value in the live driver database (the DRIVERS hive); SYSTEM\DriverDatabase is a stale copy, used as fallback.
pub fn driver_package_size(published_name: &str) -> Option<u64> {
    for db in [r"DRIVERS\DriverDatabase", r"SYSTEM\DriverDatabase"] {
        let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(format!(r"{db}\DriverInfFiles\{published_name}"), KEY_READ | KEY_WOW64_64KEY) else {
            continue;
        };
        let active = k.get_value::<String, _>("Active").ok().or_else(|| k.get_value::<Vec<String>, _>("Active").ok().and_then(|v| v.into_iter().next()));
        if let Some(a) = active.filter(|a| !a.trim().is_empty()) {
            return Some(tree_size(&join(&K.windows, &format!(r"System32\DriverStore\FileRepository\{a}"))));
        }
    }
    None
}

/// pnputil /delete-driver without /force, so Windows itself refuses a package that is still in use.
pub fn delete_driver(published_name: &str) -> Result<(), String> {
    let n = published_name.to_lowercase();
    if !n.starts_with("oem") || !n.ends_with(".inf") || published_name.contains([' ', '"', '\\', '/']) {
        return Err("Not a third-party driver package name.".into());
    }
    let r = proc::run_simple(&join(&K.windows, r"System32\pnputil.exe"), ["/delete-driver", published_name], minutes(5));
    if r.exit_code == 0 { Ok(()) } else { Err(KEPT_BY_WINDOWS.into()) }
}

// ------------------------------------------------------------------ other actions

pub fn clear_delivery_optimization() -> Result<(), String> {
    let r = proc::powershell("Delete-DeliveryOptimizationCache -Force", minutes(30), None);
    if r.ok() { Ok(()) } else { Err(r.lines().first().map(|s| s.to_string()).unwrap_or_else(|| format!("PowerShell exited with code {}.", r.exit_code))) }
}

static VERSION_LIKE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+(\.\d+)+$").unwrap());

/// The installed NVIDIA driver version from nvidia-smi, or None without an NVIDIA GPU. Looks only in System32
/// and the old NVSMI folder, never on PATH.
pub fn nvidia_driver_version() -> Option<VersionKey> {
    for exe in [join(&K.windows, r"System32\nvidia-smi.exe"), join(&K.program_files, r"NVIDIA Corporation\NVSMI\nvidia-smi.exe")] {
        if !std::path::Path::new(&exe).is_file() {
            continue;
        }
        let r = proc::run_simple(&exe, ["--query-gpu=driver_version", "--format=csv,noheader"], Duration::from_secs(20));
        if let Some(line) = r.lines().first().map(|l| l.trim().to_string()) {
            if r.exit_code == 0 && VERSION_LIKE.is_match(&line) {
                return Some(VersionKey::parse(&line));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(oem: &str, name: &str, ver: &str) -> DriverPackage {
        DriverPackage { published_name: oem.into(), original_name: name.into(), provider: "Intel".into(), version: VersionKey::parse(ver) }
    }

    #[test]
    fn picks_older_driver_packages_only() {
        let packages = [pkg("oem1.inf", "netwtw10.inf", "22.1"), pkg("oem2.inf", "netwtw10.inf", "23.0"), pkg("oem3.inf", "NETWTW10.inf", "22.5"), pkg("oem4.inf", "other.inf", "1.0")];
        let in_use: HashSet<String> = ["oem3.inf".to_string()].into();
        let (old, skipped) = select_old(&packages, &in_use);
        assert_eq!(old.iter().map(|(p, _)| p.published_name.as_str()).collect::<Vec<_>>(), ["oem1.inf"]);
        assert_eq!(old[0].1.to_string(), "23.0");
        assert_eq!(skipped[0].0.published_name, "oem3.inf");
    }

    #[test]
    fn refuses_odd_driver_names() {
        assert!(delete_driver("C:\\x.inf").is_err());
        assert!(delete_driver("oem1.inf /force").is_err());
    }
}
