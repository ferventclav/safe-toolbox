//! The elevated half: "safe-toolbox.exe --worker job.json". It never shows a window, installs the admin items
//! and writes its progress as JSON lines the window follows.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::model::{ItemKind, ItemStatus, UpdateItem, UpdateSource};
use super::path_health::{self, Snapshot};
use super::sources::{ChocolateySource, preflight};
use super::storage::{self, append};
use super::winget::{self, WingetOutcome};
use crate::proc::{self, ProcResult, minutes};
use crate::tools::{self, starts_ic};

/// What the window hands to the elevated helper: the admin items plus the PATH snapshot.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkerJob {
    pub stamp: String,
    pub items: Vec<JobItem>,
    pub path: Option<Snapshot>,
    pub progress_file: String,
    pub log_file: String,
    /// The account that pressed Update. If admin was approved with another account, per-user work is sent back.
    pub requesting_user_sid: Option<String>,
    pub restore_point: bool,
    /// When this file exists, the helper starts nothing new ("Stop after this update").
    pub stop_file: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobItem {
    pub key: String,
    pub source: UpdateSource,
    #[serde(default)]
    pub kind: ItemKind,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub target_version: Option<String>,
    #[serde(default)]
    pub unknown_version: bool,
}

impl JobItem {
    pub fn from(i: &UpdateItem) -> Self {
        JobItem {
            key: i.key(),
            source: i.source,
            kind: i.kind,
            id: i.id.clone(),
            name: i.name.clone(),
            target_version: i.target_version.clone(),
            unknown_version: i.unknown_version,
        }
    }

    fn to_item(&self) -> UpdateItem {
        let mut i = UpdateItem::new(self.source, &self.id, &self.name);
        i.kind = self.kind;
        i.target_version = self.target_version.clone();
        i.unknown_version = self.unknown_version;
        i
    }
}

/// One line of the progress file (JSON lines) the helper writes and the window follows.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProgressEvent {
    pub key: Option<String>,
    pub status: ItemStatus,
    pub message: Option<String>,
    pub reboot: bool,
    pub retry_locally: bool,
    pub output: Option<Vec<String>>,
    pub finished: bool,
}

impl ProgressEvent {
    fn keyed(key: &str, status: ItemStatus, message: impl Into<String>) -> Self {
        ProgressEvent { key: Some(key.to_string()), status, message: Some(message.into()), ..Default::default() }
    }
}

fn done(ji: &JobItem, message: &str, r: &ProcResult, reboot: bool) -> ProgressEvent {
    ProgressEvent { reboot, output: Some(r.tail(12)), ..ProgressEvent::keyed(&ji.key, ItemStatus::Done, message) }
}

fn fail(ji: &JobItem, message: impl Into<String>, r: &ProcResult) -> ProgressEvent {
    ProgressEvent { output: Some(r.tail(12)), ..ProgressEvent::keyed(&ji.key, ItemStatus::Failed, message) }
}

/// Entry point for "--worker job.json". Returns the process exit code.
pub fn run_file(job_path: &str) -> i32 {
    let job = std::fs::read_to_string(job_path)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str::<WorkerJob>(t.trim_start_matches('\u{feff}')).map_err(|e| e.to_string()));
    match job {
        Ok(job) => {
            run(&job);
            0
        }
        Err(e) => {
            storage::write_error(&format!("Worker failed: {e}"));
            1
        }
    }
}

