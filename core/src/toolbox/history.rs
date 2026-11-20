//! Reads the two run logs the tools keep: %LOCALAPPDATA%\SafeUpdater\runs.csv (one row per update, all rows of a
//! run share a time) and %LOCALAPPDATA%\SafeDiskCleaner\runs.csv (one row per path, then a TOTAL row).

use chrono::{Local, NaiveDateTime, TimeZone};
use serde::Serialize;

use crate::disk::format_size;
use crate::time::Stamp;
use crate::updater::model::UpdateSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum RunKind {
    Updates,
    Cleanup,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryLine {
    pub title: String,
    pub detail: String,
    pub outcome: String,
    pub problem: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRun {
    pub kind: RunKind,
    pub when: Stamp,
    pub summary: String,
    pub lines: Vec<HistoryLine>,
    pub problems: usize,
    /// Cleanup runs: the measured change in free space.
    pub freed: i64,
}

fn run(kind: RunKind, when: Stamp, summary: String, lines: Vec<HistoryLine>, freed: i64) -> HistoryRun {
    let problems = lines.iter().filter(|l| l.problem).count();
    HistoryRun { kind, when, summary, lines, problems, freed }
}

pub fn read_all(rule_title: &dyn Fn(&str) -> String) -> Vec<HistoryRun> {
    let mut all = read_updates(&crate::updater::storage::runs_csv().to_string_lossy());
    all.extend(read_cleanups(&crate::cleaner::engine::log_path(), rule_title));
    all.sort_by(|a, b| b.when.cmp(&a.when));
    all
}

fn time(s: &str) -> Option<Stamp> {
    let t = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()?;
    Local.from_local_datetime(&t).earliest()
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// Groups by key, keeping the order each key first appeared in.
fn group_by<'a>(rows: impl IntoIterator<Item = &'a Vec<String>>, key: usize) -> Vec<(String, Vec<&'a Vec<String>>)> {
    let mut groups: Vec<(String, Vec<&Vec<String>>)> = Vec::new();
    for r in rows {
        match groups.iter_mut().find(|(k, _)| *k == r[key]) {
            Some((_, g)) => g.push(r),
            None => groups.push((r[key].clone(), vec![r])),
        }
    }
    groups
}

pub fn read_updates(file: &str) -> Vec<HistoryRun> {
    let rows: Vec<Vec<String>> = rows(file).into_iter().skip(1).filter(|f| f.len() >= 7).collect();
    let mut runs = Vec::new();
    for (key, group) in group_by(&rows, 0) {
        let Some(when) = time(&key) else { continue };
        let (mut done, mut failed, mut other) = (0, 0, 0);
        let mut lines = Vec::new();
        for f in group {
            let (status, result) = (f[6].as_str(), f.get(7).map_or("", String::as_str));
            let problem = status == "Failed";
            match status {
                "Done" => done += 1,
                "Failed" => failed += 1,
                _ => other += 1,
            }
            let versions = if f[4].is_empty() && f[5].is_empty() { String::new() } else { format!(" · {} → {}", if f[4].is_empty() { "?" } else { &f[4] }, f[5]) };
            let source = UpdateSource::parse(&f[1]).map_or(f[1].clone(), |s| s.title().to_string());
            lines.push(HistoryLine { title: f[3].clone(), detail: format!("{source}{versions}"), outcome: (if result.is_empty() { status } else { result }).to_string(), problem });
        }
        let mut parts = vec![format!("{done} updated")];
        if failed > 0 {
            parts.push(format!("{failed} didn't"));
        }
        if other > 0 {
            parts.push(format!("{other} kept back"));
        }
        lines.sort_by(|a, b| b.problem.cmp(&a.problem).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase())));
        runs.push(run(RunKind::Updates, when, parts.join(", "), lines, 0));
    }
    runs
}

fn clean_ok(result: &str) -> bool {
    matches!(result, "deleted" | "done" | "done by Windows" | "dry run")
}

