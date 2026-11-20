//! Drive space and size wording shared by every page.

use std::path::Path;

use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::core::HSTRING;

/// (total, free for this user) bytes of the drive holding `path`.
pub fn usage(path: &Path) -> Option<(u64, u64)> {
    let mut free = 0u64;
    let mut total = 0u64;
    let root = HSTRING::from(path.as_os_str());
    unsafe { GetDiskFreeSpaceExW(&root, Some(&mut free), Some(&mut total), None).ok()? };
    Some((total, free))
}

/// (total, free) bytes of the drive Windows runs from.
pub fn system_drive_usage() -> Option<(u64, u64)> {
    usage(Path::new(&format!("{}\\", crate::tools::system_drive())))
}

/// "4.2" or "4" (C#'s "0.#").
pub fn one_decimal(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

const KB: f64 = 1024.0;
const MB: f64 = KB * 1024.0;
const GB: f64 = MB * 1024.0;

/// "1.5 GB", "512 MB", "12 KB" (fmt() from the cleaner's reference).
pub fn format_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{:.0} KB", b / KB)
    }
}

/// format_size, or "?" when the size is unknown.
pub fn format_opt(bytes: Option<u64>) -> String {
    bytes.map(format_size).unwrap_or_else(|| "?".into())
}

/// "556.9 GB" for drive totals, always in GB.
pub fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(one_decimal(4.0), "4");
        assert_eq!(one_decimal(4.25), "4.2");
        assert_eq!(format_size(0), "0 KB");
        assert_eq!(format_opt(None), "?");
        assert_eq!(format_size(12 * 1024), "12 KB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.0 GB");
        assert_eq!(format_size(803 * 1024 * 1024), "803 MB");
        assert_eq!(format_size(1_503_238_553), "1.4 GB");
        assert_eq!(format_gb(598_653_100_032), "557.5 GB");
    }

    #[test]
    fn reads_the_system_drive() {
        let (total, free) = system_drive_usage().unwrap();
        assert!(total > 0 && free <= total);
    }
}
