//! Every delete goes through `Guard::check`. It refuses protected paths, top-level folders, anything outside the
//! allowed roots, anything reached through a junction or symlink, and any folder that holds an installed
//! program, a PATH entry or a WinGet link target. Deleting %LOCALAPPDATA%\Microsoft\WinGet\Packages once broke
//! ffmpeg, yt-dlp, ripgrep, jq, Deno, uv and Poppler; this exists so that class of mistake cannot happen.

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

use super::paths::{self, ATTR_DIRECTORY, ATTR_REPARSE, K, attributes, expand, glob, join, norm, parent, under_normalized, wild_match};
use super::protect;
use super::rules::reset_backup_dirs;
use crate::tools;

pub const GONE_ALREADY: &str = "gone already";
pub const TOP_LEVEL: &str = "top-level folder";
pub const OUTSIDE: &str = "outside the folders this tool may touch";
pub const THROUGH_LINK: &str = "reached through a junction or link";
pub const THROUGH_SPECIAL: &str = "reached through a cloud or special folder";
pub const INVALID_PATH: &str = "not a valid path";

/// The lists a guard works from. Tests build their own from temp folders.
#[derive(Clone, Debug, Default)]
pub struct GuardConfig {
    pub allowed_roots: Vec<String>,
    /// Folders that are never deleted themselves (NEVER_EXACT), though things inside them may be.
    pub never_exact: Vec<String>,
    /// Never deleted, nor anything inside them, nor any folder containing them.
    pub protected: Vec<String>,
    /// Install locations, PATH folders and WinGet link targets: no folder containing one is deleted.
    pub in_use: Vec<String>,
    /// ALLOWED_FILES: loose files allowed directly inside a folder, by file-name pattern.
    pub allowed_files: Vec<(String, Vec<String>)>,
}

impl GuardConfig {
    pub fn from_system() -> GuardConfig {
        let mut protected = protected_paths();
        protected.extend(protect::read(None));
        let mut in_use = install_locations();
        in_use.extend(path_dirs());
        in_use.extend(link_targets(&format!(r"{}\Microsoft\WinGet\Links", K.local)));
        GuardConfig {
            allowed_roots: allowed_roots(),
            allowed_files: vec![(K.windows.clone(), WINDOWS_FILE_PATTERNS.iter().map(|s| s.to_string()).collect())],
            never_exact: never_exact(),
            protected,
            in_use,
        }
    }
}

pub struct Guard {
    allowed: Vec<String>,
    never: Vec<String>,
    protected: Vec<String>,
    in_use: Vec<String>,
    allowed_files: Vec<(String, Vec<String>)>,
}

fn norm_all(v: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in v.iter().filter_map(|p| norm(p)) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&p)) {
            out.push(p);
        }
    }
    out
}

impl Guard {
    pub fn new(c: &GuardConfig) -> Guard {
        let never = norm_all(&c.never_exact);
        let in_use = norm_all(&c.in_use).into_iter().filter(|p| !never.iter().any(|r| r.eq_ignore_ascii_case(p))).collect();
        Guard {
            allowed: norm_all(&c.allowed_roots),
            allowed_files: c.allowed_files.iter().filter_map(|(d, pats)| Some((norm(d)?, pats.clone()))).collect(),
            protected: norm_all(&c.protected),
            never,
            in_use,
        }
    }

    /// The guard for this PC: reference rules plus registry, PATH, WinGet links and protect.txt.
    pub fn create_default() -> Guard {
        Guard::new(&GuardConfig::from_system())
    }

    /// None when `path` may be deleted, else the reason it may not.
    pub fn check(&self, path: &str) -> Option<String> {
        if path.trim().is_empty() {
            return Some(INVALID_PATH.into());
        }
        if !paths::exists(path) {
            return Some(GONE_ALREADY.into());
        }
        let Some(p) = norm(path) else { return Some(INVALID_PATH.into()) };
        if self.never.iter().any(|r| r.eq_ignore_ascii_case(&p)) {
            return Some(TOP_LEVEL.into());
        }
        let mut root = self.allowed.iter().find(|r| under_normalized(&p, r)).cloned();
        if root.is_none() && self.is_allowed_file(&p) && attributes(&p).is_some_and(|a| a & ATTR_DIRECTORY == 0) {
            // a loose file Disk Cleanup also clears, directly inside its folder (C:\Windows\MEMORY.DMP)
            root = parent(&p).map(str::to_string);
        }
        let Some(root) = root else { return Some(OUTSIDE.into()) };

        // no junction or symlink anywhere between the allowed root and the target (target included)
        let mut cur = Some(p.as_str());
        while let Some(c) = cur {
            if c.eq_ignore_ascii_case(&root) {
                break;
            }
            let Some(a) = attributes(c) else { return Some(GONE_ALREADY.into()) };
            if a & ATTR_REPARSE != 0 {
                return Some(if paths::is_link_reparse(c) { THROUGH_LINK } else { THROUGH_SPECIAL }.into());
            }
            cur = parent(c);
        }
        if cur.is_none() {
            return Some(OUTSIDE.into());
        }
        if let Some(prot) = self.protected.iter().find(|prot| under_normalized(&p, prot) || under_normalized(prot, &p)) {
            return Some(format!("protected ({prot})"));
        }
        if let Some(used) = self.in_use.iter().find(|used| under_normalized(used, &p)) {
            return Some(format!("holds an installed program or PATH entry ({used})"));
        }
        None
    }

