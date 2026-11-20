//! Opening files, folders and links the way Explorer does, and starting a program as admin.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};
use windows::core::{HSTRING, PCWSTR, w};

/// Opens a file, folder or https link with its default program. False when Windows refused.
pub fn open(target: &str) -> bool {
    let t = HSTRING::from(target);
    let r = unsafe { ShellExecuteW(None, w!("open"), &t, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    r.0 as isize > 32
}

/// Opens Explorer with the file selected.
pub fn reveal(path: &str) -> bool {
    Command::new(crate::tools::windows_dir().join("explorer.exe"))
        .raw_arg(format!("/select,\"{path}\""))
        .spawn()
        .is_ok()
}

/// Starts `exe` elevated with `args`, hidden. None when the admin prompt was declined or failed; the process
/// handle otherwise, so the caller can wait for it.
pub fn run_as_admin(exe: &str, args: &str, dir: &str) -> Result<ElevatedProcess, String> {
    let file = HSTRING::from(exe);
    let params = HSTRING::from(args);
    let directory = HSTRING::from(dir);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: PCWSTR(directory.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) if !info.hProcess.is_invalid() => Ok(ElevatedProcess(info.hProcess)),
        Ok(()) => Err("Windows started nothing".into()),
        Err(e) if e.code().0 as u32 == 0x800704C7 => Err("declined".into()),
        Err(e) => Err(e.message().to_string()),
    }
}

/// A process started with ShellExecuteEx, for waiting on it.
pub struct ElevatedProcess(windows::Win32::Foundation::HANDLE);

unsafe impl Send for ElevatedProcess {}

impl ElevatedProcess {
    pub fn has_exited(&self) -> bool {
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        use windows::Win32::System::Threading::WaitForSingleObject;
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }

    pub fn exit_code(&self) -> Option<u32> {
        use windows::Win32::System::Threading::GetExitCodeProcess;
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.0, &mut code).ok()? };
        Some(code)
    }
}

impl Drop for ElevatedProcess {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Starts a program in its own visible console window (repairs the user watches).
pub fn start_visible(exe: &str, args: &[&str]) -> bool {
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    Command::new(exe).args(args).creation_flags(CREATE_NEW_CONSOLE).spawn().is_ok()
}