pub fn run(job: &WorkerJob) {
    let progress = PathBuf::from(&job.progress_file);
    let log = PathBuf::from(&job.log_file);
    let emit = |e: ProgressEvent| append(&progress, &serde_json::to_string(&e).unwrap_or_default());

    // When a standard user types an admin's password, this process is that admin: its HKCU, AppData and
    // per-user installs belong to the wrong person. Machine-wide work is still fine; per-user work goes back.
    let same_user = match &job.requesting_user_sid {
        None => true,
        Some(sid) => tools::current_user_sid().is_some_and(|me| me.eq_ignore_ascii_case(sid)),
    };

    if job.restore_point && !job.items.is_empty() {
        emit(ProgressEvent { key: Some("status".into()), message: Some("Making a restore point…".into()), ..Default::default() });
        let rp = proc::powershell(preflight::RESTORE_POINT_SCRIPT, minutes(5), None);
        append(&log, &format!("=== Restore point\n{}", rp.output));
        emit(ProgressEvent::keyed("restore-point", ItemStatus::Done, preflight::describe_restore_point(&rp.output)));
    }

    // "Stop after this update" in the window creates this file; nothing new starts once it exists.
    let stopped = |ji: &JobItem| -> bool {
        match &job.stop_file {
            Some(f) if Path::new(f).exists() => {
                emit(ProgressEvent::keyed(&ji.key, ItemStatus::Skipped, "Stopped before it started"));
                true
            }
            _ => false,
        }
    };

    // Runtimes first so later installers see them; Visual Studio last because it is slow.
    let mut apps: Vec<&JobItem> = job.items.iter().filter(|i| i.kind == ItemKind::Package && i.source == UpdateSource::Apps).collect();
    apps.sort_by_key(|i| {
        if starts_ic(&i.id, "Python.") || starts_ic(&i.id, "OpenJS.") {
            0
        } else if starts_ic(&i.id, "Microsoft.VisualStudio") {
            2
        } else {
            1
        }
    });

    let winget_exe = tools::winget();
    for ji in apps {
        if stopped(ji) {
            continue;
        }
        if !same_user {
            emit(ProgressEvent {
                retry_locally: true,
                ..ProgressEvent::keyed(&ji.key, ItemStatus::Queued, "Admin was approved with a different account, so this installs as you instead.")
            });
            continue;
        }
        let item = ji.to_item();
        emit(ProgressEvent::keyed(&ji.key, ItemStatus::Running, "Installing…"));
        append(&log, &format!("=== {} [{}] (admin)", ji.name, ji.id));
        let mut r = proc::run_simple(&winget_exe, winget::upgrade_args(&item, false), minutes(45));
        append(&log, &format!("{}exit {}", r.output, r.exit_code));
        let mut outcome = winget::classify(r.exit_code, &r.output, true);
        if outcome == WingetOutcome::RetryForce {
            append(&log, "--- retrying with --force");
            r = proc::run_simple(&winget_exe, winget::upgrade_args(&item, true), minutes(45));
            append(&log, &format!("{}exit {}", r.output, r.exit_code));
            outcome = winget::classify(r.exit_code, &r.output, true);
        }
        emit(match outcome {
            WingetOutcome::Ok => done(ji, "Updated", &r, false),
            WingetOutcome::OkReboot => done(ji, "Updated. Restart to finish.", &r, true),
            WingetOutcome::AlreadyCurrent => done(ji, "Already up to date", &r, false),
            WingetOutcome::RetryNonAdmin => ProgressEvent {
                retry_locally: true,
                output: Some(r.tail(12)),
                ..ProgressEvent::keyed(&ji.key, ItemStatus::Queued, "Its installer failed as admin. Trying again without admin…")
            },
            WingetOutcome::UserScope => ProgressEvent {
                retry_locally: true,
                output: Some(r.tail(12)),
                ..ProgressEvent::keyed(&ji.key, ItemStatus::Queued, "Installed for your account only, so it updates as you next…")
            },
            WingetOutcome::InUse => fail(ji, "A file was in use. Close the app and try again.", &r),
            _ => fail(ji, format!("Didn't install: {}", winget::failure_reason(&r)), &r),
        });
    }

    let choco = tools::choco();
    for ji in job.items.iter().filter(|i| i.source == UpdateSource::Chocolatey) {
        if stopped(ji) {
            continue;
        }
        let Some(choco) = &choco else {
            emit(ProgressEvent::keyed(&ji.key, ItemStatus::Failed, "Chocolatey was not found"));
            continue;
        };
        emit(ProgressEvent::keyed(&ji.key, ItemStatus::Running, "Installing…"));
        append(&log, &format!("=== {} (Chocolatey, admin)", ji.name));
        let r = proc::run_simple(choco, ChocolateySource::upgrade_args(&ji.to_item()), minutes(45));
        append(&log, &format!("{}exit {}", r.output, r.exit_code));
        // 1641 and 3010 are the standard "succeeded, restart needed" installer codes.
        emit(match r.exit_code {
            0 => done(ji, "Updated", &r, false),
            1641 | 3010 => done(ji, "Updated. Restart to finish.", &r, true),
            code => fail(ji, format!("Didn't install: {}", r.last_line().unwrap_or_else(|| format!("exit {code}"))), &r),
        });
    }

    if let (Some(snap), true) = (&job.path, same_user) {
        let plan = path_health::plan(snap, &path_health::user_path_entries(), &path_health::links_dir(), &path_health::packages_dir());
        let fixes = if plan.is_empty() { Vec::new() } else { path_health::apply(&plan) };
        append(&log, &format!("=== PATH check\n{}", if fixes.is_empty() { "All commands still reachable".to_string() } else { fixes.join("\n") }));
        if !fixes.is_empty() {
            emit(ProgressEvent::keyed("path-health", ItemStatus::Done, format!("{}.", fixes.join(". "))));
        }
    }

    emit(ProgressEvent { finished: true, ..Default::default() });
}

