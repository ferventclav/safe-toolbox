//! scan() and clean() from the cleaner's reference, the delete that never follows links, and runs.csv.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::os::windows::fs::MetadataExt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDateTime, TimeZone};

use super::guard::Guard;
use super::paths::{self, ATTR_DIRECTORY, ATTR_READONLY, ATTR_REPARSE, K, children, is_cloud_placeholder, join, norm};
use super::{Action, Rule, ScanContext, ProcessSnapshot, protect, rules, skip, system};
use crate::disk;
use crate::proc::Cancel;
use crate::time::Stamp;

pub const NEEDS_ADMIN: &str = "needs admin";

pub struct ScanResult {
    pub rules: Vec<Rule>,
    pub admin: bool,
    pub elapsed: Duration,
}

impl ScanResult {
    pub fn total(&self) -> u64 {
        self.rules.iter().map(Rule::total).sum()
    }
}

/// Read-only: finds, checks and sizes, never deletes. `progress(done, total, current)` may come from any thread.
pub fn scan(admin: bool, progress: &(dyn Fn(usize, usize, &str) + Sync), cancel: Option<&Cancel>) -> ScanResult {
    let watch = Instant::now();
    protect::ensure_file(None);
    let ctx = ScanContext::new(ProcessSnapshot::take());
    let guard = Guard::create_default();
    let rules: Vec<Mutex<Rule>> = rules::build().into_iter().map(Mutex::new).collect();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= rules.len() || cancel.is_some_and(Cancel::is_cancelled) {
                        break;
                    }
                    let mut r = rules[i].lock().unwrap();
                    scan_rule(&mut r, &ctx, &guard, admin);
                    progress(done.fetch_add(1, Ordering::SeqCst) + 1, rules.len(), &r.title);
                }
            });
        }
    });
    let mut rules: Vec<Rule> = rules.into_iter().map(|m| m.into_inner().unwrap()).collect();
    remove_duplicates(&mut rules);
    ScanResult { rules, admin, elapsed: watch.elapsed() }
}

/// Finds a rule's items, runs each path through the guard, and sizes what is left.
pub fn scan_rule(r: &mut Rule, ctx: &ScanContext, guard: &Guard, admin: bool) {
    if r.needs_admin && !admin {
        r.blocked = Some(NEEDS_ADMIN.into());
        r.skipped.push(skip(&r.title, NEEDS_ADMIN));
        return;
    }
    r.blocked = ctx.processes.busy(&r.busy);
    let find = r.find.clone();
    let mut skipped = Vec::new();
    let found = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| find(ctx, &mut skipped))) {
        Ok(f) => f,
        Err(_) => {
            r.blocked = Some("scan error".into());
            r.skipped.push(skip(&r.title, "scan error: the rule stopped unexpectedly"));
            return;
        }
    };
    r.skipped.extend(skipped);
    for mut it in found {
        if let Some(p) = &it.path {
            if let Some(why) = guard.check(p) {
                r.skipped.push(skip(p, why));
                continue;
            }
            if it.size.is_none() {
                it.size = Some(paths::tree_size(p));
            }
        }
        r.items.push(it);
    }
    // path-less items (actions, Windows' handlers) stay even when their size is unknown
    r.items.retain(|it| it.path.is_none() || it.size.unwrap_or(0) > 0);
}

