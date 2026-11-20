//! Saves the list of what is installed, for a new PC or after a reset: winget apps (winget export, the format
//! winget import reads), plus pip user packages, global npm packages and VS Code extensions as plain lists.
//! Nothing is installed here; restoring runs winget import in a window you can watch.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::proc::{Cancel, run};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPart {
    pub name: String,
    pub file: String,
    pub count: usize,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub folder: String,
    pub parts: Vec<ExportPart>,
    pub not_in_winget: Vec<String>,
}

static UNAVAILABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)^\s*Installed package is not available from any source\s*:?\s*(.+)$").unwrap());

fn write_lines(file: &Path, lines: &[String]) -> bool {
    std::fs::write(file, lines.iter().map(|l| format!("{l}\r\n")).collect::<String>()).is_ok()
}

pub fn export(folder: &str, include_versions: bool, cancel: Option<&Cancel>) -> Result<ExportResult, String> {
    let dir = Path::new(folder);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut parts = Vec::new();
    let mut missing = Vec::new();
    let mins = |m: u64| Duration::from_secs(m * 60);

    let winget = crate::tools::winget();
    if Path::new(&winget).exists() {
        let file = dir.join("winget-apps.json");
        let f = file.to_string_lossy().into_owned();
        let mut args = vec!["export", "-o", &f, "--accept-source-agreements", "--disable-interactivity", "--nowarn"];
        if include_versions {
            args.push("--include-versions");
        }
        let r = run(&winget, &args, mins(5), None, cancel);
        let count = std::fs::read_to_string(&file).map(|t| count_winget_packages(&t)).unwrap_or(0);
        // --nowarn hides the "not available" lines, so run once more without it only to list them.
        let probe = super::jobs().join("winget-probe.json");
        let warn = run(&winget, ["export", "-o", &probe.to_string_lossy(), "--accept-source-agreements", "--disable-interactivity"], mins(5), None, cancel);
        missing.extend(parse_unavailable(&warn.output));
        let _ = std::fs::remove_file(probe);
        let note = (!r.ok()).then(|| format!("winget reported a problem: {}", r.last_line().unwrap_or_default()));
        parts.push(ExportPart { name: "Apps (winget)".into(), file: f, count, note });
    }

    if let Some(py) = crate::tools::python() {
        let r = run(&py, ["-m", "pip", "freeze", "--user", "--disable-pip-version-check"], mins(2), None, cancel);
        if r.ok() {
            let lines: Vec<String> = r.lines().into_iter().filter(|l| l.contains("==")).map(str::to_string).collect();
            let file = dir.join("pip-user-requirements.txt");
            write_lines(&file, &lines);
            parts.push(ExportPart { name: "Python packages (pip --user)".into(), file: file.to_string_lossy().into_owned(), count: lines.len(), note: Some("Restore with: python -m pip install --user -r pip-user-requirements.txt".into()) });
        }
    }

    if let Some(npm) = crate::tools::npm() {
        let r = run(&npm, ["ls", "-g", "--depth=0", "--json"], mins(2), None, cancel);
        let pkgs = parse_npm_global(&r.output);
        if !pkgs.is_empty() {
            let file = dir.join("npm-global.txt");
            write_lines(&file, &pkgs);
            parts.push(ExportPart { name: "Node packages (npm -g)".into(), file: file.to_string_lossy().into_owned(), count: pkgs.len(), note: Some("Restore with: npm install -g (each line)".into()) });
        }
    }

    if let Some(code) = crate::tools::find("code.cmd", false) {
        let r = run(&code, ["--list-extensions"], mins(2), None, cancel);
        let ext: Vec<String> = r.lines().into_iter().filter(|l| l.contains('.') && !l.contains(' ')).map(str::to_string).collect();
        if r.ok() && !ext.is_empty() {
            let file = dir.join("vscode-extensions.txt");
            write_lines(&file, &ext);
            parts.push(ExportPart { name: "VS Code extensions".into(), file: file.to_string_lossy().into_owned(), count: ext.len(), note: Some("Restore with: code --install-extension (each line)".into()) });
        }
    }

    let _ = std::fs::write(dir.join("README.txt"), readme(&parts, &missing));
    Ok(ExportResult { folder: folder.into(), parts, not_in_winget: missing })
}

