//! Finds the command-line tools the checks need, using the PATH as it is in the registry right now,
//! and the folders Windows keeps for this user.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use regex::Regex;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

pub fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

pub fn home() -> PathBuf {
    PathBuf::from(env("USERPROFILE"))
}

pub fn local_app_data() -> PathBuf {
    PathBuf::from(env("LOCALAPPDATA"))
}

pub fn app_data() -> PathBuf {
    PathBuf::from(env("APPDATA"))
}

pub fn program_data() -> PathBuf {
    let p = env("ProgramData");
    PathBuf::from(if p.is_empty() { r"C:\ProgramData".to_string() } else { p })
}

pub fn program_files() -> PathBuf {
    let p = env("ProgramFiles");
    PathBuf::from(if p.is_empty() { r"C:\Program Files".to_string() } else { p })
}

pub fn windows_dir() -> PathBuf {
    let p = env("SystemRoot");
    PathBuf::from(if p.is_empty() { r"C:\Windows".to_string() } else { p })
}

pub fn system_dir() -> PathBuf {
    windows_dir().join("System32")
}

/// "C:" for the drive Windows runs from.
pub fn system_drive() -> String {
    let d = env("SystemDrive");
    if d.is_empty() { "C:".into() } else { d }
}

pub fn powershell() -> String {
    system_dir().join("WindowsPowerShell").join("v1.0").join("powershell.exe").to_string_lossy().into_owned()
}

/// %VAR% references expanded the way Windows does.
pub fn expand(s: &str) -> String {
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows::core::HSTRING;
    if !s.contains('%') {
        return s.to_string();
    }
    let src = HSTRING::from(s);
    unsafe {
        let n = ExpandEnvironmentStringsW(&src, None);
        if n == 0 {
            return s.to_string();
        }
        let mut buf = vec![0u16; n as usize];
        let n = ExpandEnvironmentStringsW(&src, Some(&mut buf));
        if n == 0 {
            return s.to_string();
        }
        String::from_utf16_lossy(&buf[..(n as usize).saturating_sub(1)])
    }
}

/// The raw (unexpanded) PATH value of HKCU\Environment or the machine environment key.
pub fn registry_path(machine: bool) -> String {
    let (root, key) = if machine {
        (RegKey::predef(HKEY_LOCAL_MACHINE), r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment")
    } else {
        (RegKey::predef(HKEY_CURRENT_USER), "Environment")
    };
    root.open_subkey(key).and_then(|k| k.get_value::<String, _>("Path")).unwrap_or_default()
}

/// Machine PATH, then user PATH, then anything only this process had (WindowsApps aliases).
pub fn fresh_path() -> String {
    let mut seen = HashSet::new();
    let mut parts = Vec::new();
    for block in [registry_path(true), registry_path(false), env("Path")] {
        for raw in block.split(';').filter(|s| !s.trim().is_empty()) {
            let dir = expand(raw.trim());
            if !dir.is_empty() && seen.insert(dir.trim_end_matches('\\').to_lowercase()) {
                parts.push(dir);
            }
        }
    }
    parts.join(";")
}

pub fn find(exe: &str, skip_windows_apps: bool) -> Option<String> {
    for dir in fresh_path().split(';') {
        if skip_windows_apps && dir.to_lowercase().contains("windowsapps") {
            continue;
        }
        let candidate = Path::new(dir).join(exe);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

fn existing(p: PathBuf) -> Option<String> {
    p.is_file().then(|| p.to_string_lossy().into_owned())
}

pub fn winget() -> String {
    find("winget.exe", false)
        .unwrap_or_else(|| local_app_data().join(r"Microsoft\WindowsApps\winget.exe").to_string_lossy().into_owned())
}

/// winget ships with App Installer; some PCs (LTSC, Server, stripped images) don't have it.
pub fn has_winget() -> bool {
    Path::new(&winget()).exists()
}

pub fn npm() -> Option<String> {
    find("npm.cmd", false)
}

pub fn rustup() -> Option<String> {
    find("rustup.exe", false).or_else(|| existing(home().join(r".cargo\bin\rustup.exe")))
}

pub fn claude() -> Option<String> {
    find("claude.exe", false).or_else(|| find("claude.cmd", false)).or_else(|| existing(home().join(r".local\bin\claude.exe")))
}

pub fn chocolatey_root() -> PathBuf {
    let v = env("ChocolateyInstall");
    if v.is_empty() { program_data().join("chocolatey") } else { PathBuf::from(v) }
}

pub fn choco() -> Option<String> {
    find("choco.exe", false).or_else(|| existing(chocolatey_root().join(r"bin\choco.exe")))
}

pub fn scoop() -> Option<String> {
    let root = { let s = env("SCOOP"); if s.is_empty() { home().join("scoop") } else { PathBuf::from(s) } };
    find("scoop.cmd", false).or_else(|| existing(root.join(r"shims\scoop.cmd")))
}

/// True when a file can be created in the folder as the current user (ACLs, not just the read-only flag).
pub fn can_write(dir: &Path) -> bool {
    let probe = dir.join(format!(".safeupdater-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(f) => {
            drop(f);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// The real python.exe (never the Microsoft Store stub), newest install first.
pub fn python() -> Option<String> {
    if let Some(p) = find("python.exe", true) {
        return Some(p);
    }
    let re = Regex::new(r"(?i)Python3(\d+)").unwrap();
    let mut roots: Vec<(PathBuf, u32)> = Vec::new();
    for parent in [PathBuf::from(r"C:\"), local_app_data().join(r"Programs\Python")] {
        if let Ok(rd) = std::fs::read_dir(&parent) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if e.path().is_dir() && name.to_lowercase().starts_with("python3") {
                    let minor = re.captures(&name).and_then(|c| c[1].parse().ok()).unwrap_or(0);
                    roots.push((e.path(), minor));
                }
            }
        }
    }
    roots.sort_by(|a, b| b.1.cmp(&a.1));
    roots.into_iter().map(|(d, _)| d.join("python.exe")).find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())
}

/// Lower-case, for comparing paths and ids the way Windows does.
pub fn fold(s: &str) -> String {
    s.to_lowercase()
}

pub fn eq_ic(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

pub fn contains_ic(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

pub fn starts_ic(hay: &str, prefix: &str) -> bool {
    hay.to_lowercase().starts_with(&prefix.to_lowercase())
}

pub fn ends_ic(hay: &str, suffix: &str) -> bool {
    hay.to_lowercase().ends_with(&suffix.to_lowercase())
}

/// True when running as an administrator (elevated token).
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// The SID of the account this process runs as, as "S-1-5-21-...".
pub fn current_user_sid() -> Option<String> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::core::PWSTR;
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as *mut _), len, &mut len).is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s).ok()?;
        let out = s.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut _)));
        out
    }
}