/// A folder found by two rules is kept by the first one only, so totals are not counted twice.
fn remove_duplicates(rules: &mut [Rule]) {
    let mut seen = HashSet::new();
    for r in rules.iter_mut() {
        r.items.retain(|it| match it.path.as_deref().and_then(norm) {
            Some(p) => seen.insert(p.to_lowercase()),
            None => true,
        });
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    /// Change in free space on the drives involved, measured, never summed from estimates.
    pub freed: i64,
    pub items: usize,
    /// Items refused by the guard, skipped (app open), failed, or only partly removed.
    pub problems: usize,
    /// Items not started because Stop was pressed. They are left exactly as they were.
    pub stopped: usize,
    pub logged: bool,
}

pub struct LogRow {
    pub when: Stamp,
    pub rule: String,
    pub path: String,
    pub bytes: i64,
    pub result: String,
}

/// Deletes the items of the given rules. Every path is checked by the guard again right before it is deleted,
/// with a guard rebuilt now (so a program installed since the scan still counts), and a rule whose app has been
/// opened since the scan is skipped.
pub fn clean(rules: &mut [Rule], progress: &dyn Fn(usize, usize, &str), dry_run: bool, admin: bool, log_file: Option<&str>, stop: Option<&Cancel>) -> CleanResult {
    let guard = Guard::create_default();
    let processes = ProcessSnapshot::take();
    let stop_now = || stop.is_some_and(Cancel::is_cancelled);
    // an admin rule is skipped, never half-run, if the app is not elevated now
    let busy: Vec<Option<String>> = rules.iter().map(|r| if r.needs_admin && !admin { Some(NEEDS_ADMIN.into()) } else { processes.busy(&r.busy) }).collect();
    let work: Vec<(usize, usize)> = rules.iter().enumerate().filter(|(_, r)| r.can_clean()).flat_map(|(ri, r)| (0..r.items.len()).map(move |ii| (ri, ii))).collect();
    let (handler_work, work): (Vec<_>, Vec<_>) = work.into_iter().partition(|&(ri, ii)| rules[ri].items[ii].handler.is_some());
    let total = work.len() + usize::from(!handler_work.is_empty());

    let drives = drives_to_measure(rules, &work, &handler_work);
    let before: BTreeMap<String, Option<u64>> = drives.iter().map(|d| (d.clone(), free_bytes(d))).collect();

    let mut rows = Vec::new();
    let mut problems = 0;
    let mut stopped = 0;
    for (n, &(ri, ii)) in work.iter().enumerate() {
        let rule_id = rules[ri].id.clone();
        let it = &mut rules[ri].items[ii];
        progress(n, total, &it.label);
        let result = if stop_now() {
            // Stop never interrupts a delete in progress; it only keeps the rest from starting.
            stopped += 1;
            "stopped: not started".to_string()
        } else if dry_run {
            "dry run".into()
        } else if let Some(why) = &busy[ri] {
            format!("skipped: {why}")
        } else if let Some(a) = &it.action {
            let r = match a {
                Action::EmptyRecycleBin => system::empty_recycle_bin(),
                Action::DeliveryOptimization => system::clear_delivery_optimization(),
                Action::DeleteDriver(oem) => system::delete_driver(oem),
                Action::Elevated => Err("Runs in the admin helper".into()),
            };
            match r {
                Ok(()) => "done".into(),
                Err(e) if e == system::KEPT_BY_WINDOWS => e,
                Err(e) => format!("failed: {e}"),
            }
        } else {
            delete_path(it.path.as_deref().unwrap_or(""), &guard)
        };
        if !matches!(result.as_str(), "deleted" | "done" | "dry run" | "stopped: not started") {
            problems += 1;
        }
        rows.push(LogRow { when: Local::now(), rule: rule_id, path: it.path.clone().unwrap_or_else(|| it.label.clone()), bytes: if result == "stopped: not started" { 0 } else { it.size.unwrap_or(0) as i64 }, result: result.clone() });
        it.result = Some(result);
    }
    if !handler_work.is_empty() {
        progress(work.len(), total, "Windows Disk Cleanup is running (Update Cleanup can take 10+ minutes)");
        let runnable: Vec<String> = handler_work.iter().filter(|&&(ri, _)| busy[ri].is_none()).filter_map(|&(ri, ii)| rules[ri].items[ii].handler.clone()).collect();
        let result = if stop_now() {
            stopped += handler_work.len();
            "stopped: not started".to_string()
        } else if dry_run {
            "dry run".into()
        } else {
            match if runnable.is_empty() { Ok(Vec::new()) } else { system::run_handlers(&runnable) } {
                Ok(_) => "done by Windows".into(),
                Err(e) => format!("failed: {e}"),
            }
        };
        for &(ri, ii) in &handler_work {
            let res = match &busy[ri] {
                Some(why) if !dry_run => format!("skipped: {why}"),
                _ => result.clone(),
            };
            if !matches!(res.as_str(), "done by Windows" | "dry run" | "stopped: not started") {
                problems += 1;
            }
            let it = &mut rules[ri].items[ii];
            rows.push(LogRow { when: Local::now(), rule: rules_id(&rules[ri].id), path: it.label.clone(), bytes: it.size.unwrap_or(0) as i64, result: res.clone() });
            rules[ri].items[ii].result = Some(res);
        }
    }
    progress(total, total, "");

    let freed: i64 = drives.iter().filter_map(|d| Some(free_bytes(d)? as i64 - (*before.get(d)?)? as i64)).sum();
    let mut logged = false;
    if !dry_run && !rows.is_empty() {
        match append_log(&rows, freed, &Local::now(), log_file) {
            Ok(()) => logged = true,
            Err(e) => write_error(&format!("Could not write runs.csv: {e}")),
        }
    }
    CleanResult { freed, items: work.len() + handler_work.len(), problems, stopped, logged }
}

fn rules_id(id: &str) -> String {
    id.to_string()
}

/// Re-checks with the guard, deletes, and describes what happened.
pub fn delete_path(path: &str, guard: &Guard) -> String {
    if let Some(why) = guard.check(path) {
        return format!("refused: {why}");
    }
    let rep = safe_delete(path);
    if !paths::exists(path) {
        return "deleted".into();
    }
    if rep.placeholders_kept > 0 {
        return format!("partly kept: {} cloud files left in place", rep.placeholders_kept);
    }
    if rep.special_kept > 0 {
        return format!("partly kept: {} special files left in place", rep.special_kept);
    }
    let left = paths::tree_size(path);
    if left > 0 { format!("partly in use, {} left", disk::format_size(left)) } else { "deleted".into() }
}

#[derive(Debug, Default)]
pub struct DeleteReport {
    pub files_deleted: usize,
    pub links_removed: usize,
    /// OneDrive-style placeholders (RecallOnDataAccess, RecallOnOpen, Offline) left in place.
    pub placeholders_kept: usize,
    /// Other reparse points that are neither links nor placeholders, left in place.
    pub special_kept: usize,
    /// Files or folders that could not be removed (usually in use).
    pub failed: usize,
}

/// The recursive delete, written out so every rule is visible: a junction or symlink is removed as a link and
/// never descended into, read-only attributes are cleared first, and cloud placeholder files are never deleted
/// (or hydrated).
pub fn safe_delete(path: &str) -> DeleteReport {
    let mut r = DeleteReport::default();
    delete_entry(path, &mut r);
    r
}

fn clear_read_only(path: &str, a: u32) {
    if a & ATTR_READONLY != 0 {
        if let Ok(m) = fs::symlink_metadata(path) {
            let mut p = m.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            p.set_readonly(false);
            let _ = fs::set_permissions(path, p);
        }
    }
}

fn delete_entry(path: &str, r: &mut DeleteReport) {
    let a = match fs::symlink_metadata(path) {
        Ok(m) => m.file_attributes(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            r.failed += 1;
            return;
        }
    };
    // Placeholders first: they are often reparse points too, and touching them would download them.
    if is_cloud_placeholder(a) {
        r.placeholders_kept += 1;
        return;
    }
    let is_dir = a & ATTR_DIRECTORY != 0;
    if a & ATTR_REPARSE != 0 {
        if paths::is_link_reparse(path) {
            // RemoveDirectory on a junction or directory symlink never touches the target.
            clear_read_only(path, a);
            let ok = if is_dir { fs::remove_dir(path).is_ok() } else { fs::remove_file(path).is_ok() };
            if ok { r.links_removed += 1 } else { r.failed += 1 }
        } else {
            r.special_kept += 1;
        }
        return;
    }
    if is_dir {
        let Some(kids) = children(path, false) else {
            r.failed += 1;
            return;
        };
        let problems_before = r.failed + r.placeholders_kept + r.special_kept;
        for (child, _) in kids {
            delete_entry(&child, r);
        }
        clear_read_only(path, a);
        if fs::remove_dir(path).is_err() && r.failed + r.placeholders_kept + r.special_kept == problems_before {
            // Usually not empty because something inside was kept or is in use, which is already counted.
            r.failed += 1;
        }
    } else {
        clear_read_only(path, a);
        if fs::remove_file(path).is_ok() { r.files_deleted += 1 } else { r.failed += 1 }
    }
}

/// Free bytes on the drive holding `path`.
pub fn free_bytes(path: &str) -> Option<u64> {
    disk::usage(std::path::Path::new(&path[..path.len().min(3)])).map(|(_, f)| f)
}

/// Fixed drives, as "C:\".
pub fn fixed_drives() -> Vec<String> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows::core::HSTRING;
    const DRIVE_FIXED: u32 = 3;
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i) as char))
        .filter(|d| unsafe { GetDriveTypeW(&HSTRING::from(d.as_str())) } == DRIVE_FIXED)
        .collect()
}

