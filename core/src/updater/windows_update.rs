//! Windows Update through its COM API, driven from PowerShell. Searching works without admin; installing needs it.

use chrono::NaiveDate;
use serde_json::Value;

use super::model::{ItemKind, SourceResult, UpdateItem, UpdateSource};
use super::sources::{rows_of, str_of};
use crate::proc::{self, Cancel, minutes};

#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub id: String,
    pub title: String,
    pub driver: bool,
    pub driver_date: Option<NaiveDate>,
    pub size_mb: f64,
    pub kb: String,
    pub installed_date: Option<NaiveDate>,
    pub optional: bool,
}

/// Lists offered updates. For each driver it finds the device by hardware id and reads the date of the driver
/// already installed on it, so an offered driver can be checked as really newer. "optional" is Windows' own
/// flag (not auto-selected), so this works in any display language.
pub const SEARCH_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$s = New-Object -ComObject Microsoft.Update.Session
$q = $s.CreateUpdateSearcher()
$r = $q.Search("IsInstalled=0 and IsHidden=0")
$devices = $null
$list = foreach ($u in $r.Updates) {
    # Definition Updates (Defender): Defender refreshes them by itself several times a day and Settings
    # doesn't list them. Matched by classification id, so it works in any display language.
    $definition = $false
    foreach ($c in $u.Categories) { if ($c.CategoryID -eq 'e0789628-ce08-4437-be74-2495b842f43b') { $definition = $true } }
    if ($definition) { continue }
    $isDriver = $u.Type -eq 2
    $date = $null; $installed = $null
    if ($isDriver) {
        try { $date = $u.DriverVerDate.ToString('yyyy-MM-dd') } catch { }
        try {
            if ($null -eq $devices) {
                $byDevice = @{}
                foreach ($d in Get-CimInstance Win32_PnPSignedDriver) { if ($d.DeviceID) { $byDevice[$d.DeviceID] = $d } }
                $devices = foreach ($e in Get-CimInstance Win32_PnPEntity) {
                    [pscustomobject]@{ ids = @($e.HardwareID) + @($e.CompatibleID); driver = $byDevice[$e.DeviceID] }
                }
            }
            $hw = $u.DriverHardwareID
            $match = $devices | Where-Object { $_.driver -and $_.driver.DriverDate -and ($_.ids -contains $hw) } | Select-Object -First 1
            if ($match) { $installed = ([datetime]$match.driver.DriverDate).ToString('yyyy-MM-dd') }
        } catch { }
    }
    [pscustomobject]@{
        id = $u.Identity.UpdateID; title = $u.Title; driver = $isDriver; driverDate = $date; installedDate = $installed
        optional = -not $u.AutoSelectOnWebSites
        sizeMb = [math]::Round($u.MaxDownloadSize / 1MB, 1); kb = (@($u.KBArticleIDs) | ForEach-Object { "KB$_" }) -join ', '
    }
}
ConvertTo-Json -InputObject @($list) -Compress -Depth 3
"#;

pub fn check(cancel: &Cancel) -> SourceResult {
    let r = proc::powershell(SEARCH_SCRIPT, minutes(6), Some(cancel));
    let Some(records) = parse_search(&r.output) else {
        return SourceResult::failed(UpdateSource::Windows, format!("Windows Update could not be searched: {}", r.last_line().unwrap_or_default()));
    };
    let items = records
        .iter()
        .map(|rec| {
            let mut i = UpdateItem::new(UpdateSource::Windows, &rec.id, &rec.title);
            i.kind = if rec.driver { ItemKind::Driver } else { ItemKind::WindowsUpdate };
            i.current = if rec.driver { "Driver".into() } else { String::new() };
            // Windows reports the largest package; the real download is often a much smaller part of it.
            i.available = if rec.size_mb > 0.0 { size_text(rec.size_mb) } else { String::new() };
            i.driver_date = rec.driver_date;
            i.installed_driver_date = rec.installed_date;
            i.optional = rec.optional;
            i.detail = Some(rec.kb.clone());
            // Windows Update installs these itself; the list only says which are worth installing.
            i.selectable = false;
            i
        })
        .collect();
    SourceResult::items(UpdateSource::Windows, items, "Windows is up to date")
}

pub fn size_text(mb: f64) -> String {
    if mb >= 1024.0 { format!("up to {} GB", crate::disk::one_decimal(mb / 1024.0)) } else { format!("up to {} MB", crate::disk::one_decimal(mb)) }
}

fn date(e: &Value, name: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(e.get(name)?.as_str()?, "%Y-%m-%d").ok()
}

/// The JSON line from SEARCH_SCRIPT; None when the search failed.
pub fn parse_search(output: &str) -> Option<Vec<Record>> {
    let doc: Value = serde_json::from_str(proc::last_json_line(output, true)?).ok()?;
    Some(
        rows_of(&doc)
            .into_iter()
            .filter(|e| e.is_object())
            .map(|e| Record {
                id: str_of(e, "id"),
                title: str_of(e, "title"),
                driver: e.get("driver") == Some(&Value::Bool(true)),
                driver_date: date(e, "driverDate"),
                size_mb: e.get("sizeMb").and_then(Value::as_f64).unwrap_or(0.0),
                kb: str_of(e, "kb"),
                installed_date: date(e, "installedDate"),
                optional: e.get("optional") == Some(&Value::Bool(true)),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_results() {
        let out = r#"[{"id":"a1","title":"2026-11 Security Update","driver":false,"driverDate":null,"installedDate":null,"optional":false,"sizeMb":92712.7,"kb":"KB5132061"},{"id":"b2","title":"Intel - Chipset","driver":true,"driverDate":"2017-01-02","installedDate":"2006-06-21","optional":true,"sizeMb":0,"kb":""}]"#;
        let r = parse_search(out).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r[1].driver && r[1].optional);
        assert_eq!(r[1].driver_date, NaiveDate::from_ymd_opt(2017, 1, 2));
        assert_eq!(size_text(r[0].size_mb), "up to 90.5 GB");
        assert_eq!(size_text(803.0), "up to 803 MB");
        assert!(parse_search("error text").is_none());
        assert_eq!(parse_search("[]").unwrap().len(), 0);
    }

}
