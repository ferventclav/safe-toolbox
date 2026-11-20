//! One Task Scheduler task, "Safe Toolbox background check", that runs "<exe> --background" as you, never
//! elevated. It only checks and notifies. A missed run (PC off or asleep) runs at the next chance. The exe
//! starts through a windowless cmd with RunAsInvoker, so a "Run as administrator" compatibility setting on the
//! exe can't stop the task: without it Windows refuses the launch with 0x800702E4.

use std::sync::LazyLock;
use std::time::Duration;

use chrono::{Local, NaiveDateTime, TimeDelta};
use regex::Regex;

use super::{Frequency, ToolboxSettings, jobs, xml_escape, xml_unescape};

pub const TASK_NAME: &str = "Safe Toolbox background check";
const RUN_AS_INVOKER: &str = "__COMPAT_LAYER=RunAsInvoker";
const BACKGROUND_SUFFIX: &str = "\" --background";
const ELEVATION_REQUIRED: i32 = 0x800702E4u32 as i32;

/// What the task starts: conhost without a window, then cmd, then the exe as the current user.
pub fn task_action(exe: &str) -> (String, String) {
    (
        crate::tools::system_dir().join("conhost.exe").to_string_lossy().into_owned(),
        format!("--headless cmd.exe /d /c \"set \"{RUN_AS_INVOKER}\"&\"{exe}{BACKGROUND_SUFFIX}\""),
    )
}

/// Daily when either job is daily, weekly when both are weekly or one is off, None when both are off.
pub fn task_frequency(s: &ToolboxSettings) -> Option<Frequency> {
    use Frequency::*;
    if s.update_check == Daily || s.cleanup_reminder == Daily {
        Some(Daily)
    } else if s.update_check == Weekly || s.cleanup_reminder == Weekly {
        Some(Weekly)
    } else {
        None
    }
}

/// The task XML. Weekly runs on Sunday; both start at 12:30 with up to 30 minutes of random delay.
pub fn build_xml(exe: &str, frequency: Frequency, now: NaiveDateTime, user_id: Option<&str>) -> String {
    let user = user_id.map(str::to_string).or_else(crate::tools::current_user_sid).unwrap_or_default();
    let mut start = now.date().and_hms_opt(12, 30, 0).unwrap();
    if start <= now {
        start += TimeDelta::days(1);
    }
    let schedule = if frequency == Frequency::Daily {
        "<ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>"
    } else {
        "<ScheduleByWeek><DaysOfWeek><Sunday /></DaysOfWeek><WeeksInterval>1</WeeksInterval></ScheduleByWeek>"
    };
    let dir = std::path::Path::new(exe).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let (command, arguments) = task_action(exe);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Author>Safe Toolbox</Author>
    <Description>Checks for safe updates and space to free, and shows a notification. It never installs or deletes anything by itself. Turn it off in Safe Toolbox, Settings.</Description>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>{start}</StartBoundary>
      <RandomDelay>PT30M</RandomDelay>
      <Enabled>true</Enabled>
      {schedule}
    </CalendarTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>true</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT1H</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        start = start.format("%Y-%m-%dT%H:%M:%S"),
        user = xml_escape(&user),
        command = xml_escape(&command),
        arguments = xml_escape(&arguments),
        dir = xml_escape(&dir),
    )
}

fn schtasks(args: &[&str]) -> (i32, String) {
    let exe = crate::tools::system_dir().join("schtasks.exe");
    let r = crate::proc::run_simple(&exe.to_string_lossy(), args, Duration::from_secs(30));
    (r.exit_code, r.output)
}

/// Creates, updates or removes the task to match the settings. Returns an error message, or None.
pub fn sync(s: &ToolboxSettings, exe: &str) -> Option<String> {
    let Some(f) = task_frequency(s) else { return if exists() { remove() } else { None } };
    let file = jobs().join("task.xml");
    let xml = build_xml(exe, f, Local::now().naive_local(), None);
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(|u| u.to_le_bytes()));
    if let Err(e) = std::fs::write(&file, bytes) {
        return Some(e.to_string());
    }
    let (code, output) = schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &file.to_string_lossy(), "/F"]);
    let _ = std::fs::remove_file(&file);
    (code != 0).then(|| format!("Task Scheduler refused the task: {}", output.trim()))
}

pub fn exists() -> bool {
    schtasks(&["/Query", "/TN", TASK_NAME]).0 == 0
}

fn element(xml: &str, name: &str) -> String {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    match (xml.find(&open), xml.find(&close)) {
        (Some(a), Some(b)) if b > a => xml_unescape(&xml[a + open.len()..b]),
        _ => String::new(),
    }
}

/// The exe a registered task runs, and whether the task already starts it the current way.
pub fn parse_registered(xml: &str) -> (Option<String>, bool) {
    let command = element(xml, "Command").trim_matches('"').to_string();
    let arguments = element(xml, "Arguments");
    if let Some(end) = arguments.rfind(BACKGROUND_SUFFIX).filter(|&e| e > 0 && arguments.contains(RUN_AS_INVOKER)) {
        let start = arguments[..end].rfind('"').map_or(0, |s| s + 1);
        return (Some(arguments[start..end].to_string()), true);
    }
    ((!command.is_empty()).then_some(command), false)
}

