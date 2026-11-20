//! Keeps command-line tools reachable across updates. On 3 Nov 2026 force-replacing yt-dlp's FFmpeg deleted the
//! shared ffmpeg.exe link in WinGet\Links and left a user PATH entry pointing at a deleted version folder.
//! A snapshot is taken before updating; afterwards missing links are recreated and dead PATH entries are moved
//! to the package's new version folder.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, RegType};

use crate::tools::{expand, local_app_data};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Snapshot {
    /// Link file name in WinGet\Links to the file it pointed at.
    pub links: BTreeMap<String, String>,
    /// User PATH entries inside WinGet\Packages, with the .exe names each held.
    pub package_dirs: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Repair {
    pub kind: &'static str,
    pub path: String,
    pub target: String,
    pub description: String,
}

fn winget_root() -> PathBuf {
    local_app_data().join(r"Microsoft\WinGet")
}
pub fn links_dir() -> PathBuf {
    winget_root().join("Links")
}
pub fn packages_dir() -> PathBuf {
    winget_root().join("Packages")
}

pub fn take() -> Snapshot {
    take_from(&user_path_entries(), &links_dir(), &packages_dir())
}

fn link_target(link: &Path, dir: &Path) -> Option<String> {
    let t = fs::read_link(link).ok()?;
    let full = if t.is_absolute() { t } else { dir.join(t) };
    Some(full.to_string_lossy().into_owned())
}

fn exe_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

pub fn take_from(path_entries: &[String], links: &Path, packages: &Path) -> Snapshot {
    let mut snap = Snapshot::default();
    if let Ok(rd) = fs::read_dir(links) {
        for e in rd.flatten() {
            if let Some(t) = link_target(&e.path(), links) {
                snap.links.insert(e.file_name().to_string_lossy().into_owned(), t);
            }
        }
    }
    for entry in path_entries {
        if is_under(entry, packages) && Path::new(entry).is_dir() {
            snap.package_dirs.insert(entry.clone(), exe_names(Path::new(entry)));
        }
    }
    snap
}

pub fn plan(before: &Snapshot, path_entries_now: &[String], links: &Path, packages: &Path) -> Vec<Repair> {
    let mut plan = Vec::new();
    for (name, target) in &before.links {
        let link = links.join(name);
        if link.exists() || fs::symlink_metadata(&link).is_ok() {
            continue;
        }
        let to = if Path::new(target).is_file() { Some(target.clone()) } else { newest_in_package(target, packages, std::slice::from_ref(name)) };
        if let Some(to) = to {
            let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            plan.push(Repair { kind: "link", path: link.to_string_lossy().into_owned(), target: to, description: format!("Recreated the {stem} command") });
        }
    }
    for entry in path_entries_now {
        if Path::new(entry).is_dir() {
            continue;
        }
        let Some(exes) = before.package_dirs.iter().find(|(k, _)| k.eq_ignore_ascii_case(entry)).map(|(_, v)| v) else { continue };
        if exes.is_empty() {
            continue;
        }
        let old = Path::new(entry).join(&exes[0]).to_string_lossy().into_owned();
        if let Some(file) = newest_in_package(&old, packages, exes) {
            let dir = Path::new(&file).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            plan.push(Repair {
                kind: "path",
                path: entry.clone(),
                target: dir,
                description: format!("Pointed PATH at the new {} folder", package_name(entry, packages)),
            });
        }
    }
    plan
}

/// Applies a plan. Needs admin for links unless Developer Mode is on.
pub fn apply(plan: &[Repair]) -> Vec<String> {
    let mut done = Vec::new();
    for r in plan.iter().filter(|r| r.kind == "link") {
        match std::os::windows::fs::symlink_file(&r.target, &r.path) {
            Ok(()) => done.push(r.description.clone()),
            Err(e) => done.push(format!("Could not recreate {}: {e}", Path::new(&r.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
        }
    }
    let paths: Vec<&Repair> = plan.iter().filter(|r| r.kind == "path").collect();
    if !paths.is_empty() {
        if let Ok(env) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE) {
            let raw_value = env.get_raw_value("Path").ok();
            let kind = raw_value.as_ref().map(|v| v.vtype.clone()).unwrap_or(RegType::REG_EXPAND_SZ);
            let raw: String = env.get_value("Path").unwrap_or_default();
            let mut parts: Vec<String> = raw.split(';').map(str::to_string).collect();
            for r in &paths {
                if let Some(i) = parts.iter().position(|p| expand(p).trim_end_matches('\\').eq_ignore_ascii_case(r.path.trim_end_matches('\\'))) {
                    parts[i] = r.target.clone();
                    done.push(r.description.clone());
                }
            }
            let joined = parts.join(";");
            let _ = crate::path_env::write_user_path(&env, &joined, kind == RegType::REG_EXPAND_SZ);
            crate::path_env::broadcast_environment_change();
        }
    }
    done
}

pub fn user_path_entries() -> Vec<String> {
    crate::tools::registry_path(false).split(';').map(str::trim).filter(|p| !p.is_empty()).map(expand).collect()
}

/// Inside the same WinGet package folder, the newest directory that holds every one of `exes`.
fn newest_in_package(old_file: &str, packages: &Path, exes: &[String]) -> Option<String> {
    let root = package_root(old_file, packages)?;
    if !root.is_dir() {
        return None;
    }
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    walk(&root, &mut |p| {
        if p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&exes[0])) {
            if let Some(d) = p.parent() {
                if exes.iter().all(|e| d.join(e).is_file()) {
                    let t = fs::metadata(d).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                    found.push((t, d.join(&exes[0])));
                }
            }
        }
    });
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().next().map(|(_, p)| p.to_string_lossy().into_owned())
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() && !ft.is_symlink() {
                walk(&p, f);
            } else if ft.is_file() {
                f(&p);
            }
        }
    }
}

