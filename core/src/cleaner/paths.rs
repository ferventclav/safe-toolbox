//! Path helpers (nc/under/is_link/tree_size/mtime/idle_days/glob from the cleaner's reference), the base folders,
//! and version keys for folder names.

use std::cmp::Ordering;
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use regex::Regex;

use crate::tools;

pub const ATTR_READONLY: u32 = 0x1;
pub const ATTR_DIRECTORY: u32 = 0x10;
pub const ATTR_REPARSE: u32 = 0x400;
pub const ATTR_OFFLINE: u32 = 0x1000;
pub const ATTR_RECALL_ON_OPEN: u32 = 0x0004_0000;
pub const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

/// The base folders, read from the environment of whoever runs the app.
pub struct Known {
    pub home: String,
    pub local: String,
    pub roaming: String,
    pub local_low: String,
    pub program_data: String,
    pub windows: String,
    /// LOCALAPPDATA\Temp, not %TEMP%, like the reference.
    pub temp: String,
    /// "C:\"
    pub system_drive: String,
    pub drive_temp: String,
    pub program_files: String,
    pub settings_dir: String,
    pub log_dir: String,
}

pub static K: LazyLock<Known> = LazyLock::new(|| {
    let s = |p: PathBuf| p.to_string_lossy().into_owned();
    let home = s(tools::home());
    let local = s(tools::local_app_data());
    let roaming = s(tools::app_data());
    let windows = s(tools::windows_dir());
    let system_drive = format!("{}\\", tools::system_drive());
    let pf = { let w = tools::env("ProgramW6432"); if w.is_empty() { s(tools::program_files()) } else { w } };
    Known {
        local_low: format!(r"{home}\AppData\LocalLow"),
        temp: format!(r"{local}\Temp"),
        drive_temp: format!("{system_drive}Temp"),
        settings_dir: format!(r"{roaming}\SafeDiskCleaner"),
        log_dir: format!(r"{local}\SafeDiskCleaner"),
        program_data: s(tools::program_data()),
        program_files: pf,
        home,
        local,
        roaming,
        windows,
        system_drive,
    }
});

impl Known {
    pub fn downloads(&self) -> String {
        format!(r"{}\Downloads", self.home)
    }
    pub fn documents(&self) -> String {
        format!(r"{}\Documents", self.home)
    }
    pub fn desktop(&self) -> String {
        format!(r"{}\Desktop", self.home)
    }
}

/// {H} home, {L} local app data, {R} roaming, {P} ProgramData, {W} Windows, {D} system drive (no slash).
pub fn expand(pattern: &str) -> String {
    pattern
        .replace("{H}", &K.home)
        .replace("{L}", &K.local)
        .replace("{R}", &K.roaming)
        .replace("{P}", &K.program_data)
        .replace("{W}", &K.windows)
        .replace("{D}", K.system_drive.trim_end_matches('\\'))
}

pub fn join(a: &str, b: &str) -> String {
    if a.ends_with('\\') { format!("{a}{b}") } else { format!("{a}\\{b}") }
}

pub fn file_name(p: &str) -> &str {
    p.trim_end_matches('\\').rsplit('\\').next().unwrap_or(p)
}

pub fn parent(p: &str) -> Option<&str> {
    let t = p.trim_end_matches('\\');
    let i = t.rfind('\\')?;
    let up = &t[..i];
    if up.len() == 2 && up.ends_with(':') { Some(&t[..=i]) } else if up.is_empty() { None } else { Some(up) }
}

/// Normalises a path for comparison: absolute, no \\?\ prefix, 8.3 names expanded, no trailing separator
/// except on a drive root. None for a malformed path.
pub fn norm(path: &str) -> Option<String> {
    let mut p = path.trim().trim_matches('"').trim().to_string();
    if p.is_empty() {
        return None;
    }
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        p = format!(r"\\{rest}");
    } else if p.starts_with(r"\\?\") || p.starts_with(r"\??\") {
        p = p[4..].to_string();
    }
    let mut full = std::path::absolute(&p).ok()?.to_string_lossy().replace('/', "\\");
    if full.contains('~') {
        if let Some(long) = long_path(&full) {
            full = long;
        }
    }
    if full.len() > 3 {
        full = full.trim_end_matches(['\\', '/']).to_string();
    }
    Some(full)
}

fn long_path(p: &str) -> Option<String> {
    use windows::Win32::Storage::FileSystem::GetLongPathNameW;
    use windows::core::HSTRING;
    let src = HSTRING::from(p);
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetLongPathNameW(&src, Some(&mut buf)) } as usize;
    if n == 0 || n > buf.len() {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..n]))
}

/// `path` equals `root` or lies inside it. Both normalised.
pub fn under_normalized(path: &str, root: &str) -> bool {
    if path.eq_ignore_ascii_case(root) {
        return true;
    }
    let prefix = if root.ends_with('\\') { root.to_lowercase() } else { root.to_lowercase() + "\\" };
    path.to_lowercase().starts_with(&prefix)
}

