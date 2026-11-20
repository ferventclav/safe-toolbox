//! winget: reads its fixed-width tables (it has no JSON output for upgrades) and judges how an upgrade went.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use super::model::{SourceResult, UpdateItem, UpdateSource};
use super::{policy, versions};
use crate::proc::{self, Cancel, ProcResult, clean_line, minutes};
use crate::tools::{self, contains_ic, starts_ic};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WingetOutcome {
    Ok,
    OkReboot,
    AlreadyCurrent,
    RetryForce,
    RetryNonAdmin,
    InUse,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub id: String,
    pub version: String,
    pub available: String,
    pub source: String,
    pub pinned: bool,
}

static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\S+").unwrap());

// HRESULTs seen on real runs. Checked before any text, because winget's messages are translated.
const PORTABLE_MODIFIED: i32 = 0x8A15_0057_u32 as i32;
const SHELL_EXEC_INSTALL_FAILED: i32 = 0x8A15_0006_u32 as i32;
const UPDATE_NOT_APPLICABLE: i32 = 0x8A15_002B_u32 as i32;
const REBOOT_REQUIRED_TO_FINISH: i32 = 0x8A15_0109_u32 as i32;

fn lines_of(output: &str) -> Vec<String> {
    output.replace("\r\n", "\n").split('\n').map(clean_line).collect()
}

fn is_dashes(line: &str) -> bool {
    let t = line.trim();
    t.chars().count() >= 10 && t.chars().all(|c| c == '-')
}

/// Cuts a line at character positions (winget pads by characters, not bytes).
fn cut(line: &str, starts: &[usize]) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    (0..starts.len())
        .map(|k| {
            let from = starts[k];
            let to = if k + 1 < starts.len() { starts[k + 1] } else { chars.len() };
            if from >= chars.len() { String::new() } else { chars[from..to.min(chars.len())].iter().collect::<String>().trim().to_string() }
        })
        .collect()
}

/// Works in any display language: the header is whatever line sits on the dashes, columns come from where its
/// words start, and a second table is always the "needs explicit targeting" (pinned) list.
pub fn parse_upgrade_table(output: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    let lines = lines_of(output);
    let mut table = 0;
    let mut i = 0;
    while i + 1 < lines.len() {
        if !is_dashes(&lines[i + 1]) {
            i += 1;
            continue;
        }
        let header = &lines[i];
        let starts: Vec<usize> = WORD.find_iter(header).map(|m| header[..m.start()].chars().count()).collect();
        if starts.len() < 4 {
            i += 1;
            continue;
        }
        table += 1;
        let mut j = i + 2;
        while j < lines.len() {
            let d = &lines[j];
            if d.trim().is_empty() {
                break;
            }
            // The "28 upgrades available." footer is too short to reach the Id column.
            if d.chars().count() > starts[1] {
                let cells = cut(d, &starts);
                let id = &cells[1];
                // A cut-off id ("Microsoft.VisualStudio.Bui…") cannot be targeted, so it is left out.
                if !(id.is_empty() || id.contains(' ') || id.ends_with('…') || cells[2].is_empty() || cells[3].is_empty()) {
                    rows.push(Row {
                        name: cells[0].clone(),
                        id: id.clone(),
                        version: cells[2].clone(),
                        available: cells[3].clone(),
                        source: cells.get(4).cloned().unwrap_or_default(),
                        pinned: table > 1,
                    });
                }
            }
            j += 1;
        }
        i += 2;
    }
    rows
}

/// Versions listed by "winget show --id X --versions", newest first as winget prints them.
pub fn parse_versions(output: &str) -> Vec<String> {
    let lines = lines_of(output);
    // Here the rule under "Version" is only as wide as the column ("-------").
    let Some(dash) = lines.iter().position(|l| l.trim().len() >= 3 && l.trim().chars().all(|c| c == '-')) else { return Vec::new() };
    lines[dash + 1..].iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && versions::parse(l).is_some()).collect()
}

/// The newest version in the same family as `current` that is newer than it.
pub fn highest_in_family(versions_list: &[String], current: &str) -> Option<String> {
    versions_list
        .iter()
        .filter(|v| versions::same_family(v, current) && versions::compare(v, current).is_gt())
        .max_by(|a, b| versions::compare(a, b))
        .cloned()
}

