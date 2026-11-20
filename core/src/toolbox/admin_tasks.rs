//! Small read-or-create jobs that need admin: drive wear counters and a restore point. The window starts
//! "<exe> --admin-task name output" elevated, waits for it, then reads what it wrote.

use std::path::Path;
use std::time::{Duration, Instant};

fn script(name: &str) -> Option<&'static str> {
    match name {
        "drive-wear" => Some(super::health::DRIVE_WEAR_SCRIPT),
        "restore-point" => Some(crate::updater::sources::preflight::RESTORE_POINT_SCRIPT),
        _ => None,
    }
}

/// The elevated side. Writes the script's output to `output` and returns its exit code.
pub fn run_task(name: &str, output: &str) -> i32 {
    let Some(s) = script(name) else {
        let _ = std::fs::write(output, format!("unknown task {name}"));
        return 2;
    };
    let r = crate::proc::powershell(s, Duration::from_secs(600), None);
    let _ = std::fs::write(output, &r.output);
    r.exit_code
}

/// Runs a task elevated and returns what it printed. Err("declined") when the admin prompt was declined.
pub fn run_elevated(name: &str) -> Result<String, String> {
    let out = super::jobs().join(format!("admin-{name}-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let p = crate::shell::run_as_admin(&exe.to_string_lossy(), &format!("--admin-task {name} \"{}\"", out.to_string_lossy()), &dir)?;
    let start = Instant::now();
    while !p.has_exited() {
        if start.elapsed() > Duration::from_secs(900) {
            return Err("The admin task didn't finish in 15 minutes.".into());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let text = std::fs::read_to_string(&out).map_err(|_| "The admin task wrote nothing.".to_string())?;
    let _ = std::fs::remove_file(Path::new(&out));
    Ok(text)
}
