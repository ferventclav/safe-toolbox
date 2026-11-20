//! Fixes for common Windows problems, each a documented Windows command, run one step at a time with the output
//! kept. System files follow Microsoft's order: DISM /RestoreHealth first, then sfc /scannow.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

use crate::proc::Cancel;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairStep {
    pub label: &'static str,
    pub exe: String,
    pub args: Vec<String>,
    /// Run as a PowerShell script instead of exe + args.
    pub script: Option<&'static str>,
    pub ok_codes: Vec<i32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repair {
    pub id: &'static str,
    pub title: &'static str,
    pub what: &'static str,
    /// What to expect: time, a restart, windows that close.
    pub heads: &'static str,
    pub needs_admin: bool,
    pub restart_after: bool,
    pub steps: Vec<RepairStep>,
}

fn sys(exe: &str) -> String {
    crate::tools::system_dir().join(exe).to_string_lossy().into_owned()
}

fn step(label: &'static str, exe: String, args: &[&str], ok: &[i32]) -> RepairStep {
    RepairStep { label, exe, args: args.iter().map(|s| s.to_string()).collect(), script: None, ok_codes: ok.to_vec() }
}

fn ps(label: &'static str, script: &'static str) -> RepairStep {
    RepairStep { label, exe: crate::tools::powershell(), args: Vec::new(), script: Some(script), ok_codes: vec![0] }
}

pub fn all() -> Vec<Repair> {
    let winget = crate::tools::winget();
    vec![
        Repair {
            id: "system-files",
            title: "Repair Windows system files",
            what: "Checks Windows' own files and replaces any that are damaged, first from Windows Update (DISM), then on the PC (SFC).",
            heads: "Takes 10 to 30 minutes. Your files and apps are not touched.",
            needs_admin: true,
            restart_after: false,
            steps: vec![
                step("Repairing the Windows image (DISM)", sys("dism.exe"), &["/Online", "/Cleanup-Image", "/RestoreHealth"], &[0, 3010]),
                step("Checking system files (SFC)", sys("sfc.exe"), &["/scannow"], &[0, 1]),
            ],
        },
        Repair {
            id: "check-disk",
            title: "Check the drive for errors",
            what: "Scans the C: drive's file system while Windows runs (chkdsk /scan) and fixes what it can without a restart.",
            heads: "Takes a few minutes. If it needs more, it tells you to schedule a check at the next restart.",
            needs_admin: true,
            restart_after: false,
            steps: vec![step("Scanning C:", sys("chkdsk.exe"), &["C:", "/scan"], &[0, 1, 2])],
        },
        Repair {
            id: "dns",
            title: "Flush the DNS cache",
            what: "Forgets saved website addresses, so a site that moved or a changed hosts file takes effect.",
            heads: "Instant. One admin prompt: this Windows build refuses it without one.",
            needs_admin: true,
            restart_after: false,
            steps: vec![step("Flushing DNS", sys("ipconfig.exe"), &["/flushdns"], &[0])],
        },
        Repair {
            id: "network",
            title: "Reset network settings",
            what: "Resets Winsock and TCP/IP to their defaults (netsh winsock reset, netsh int ip reset). Fixes many \"connected, no internet\" problems.",
            heads: "Needs a restart. VPN adapters and static IP settings may need setting up again.",
            needs_admin: true,
            restart_after: true,
            steps: vec![
                step("Resetting Winsock", sys("netsh.exe"), &["winsock", "reset"], &[0, 1]),
                step("Resetting TCP/IP", sys("netsh.exe"), &["int", "ip", "reset"], &[0, 1]),
            ],
        },
        Repair {
            id: "windows-update",
            title: "Reset Windows Update",
            what: "Stops the update services, renames the SoftwareDistribution and catroot2 download folders, and starts the services again, the reset Microsoft documents for stuck updates.",
            heads: "Takes a minute. The old folders are kept with a .bak name, so nothing is lost.",
            needs_admin: true,
            restart_after: false,
            steps: vec![
                ps("Stopping update services", "foreach ($s in 'wuauserv','bits','cryptsvc','usosvc') { Stop-Service -Name $s -Force -ErrorAction SilentlyContinue }; 'stopped'"),
                ps("Renaming the download folders", "$t = Get-Date -Format yyyyMMdd-HHmmss; foreach ($p in \"$env:SystemRoot\\SoftwareDistribution\", \"$env:SystemRoot\\System32\\catroot2\") { if (Test-Path $p) { Rename-Item -Path $p -NewName ((Split-Path $p -Leaf) + '.bak-' + $t) -ErrorAction Stop; \"renamed $p\" } }"),
                ps("Starting update services", "foreach ($s in 'cryptsvc','bits','wuauserv','usosvc') { Start-Service -Name $s -ErrorAction SilentlyContinue }; 'started'"),
            ],
        },
        Repair {
            id: "winget-sources",
            title: "Repair winget sources",
            what: "Resets winget's package sources (winget source reset --force), which fixes \"failed when searching source\" errors in Updates.",
            heads: "Takes under a minute.",
            needs_admin: true,
            restart_after: false,
            steps: vec![
                step("Resetting sources", winget.clone(), &["source", "reset", "--force", "--disable-interactivity"], &[0]),
                step("Updating sources", winget, &["source", "update", "--disable-interactivity"], &[0]),
            ],
        },
        Repair {
            id: "explorer",
            title: "Restart File Explorer",
            what: "Closes and reopens Explorer, the taskbar and the Start button. Fixes a frozen taskbar or a missing tray icon.",
            heads: "Open Explorer windows close. No admin needed.",
            needs_admin: false,
            restart_after: false,
            steps: vec![ps("Restarting Explorer", "Stop-Process -Name explorer -Force; Start-Sleep -Seconds 2; if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe }; 'restarted'")],
        },
        Repair {
            id: "store",
            title: "Reset the Microsoft Store cache",
            what: "Clears the Store's cache (wsreset). Fixes Store apps that won't download or update.",
            heads: "The Store opens when it's done. No admin needed.",
            needs_admin: false,
            restart_after: false,
            steps: vec![step("Resetting the Store cache", sys("wsreset.exe"), &[], &[0])],
        },
    ]
}

pub fn find(id: &str) -> Option<Repair> {
    all().into_iter().find(|r| r.id == id)
}

/// The command a step runs, for the details shown before you start.
pub fn describe(s: &RepairStep) -> String {
    match s.script {
        Some(script) => format!("powershell: {script}"),
        None => std::iter::once(Path::new(&s.exe).file_name().map_or(s.exe.clone(), |n| n.to_string_lossy().into_owned())).chain(s.args.iter().cloned()).collect::<Vec<_>>().join(" "),
    }
}

/// Runs every step, writing each output line to `log`. True when every step succeeded. A failed step stops the
/// repair, except in the Windows Update reset, where the services must be started again whatever happened.
pub fn run(repair: &Repair, log: &(dyn Fn(&str) + Sync), cancel: Option<&Cancel>) -> bool {
    let mut ok = true;
    for s in &repair.steps {
        log(&format!("## {}", s.label));
        if !Path::new(&s.exe).exists() {
            log(&format!("!! {} was not found on this PC.", Path::new(&s.exe).file_name().map_or(s.exe.clone(), |n| n.to_string_lossy().into_owned())));
            return false;
        }
        // SFC writes UTF-16 and DISM redraws a progress bar: drop the zero bytes and repeated lines.
        let last = Mutex::new(String::new());
        let line = |raw: &str| {
            let clean = raw.replace('\0', "");
            let clean = clean.trim();
            let mut l = last.lock().unwrap();
            if !clean.is_empty() && *l != clean {
                *l = clean.to_string();
                log(clean);
            }
        };
        let timeout = Duration::from_secs(2 * 3600);
        let code = match s.script {
            Some(script) => {
                let r = crate::proc::powershell(script, timeout, cancel);
                r.output.lines().for_each(&line);
                r.exit_code
            }
            None => crate::proc::run(&s.exe, &s.args, timeout, Some(&line), cancel).exit_code,
        };
        let step_ok = s.ok_codes.contains(&code);
        log(&if step_ok { format!("== done (exit {code})") } else { format!("!! failed (exit {code})") });
        if !step_ok {
            ok = false;
            if repair.id != "windows-update" {
                break;
            }
        }
    }
    ok
}

/// The elevated side: "<exe> --repair id log". Writes "##EXIT ok|fail" last.
pub fn run_elevated(id: &str, log_file: &str) -> i32 {
    let log = |l: &str| crate::updater::storage::append(Path::new(log_file), l);
    let Some(r) = find(id) else {
        log(&format!("!! unknown repair {id}"));
        log("##EXIT fail");
        return 2;
    };
    let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&r, &log, None))).unwrap_or_else(|_| {
        log("!! the repair stopped with an internal error");
        false
    });
    log(if ok { "##EXIT ok" } else { "##EXIT fail" });
    if ok { 0 } else { 1 }
}