pub fn under(path: &str, root: &str) -> bool {
    matches!((norm(path), norm(root)), (Some(p), Some(r)) if under_normalized(&p, &r))
}

pub fn same_path(a: &str, b: &str) -> bool {
    matches!((norm(a), norm(b)), (Some(x), Some(y)) if x.eq_ignore_ascii_case(&y))
}

/// Attributes of the entry itself (links not followed); None when it can't be read.
pub fn attributes(path: &str) -> Option<u32> {
    fs::symlink_metadata(path).ok().map(|m| m.file_attributes())
}

/// lexists: true for files, folders and links, even broken ones.
pub fn exists(path: &str) -> bool {
    attributes(path).is_some()
}

pub fn is_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

pub fn is_file(path: &str) -> bool {
    Path::new(path).is_file()
}

/// The reparse tag of an entry, from its directory listing.
fn reparse_tag(path: &str) -> Option<u32> {
    use windows::Win32::Storage::FileSystem::{FindClose, FindFirstFileW, WIN32_FIND_DATAW};
    use windows::core::HSTRING;
    let mut data = WIN32_FIND_DATAW::default();
    let h = unsafe { FindFirstFileW(&HSTRING::from(path.trim_end_matches('\\')), &mut data) }.ok()?;
    unsafe {
        let _ = FindClose(h);
    }
    Some(data.dwReserved0)
}

/// For an entry known to be a reparse point: a junction or symlink (not a cloud file, dedup file and so on).
/// Unreadable reparse data counts as a link, which is the cautious answer.
pub fn is_link_reparse(path: &str) -> bool {
    const SYMLINK: u32 = 0xA000_000C;
    const MOUNT_POINT: u32 = 0xA000_0003;
    match reparse_tag(path) {
        Some(tag) => tag == SYMLINK || tag == MOUNT_POINT,
        None => true,
    }
}

pub fn is_link(path: &str) -> bool {
    attributes(path).is_some_and(|a| a & ATTR_REPARSE != 0) && is_link_reparse(path)
}

pub fn is_cloud_placeholder(a: u32) -> bool {
    a & (ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS | ATTR_OFFLINE) != 0
}

/// Children of a folder, one level, with their metadata. `skip_reparse` leaves out links and placeholders.
pub fn children(dir: &str, skip_reparse: bool) -> Option<Vec<(String, fs::Metadata)>> {
    let rd = fs::read_dir(dir).ok()?;
    Some(
        rd.flatten()
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                if skip_reparse && m.file_attributes() & ATTR_REPARSE != 0 {
                    return None;
                }
                Some((e.path().to_string_lossy().into_owned(), m))
            })
            .collect(),
    )
}

fn mtime(m: &fs::Metadata) -> SystemTime {
    m.modified().unwrap_or_else(|_| SystemTime::now())
}

/// Total bytes of a file or folder tree. Links and placeholders are not followed or counted.
pub fn tree_size(path: &str) -> u64 {
    let Ok(m) = fs::symlink_metadata(path) else { return 0 };
    let a = m.file_attributes();
    if a & ATTR_DIRECTORY == 0 {
        return m.len();
    }
    if a & ATTR_REPARSE != 0 {
        return 0;
    }
    let mut total = 0;
    let mut stack = vec![path.to_string()];
    while let Some(d) = stack.pop() {
        for (p, m) in children(&d, true).unwrap_or_default() {
            if m.is_dir() { stack.push(p) } else { total += m.len() }
        }
    }
    total
}

/// Size of a tree plus the newest write time of anything in it (the entry itself included).
pub fn tree_stats(path: &str) -> (u64, SystemTime) {
    let mut newest = mtime_of(path);
    let Some(a) = attributes(path) else { return (0, SystemTime::now()) };
    if a & ATTR_DIRECTORY == 0 || a & ATTR_REPARSE != 0 {
        return (tree_size(path), newest);
    }
    let mut total = 0;
    let mut stack = vec![path.to_string()];
    while let Some(d) = stack.pop() {
        let Some(kids) = children(&d, true) else {
            // Treat an unreadable tree as recently used so it is never picked by an age rule.
            return (total, SystemTime::now());
        };
        for (p, m) in kids {
            newest = newest.max(mtime(&m));
            if m.is_dir() { stack.push(p) } else { total += m.len() }
        }
    }
    (total, newest)
}

/// Last write time of the entry itself; "now" when it cannot be read, so age rules leave it alone.
pub fn mtime_of(path: &str) -> SystemTime {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            let t = mtime(&m);
            if t <= SystemTime::UNIX_EPOCH - Duration::from_secs(369 * 365 * 86400) { SystemTime::now() } else { t }
        }
        Err(_) => SystemTime::now(),
    }
}

pub fn days_since(t: SystemTime) -> f64 {
    SystemTime::now().duration_since(t).map(|d| d.as_secs_f64() / 86400.0).unwrap_or(0.0)
}

/// Days since the path (or, with deep, anything inside it) last changed.
pub fn idle_days(path: &str, deep: bool) -> f64 {
    days_since(if deep { tree_stats(path).1 } else { mtime_of(path) })
}

