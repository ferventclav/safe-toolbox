//! Checking every source at once, the exact command each item runs, and apps whose installer would close them.

use std::sync::LazyLock;

use regex::Regex;

use super::dev::{ClaudeCodeSource, NodeSource, PythonSource, RustSource};
use super::model::{SourceResult, UpdateItem, UpdateSource};
use super::sources::{ChocolateySource, ScoopSource};
use super::{nvidia, windows_update, winget};
use crate::proc::Cancel;

pub fn check(source: UpdateSource, cancel: &Cancel) -> SourceResult {
    let mut r = match source {
        UpdateSource::Apps => winget::check(cancel),
        UpdateSource::Chocolatey => ChocolateySource::check(cancel),
        UpdateSource::Scoop => ScoopSource::check(cancel),
        UpdateSource::Windows => windows_update::check(cancel),
        UpdateSource::Nvidia => nvidia::check(cancel),
        UpdateSource::Python => PythonSource::check(cancel),
        UpdateSource::Node => NodeSource::check(cancel),
        UpdateSource::Rust => RustSource::check(cancel),
        UpdateSource::ClaudeCode => ClaudeCodeSource::check(cancel),
    };
    r.source = Some(source);
    r
}

/// Checks every source at once; `done` hears about each as it finishes (any thread), in finishing order.
pub fn check_all(cancel: &Cancel, done: &(dyn Fn(SourceResult) + Sync)) {
    std::thread::scope(|s| {
        for source in UpdateSource::ALL {
            s.spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(source, cancel)))
                    .unwrap_or_else(|_| SourceResult::failed(source, "The check stopped with an internal error."));
                done(r);
            });
        }
    });
}

/// The exact command each item runs, shown in the details pane so nothing is hidden.
pub fn describe_command(i: &UpdateItem) -> String {
    let quote = |a: &String| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() };
    match i.source {
        UpdateSource::Apps => format!("winget {}", winget::upgrade_args(i, false).iter().map(quote).collect::<Vec<_>>().join(" ")),
        UpdateSource::Chocolatey => format!("choco {}", ChocolateySource::upgrade_args(i).join(" ")),
        UpdateSource::Scoop => format!("scoop update {}", i.id),
        UpdateSource::Windows => "None here. Windows Update installs it (Settings, Windows Update).".into(),
        UpdateSource::Nvidia => "Open the NVIDIA app and install it there".into(),
        UpdateSource::Python => format!(
            "python -m pip install{} --upgrade {}=={}",
            if i.scope.as_deref() == Some("user") { " --user" } else { "" },
            i.id,
            i.available
        ),
        UpdateSource::Node => format!("npm install -g {}@{}", i.id, i.available),
        UpdateSource::Rust if i.id == "rustup" => "rustup self update".into(),
        UpdateSource::Rust => format!("rustup update {}", i.id),
        UpdateSource::ClaudeCode => "claude update".into(),
    }
}

static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z][A-Za-z0-9]+").unwrap());
const IGNORE: [&str; 10] = ["microsoft", "windows", "nvidia", "google", "desktop", "runtime", "edition", "shared", "framework", "tools"];
const ALIASES: [(&str, &str); 2] = [("libreoffice", "soffice"), ("steelseries", "SteelSeriesGG")];

/// Open apps whose installer is about to run, so their windows can be saved and closed first.
pub fn running_apps<'a>(items: impl IntoIterator<Item = &'a UpdateItem>, running: &[String]) -> Vec<String> {
    let mut open: Vec<String> = Vec::new();
    for i in items.into_iter().filter(|i| i.source == UpdateSource::Apps) {
        for word in TOKEN.find_iter(&i.name).map(|m| m.as_str()).filter(|w| w.len() >= 4 && !IGNORE.iter().any(|x| x.eq_ignore_ascii_case(w))) {
            let probe = ALIASES.iter().find(|(k, _)| k.eq_ignore_ascii_case(word)).map(|(_, v)| *v).unwrap_or(word).to_lowercase();
            if running.iter().any(|p| p.to_lowercase().starts_with(&probe)) {
                if !open.contains(&i.name) {
                    open.push(i.name.clone());
                }
                break;
            }
        }
    }
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_shown_exactly() {
        let mut i = UpdateItem::new(UpdateSource::Python, "numpy", "numpy").versions("2.1", "2.2");
        i.scope = Some("user".into());
        assert_eq!(describe_command(&i), "python -m pip install --user --upgrade numpy==2.2");
        let r = UpdateItem::new(UpdateSource::Rust, "rustup", "rustup");
        assert_eq!(describe_command(&r), "rustup self update");
        let a = UpdateItem::new(UpdateSource::Apps, "Git.Git", "Git");
        assert!(describe_command(&a).starts_with("winget upgrade --id Git.Git -e --silent"));
    }

    #[test]
    fn finds_open_apps() {
        let items = [
            UpdateItem::new(UpdateSource::Apps, "TheDocumentFoundation.LibreOffice", "LibreOffice 25.8"),
            UpdateItem::new(UpdateSource::Apps, "Microsoft.Teams", "Microsoft Teams"),
            UpdateItem::new(UpdateSource::Apps, "Google.Chrome", "Google Chrome"),
        ];
        let running = vec!["soffice".to_string(), "explorer".to_string()];
        assert_eq!(running_apps(&items, &running), ["LibreOffice 25.8"]);
        assert!(running_apps(&items, &["chrome".to_string()]).contains(&"Google Chrome".to_string()));
    }
}
