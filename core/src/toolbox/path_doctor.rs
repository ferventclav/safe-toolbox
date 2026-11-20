//! Finds PATH entries that point nowhere, repeat an earlier entry or are empty; winget command links whose target
//! was deleted; and which copy of common commands actually runs. It edits only your user PATH, after saving the
//! old value so it can be put back.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};

use super::exports;

const MACHINE_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

pub const COMMON_COMMANDS: [&str; 18] = ["python", "py", "pip", "node", "npm", "npx", "git", "winget", "code", "ffmpeg", "yt-dlp", "rg", "uv", "cargo", "rustc", "dotnet", "java", "claude"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum PathScope {
    Machine,
    User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum PathProblem {
    None,
    Missing,
    Duplicate,
    Empty,
    NotAFolder,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathEntry {
    pub scope: PathScope,
    pub index: usize,
    pub raw: String,
    pub expanded: String,
    pub problem: PathProblem,
    pub duplicate_of: Option<String>,
    pub fixable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandHit {
    pub command: String,
    /// In PATH order: the first is the one that runs when you type the command.
    pub locations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrokenLink {
    pub link: String,
    pub target: String,
}

/// The PATH value exactly as stored, with %VARIABLES% unexpanded.
pub fn raw_path(scope: PathScope) -> String {
    let key = match scope {
        PathScope::User => RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Environment", KEY_READ),
        PathScope::Machine => RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(MACHINE_KEY, KEY_READ),
    };
    key.and_then(|k| k.get_value::<String, _>("Path")).unwrap_or_default()
}

/// Both PATHs in the order Windows joins them (machine first, then user), each entry judged.
pub fn analyze_system() -> Vec<PathEntry> {
    analyze(&raw_path(PathScope::Machine), &raw_path(PathScope::User), &|d| Path::new(d).is_dir(), &|f| Path::new(f).is_file())
}

pub fn analyze(machine: &str, user: &str, dir_exists: &dyn Fn(&str) -> bool, file_exists: &dyn Fn(&str) -> bool) -> Vec<PathEntry> {
    let mut list = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for (scope, raw) in [(PathScope::Machine, machine), (PathScope::User, user)] {
        let parts: Vec<&str> = raw.split(';').collect();
        for (i, r) in parts.iter().enumerate() {
            let entry = |problem, expanded: String, duplicate_of| PathEntry { scope, index: i, raw: r.to_string(), expanded, problem, duplicate_of, fixable: scope == PathScope::User && problem != PathProblem::None };
            // A trailing ";" is harmless and common; only empty entries in the middle are flagged.
            if r.trim().is_empty() {
                if i < parts.len() - 1 {
                    list.push(entry(PathProblem::Empty, String::new(), None));
                }
                continue;
            }
            let expanded = crate::tools::expand(r.trim().trim_matches('"'));
            let key = normalize(&expanded).to_lowercase();
            let (problem, dup) = match seen.get(&key) {
                Some(first) => (PathProblem::Duplicate, Some(first.clone())),
                None if !dir_exists(&expanded) => (if file_exists(&expanded) { PathProblem::NotAFolder } else { PathProblem::Missing }, None),
                None => (PathProblem::None, None),
            };
            seen.entry(key).or_insert_with(|| format!("{} PATH entry {}", if scope == PathScope::Machine { "system" } else { "user" }, i + 1));
            list.push(entry(problem, expanded, dup));
        }
    }
    list
}

fn normalize(p: &str) -> String {
    std::path::absolute(p).map(|a| a.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string()).trim_end_matches('\\').to_string()
}

/// The user PATH with the fixable entries removed, keeping order and the rest exactly as written.
pub fn cleaned(user_raw: &str, entries: &[PathEntry]) -> String {
    let drop: HashSet<usize> = entries.iter().filter(|e| e.scope == PathScope::User && e.problem != PathProblem::None).map(|e| e.index).collect();
    user_raw.split(';').enumerate().filter(|(i, p)| !drop.contains(i) && !p.trim().is_empty()).map(|(_, p)| p).collect::<Vec<_>>().join(";")
}

fn write_user_path(value: &str) -> Result<(), String> {
    let env = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE).map_err(|_| "Your Environment registry key could not be opened.".to_string())?;
    let expandable = env.get_raw_value("Path").map(|v| v.vtype != winreg::enums::RegType::REG_SZ).unwrap_or(true);
    crate::path_env::write_user_path(&env, value, expandable).map_err(|e| e.to_string())?;
    crate::path_env::broadcast_environment_change();
    Ok(())
}

/// Removes the problem entries from your user PATH. Saves the old value first and returns the backup path.
pub fn fix_user_path(entries: &[PathEntry]) -> Result<String, String> {
    let raw = raw_path(PathScope::User);
    let backup = exports().join(format!("user-path-backup-{}.txt", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    std::fs::write(&backup, &raw).map_err(|e| e.to_string())?;
    write_user_path(&cleaned(&raw, entries))?;
    Ok(backup.to_string_lossy().into_owned())
}

/// Puts back a user PATH saved by fix_user_path.
pub fn restore(backup_file: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(backup_file).map_err(|e| e.to_string())?;
    write_user_path(text.trim())
}

pub fn latest_backup() -> Option<String> {
    let mut files: Vec<String> = std::fs::read_dir(exports()).ok()?.flatten().map(|e| e.path().to_string_lossy().into_owned()).filter(|p| {
        let n = Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        n.starts_with("user-path-backup-") && n.ends_with(".txt")
    }).collect();
    files.sort();
    files.pop()
}

/// Where each common command is found, in PATH order, so a shadowed python or node shows up.
pub fn commands(entries: &[PathEntry]) -> Vec<CommandHit> {
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let exts: Vec<String> = pathext.split(';').filter(|s| !s.is_empty()).map(str::to_lowercase).collect();
    let dirs: Vec<&str> = entries.iter().filter(|e| e.problem == PathProblem::None).map(|e| e.expanded.as_str()).collect();
    COMMON_COMMANDS
        .iter()
        .filter_map(|cmd| {
            let found: Vec<String> = dirs.iter().filter_map(|d| exts.iter().map(|x| Path::new(d).join(format!("{cmd}{x}"))).find(|f| f.is_file())).map(|f| f.to_string_lossy().into_owned()).collect();
            (!found.is_empty()).then(|| CommandHit { command: cmd.to_string(), locations: found })
        })
        .collect()
}

/// Command links in WinGet\Links whose target file was deleted (the yt-dlp FFmpeg case of 3 Nov 2026).
pub fn broken_winget_links() -> Vec<BrokenLink> {
    let links = crate::updater::path_health::links_dir();
    let Ok(rd) = std::fs::read_dir(&links) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let target = std::fs::read_link(e.path()).ok()?;
            let full = if target.is_absolute() { target } else { links.join(target) };
            (!full.exists()).then(|| BrokenLink { link: e.path().to_string_lossy().into_owned(), target: full.to_string_lossy().into_owned() })
        })
        .collect()
}

/// Whether a path is the Windows Store "python" alias, which opens the Store instead of running Python.
pub fn is_store_alias(path: &str) -> bool {
    crate::tools::contains_ic(path, r"\Microsoft\WindowsApps\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(machine: &str, user: &str) -> Vec<PathEntry> {
        let dirs = [r"C:\Windows", r"C:\Tools", r"C:\Users\x\bin"];
        analyze(machine, user, &|d| dirs.iter().any(|x| x.eq_ignore_ascii_case(d.trim_end_matches('\\'))), &|f| f.to_lowercase().ends_with(".exe"))
    }

    #[test]
    fn finds_missing_duplicate_empty_and_file_entries() {
        let e = run(r"C:\Windows;C:\Tools", r"C:\Users\x\bin;C:\Gone;C:\tools\;;C:\Tools\app.exe;");
        let by = |raw: &str| e.iter().find(|x| x.raw == raw).unwrap();
        assert_eq!(by(r"C:\Gone").problem, PathProblem::Missing);
        assert_eq!((by(r"C:\tools\").problem, by(r"C:\tools\").duplicate_of.as_deref()), (PathProblem::Duplicate, Some("system PATH entry 2")));
        assert_eq!(e.iter().filter(|x| x.problem == PathProblem::Empty).count(), 1);
        assert_eq!(by(r"C:\Tools\app.exe").problem, PathProblem::NotAFolder);
        assert_eq!(e.iter().filter(|x| x.fixable).count(), 4);
        assert!(e.iter().filter(|x| x.scope == PathScope::Machine).all(|x| !x.fixable));
        assert!(run("", r"C:\Tools;").iter().all(|x| x.problem == PathProblem::None));
    }

    #[test]
    fn cleaning_keeps_good_entries_as_written() {
        let e = |i, raw: &str, p| PathEntry { scope: PathScope::User, index: i, raw: raw.into(), expanded: raw.into(), problem: p, duplicate_of: None, fixable: true };
        let entries = [e(1, r"C:\Gone", PathProblem::Missing), e(2, r"C:\Users\x\bin", PathProblem::Duplicate), e(3, "", PathProblem::Empty)];
        assert_eq!(cleaned(r"%USERPROFILE%\bin;C:\Gone;C:\Users\x\bin;;C:\tools", &entries), r"%USERPROFILE%\bin;C:\tools");
        assert!(is_store_alias(r"C:\Users\x\AppData\Local\Microsoft\WindowsApps\python.exe"));
    }
}