    /// True for a file name that ALLOWED_FILES lists, sitting directly in its folder. The pattern is matched
    /// against the file name only, never the full path, so C:\Windows\*.dmp does not reach C:\Windows\Minidump\x.dmp.
    pub fn is_allowed_file(&self, path: &str) -> bool {
        let Some(p) = norm(path) else { return false };
        let Some(dir) = parent(&p) else { return false };
        let name = paths::file_name(&p);
        self.allowed_files.iter().any(|(d, pats)| d.eq_ignore_ascii_case(dir) && pats.iter().any(|pat| wild_match(pat, name)))
    }
}

/// PROTECTED_PATTERNS from the reference, plus the C# port's additions.
pub const PROTECTED_PATTERNS: &[&str] = &[
    // local AI models and runtimes
    r"{L}\Claude-Data\*\vm_bundles", r"{H}\.ollama", r"{H}\.cache\huggingface", r"{H}\.lmstudio", r"{L}\ms-playwright", r"{R}\Python",
    // agent tools' own state
    r"{H}\.claude\skills", r"{H}\.claude\hooks", r"{H}\.claude\projects\*\memory", r"{H}\.codex\skills", r"{H}\.codex\plugins", r"{H}\Documents\Codex",
    // synced and personal folders
    r"{H}\CrossDevice", r"{H}\OneDrive", r"{H}\OneDrive *", r"{H}\.ssh", r"{H}\.gnupg",
    // installed tools and installer databases
    r"{L}\Microsoft\WinGet", r"{L}\Programs", r"{L}\Package Cache", r"{P}\Package Cache", r"{W}\Installer",
    // crash dumps, NVIDIA shader caches, Chrome's on-device AI model. The DirectX shader cache is opt-in instead.
    r"{W}\Minidump", r"{W}\LiveKernelReports", r"{L}\NVIDIA", r"{H}\AppData\LocalLow\NVIDIA", r"{L}\Google\Chrome\User Data\OptGuideOnDeviceModel",
    // AMD's shader caches, personal media and game saves, and this app's own settings and logs
    r"{L}\AMD", r"{H}\Pictures", r"{H}\Music", r"{H}\Videos", r"{H}\Saved Games", r"{H}\Documents\My Games", r"{H}\Favorites", r"{H}\Contacts",
    r"{R}\SafeDiskCleaner", r"{L}\SafeDiskCleaner", r"{L}\SafeToolbox", r"{L}\SafeUpdater",
    // the WebView2 data the Rust app's window uses
    r"{L}\io.github.ferventclav.safetoolbox",
];

pub fn protected_paths() -> Vec<String> {
    let mut list = Vec::new();
    for pat in PROTECTED_PATTERNS {
        let e = expand(pat);
        if pat.contains('*') { list.extend(glob(&e)) } else { list.push(e) }
    }
    // the folder this app runs from
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_string_lossy().into_owned())) {
        list.push(dir);
    }
    list
}

/// C:\FOUND.000 to C:\FOUND.009: fragments an old ChkDsk run saved.
pub fn found_dirs() -> Vec<String> {
    (0..10).map(|i| format!("{}FOUND.{i:03}", K.system_drive)).collect()
}

pub fn allowed_roots() -> Vec<String> {
    let (w, pd) = (&K.windows, &K.program_data);
    let mut roots = vec![
        K.home.clone(),
        join(pd, "NVIDIA Corporation"),
        K.drive_temp.clone(),
        join(w, "Temp"),
        join(w, r"SoftwareDistribution\Download"),
        join(pd, r"Microsoft\Windows\WER"),
        format!("{}NVIDIA", K.system_drive),
        join(w, "Logs"),
        join(w, r"System32\LogFiles"),
        join(w, "SystemTemp"),
        join(pd, r"Microsoft\WDF"),
        join(pd, r"Microsoft\Windows\HiveUpload"),
        join(pd, r"Microsoft\Windows Defender\LocalCopy"),
        join(pd, r"Microsoft\Windows Defender\Support"),
        join(pd, r"Microsoft\Diagnosis\FeedbackArchive"),
        join(pd, r"Microsoft\Diagnosis\EventTranscript"),
        join(pd, r"Microsoft\Windows\RetailDemo"),
    ];
    roots.extend(found_dirs());
    // the renamed folders a Windows Update reset leaves; only their contents can go
    roots.extend(reset_backup_dirs());
    roots
}

/// Loose files directly inside the Windows folder that Disk Cleanup also clears, never in its subfolders.
pub const WINDOWS_FILE_PATTERNS: [&str; 5] = ["*.dmp", "setup*.log", "setup*.old", "setuplog.txt", "winnt32.log"];