/// Starts the elevated side and returns it with the log file it writes. Err("declined") when the prompt was declined.
pub fn launch_elevated(id: &str) -> Result<(crate::shell::ElevatedProcess, PathBuf), String> {
    let log = super::jobs().join(format!("repair-{id}-{}.log", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    let _ = std::fs::write(&log, "");
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let p = crate::shell::run_as_admin(&exe.to_string_lossy(), &format!("--repair {id} \"{}\"", log.to_string_lossy()), &dir)?;
    Ok((p, log))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_are_well_formed() {
        let r = find("system-files").unwrap();
        assert!(r.steps[0].exe.to_lowercase().contains("dism") && r.steps[0].args.contains(&"/RestoreHealth".to_string()));
        assert!(r.steps[1].exe.to_lowercase().contains("sfc") && r.needs_admin);
        let all = all();
        let mut ids: Vec<_> = all.iter().map(|r| r.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        assert!(all.iter().all(|r| !r.what.is_empty() && !r.heads.is_empty() && !r.steps.is_empty()));
        let dns = find("dns").unwrap();
        assert_eq!((dns.needs_admin, dns.steps.len(), dns.steps[0].args.join(" ")), (true, 1, "/flushdns".to_string()));
        assert_eq!(describe(&dns.steps[0]), "ipconfig.exe /flushdns");
    }
}
