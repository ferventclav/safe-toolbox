//! %APPDATA%\SafeDiskCleaner\protect.txt: one folder per line, '#' starts a comment, environment variables are
//! expanded. Anything listed is treated like a protected path.

use std::fs;

use super::paths::{K, join, norm};

pub fn file_path() -> String {
    join(&K.settings_dir, "protect.txt")
}

pub const HEADER: &str = "# Safe Disk Cleaner: folders you want left alone.\r\n#\r\n# A folder listed here is never deleted, nor is anything inside it, nor any folder\r\n# that contains it. One folder per line. Lines starting with # are ignored.\r\n# Environment variables work, for example:\r\n#   %USERPROFILE%\\Games\\Mods\r\n#   %LOCALAPPDATA%\\SomeApp\\Cache\r\n#\r\n# The \"Protect a folder\" button in the app adds lines here for you.\r\n";

/// Creates the file with its comment header if it does not exist yet. Returns its path.
pub fn ensure_file(file: Option<&str>) -> String {
    let f = file.map(str::to_string).unwrap_or_else(file_path);
    if !std::path::Path::new(&f).exists() {
        if let Some(dir) = std::path::Path::new(&f).parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(&f, HEADER);
    }
    f
}

/// The folders listed in the file, expanded. Missing file means none.
pub fn read(file: Option<&str>) -> Vec<String> {
    let f = file.map(str::to_string).unwrap_or_else(file_path);
    fs::read_to_string(f).map(|t| parse(&t)).unwrap_or_default()
}

pub fn parse(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(|s| crate::tools::expand(s.trim_matches(['"', ' '])))
        .filter(|s| s.len() > 3)
        .collect()
}

/// Appends a folder to the file (creating it first). Does nothing if it is already listed.
pub fn add(folder: &str, file: Option<&str>) -> Result<(), String> {
    let f = ensure_file(file);
    let wanted = norm(folder).ok_or("Not a valid folder path.")?;
    if read(Some(&f)).iter().any(|e| norm(e).is_some_and(|n| n.eq_ignore_ascii_case(&wanted))) {
        return Ok(());
    }
    let text = fs::read_to_string(&f).unwrap_or_default();
    let prefix = if !text.is_empty() && !text.ends_with('\n') { "\r\n" } else { "" };
    fs::write(&f, format!("{text}{prefix}{wanted}\r\n")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_adds_once() {
        assert_eq!(parse("# c\r\n\r\n\"C:\\Games\\Mods\"\r\nC:\\\r\n"), [r"C:\Games\Mods"]);
        let f = std::env::temp_dir().join(format!("st-protect-{}.txt", std::process::id())).to_string_lossy().into_owned();
        let _ = fs::remove_file(&f);
        add(r"C:\Games\Mods\", Some(&f)).unwrap();
        add(r"c:\games\mods", Some(&f)).unwrap();
        assert_eq!(read(Some(&f)), [r"C:\Games\Mods"]);
        assert!(fs::read_to_string(&f).unwrap().starts_with("# Safe Disk Cleaner"));
        let _ = fs::remove_file(&f);
    }
}
