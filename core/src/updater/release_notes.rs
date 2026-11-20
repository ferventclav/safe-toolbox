//! Where to read what changed before updating. Most sources have a fixed page per version; winget apps carry a
//! release-notes URL and a homepage in their manifest, read on demand with "winget show".

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde::Serialize;

use super::model::{UpdateItem, UpdateSource};
use crate::proc;
use crate::tools;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NotesLink {
    pub label: String,
    pub url: String,
}

fn link(label: impl Into<String>, url: impl Into<String>) -> NotesLink {
    NotesLink { label: label.into(), url: url.into() }
}

/// Percent-encodes everything but unreserved characters, like Uri.EscapeDataString.
pub fn escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

static KB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)KB(\d{6,8})").unwrap());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)https?://\S+").unwrap());

pub fn for_item(i: &UpdateItem) -> Vec<NotesLink> {
    let v = escape(i.target_version.as_deref().unwrap_or(&i.available));
    let id = escape(&i.id);
    match i.source {
        UpdateSource::Python => vec![link("PyPI page for this version", format!("https://pypi.org/project/{id}/{v}/"))],
        UpdateSource::Node => vec![link("npm page for this version", format!("https://www.npmjs.com/package/{}/v/{v}", i.id))],
        UpdateSource::Chocolatey => vec![link("Chocolatey package page", format!("https://community.chocolatey.org/packages/{id}/{v}"))],
        UpdateSource::Scoop => vec![link("Find it on scoop.sh", format!("https://scoop.sh/#/apps?q={id}"))],
        UpdateSource::Rust if i.id == "rustup" => vec![link("rustup changelog", "https://github.com/rust-lang/rustup/blob/master/CHANGELOG.md")],
        UpdateSource::Rust => vec![link("Rust release notes", "https://doc.rust-lang.org/stable/releases.html")],
        UpdateSource::ClaudeCode => vec![link("Claude Code changelog", "https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md")],
        UpdateSource::Nvidia => vec![link("NVIDIA driver release notes", "https://www.nvidia.com/en-us/geforce/drivers/")],
        UpdateSource::Windows => {
            let text = format!("{} {}", i.detail.as_deref().unwrap_or(""), i.name);
            KB.captures(&text)
                .map(|m| vec![link(format!("Microsoft support article KB{}", &m[1]), format!("https://support.microsoft.com/help/{}", &m[1]))])
                .unwrap_or_default()
        }
        UpdateSource::Apps => {
            let first = i.id.chars().next().map(|c| c.to_ascii_lowercase()).unwrap_or('_');
            vec![link("winget manifest", format!("https://github.com/microsoft/winget-pkgs/tree/master/manifests/{first}/{}", i.id.replace('.', "/")))]
        }
    }
}

/// Release notes and homepage from "winget show" output. The labels are in the display language, so a line
/// counts when its label mentions notes or home in English, else the first URL is used.
pub fn parse_winget_show(output: &str) -> Vec<NotesLink> {
    let (mut notes, mut home) = (None, None);
    let mut urls = Vec::new();
    for raw in output.split('\n') {
        let line = raw.trim();
        let Some(m) = URL.find(line) else { continue };
        let url = m.as_str().trim_end_matches(['.', ',', ')']).to_string();
        let label = line[..m.start()].to_lowercase();
        urls.push(url.clone());
        if label.contains("release notes url") || label.contains("releasenotesurl") {
            notes.get_or_insert(url);
        } else if label.starts_with("homepage") {
            home.get_or_insert(url);
        }
    }
    let mut links = Vec::new();
    if let Some(n) = notes {
        links.push(link("Release notes", n));
    }
    if let Some(h) = home {
        links.push(link("Homepage", h));
    }
    if links.is_empty() {
        if let Some(u) = urls.into_iter().next() {
            links.push(link("Project page", u));
        }
    }
    links
}

pub fn winget(id: &str) -> Vec<NotesLink> {
    if !tools::has_winget() {
        return Vec::new();
    }
    let r = proc::run_simple(&tools::winget(), ["show", "--id", id, "-e", "--accept-source-agreements", "--disable-interactivity"], Duration::from_secs(40));
    if r.ok() { parse_winget_show(&r.output) } else { Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_pages() {
        let mut i = UpdateItem::new(UpdateSource::Python, "faster_whisper", "faster_whisper").versions("1.0", "1.1.0");
        assert_eq!(for_item(&i)[0].url, "https://pypi.org/project/faster_whisper/1.1.0/");
        i.source = UpdateSource::Apps;
        i.id = "Git.Git".into();
        assert_eq!(for_item(&i)[0].url, "https://github.com/microsoft/winget-pkgs/tree/master/manifests/g/Git/Git");
        let mut w = UpdateItem::new(UpdateSource::Windows, "guid", "2026-11 Cumulative Update (KB5132061)");
        assert_eq!(for_item(&w)[0].url, "https://support.microsoft.com/help/5132061");
        w.name = "Some driver".into();
        assert!(for_item(&w).is_empty());
    }

    #[test]
    fn winget_show_links() {
        let out = "Found Git [Git.Git]\nVersion: 2.50.0\nHomepage: https://git-scm.com/\nRelease Notes Url: https://github.com/git-for-windows/git/releases/tag/v2.50.0.windows.1\n";
        let l = parse_winget_show(out);
        assert_eq!(l[0].label, "Release notes");
        assert_eq!(l[1].url, "https://git-scm.com/");
        let other = parse_winget_show("Startseite: https://example.org/x.\n");
        assert_eq!(other, [link("Project page", "https://example.org/x")]);
    }
}