pub fn read_cleanups(file: &str, rule_title: &dyn Fn(&str) -> String) -> Vec<HistoryRun> {
    let mut runs = Vec::new();
    let mut pending: Vec<Vec<String>> = Vec::new();
    for f in rows(file).into_iter().skip(1).filter(|f| f.len() >= 5) {
        if f[1] != "TOTAL" {
            pending.push(f);
            continue;
        }
        let Some(when) = time(&f[0]) else {
            pending.clear();
            continue;
        };
        let freed: i64 = f[3].parse().unwrap_or(0);
        let problems = pending.iter().filter(|p| !clean_ok(&p[4])).count();
        let mut summary = if freed > 0 { format!("Freed {}", format_size(freed as u64)) } else { "Nothing measurable freed".to_string() };
        summary += &format!(", {}", plural(pending.len(), "item"));
        if problems > 0 {
            summary += &format!(", {problems} left alone");
        }
        let lines = group_by(&pending, 1)
            .into_iter()
            .map(|(rule, g)| {
                let bytes: u64 = g.iter().map(|p| p[3].parse::<u64>().unwrap_or(0)).sum();
                let bad: Vec<_> = g.iter().filter(|p| !clean_ok(&p[4])).collect();
                let outcome = if bad.is_empty() {
                    "Cleaned".to_string()
                } else if bad.len() == g.len() {
                    let r = &bad[0][4];
                    if r.chars().count() > 80 { format!("{}…", r.chars().take(80).collect::<String>()) } else { r.clone() }
                } else {
                    format!("{} of {} left alone", bad.len(), g.len())
                };
                let size = if bytes > 0 { format!(" · {}", format_size(bytes)) } else { String::new() };
                HistoryLine { title: rule_title(&rule), detail: format!("{}{size}", plural(g.len(), "item")), outcome, problem: !bad.is_empty() }
            })
            .collect();
        runs.push(run(RunKind::Cleanup, when, summary, lines, freed));
        pending.clear();
    }
    runs
}

/// CSV rows, rebuilt across lines because a quoted field can hold a newline (pip output in a result).
pub fn rows(file: &str) -> Vec<Vec<String>> {
    let Ok(bytes) = std::fs::read(file) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    let mut out = Vec::new();
    let mut buf = String::new();
    for raw in text.trim_start_matches('\u{feff}').split('\n') {
        buf.push_str(raw.trim_end_matches('\r'));
        if buf.matches('"').count() % 2 == 1 {
            buf.push('\n');
            continue;
        }
        if !buf.is_empty() {
            out.push(parse_line(&buf));
        }
        buf.clear();
    }
    out
}

pub fn parse_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (false, '"') => quoted = true,
            (false, ',') => fields.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, text: &str) -> String {
        let f = std::env::temp_dir().join(format!("st-{name}-{}.csv", std::process::id()));
        std::fs::write(&f, text).unwrap();
        f.to_string_lossy().into_owned()
    }

    #[test]
    fn update_runs_group_by_time_and_count_outcomes() {
        let f = temp("upd", "when,source,id,name,from,to,status,result\n2026-11-03 17:56:00,Apps,Git.Git,Git,2.55.0,2.56.0,Done,Updated\n2026-11-03 17:56:00,Python,numpy,numpy,2.5.1,2.5.4,Failed,\"Didn't install: line one\nline two, with a comma\"\n2026-11-03 17:56:00,Node,docx,docx,9.8.0,9.15.2,Skipped,Stopped before it started\n2026-11-05 09:00:00,Apps,Ollama.Ollama,Ollama,0.40.14,0.40.15,Done,Updated\n");
        let runs = read_updates(&f);
        assert_eq!(runs.len(), 2);
        let first = &runs[0];
        assert_eq!(first.summary, "1 updated, 1 didn't, 1 kept back");
        assert_eq!(first.problems, 1);
        assert_eq!(first.lines[0].title, "numpy");
        assert!(first.lines[0].outcome.contains("line two, with a comma"));
        assert_eq!(first.lines[0].detail, "Python packages · 2.5.1 → 2.5.4");
        assert!(read_updates(r"C:\nowhere\runs.csv").is_empty());
        let _ = std::fs::remove_file(f);
    }

    #[test]
    fn cleanups_end_at_each_total_row_and_group_by_rule() {
        let f = temp("cln", "time,rule,path,bytes,result\n2026-11-03 17:40:01,temp,C:\\a,1000,deleted\n2026-11-03 17:40:02,temp,C:\\b,2000,deleted\n2026-11-03 17:40:03,cache-chrome,C:\\c,500,skipped: Chrome is open\n2026-11-03 17:40:04,TOTAL,drive free-space change,2048,measured\n2026-11-04 10:00:00,recycle,Recycle Bin,0,done\n2026-11-04 10:00:01,TOTAL,drive free-space change,0,measured\n");
        let runs = read_cleanups(&f, &|id| if id == "temp" { "Old temp files".into() } else { id.into() });
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].freed, 2048);
        assert_eq!(runs[0].summary, "Freed 2 KB, 3 items, 1 left alone");
        let temp_line = runs[0].lines.iter().find(|l| l.title == "Old temp files").unwrap();
        assert!(!temp_line.problem && temp_line.detail.contains("2 items"));
        assert!(runs[0].lines.iter().find(|l| l.title == "cache-chrome").unwrap().problem);
        assert_eq!(runs[1].summary, "Nothing measurable freed, 1 item");
        let _ = std::fs::remove_file(f);
    }
}