pub fn classify(exit_code: i32, output: &str, elevated: bool) -> WingetOutcome {
    let has = |s: &str| contains_ic(output, s);
    // A portable package whose files changed since install refuses to upgrade without --force.
    if exit_code == PORTABLE_MODIFIED || (has("has been modified") && has("--force")) {
        return WingetOutcome::RetryForce;
    }
    if exit_code == 0 {
        return if has("restart your pc") || has("restart is required") || has("reboot") { WingetOutcome::OkReboot } else { WingetOutcome::Ok };
    }
    if exit_code == REBOOT_REQUIRED_TO_FINISH {
        return WingetOutcome::OkReboot;
    }
    if exit_code == UPDATE_NOT_APPLICABLE || has("No available upgrade found") || has("No newer package versions are available") {
        return WingetOutcome::AlreadyCurrent;
    }
    if has("Access is denied") || has("being used by another process") {
        return WingetOutcome::InUse;
    }
    // Some installers crash when started as admin (GeForce NOW: 0xC0000005); they work as the user.
    if elevated && (exit_code == SHELL_EXEC_INSTALL_FAILED || has("Installer failed with exit code")) {
        return WingetOutcome::RetryNonAdmin;
    }
    WingetOutcome::Failed
}

/// The reason to show for a failed upgrade. winget's last line is often just its progress ("Downloading ..."),
/// so a line that names the error wins, and a run that stopped mid-download says so.
pub fn failure_reason(r: &ProcResult) -> String {
    if r.timed_out {
        return "it ran out of time".into();
    }
    let lines: Vec<&str> = r.lines().into_iter().map(str::trim).filter(|l| !l.is_empty()).collect();
    if let Some(cause) = lines.iter().rev().find(|l| starts_ic(l, "0x") || contains_ic(l, "failed")) {
        return cause.to_string();
    }
    match lines.last() {
        None => format!("exit {}", r.exit_code),
        Some(last) if starts_ic(last, "Downloading ") => "the download didn't finish. Check your connection and try again".into(),
        Some(last) => last.to_string(),
    }
}

pub fn upgrade_args(item: &UpdateItem, force: bool) -> Vec<String> {
    let mut args: Vec<String> = ["upgrade", "--id", &item.id, "-e", "--silent", "--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Some(t) = &item.target_version {
        args.push("--version".into());
        args.push(t.clone());
    }
    if item.unknown_version {
        args.push("--include-unknown".into());
    }
    if force {
        args.push("--force".into());
    }
    args
}

