//! Running other programs: UTF-8 output, a timeout, a stop switch, and the PATH as the registry has it now.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A stop switch shared between the window and running work.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProcResult {
    pub exit_code: i32,
    pub output: String,
    pub timed_out: bool,
}

impl ProcResult {
    pub fn new(exit_code: i32, output: impl Into<String>, timed_out: bool) -> Self {
        Self { exit_code, output: output.into(), timed_out }
    }

    pub fn ok(&self) -> bool {
        self.exit_code == 0 && !self.timed_out
    }

    pub fn lines(&self) -> Vec<&str> {
        self.output.split('\n').map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty()).collect()
    }

    pub fn tail(&self, count: usize) -> Vec<String> {
        let lines = self.lines();
        let from = lines.len().saturating_sub(count);
        lines[from..].iter().map(|s| s.to_string()).collect()
    }

    pub fn last_line(&self) -> Option<String> {
        self.tail(1).into_iter().next()
    }
}

/// winget and pip redraw progress with \r and block characters; keep only the final text of a line.
pub fn clean_line(raw: &str) -> String {
    let s = raw.trim_end_matches('\r');
    let s = match s.rfind('\r') {
        Some(cr) => &s[cr + 1..],
        None => s,
    };
    if s.chars().any(|c| ('\u{2580}'..='\u{259F}').contains(&c)) {
        return String::new();
    }
    let t = s.trim();
    if matches!(t, "-" | "\\" | "|" | "/") {
        return String::new();
    }
    s.trim_end().to_string()
}

/// Runs a program and waits for it, collecting stdout and stderr as cleaned lines.
pub fn run<I, S>(
    file: &str,
    args: I,
    timeout: Duration,
    on_line: Option<&(dyn Fn(&str) + Sync)>,
    cancel: Option<&Cancel>,
) -> ProcResult
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(file);
    cmd.args(args)
        .env("Path", crate::tools::fresh_path())
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let name = std::path::Path::new(file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            return ProcResult::new(-1, format!("Could not start {name}: {e}"), false);
        }
    };
    drop(child.stdin.take());
    let out = Mutex::new(String::new());
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut timed_out = false;
    let mut code = -1;
    std::thread::scope(|scope| {
        let handle = |stream: Box<dyn Read + Send>| {
            let out = &out;
            scope.spawn(move || {
                let mut reader = BufReader::new(stream);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let text = String::from_utf8_lossy(&buf);
                            let line = clean_line(text.trim_end_matches('\n'));
                            if line.is_empty() {
                                continue;
                            }
                            {
                                let mut o = out.lock().unwrap();
                                o.push_str(&line);
                                o.push('\n');
                            }
                            if let Some(f) = on_line {
                                f(&line);
                            }
                        }
                    }
                }
            });
        };
        if let Some(s) = stdout {
            handle(Box::new(s));
        }
        if let Some(s) = stderr {
            handle(Box::new(s));
        }
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    code = status.code().unwrap_or(-1);
                    break;
                }
                Ok(None) => {}
                Err(_) => break,
            }
            if start.elapsed() >= timeout || cancel.is_some_and(|c| c.is_cancelled()) {
                timed_out = true;
                kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let mut output = out.into_inner().unwrap();
    if timed_out {
        output.push_str(&format!("Stopped after {:.0} minutes without finishing.\n", timeout.as_secs_f64() / 60.0));
    }
    ProcResult::new(if timed_out { -1 } else { code }, output, timed_out)
}

/// Runs without a line callback or stop switch.
pub fn run_simple<I, S>(file: &str, args: I, timeout: Duration) -> ProcResult
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run(file, args, timeout, None, None)
}

fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

/// Runs a PowerShell script without quoting trouble (passed as -EncodedCommand).
pub fn powershell(script: &str, timeout: Duration, cancel: Option<&Cancel>) -> ProcResult {
    // No progress records: with redirected output PowerShell writes them as CLIXML into the text we parse.
    let wrapped = format!("[Console]::OutputEncoding = [Text.Encoding]::UTF8\n$ProgressPreference = 'SilentlyContinue'\n{script}");
    let utf16: Vec<u8> = wrapped.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64(&utf16);
    run(
        &crate::tools::powershell(),
        ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encoded],
        timeout,
        None,
        cancel,
    )
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

pub fn minutes(m: u64) -> Duration {
    Duration::from_secs(m * 60)
}

/// The last line of the text that starts a JSON value, as the PowerShell and Python scripts print it.
pub fn last_json_line(output: &str, object_ok: bool) -> Option<&str> {
    output
        .split('\n')
        .map(str::trim)
        .filter(|l| l.starts_with('[') || (object_ok && l.starts_with('{')))
        .last()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_line_keeps_the_final_redraw() {
        assert_eq!(clean_line("  10%\r  55%\r  Done"), "  Done");
        assert_eq!(clean_line("████▌ 40%"), "");
        assert_eq!(clean_line(" / "), "");
        assert_eq!(clean_line("Found Git [Git.Git]   "), "Found Git [Git.Git]");
    }

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn tail_skips_blank_lines() {
        let r = ProcResult::new(0, "a\r\n\nb\nc\n", false);
        assert_eq!(r.tail(2), vec!["b", "c"]);
        assert_eq!(r.last_line().as_deref(), Some("c"));
    }
}
