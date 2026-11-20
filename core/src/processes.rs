//! What is running right now, read with a ToolHelp snapshot.

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};

#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    /// "chrome.exe"
    pub exe: String,
}

impl Proc {
    /// "chrome", as .NET's Process.ProcessName gives it.
    pub fn name(&self) -> &str {
        self.exe.strip_suffix(".exe").or_else(|| self.exe.strip_suffix(".EXE")).unwrap_or(&self.exe)
    }
}

pub fn list() -> Vec<Proc> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                out.push(Proc { pid: e.th32ProcessID, exe: String::from_utf16_lossy(&e.szExeFile[..len]) });
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Distinct process names without ".exe".
pub fn names() -> Vec<String> {
    let mut v: Vec<String> = list().iter().map(|p| p.name().to_string()).collect();
    v.sort_by_key(|s| s.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

/// The full path of a running process's exe, when this account may read it.
pub fn image_path(pid: u32) -> Option<String> {
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows::core::PWSTR;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sees_itself() {
        let me = std::process::id();
        assert!(super::list().iter().any(|p| p.pid == me));
        assert!(super::image_path(me).is_some());
    }
}