fn drives_to_measure(rules: &[Rule], work: &[(usize, usize)], handler_work: &[(usize, usize)]) -> Vec<String> {
    let mut drives: Vec<String> = vec![K.system_drive.clone()];
    let mut any_action = false;
    for &(ri, ii) in work.iter().chain(handler_work) {
        let it = &rules[ri].items[ii];
        if let Some(p) = &it.path {
            if p.len() >= 3 && p.as_bytes()[1] == b':' {
                let root = p[..3].to_uppercase();
                if !drives.iter().any(|d| d.eq_ignore_ascii_case(&root)) {
                    drives.push(root);
                }
            }
        }
        any_action |= it.action.is_some();
    }
    if any_action {
        // the Recycle Bin spans every drive
        for d in fixed_drives() {
            if !drives.iter().any(|x| x.eq_ignore_ascii_case(&d)) {
                drives.push(d);
            }
        }
    }
    drives
}

// ------------------------------------------------------------------ runs.csv

pub const TOTAL: &str = "TOTAL";

pub fn log_path() -> String {
    join(&K.log_dir, "runs.csv")
}

pub fn error_log_path() -> String {
    join(&K.log_dir, "error.log")
}

/// CSV-quotes a text field, and defuses a leading = + @ so a spreadsheet never runs it as a formula.
pub fn field(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '@']) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\r', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