/// Follows the helper's progress file from the window side.
pub struct ProgressReader {
    path: PathBuf,
    offset: u64,
    partial: String,
}

impl ProgressReader {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        ProgressReader { path: path.into(), offset: 0, partial: String::new() }
    }

    pub fn read_new(&mut self) -> Vec<ProgressEvent> {
        let mut events = Vec::new();
        let Ok(mut f) = std::fs::File::open(&self.path) else { return events };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len <= self.offset || f.seek(SeekFrom::Start(self.offset)).is_err() {
            return events;
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_err() {
            return events;
        }
        self.offset += buf.len() as u64;
        let text = std::mem::take(&mut self.partial) + &String::from_utf8_lossy(&buf);
        let Some(last) = text.rfind('\n') else {
            self.partial = text;
            return events;
        };
        self.partial = text[last + 1..].to_string();
        for line in text[..last].split('\n').map(str::trim).filter(|l| !l.is_empty()) {
            if let Ok(e) = serde_json::from_str::<ProgressEvent>(line) {
                events.push(e);
            }
        }
        events
    }
}

/// Starts this exe elevated in worker mode. Err("declined") when the admin prompt was declined.
pub fn launch(job_path: &Path) -> Result<crate::shell::ElevatedProcess, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    crate::shell::run_as_admin(&exe.to_string_lossy(), &format!("--worker \"{}\"", job_path.to_string_lossy()), &dir)
}

/// Waits briefly between progress reads.
pub const POLL: Duration = Duration::from_millis(500);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reader_handles_partial_lines() {
        let f = std::env::temp_dir().join(format!("st-progress-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&f);
        let mut r = ProgressReader::new(&f);
        assert!(r.read_new().is_empty());
        std::fs::write(&f, "{\"key\":\"Apps|Git.Git\",\"status\":\"Running\",\"message\":\"Installing…\"}\n{\"key\":\"Apps|Git.G").unwrap();
        let a = r.read_new();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].status, ItemStatus::Running);
        append(&f, "it\",\"status\":\"Done\",\"reboot\":true}\n{\"finished\":true}");
        let b = r.read_new();
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].key.as_deref(), Some("Apps|Git.Git"));
        assert!(b[0].reboot && b[1].finished);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn job_round_trips_in_the_csharp_shape() {
        let job: WorkerJob = serde_json::from_str(
            r#"{"stamp":"20261119-001","items":[{"key":"Apps|Git.Git","source":"Apps","kind":"Package","id":"Git.Git","name":"Git","targetVersion":null,"unknownVersion":false}],"path":{"links":{"ffmpeg.exe":"C:\\x"},"packageDirs":{}},"progressFile":"p","logFile":"l","requestingUserSid":"S-1-5-21-1","restorePoint":true,"stopFile":null}"#,
        )
        .unwrap();
        assert_eq!(job.items[0].source, UpdateSource::Apps);
        assert_eq!(job.path.unwrap().links["ffmpeg.exe"], r"C:\x");
    }
}