/// Case-insensitive match of a name against '*' and '?'.
pub fn wild_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            pi += 1;
            mark = ni;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// A small glob for rule tables: '*' and '?' inside path segments only. Like Python's glob, a wildcard does
/// not match names starting with '.' unless the pattern segment does. Returns full paths, sorted.
pub fn glob(pattern: &str) -> Vec<String> {
    let mut results = Vec::new();
    if pattern.trim().is_empty() {
        return results;
    }
    let parts: Vec<&str> = pattern.split('\\').collect();
    let Some(first) = parts.iter().position(|s| s.contains(['*', '?'])) else {
        if exists(pattern) {
            results.push(pattern.to_string());
        }
        return results;
    };
    if first == 0 {
        return results;
    }
    let mut base = parts[..first].join("\\");
    if base.ends_with(':') {
        base.push('\\');
    }
    if is_dir(&base) {
        expand_glob(&base, &parts, first, &mut results);
    }
    results.sort_by_key(|s| s.to_lowercase());
    results
}

/// glob of path parts joined with '\'.
pub fn g(parts: &[&str]) -> Vec<String> {
    glob(&parts.join("\\"))
}

fn expand_glob(dir: &str, parts: &[&str], i: usize, results: &mut Vec<String>) {
    if i == parts.len() {
        results.push(dir.to_string());
        return;
    }
    let seg = parts[i];
    let last = i == parts.len() - 1;
    if seg.is_empty() {
        expand_glob(dir, parts, i + 1, results);
        return;
    }
    if !seg.contains(['*', '?']) {
        let next = join(dir, seg);
        if if last { exists(&next) } else { is_dir(&next) } {
            expand_glob(&next, parts, i + 1, results);
        }
        return;
    }
    let Some(kids) = children(dir, false) else { return };
    for (p, m) in kids {
        if !last && !m.is_dir() {
            continue;
        }
        let name = file_name(&p);
        if !wild_match(seg, name) || (name.starts_with('.') && !seg.starts_with('.')) {
            continue;
        }
        expand_glob(&p, parts, i + 1, results);
    }
}

/// Every run of digits in a name, compared like a Python tuple: "app-1.0.9001" < "app-1.0.10000".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionKey(pub Vec<u64>);

static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").unwrap());

impl VersionKey {
    pub fn parse(name: &str) -> VersionKey {
        let mut parts: Vec<u64> = DIGITS.find_iter(name).map(|m| m.as_str().parse().unwrap_or(u64::MAX)).collect();
        if parts.is_empty() {
            parts.push(0);
        }
        VersionKey(parts)
    }
}

impl PartialOrd for VersionKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VersionKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl std::fmt::Display for VersionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.iter().map(u64::to_string).collect::<Vec<_>>().join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_and_compares() {
        assert_eq!(norm(r"C:\Windows\").as_deref(), Some(r"C:\Windows"));
        assert_eq!(norm(r"\\?\C:\Windows").as_deref(), Some(r"C:\Windows"));
        assert_eq!(norm(r"C:\").as_deref(), Some(r"C:\"));
        assert!(under_normalized(r"C:\a\b", r"C:\A"));
        assert!(!under_normalized(r"C:\ab", r"C:\a"));
        assert!(under_normalized(r"C:\x", r"C:\"));
        assert_eq!(parent(r"C:\a\b"), Some(r"C:\a"));
        assert_eq!(parent(r"C:\a"), Some(r"C:\"));
    }

    #[test]
    fn wildcards() {
        assert!(wild_match("thumbcache_*.db", "THUMBCACHE_256.db"));
        assert!(wild_match("app-*", "app-1.0.9"));
        assert!(!wild_match("*.dmp", "x.dmp.old"));
        assert!(wild_match("setup?.log", "setup1.log"));
    }

    #[test]
    fn version_keys_compare_like_tuples() {
        assert!(VersionKey::parse("app-1.0.9001") < VersionKey::parse("app-1.0.10000"));
        assert!(VersionKey::parse("566.36") <= VersionKey::parse("566.36"));
        assert!(VersionKey::parse("1.2") < VersionKey::parse("1.2.0"));
        assert_eq!(VersionKey::parse("none").to_string(), "0");
    }

    #[test]
    fn glob_skips_dot_names_and_sizes_trees() {
        let t = std::env::temp_dir().join(format!("st-glob-{}", std::process::id()));
        let t = t.to_string_lossy().into_owned();
        for d in ["app-1.0", "app-2.0", ".app-hidden"] {
            fs::create_dir_all(join(&t, d)).unwrap();
        }
        fs::write(join(&t, r"app-1.0\f.bin"), vec![0u8; 1000]).unwrap();
        let found: Vec<String> = g(&[&t, "app-*"]).iter().map(|p| file_name(p).to_string()).collect();
        assert_eq!(found, ["app-1.0", "app-2.0"]);
        assert_eq!(tree_size(&t), 1000);
        assert!(idle_days(&t, true) < 1.0);
        let _ = fs::remove_dir_all(&t);
    }
}