pub fn never_exact() -> Vec<String> {
    let h = &K.home;
    let mut list = vec![
        h.clone(),
        K.local.clone(),
        K.roaming.clone(),
        K.temp.clone(),
        join(h, "AppData"),
        K.downloads(),
        K.documents(),
        K.desktop(),
        join(h, "source"),
        K.program_data.clone(),
        K.windows.clone(),
        K.drive_temp.clone(),
        K.system_drive.clone(),
        K.local_low.clone(),
        join(h, ".cache"),
        join(h, ".local"),
        join(h, ".config"),
        join(&K.local, "Packages"),
        join(&K.local, "Microsoft"),
        join(&K.roaming, "Microsoft"),
        format!(r"{}NVIDIA\DisplayDriver", K.system_drive),
        K.program_files.clone(),
    ];
    list.extend(allowed_roots());
    list
}

/// InstallLocation of every uninstall entry in HKCU, HKLM and HKLM\WOW6432Node.
pub fn install_locations() -> Vec<String> {
    let mut locs = Vec::new();
    for (hive, key) in [
        (HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
    ] {
        let Ok(k) = RegKey::predef(hive).open_subkey_with_flags(key, KEY_READ | KEY_WOW64_64KEY) else { continue };
        for sub in k.enum_keys().flatten() {
            if let Ok(loc) = k.open_subkey(&sub).and_then(|s| s.get_value::<String, _>("InstallLocation")) {
                if loc.trim_matches(['"', '\\', ' ']).len() > 3 {
                    locs.push(tools::expand(loc.trim_matches(['"', ' '])));
                }
            }
        }
    }
    locs
}

/// Every folder on the process, user and machine PATH.
pub fn path_dirs() -> Vec<String> {
    [tools::env("PATH"), tools::registry_path(false), tools::registry_path(true)]
        .iter()
        .flat_map(|r| r.split(';').map(str::to_string).collect::<Vec<_>>())
        .map(|p| p.trim_matches(['"', ' ']).to_string())
        .filter(|p| p.len() > 3)
        .map(|p| tools::expand(&p))
        .collect()
}

/// For each symbolic link directly inside `folder`, the folder holding its final target.
pub fn link_targets(folder: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (p, _) in paths::children(folder, false).unwrap_or_default() {
        let Ok(raw) = std::fs::read_link(&p) else { continue };
        let target = match std::fs::canonicalize(&p) {
            Ok(t) => t.to_string_lossy().trim_start_matches(r"\\?\").to_string(),
            Err(_) if raw.is_absolute() => raw.to_string_lossy().into_owned(),
            Err(_) => join(folder, &raw.to_string_lossy()),
        };
        if let Some(d) = parent(&target) {
            out.push(d.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> (String, Guard) {
        let t = std::env::temp_dir().join(format!("st-guard-{}", std::process::id())).to_string_lossy().into_owned();
        for d in [r"root\cache\a", r"root\keep", r"root\tool\bin", r"outside"] {
            fs::create_dir_all(join(&t, d)).unwrap();
        }
        fs::write(join(&t, r"root\cache\a\f.txt"), b"x").unwrap();
        fs::write(join(&t, "crash.dmp"), b"x").unwrap();
        let cfg = GuardConfig {
            allowed_roots: vec![join(&t, "root")],
            never_exact: vec![join(&t, "root"), join(&t, r"root\cache")],
            protected: vec![join(&t, r"root\keep")],
            in_use: vec![join(&t, r"root\tool\bin")],
            allowed_files: vec![(t.clone(), vec!["*.dmp".into()])],
        };
        let g = Guard::new(&cfg);
        (t, g)
    }

    #[test]
    fn refuses_what_it_must() {
        let (t, g) = setup();
        assert_eq!(g.check(&join(&t, r"root\cache\a")), None);
        assert_eq!(g.check(&join(&t, r"root\cache")).as_deref(), Some(TOP_LEVEL));
        assert!(g.check(&join(&t, r"root\keep")).unwrap().starts_with("protected"));
        assert!(g.check(&join(&t, r"root\tool")).unwrap().starts_with("holds an installed program"));
        assert_eq!(g.check(&join(&t, "outside")).as_deref(), Some(OUTSIDE));
        assert_eq!(g.check(&join(&t, r"root\nope")).as_deref(), Some(GONE_ALREADY));
        assert_eq!(g.check(&join(&t, "crash.dmp")), None);
        assert_eq!(g.check("  ").as_deref(), Some(INVALID_PATH));
        let _ = fs::remove_dir_all(&t);
    }

    #[test]
    fn refuses_a_path_through_a_junction() {
        let (t, g) = setup();
        let link = join(&t, r"root\cache\link");
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J", &link, &join(&t, "outside")]).output().map(|o| o.status.success()).unwrap_or(false);
        if ok {
            fs::write(join(&t, r"outside\x.txt"), b"x").unwrap();
            assert_eq!(g.check(&join(&link, "x.txt")).as_deref(), Some(THROUGH_LINK));
            assert_eq!(g.check(&link).as_deref(), Some(THROUGH_LINK));
        }
        let _ = fs::remove_dir(&link);
        let _ = fs::remove_dir_all(&t);
    }
}