fn package_root(path: &str, packages: &Path) -> Option<PathBuf> {
    if !is_under(path, packages) {
        return None;
    }
    let base = packages.to_string_lossy();
    let rest = &path[base.trim_end_matches('\\').len() + 1..];
    let first = rest.split(['\\', '/']).next()?;
    Some(packages.join(first))
}

fn package_name(path: &str, packages: &Path) -> String {
    let name = package_root(path, packages).and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "package".into());
    match name.to_lowercase().find("_microsoft.winget") {
        Some(cut) if cut > 0 => name[..cut].to_string(),
        _ => name,
    }
}

fn is_under(path: &str, dir: &Path) -> bool {
    let d = dir.to_string_lossy().trim_end_matches('\\').to_lowercase() + "\\";
    path.to_lowercase().starts_with(&d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_a_moved_package_folder() {
        let tmp = std::env::temp_dir().join(format!("st-pathhealth-{}", std::process::id()));
        let packages = tmp.join("Packages");
        let links = tmp.join("Links");
        let old = packages.join(r"yt-dlp.FFmpeg_Microsoft.Winget.Source_8wekyb\ffmpeg-7.0\bin");
        let new = packages.join(r"yt-dlp.FFmpeg_Microsoft.Winget.Source_8wekyb\ffmpeg-8.0\bin");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&links).unwrap();
        fs::write(old.join("ffmpeg.exe"), b"x").unwrap();
        fs::write(old.join("ffprobe.exe"), b"x").unwrap();
        let entry = old.to_string_lossy().into_owned();
        let snap = take_from(std::slice::from_ref(&entry), &links, &packages);
        assert_eq!(snap.package_dirs[&entry], ["ffmpeg.exe", "ffprobe.exe"]);

        fs::remove_dir_all(old.parent().unwrap()).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("ffmpeg.exe"), b"x").unwrap();
        fs::write(new.join("ffprobe.exe"), b"x").unwrap();
        let p = plan(&snap, &[entry.clone()], &links, &packages);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].kind, "path");
        assert_eq!(p[0].target, new.to_string_lossy());
        assert_eq!(p[0].description, "Pointed PATH at the new yt-dlp.FFmpeg folder");
        let _ = fs::remove_dir_all(&tmp);
    }
}