fn packages(json: &str) -> Vec<Value> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    v.get("Sources").and_then(Value::as_array).into_iter().flatten().filter_map(|s| s.get("Packages").and_then(Value::as_array)).flatten().cloned().collect()
}

pub fn count_winget_packages(json: &str) -> usize {
    packages(json).len()
}

/// Package ids from a winget export file, for showing what an import would install.
pub fn winget_ids(json: &str) -> Vec<String> {
    packages(json).iter().filter_map(|p| p.get("PackageIdentifier").and_then(Value::as_str).map(str::to_string)).collect()
}

pub fn parse_unavailable(output: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in UNAVAILABLE.captures_iter(output) {
        let s = c[1].trim().to_string();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

pub fn parse_npm_global(json: &str) -> Vec<String> {
    let Some(deps) = serde_json::from_str::<Value>(json).ok().and_then(|v| v.get("dependencies").and_then(Value::as_object).cloned()) else { return Vec::new() };
    deps.iter()
        .filter(|(name, _)| *name != "npm" && *name != "corepack")
        .map(|(name, v)| match v.get("version").and_then(Value::as_str) {
            Some(ver) => format!("{name}@{ver}"),
            None => name.clone(),
        })
        .collect()
}

/// Opens a terminal window that runs winget import on the file, so every prompt is visible.
pub fn start_import(winget_json: &str) -> bool {
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let cmd = crate::tools::system_dir().join("cmd.exe");
    let winget = crate::tools::winget();
    std::process::Command::new(cmd)
        .raw_arg(format!("/k \"\"{winget}\" import -i \"{winget_json}\" --ignore-unavailable --accept-package-agreements --accept-source-agreements\""))
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .is_ok()
}

fn readme(parts: &[ExportPart], missing: &[String]) -> String {
    let mut s = format!("App list saved by Safe Toolbox on {}.\r\n\r\n", chrono::Local::now().format("%-d %b %Y, %H:%M"));
    for p in parts {
        let name = Path::new(&p.file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        s += &format!("{name}: {}, {} items.{}\r\n", p.name, p.count, p.note.as_ref().map(|n| format!(" {n}")).unwrap_or_default());
    }
    s += "\r\nApps: open Safe Toolbox, App list, Restore, and pick winget-apps.json. Or run:\r\n  winget import -i winget-apps.json --ignore-unavailable\r\n";
    if !missing.is_empty() {
        s += "\r\nNot in winget, so reinstall these by hand:\r\n";
        for m in missing {
            s += &format!("  {m}\r\n");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_files_parse() {
        let export = r#"{"$schema":"https://aka.ms/winget-packages.schema.2.0.json","Sources":[{"Packages":[{"PackageIdentifier":"Git.Git"},{"PackageIdentifier":"Ollama.Ollama"}],"SourceDetails":{"Name":"winget"}},{"Packages":[{"PackageIdentifier":"9NBLGGH4NNS1"}],"SourceDetails":{"Name":"msstore"}}]}"#;
        assert_eq!(count_winget_packages(export), 3);
        assert_eq!(winget_ids(export), ["Git.Git", "Ollama.Ollama", "9NBLGGH4NNS1"]);
        assert_eq!(count_winget_packages("{}"), 0);
        let out = "Installed package is not available from any source: Ubisoft Connect\r\nInstalled package is not available from any source: FL Studio 2026\r\nExported package requires license agreement to install: Something\r\n";
        assert_eq!(parse_unavailable(out), ["Ubisoft Connect", "FL Studio 2026"]);
        let npm = r#"{"dependencies":{"npm":{"version":"11.0.0"},"docx":{"version":"9.15.2"},"@anthropic-ai/claude-code":{"version":"2.1.317"}}}"#;
        let mut got = parse_npm_global(npm);
        got.sort();
        assert_eq!(got, ["@anthropic-ai/claude-code@2.1.317", "docx@9.15.2"]);
    }
}