/// Brings an existing task up to date when this exe owns it, or when the exe it names is gone. A copy run from
/// somewhere else leaves a working task alone. Returns an error message, or None.
pub fn repair(s: &ToolboxSettings, exe: &str) -> Option<String> {
    task_frequency(s)?;
    let (code, xml) = schtasks(&["/Query", "/TN", TASK_NAME, "/XML"]);
    if code != 0 {
        return None;
    }
    let (registered, current) = parse_registered(&xml);
    let mine = registered.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(exe));
    let gone = registered.as_deref().is_none_or(|r| !std::path::Path::new(r).exists());
    if (mine && !current) || (!mine && gone) { sync(s, exe) } else { None }
}

static CSV_FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"]*)""#).unwrap());

/// The "Last Result" column of schtasks /Query /FO CSV /V /NH, as a signed HRESULT.
pub fn parse_last_result(csv: &str) -> Option<i32> {
    let v: i64 = CSV_FIELD.captures_iter(csv).nth(6)?.get(1)?.as_str().parse().ok()?;
    Some(v as i32)
}

/// What went wrong last time in plain words, or None when the task ran, is running or hasn't run yet.
pub fn describe_last_result(result: Option<i32>) -> Option<String> {
    match result? {
        0 | 0x41300..=0x41303 | 0x41325 => None,
        ELEVATION_REQUIRED => Some("The last background check didn't start, because Windows asked for admin first.".into()),
        r => Some(format!("The last background check didn't finish (code 0x{:08X}). Details may be in error.log.", r as u32)),
    }
}

pub fn last_problem() -> Option<String> {
    let (code, csv) = schtasks(&["/Query", "/TN", TASK_NAME, "/FO", "CSV", "/V", "/NH"]);
    if code == 0 { describe_last_result(parse_last_result(&csv)) } else { None }
}

pub fn remove() -> Option<String> {
    let (code, output) = schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]);
    (code != 0).then(|| output.trim().to_string())
}

/// Runs the task now, for "Check now in the background".
pub fn run_now() -> bool {
    schtasks(&["/Run", "/TN", TASK_NAME]).0 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn task_runs_least_privilege_through_conhost_and_catches_up() {
        let xml = build_xml(r"C:\Apps\Safe Toolbox\safe-toolbox.exe", Frequency::Weekly, at("2026-11-06 23:00"), Some("S-1-5-21-1"));
        for want in ["<RunLevel>LeastPrivilege</RunLevel>", "<StartWhenAvailable>true</StartWhenAvailable>", "<Sunday />", "<StartBoundary>2026-11-07T12:30:00</StartBoundary>", "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"] {
            assert!(xml.contains(want), "{want}");
        }
        assert!(element(&xml, "Command").to_lowercase().ends_with("conhost.exe"));
        assert_eq!(element(&xml, "Arguments"), "--headless cmd.exe /d /c \"set \"__COMPAT_LAYER=RunAsInvoker\"&\"C:\\Apps\\Safe Toolbox\\safe-toolbox.exe\" --background\"");
    }

    #[test]
    fn registered_tasks_read_back() {
        let xml = build_xml(r"C:\Apps\R&D Tools\Safe Toolbox.exe", Frequency::Daily, at("2026-11-06 09:00"), Some("S-1-5-21-1"));
        assert_eq!(parse_registered(&xml), (Some(r"C:\Apps\R&D Tools\Safe Toolbox.exe".into()), true));
        let old = "<Exec><Command>\"C:\\Apps\\Safe Toolbox.exe\"</Command><Arguments>--background</Arguments></Exec>";
        assert_eq!(parse_registered(old), (Some(r"C:\Apps\Safe Toolbox.exe".into()), false));
    }

    #[test]
    fn last_results_and_frequency() {
        let row = |v: &str| format!("\"PC\",\"\\Task\",\"19-11\",\"Ready\",\"Interactive\",\"18-11\",\"{v}\",\"me\"");
        assert_eq!(parse_last_result(&row("-2147024156")), Some(ELEVATION_REQUIRED));
        assert_eq!(parse_last_result(&row("2147943140")), Some(ELEVATION_REQUIRED));
        assert_eq!(parse_last_result(&row("0")), Some(0));
        for ok in [0, 0x41301, 0x41303] {
            assert!(describe_last_result(Some(ok)).is_none());
        }
        assert!(describe_last_result(Some(ELEVATION_REQUIRED)).unwrap().contains("admin"));
        assert!(describe_last_result(Some(1)).unwrap().contains("0x00000001"));
        let f = |u, c| task_frequency(&ToolboxSettings { update_check: u, cleanup_reminder: c, ..Default::default() });
        assert_eq!(f(Frequency::Off, Frequency::Off), None);
        assert_eq!(f(Frequency::Weekly, Frequency::Off), Some(Frequency::Weekly));
        assert_eq!(f(Frequency::Weekly, Frequency::Daily), Some(Frequency::Daily));
    }
}