pub fn parse_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if quoted {
            if c == '"' && chars.get(i + 1) == Some(&'"') {
                cur.push('"');
                i += 1;
            } else if c == '"' {
                quoted = false;
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            quoted = true;
        } else if c == ',' {
            fields.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        i += 1;
    }
    fields.push(cur);
    fields
}

fn row(when: &Stamp, rule: &str, path: &str, bytes: i64, result: &str) -> String {
    format!("{},{},{},{},{}\r\n", field(&when.format("%Y-%m-%d %H:%M:%S").to_string()), field(rule), field(path), bytes, field(result))
}

pub fn append_log(rows: &[LogRow], freed: i64, when: &Stamp, file: Option<&str>) -> std::io::Result<()> {
    let file = file.map(str::to_string).unwrap_or_else(log_path);
    if let Some(dir) = std::path::Path::new(&file).parent() {
        fs::create_dir_all(dir)?;
    }
    let mut s = String::new();
    if !std::path::Path::new(&file).exists() {
        s.push_str("time,rule,path,bytes,result\r\n");
    }
    for r in rows {
        s.push_str(&row(&r.when, &r.rule, &r.path, r.bytes, &r.result));
    }
    s.push_str(&row(when, TOTAL, "drive free-space change", freed, "measured"));
    fs::OpenOptions::new().create(true).append(true).open(file)?.write_all(s.as_bytes())
}

/// When the last clean ran and what it freed, from the newest TOTAL row.
pub fn read_last(file: Option<&str>) -> Option<(Stamp, i64)> {
    let text = fs::read_to_string(file.map(str::to_string).unwrap_or_else(log_path)).ok()?;
    text.lines().skip(1).collect::<Vec<_>>().into_iter().rev().find_map(|l| {
        let f = parse_line(l);
        if f.len() >= 4 && f[1] == TOTAL {
            let when = NaiveDateTime::parse_from_str(&f[0], "%Y-%m-%d %H:%M:%S").ok().and_then(|n| Local.from_local_datetime(&n).earliest())?;
            Some((when, f[3].parse().ok()?))
        } else {
            None
        }
    })
}

pub fn write_error(text: &str) {
    let _ = fs::create_dir_all(&K.log_dir);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(error_log_path()) {
        let _ = write!(f, "{}\r\n{text}\r\n\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_fields_round_trip() {
        assert_eq!(field("=cmd|x"), "'=cmd|x");
        assert_eq!(field("a,b"), "\"a,b\"");
        assert_eq!(parse_line("2026-11-18 01:00:00,TOTAL,\"drive, free\",5368709120,measured"), ["2026-11-18 01:00:00", "TOTAL", "drive, free", "5368709120", "measured"]);
    }

    #[test]
    fn log_appends_and_reads_the_last_total() {
        let f = std::env::temp_dir().join(format!("st-runs-{}.csv", std::process::id())).to_string_lossy().into_owned();
        let _ = fs::remove_file(&f);
        let rows = [LogRow { when: Local::now(), rule: "temp".into(), path: r"C:\x".into(), bytes: 10, result: "deleted".into() }];
        append_log(&rows, 1234, &Local::now(), Some(&f)).unwrap();
        append_log(&rows, 99, &Local::now(), Some(&f)).unwrap();
        assert_eq!(read_last(Some(&f)).unwrap().1, 99);
        assert!(fs::read_to_string(&f).unwrap().starts_with("time,rule,path,bytes,result"));
        let _ = fs::remove_file(&f);
    }

    #[test]
    fn safe_delete_removes_a_junction_not_its_target() {
        let t = std::env::temp_dir().join(format!("st-del-{}", std::process::id())).to_string_lossy().into_owned();
        fs::create_dir_all(join(&t, r"victim\a")).unwrap();
        fs::create_dir_all(join(&t, "target")).unwrap();
        fs::write(join(&t, r"target\keep.txt"), b"x").unwrap();
        fs::write(join(&t, r"victim\a\f.txt"), b"x").unwrap();
        let ro = join(&t, r"victim\ro.txt");
        fs::write(&ro, b"x").unwrap();
        let mut p = fs::metadata(&ro).unwrap().permissions();
        p.set_readonly(true);
        fs::set_permissions(&ro, p).unwrap();
        let link = join(&t, r"victim\link");
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J", &link, &join(&t, "target")]).output().map(|o| o.status.success()).unwrap_or(false);
        let rep = safe_delete(&join(&t, "victim"));
        assert!(!paths::exists(&join(&t, "victim")), "{rep:?}");
        assert!(paths::exists(&join(&t, r"target\keep.txt")));
        if made {
            assert_eq!(rep.links_removed, 1);
        }
        let _ = fs::remove_dir_all(&t);
    }
}