pub fn check(cancel: &Cancel) -> SourceResult {
    if !tools::has_winget() {
        return SourceResult::failed(
            UpdateSource::Apps,
            "winget isn't on this PC. Install or update \"App Installer\" from the Microsoft Store, then check again.",
        );
    }
    let winget = tools::winget();
    let r = proc::run(&winget, ["upgrade", "--include-unknown", "--accept-source-agreements", "--disable-interactivity"], minutes(4), None, Some(cancel));
    let rows = parse_upgrade_table(&r.output);
    if rows.is_empty() && r.exit_code != 0 && !contains_ic(&r.output, "No installed package") && !contains_ic(&r.output, "No available upgrade") {
        return SourceResult::failed(UpdateSource::Apps, format!("winget could not check: {}", r.last_line().unwrap_or_default()));
    }
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    // winget lists a package twice when it is installed in two scopes; one row per id keeps keys unique.
    for row in rows.into_iter().filter(|r| seen.insert(r.id.to_lowercase())) {
        let major = versions::is_major_jump(&row.version, &row.available);
        let name = row.name.trim_end_matches('…').trim().to_string();
        let mut item = UpdateItem::new(UpdateSource::Apps, &row.id, &name).versions(&row.version, &row.available);
        item.unknown_version = versions::is_unknown(&row.version);
        item.is_major = major;
        item.pinned = row.pinned;
        item.needs_admin = true;
        items.push(item);
        if major && policy::is_runtime_name(&row.id, &row.name) {
            let show = proc::run(
                &winget,
                ["show", "--id", &row.id, "-e", "--versions", "--accept-source-agreements", "--disable-interactivity"],
                minutes(2),
                None,
                Some(cancel),
            );
            if let Some(same) = highest_in_family(&parse_versions(&show.output), &row.version) {
                let mut stay = UpdateItem::new(UpdateSource::Apps, &row.id, format!("{name} (stay on {})", versions::family(&row.version)))
                    .versions(&row.version, &same);
                stay.target_version = Some(same);
                stay.needs_admin = true;
                items.push(stay);
            }
        }
    }
    SourceResult::items(UpdateSource::Apps, items, "All apps are up to date")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(a: &str, b: &str, c: &str, d: &str, e: &str) -> String {
        format!("{a:<28}{b:<29}{c:<13}{d:<13}{e}\r\n")
    }

    fn table() -> String {
        let mut t = row("Name", "Id", "Version", "Available", "Source");
        t += &"-".repeat(90);
        t += "\r\n";
        t += &row("GitHub CLI", "GitHub.cli", "2.104.0", "2.105.0", "winget");
        t += &row("Microsoft Visual Studio Bu…", "Microsoft.VisualStudio.Bui…", "17.0", "17.1", "winget");
        t += &row("Node.js", "OpenJS.NodeJS", "22.1.0", "24.15.0", "winget");
        t += "3 upgrades available.\r\n\r\n";
        t += "The following packages have an upgrade available, but require explicit targeting for upgrade:\r\n";
        t += &row("Name", "Id", "Version", "Available", "Source");
        t += &"-".repeat(90);
        t += "\r\n";
        t += &row("Pinned App", "Pinned.App", "1.0", "2.0", "winget");
        t
    }

    #[test]
    fn reads_both_tables() {
        let rows = parse_upgrade_table(&table());
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["GitHub.cli", "OpenJS.NodeJS", "Pinned.App"]);
        assert_eq!(rows[0].version, "2.104.0");
        assert_eq!(rows[0].available, "2.105.0");
        assert!(!rows[0].pinned);
        assert!(rows[2].pinned);
    }

    #[test]
    fn reads_versions_and_finds_the_family() {
        let out = "Found Node.js [OpenJS.NodeJS]\nVersion\n-------\n25.0.0\n24.15.0\n24.14.1\n22.20.0\n";
        let v = parse_versions(out);
        assert_eq!(v, ["25.0.0", "24.15.0", "24.14.1", "22.20.0"]);
        assert_eq!(highest_in_family(&v, "24.1.0").as_deref(), Some("24.15.0"));
        assert_eq!(highest_in_family(&v, "24.15.0"), None);
    }

    #[test]
    fn classifies_outcomes() {
        assert_eq!(classify(0, "Successfully installed", false), WingetOutcome::Ok);
        assert_eq!(classify(0, "Restart your PC to finish installation.", false), WingetOutcome::OkReboot);
        assert_eq!(classify(PORTABLE_MODIFIED, "", false), WingetOutcome::RetryForce);
        assert_eq!(classify(UPDATE_NOT_APPLICABLE, "", false), WingetOutcome::AlreadyCurrent);
        assert_eq!(classify(1, "Access is denied.", false), WingetOutcome::InUse);
        assert_eq!(classify(SHELL_EXEC_INSTALL_FAILED, "", true), WingetOutcome::RetryNonAdmin);
        assert_eq!(classify(SHELL_EXEC_INSTALL_FAILED, "", false), WingetOutcome::Failed);
    }

    #[test]
    fn failure_reasons() {
        assert_eq!(failure_reason(&ProcResult::new(1, "Found GitHub CLI [GitHub.cli]\nDownloading https://x\n", false)), "the download didn't finish. Check your connection and try again");
        assert_eq!(failure_reason(&ProcResult::new(1, "Installer failed with exit code: 5\nmore\n", false)), "Installer failed with exit code: 5");
        assert_eq!(failure_reason(&ProcResult::new(1, "0x80072ee2 : timeout\nDownloading x\n", false)), "0x80072ee2 : timeout");
        assert_eq!(failure_reason(&ProcResult::new(7, "", false)), "exit 7");
        assert_eq!(failure_reason(&ProcResult::new(-1, "", true)), "it ran out of time");
    }

    #[test]
    fn upgrade_args_add_flags() {
        let mut i = UpdateItem::new(UpdateSource::Apps, "Git.Git", "Git");
        i.target_version = Some("2.50.1".into());
        i.unknown_version = true;
        let a = upgrade_args(&i, true);
        assert!(a.ends_with(&["--version".into(), "2.50.1".into(), "--include-unknown".into(), "--force".into()]));
    }
}
