//! Read-only health checks that work without admin: the System log (Kernel-Power 41, BugCheck 1001, EventLog
//! 6008), the Application log (Application Error 1000), MSFT_PhysicalDisk, powercfg /batteryreport and Windows
//! Security Center. Event 41 stores its stop code in decimal; it is shown in hex (159 is 0x9F).

use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Local, TimeDelta};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use super::{exports, jobs, write_error, xml_unescape};
use crate::proc::{last_json_line, powershell, run_simple};
use crate::time::Stamp;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CrashKind {
    BlueScreen,
    UnexpectedShutdown,
    PowerLoss,
}

/// One time Windows went down without shutting down properly.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crash {
    pub when: Stamp,
    pub kind: CrashKind,
    pub stop_code: Option<u32>,
    pub source: String,
}

impl Crash {
    pub fn stop_code_hex(&self) -> String {
        self.stop_code.map(|c| format!("0x{c:X}")).unwrap_or_default()
    }
    pub fn stop_name(&self) -> Option<&'static str> {
        self.stop_code.and_then(stop_codes::name)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCrash {
    pub app: String,
    pub count: usize,
    pub last: Stamp,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveHealth {
    pub name: String,
    pub media: String,
    pub bus: String,
    pub health: String,
    pub status: String,
    pub size: u64,
}

/// Admin-only SMART-style counters from MSFT_StorageReliabilityCounter. None means not reported.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveWear {
    pub device_id: String,
    pub wear: Option<i32>,
    pub temperature: Option<i32>,
    pub temperature_max: Option<i32>,
    pub power_on_hours: Option<i64>,
    pub read_errors: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryHealth {
    pub name: String,
    pub design_capacity: u64,
    pub full_charge_capacity: u64,
    pub cycle_count: Option<u32>,
}

impl BatteryHealth {
    /// How much of its new capacity the battery still holds, in percent.
    pub fn percent(&self) -> f64 {
        if self.design_capacity == 0 { 0.0 } else { (100.0 * self.full_charge_capacity as f64 / self.design_capacity as f64).round() }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AntivirusState {
    pub name: String,
    pub on: bool,
    pub up_to_date: bool,
}

// ------------------------------------------------------------------ event log (via wevtutil, XML out)

struct Event {
    id: u32,
    provider: String,
    when: Stamp,
    data: Vec<(String, String)>,
}

static EVENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<Event[ >].*?</Event>").unwrap());
static EVENT_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<EventID[^>]*>(\d+)</EventID>").unwrap());
static PROVIDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<Provider Name=['"]([^'"]+)['"]"#).unwrap());
static CREATED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<TimeCreated SystemTime=['"]([^'"]+)['"]"#).unwrap());
static DATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<Data(?:\s+Name=['"]([^'"]*)['"])?\s*>([^<]*)</Data>"#).unwrap());

fn parse_events(xml: &str) -> Vec<Event> {
    EVENT
        .find_iter(xml)
        .filter_map(|m| {
            let e = m.as_str();
            let id = EVENT_ID.captures(e)?[1].parse().ok()?;
            let provider = PROVIDER.captures(e).map(|c| c[1].to_string()).unwrap_or_default();
            let when = DateTime::parse_from_rfc3339(&CREATED.captures(e)?[1]).ok()?.with_timezone(&Local);
            let data = DATA.captures_iter(e).map(|c| (c.get(1).map_or("", |n| n.as_str()).to_string(), xml_unescape(&c[2]))).collect();
            Some(Event { id, provider, when, data })
        })
        .collect()
}

fn read_log(log: &str, xpath: &str) -> Vec<Event> {
    let exe = crate::tools::system_dir().join("wevtutil.exe");
    let r = run_simple(&exe.to_string_lossy(), ["qe", log, &format!("/q:{xpath}"), "/rd:true", "/c:2000", "/f:xml"], Duration::from_secs(60));
    if !r.ok() {
        write_error(&format!("Reading the {log} log failed: {}", r.last_line().unwrap_or_default()));
    }
    parse_events(&r.output)
}

fn ms(days: i64) -> i64 {
    days * 86_400_000
}

pub fn crashes(days: i64) -> Vec<Crash> {
    let xpath = format!("*[System[(EventID=41 or EventID=1001 or EventID=6008) and TimeCreated[timediff(@SystemTime) <= {}]]]", ms(days));
    let mut list = Vec::new();
    for e in read_log("System", &xpath) {
        let p = e.provider.to_lowercase();
        if e.id == 41 && p.contains("kernel-power") {
            let code = e.data.iter().find(|(n, _)| n == "BugcheckCode").and_then(|(_, v)| v.trim().parse::<u32>().ok()).filter(|&c| c != 0);
            let kind = if code.is_some() { CrashKind::BlueScreen } else { CrashKind::PowerLoss };
            list.push(Crash { when: e.when, kind, stop_code: code, source: "Kernel-Power 41".into() });
        } else if e.id == 1001 && p.contains("wer-systemerrorreporting") {
            let code = e.data.first().and_then(|(_, v)| parse_bugcheck_text(v));
            list.push(Crash { when: e.when, kind: CrashKind::BlueScreen, stop_code: code, source: "BugCheck 1001".into() });
        } else if e.id == 6008 && p == "eventlog" {
            list.push(Crash { when: e.when, kind: CrashKind::UnexpectedShutdown, stop_code: None, source: "EventLog 6008".into() });
        }
    }
    merge(list)
}

/// One crash often leaves two or three events (41 at the next boot, 1001 with the stop code, 6008). Events within
/// ten minutes of each other count once, keeping the most informative. Newest first.
pub fn merge(mut events: Vec<Crash>) -> Vec<Crash> {
    let rank = |c: &Crash| if c.stop_code.is_some() { 3 } else if c.kind == CrashKind::PowerLoss { 2 } else { 1 };
    events.sort_by_key(|c| c.when);
    let mut merged: Vec<Crash> = Vec::new();
    for c in events {
        if let Some(last) = merged.last_mut() {
            if (c.when - last.when).abs() <= TimeDelta::minutes(10) {
                if rank(&c) > rank(last) {
                    *last = Crash { when: last.when, ..c };
                }
                continue;
            }
        }
        merged.push(c);
    }
    merged.reverse();
    merged
}

/// "0x00000154 (0xffff..., ...)" from a BugCheck event, as 0x154.
pub fn parse_bugcheck_text(text: &str) -> Option<u32> {
    let i = text.to_lowercase().find("0x")?;
    let hex: String = text[i + 2..].chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    u32::from_str_radix(&hex, 16).ok().filter(|&v| v != 0)
}

pub fn app_crashes(days: i64) -> Vec<AppCrash> {
    let xpath = format!("*[System[Provider[@Name='Application Error'] and EventID=1000 and TimeCreated[timediff(@SystemTime) <= {}]]]", ms(days));
    let mut by_app: Vec<AppCrash> = Vec::new();
    for e in read_log("Application", &xpath) {
        let app = e.data.first().map(|(_, v)| v.clone()).filter(|v| !v.is_empty()).unwrap_or_else(|| "unknown".into());
        match by_app.iter_mut().find(|a| a.app.eq_ignore_ascii_case(&app)) {
            Some(a) => {
                a.count += 1;
                a.last = a.last.max(e.when);
            }
            None => by_app.push(AppCrash { app, count: 1, last: e.when }),
        }
    }
    by_app.sort_by(|a, b| b.count.cmp(&a.count).then(b.last.cmp(&a.last)));
    by_app
}

// ------------------------------------------------------------------ drives and antivirus (one PowerShell start)

const DEVICES_SCRIPT: &str = r#"
$d = @(Get-CimInstance -Namespace root\Microsoft\Windows\Storage -ClassName MSFT_PhysicalDisk -ErrorAction SilentlyContinue | ForEach-Object { [pscustomobject]@{ Name = $_.FriendlyName; Media = $_.MediaType; Bus = $_.BusType; Health = $_.HealthStatus; Ops = @($_.OperationalStatus); Size = $_.Size } })
$a = @(Get-CimInstance -Namespace root\SecurityCenter2 -ClassName AntiVirusProduct -ErrorAction SilentlyContinue | ForEach-Object { [pscustomobject]@{ Name = $_.displayName; State = $_.productState } })
[pscustomobject]@{ Drives = $d; Antivirus = $a } | ConvertTo-Json -Compress -Depth 4
"#;

fn num(v: &Value, name: &str) -> Option<u64> {
    v.get(name).and_then(Value::as_u64)
}

fn list(v: &Value, name: &str) -> Vec<Value> {
    match v.get(name) {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Null) | None => Vec::new(),
        Some(one) => vec![one.clone()],
    }
}

fn operational(v: u64) -> String {
    match v {
        2 => "OK".into(),
        3 => "Degraded".into(),
        5 => "Predictive failure".into(),
        6 => "Error".into(),
        0xD00D => "In service".into(),
        0xD00E => "Transient error".into(),
        _ => format!("Status {v}"),
    }
}

pub fn parse_devices(json: &str) -> (Vec<DriveHealth>, Vec<AntivirusState>) {
    let Ok(root) = serde_json::from_str::<Value>(json) else { return Default::default() };
    let drives = list(&root, "Drives")
        .iter()
        .map(|o| {
            let ops: Vec<String> = list(o, "Ops").iter().filter_map(Value::as_u64).map(operational).collect();
            DriveHealth {
                name: o.get("Name").and_then(Value::as_str).unwrap_or("Drive").trim().to_string(),
                media: match num(o, "Media") { Some(3) => "Hard drive", Some(4) => "SSD", Some(5) => "Storage class memory", _ => "Drive" }.into(),
                bus: match num(o, "Bus") { Some(17) => "NVMe", Some(11) => "SATA", Some(7) => "USB", Some(8) => "RAID", Some(10) => "SAS", _ => "" }.into(),
                health: match num(o, "Health") { Some(0) => "Healthy", Some(1) => "Warning", Some(2) => "Unhealthy", _ => "Unknown" }.into(),
                status: if ops.is_empty() { "OK".into() } else { ops.join(", ") },
                size: num(o, "Size").unwrap_or(0),
            }
        })
        .collect();
    let mut av: Vec<AntivirusState> = list(&root, "Antivirus")
        .iter()
        .map(|o| {
            let state = num(o, "State").unwrap_or(0);
            AntivirusState { name: o.get("Name").and_then(Value::as_str).unwrap_or("Antivirus").into(), on: state & 0x1000 != 0, up_to_date: state & 0x10 == 0 }
        })
        .collect();
    av.sort_by_key(|a| !a.on);
    (drives, av)
}

pub fn drives_and_antivirus() -> (Vec<DriveHealth>, Vec<AntivirusState>) {
    let r = powershell(DEVICES_SCRIPT, Duration::from_secs(60), None);
    match last_json_line(&r.output, true) {
        Some(j) => parse_devices(j),
        None => {
            write_error(&format!("Drive health read failed: {}", r.last_line().unwrap_or_default()));
            Default::default()
        }
    }
}

/// Reads wear and temperature. It needs admin, so the page runs it elevated (admin_tasks "drive-wear").
pub const DRIVE_WEAR_SCRIPT: &str = "Get-PhysicalDisk | ForEach-Object { $d = $_; $r = $d | Get-StorageReliabilityCounter; [pscustomobject]@{ DeviceId = $d.FriendlyName; Wear = $r.Wear; Temperature = $r.Temperature; TemperatureMax = $r.TemperatureMax; PowerOnHours = $r.PowerOnHours; ReadErrors = $r.ReadErrorsUncorrected } } | ConvertTo-Json -Compress";

pub fn parse_drive_wear(json: &str) -> Vec<DriveWear> {
    let Ok(v) = serde_json::from_str::<Value>(json.trim()) else { return Vec::new() };
    let rows = match v {
        Value::Array(a) => a,
        o @ Value::Object(_) => vec![o],
        _ => return Vec::new(),
    };
    let int = |o: &Value, n: &str| o.get(n).and_then(Value::as_i64);
    rows.iter()
        .map(|o| DriveWear {
            device_id: o.get("DeviceId").and_then(Value::as_str).unwrap_or("Drive").into(),
            wear: int(o, "Wear").map(|x| x as i32),
            temperature: int(o, "Temperature").map(|x| x as i32),
            temperature_max: int(o, "TemperatureMax").map(|x| x as i32),
            power_on_hours: int(o, "PowerOnHours"),
            read_errors: int(o, "ReadErrors"),
        })
        .collect()
}

// ------------------------------------------------------------------ battery

fn powercfg(args: &[&str], secs: u64) -> bool {
    let exe = crate::tools::system_dir().join("powercfg.exe");
    run_simple(&exe.to_string_lossy(), args, Duration::from_secs(secs)).ok()
}

/// From "powercfg /batteryreport /xml", which works without admin. Empty on a desktop PC.
pub fn batteries() -> Vec<BatteryHealth> {
    let xml = jobs().join("battery.xml");
    let out = if powercfg(&["/batteryreport", "/xml", "/output", &xml.to_string_lossy()], 30) {
        std::fs::read(&xml).map(|b| parse_battery_report(&decode(&b))).unwrap_or_default()
    } else {
        Vec::new()
    };
    let _ = std::fs::remove_file(&xml);
    out
}

/// powercfg writes UTF-8, sometimes with a BOM; UTF-16 is handled in case a build changes that.
fn decode(b: &[u8]) -> String {
    if b.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(b).trim_start_matches('\u{feff}').to_string()
    }
}

static BATTERY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<Battery>(.*?)</Battery>").unwrap());

pub fn parse_battery_report(xml: &str) -> Vec<BatteryHealth> {
    BATTERY
        .captures_iter(xml)
        .filter_map(|c| {
            let b = &c[1];
            let v = |n: &str| -> Option<String> {
                let re = Regex::new(&format!(r"<{n}>([^<]*)</{n}>")).ok()?;
                re.captures(b).map(|m| xml_unescape(m[1].trim()))
            };
            let design: u64 = v("DesignCapacity")?.parse().ok().filter(|&d| d > 0)?;
            let full: u64 = v("FullChargeCapacity")?.parse().ok()?;
            let cycles = v("CycleCount").and_then(|s| s.parse::<u32>().ok()).filter(|&c| c > 0);
            let name = [v("Manufacturer"), v("Id")].into_iter().flatten().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(" ");
            Some(BatteryHealth { name: if name.is_empty() { "Battery".into() } else { name }, design_capacity: design, full_charge_capacity: full, cycle_count: cycles })
        })
        .collect()
}

/// Writes the full HTML battery report and returns its path.
pub fn battery_report() -> Option<String> {
    let html = exports().join(format!("battery-report-{}.html", Local::now().format("%Y%m%d-%H%M")));
    (powercfg(&["/batteryreport", "/output", &html.to_string_lossy()], 60) && html.exists()).then(|| html.to_string_lossy().into_owned())
}

pub fn uptime() -> Duration {
    Duration::from_millis(unsafe { windows::Win32::System::SystemInformation::GetTickCount64() })
}

pub fn restart_pending() -> bool {
    crate::updater::sources::preflight::reboot_pending()
}

/// Names and a plain hint for the stop codes people actually meet. The rest link to Microsoft's reference.
pub mod stop_codes {
    const KNOWN: [(u32, &str, &str); 27] = [
        (0x0A, "IRQL_NOT_LESS_OR_EQUAL", "Usually a driver touching memory it shouldn't."),
        (0x1A, "MEMORY_MANAGEMENT", "Often memory (RAM) or a driver. A memory test is the next step."),
        (0x1E, "KMODE_EXCEPTION_NOT_HANDLED", "A driver crashed."),
        (0x3B, "SYSTEM_SERVICE_EXCEPTION", "Often a graphics, antivirus or other driver."),
        (0x50, "PAGE_FAULT_IN_NONPAGED_AREA", "Memory, a driver or antivirus software."),
        (0x7A, "KERNEL_DATA_INPAGE_ERROR", "Windows couldn't read from the drive. Check the drive and its cable or slot."),
        (0x7E, "SYSTEM_THREAD_EXCEPTION_NOT_HANDLED", "A driver crashed."),
        (0x7F, "UNEXPECTED_KERNEL_MODE_TRAP", "Often hardware: memory, overheating or overclocking."),
        (0x9C, "MACHINE_CHECK_EXCEPTION", "The processor reported a hardware error."),
        (0x9F, "DRIVER_POWER_STATE_FAILURE", "A driver didn't handle sleep or wake properly."),
        (0xA0, "INTERNAL_POWER_ERROR", "Power management, often sleep, hibernation or a driver."),
        (0xC2, "BAD_POOL_CALLER", "A driver misused memory."),
        (0xD1, "DRIVER_IRQL_NOT_LESS_OR_EQUAL", "A driver touched memory it shouldn't, often network or graphics."),
        (0xEF, "CRITICAL_PROCESS_DIED", "A core Windows process stopped. System file repair can help."),
        (0xF4, "CRITICAL_OBJECT_TERMINATION", "A core process stopped, sometimes because the drive failed to respond."),
        (0x101, "CLOCK_WATCHDOG_TIMEOUT", "A processor core stopped responding: overclocking, undervolting or heat."),
        (0x109, "CRITICAL_STRUCTURE_CORRUPTION", "Kernel memory was changed: a driver or memory fault."),
        (0x113, "VIDEO_DXGKRNL_FATAL_ERROR", "The graphics stack failed. Graphics driver."),
        (0x116, "VIDEO_TDR_FAILURE", "The graphics driver stopped responding and couldn't recover."),
        (0x117, "VIDEO_TDR_TIMEOUT_DETECTED", "The graphics driver stopped responding."),
        (0x119, "VIDEO_SCHEDULER_INTERNAL_ERROR", "Graphics driver or GPU."),
        (0x124, "WHEA_UNCORRECTABLE_ERROR", "A hardware error: processor, memory, heat or overclocking."),
        (0x133, "DPC_WATCHDOG_VIOLATION", "A driver took too long, often storage (SSD firmware) or network."),
        (0x139, "KERNEL_SECURITY_CHECK_FAILURE", "A driver or memory corrupted kernel data."),
        (0x13A, "KERNEL_MODE_HEAP_CORRUPTION", "A driver corrupted kernel memory."),
        (0x154, "UNEXPECTED_STORE_EXCEPTION", "The storage stack failed: often the SSD, its firmware or its driver."),
        (0x1E0, "ILLEGAL_ATS_INITIALIZATION", "A device or driver problem."),
    ];

    fn find(code: u32) -> Option<&'static (u32, &'static str, &'static str)> {
        KNOWN.iter().find(|k| k.0 == code)
    }
    pub fn name(code: u32) -> Option<&'static str> {
        find(code).map(|k| k.1)
    }
    pub fn hint(code: u32) -> Option<&'static str> {
        find(code).map(|k| k.2)
    }
    /// Microsoft's page for the code, or the reference list when its page name isn't known.
    pub fn link(code: u32) -> String {
        match find(code) {
            Some(k) => format!("https://learn.microsoft.com/windows-hardware/drivers/debugger/bug-check-0x{code:x}--{}", k.1.to_lowercase().replace('_', "-")),
            None => "https://learn.microsoft.com/windows-hardware/drivers/debugger/bug-check-code-reference2".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn bugcheck_codes_read_as_hex() {
        assert_eq!(parse_bugcheck_text("0x00000154 (0xffffd60e2f5dc000, 0xffffa38b, 0x2, 0x0)"), Some(0x154));
        assert_eq!(parse_bugcheck_text("The computer has rebooted from a bugcheck.  The bugcheck was: 0x0000009f (0x3, ...)"), Some(0x9F));
        assert_eq!(parse_bugcheck_text("no code here"), None);
        assert_eq!(parse_bugcheck_text("0x00000000"), None);
    }

    #[test]
    fn one_crash_counts_once_and_keeps_the_stop_code() {
        let t = Local.with_ymd_and_hms(2026, 7, 1, 10, 0, 0).unwrap();
        let c = |m: i64, kind, code, src: &str| Crash { when: t + TimeDelta::minutes(m), kind, stop_code: code, source: src.into() };
        let merged = merge(vec![
            c(2, CrashKind::PowerLoss, None, "Kernel-Power 41"),
            c(0, CrashKind::UnexpectedShutdown, None, "EventLog 6008"),
            c(3, CrashKind::BlueScreen, Some(0x154), "BugCheck 1001"),
            c(2 * 24 * 60, CrashKind::PowerLoss, None, "Kernel-Power 41"),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].when, t + TimeDelta::days(2));
        assert_eq!((merged[1].stop_code, merged[1].stop_name(), merged[1].stop_code_hex().as_str()), (Some(0x154), Some("UNEXPECTED_STORE_EXCEPTION"), "0x154"));
        assert_eq!(merged[1].when, t);
        assert_eq!(stop_codes::link(0x9F), "https://learn.microsoft.com/windows-hardware/drivers/debugger/bug-check-0x9f--driver-power-state-failure");
        assert!(stop_codes::link(0xDEAD).ends_with("bug-check-code-reference2"));
    }

    #[test]
    fn wevtutil_xml_is_read() {
        let xml = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Microsoft-Windows-Kernel-Power' Guid='{x}'/><EventID>41</EventID><TimeCreated SystemTime='2026-11-03T12:10:05.1234567Z'/></System><EventData><Data Name='BugcheckCode'>340</Data><Data Name='Other'>a &amp; b</Data></EventData></Event><Event xmlns='x'><System><Provider Name='Application Error'/><EventID Qualifiers='0'>1000</EventID><TimeCreated SystemTime='2026-11-04T01:00:00.0000000Z'/></System><EventData><Data>chrome.exe</Data></EventData></Event>";
        let ev = parse_events(xml);
        assert_eq!(ev.len(), 2);
        assert_eq!((ev[0].id, ev[0].data[0].1.as_str(), ev[0].data[1].1.as_str()), (41, "340", "a & b"));
        assert_eq!((ev[1].id, ev[1].provider.as_str(), ev[1].data[0].1.as_str()), (1000, "Application Error", "chrome.exe"));
    }

    #[test]
    fn battery_drives_and_wear_parse() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?><BatteryReport xmlns="http://schemas.microsoft.com/battery/2012"><Batteries><Battery><Id>BIF0_9</Id><Manufacturer>MSI</Manufacturer><DesignCapacity>63232</DesignCapacity><FullChargeCapacity>45478</FullChargeCapacity><CycleCount>0</CycleCount></Battery></Batteries></BatteryReport>"#;
        let b = parse_battery_report(xml);
        assert_eq!((b.len(), b[0].percent(), b[0].cycle_count, b[0].name.as_str()), (1, 72.0, None, "MSI BIF0_9"));

        let one = parse_drive_wear(r#"{"DeviceId":"SAMSUNG MZVL21T0HCLR","Wear":3,"Temperature":41,"TemperatureMax":82,"PowerOnHours":5120,"ReadErrors":0}"#);
        assert_eq!((one[0].wear, one[0].temperature), (Some(3), Some(41)));
        let two = parse_drive_wear(r#"[{"DeviceId":"A","Wear":null},{"DeviceId":"B","Wear":10}]"#);
        assert_eq!((two[0].wear, two[1].wear), (None, Some(10)));
        assert!(parse_drive_wear("not json").is_empty());

        let (d, av) = parse_devices(r#"{"Drives":[{"Name":"SSD ","Media":4,"Bus":17,"Health":0,"Ops":[2],"Size":1024}],"Antivirus":[{"Name":"Old","State":266240},{"Name":"Microsoft Defender Antivirus","State":397568}]}"#);
        assert_eq!((d[0].name.as_str(), d[0].media.as_str(), d[0].bus.as_str(), d[0].health.as_str(), d[0].status.as_str()), ("SSD", "SSD", "NVMe", "Healthy", "OK"));
        assert_eq!(av.len(), 2);
        assert!(av[0].on && av[0].up_to_date);
    }
}
