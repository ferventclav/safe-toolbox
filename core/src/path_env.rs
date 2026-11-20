//! Writing the user PATH back to the registry and telling running programs it changed.

use winreg::enums::RegType;
use winreg::{RegKey, RegValue};

/// Writes HKCU\Environment\Path as REG_EXPAND_SZ (keeps %VAR% entries working) or REG_SZ.
pub fn write_user_path(env: &RegKey, value: &str, expandable: bool) -> std::io::Result<()> {
    let mut bytes: Vec<u8> = value.encode_utf16().chain(std::iter::once(0)).flat_map(|u| u.to_le_bytes()).collect();
    if bytes.is_empty() {
        bytes = vec![0, 0];
    }
    env.set_raw_value("Path", &RegValue { bytes, vtype: if expandable { RegType::REG_EXPAND_SZ } else { RegType::REG_SZ } })
}

/// WM_SETTINGCHANGE "Environment", so new terminals see the new PATH without signing out.
pub fn broadcast_environment_change() {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE};
    let text: Vec<u16> = "Environment".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(text.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            None,
        );
    }
    let _ = HWND::default();
}
