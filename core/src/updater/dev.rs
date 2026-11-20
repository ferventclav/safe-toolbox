//! Developer tools: pip packages, global npm packages, rustup toolchains and the Claude Code command.

use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::model::{SourceResult, UpdateItem, UpdateSource};
use super::sources::str_of;
use super::{storage, versions};
use crate::proc::{self, Cancel, minutes};
use crate::tools;

pub fn normalize(name: &str) -> String {
    name.to_lowercase().replace(['_', '.'], "-")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outdated {
    pub name: String,
    pub version: String,
    pub latest: String,
    pub blockers: Vec<String>,
    pub scope: String,
    pub writable: bool,
    pub installer: String,
    pub location: String,
}

/// What the check script found: the packages, and whether another tool manages this Python.
#[derive(Clone, Debug)]
pub struct Report {
    pub managed_by: Option<String>,
    pub items: Vec<Outdated>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub package: String,
    pub requirement: String,
    pub have: String,
    pub have_version: String,
}

/// Lists outdated packages of this interpreter with where each lives (user site or the interpreter's own
/// site-packages), whether that folder is writable, and the installed packages whose rules exclude the
/// newest version (kokoro-onnx pinned phonemizer-fork==3.3.1, so 3.3.2 would break it).
/// It also reports when conda or the OS manages this Python (PEP 668), where pip must not change anything.
pub const OUTDATED_SCRIPT: &str = r#"import json, os, site, subprocess, sys, sysconfig, tempfile

def norm(n):
    return n.lower().replace("_", "-").replace(".", "-")

def writable(path):
    try:
        with tempfile.TemporaryFile(dir=path):
            return True
    except Exception:
        return False

managed = None
if os.path.isdir(os.path.join(sys.prefix, "conda-meta")):
    managed = "conda"
else:
    try:
        if os.path.isfile(os.path.join(sysconfig.get_path("stdlib"), "EXTERNALLY-MANAGED")):
            managed = "the operating system"
    except Exception:
        pass

out = subprocess.run([sys.executable, "-m", "pip", "list", "--outdated", "--format=json", "-v",
                      "--disable-pip-version-check"], capture_output=True, text=True, encoding="utf-8")
# -v also prints notes such as "Link requires a different Python" on stdout, so only the JSON line is read.
found = [l for l in (out.stdout or "").splitlines() if l.startswith("[")]
try:
    items = json.loads(found[-1])
except (IndexError, ValueError):
    print(json.dumps({"error": (out.stderr or "").strip()[-300:] or "pip did not print its list"}))
    sys.exit(0)

user_site = ""
try:
    if site.ENABLE_USER_SITE:
        user_site = os.path.normcase(os.path.abspath(site.getusersitepackages()))
except Exception:
    pass

Version = None
reqs = []
try:
    from importlib import metadata
    from packaging.requirements import Requirement
    from packaging.version import Version
    for d in metadata.distributions():
        who = d.metadata["Name"]
        for r in d.requires or []:
            try:
                q = Requirement(r)
            except Exception:
                continue
            if q.marker is not None:
                try:
                    if not q.marker.evaluate({"extra": ""}):
                        continue
                except Exception:
                    continue
            if str(q.specifier):
                reqs.append((who, q))
except Exception:
    Version = None

res = []
can_write = {}
for it in items:
    if it.get("editable_project_location"):
        continue  # a source checkout; pip install would replace it with the PyPI release
    loc = it.get("location") or ""
    key = os.path.normcase(os.path.abspath(loc)) if loc else ""
    if key not in can_write:
        can_write[key] = writable(loc) if loc else False
    blockers = set()
    if Version is not None:
        try:
            latest = Version(it["latest_version"])
            for who, q in reqs:
                if norm(q.name) == norm(it["name"]) and not q.specifier.contains(latest, prereleases=True):
                    blockers.add(f"{who} needs {q.name}{q.specifier}")
        except Exception:
            pass
    res.append({"name": it["name"], "version": it["version"], "latest": it["latest_version"],
                "scope": "user" if user_site and key.startswith(user_site) else "env",
                "writable": can_write[key], "installer": it.get("installer") or "", "location": loc,
                "blockers": sorted(blockers)})
print(json.dumps({"managedBy": managed, "items": res}))
"#;

/// Every installed package by scope, for the snapshot taken before a pip update. Editable installs are left out.
pub const SNAPSHOT_SCRIPT: &str = r#"import json, os, site, subprocess, sys
out = subprocess.run([sys.executable, "-m", "pip", "list", "--format=json", "-v", "--disable-pip-version-check"],
                     capture_output=True, text=True, encoding="utf-8")
user = ""
try:
    if site.ENABLE_USER_SITE:
        user = os.path.normcase(os.path.abspath(site.getusersitepackages()))
except Exception:
    pass
res = {"user": [], "env": []}
for p in json.loads(next((l for l in reversed((out.stdout or "").splitlines()) if l.startswith("[")), "[]")):
    if p.get("editable_project_location"):
        continue
    loc = os.path.normcase(os.path.abspath(p.get("location") or ""))
    res["user" if user and loc.startswith(user) else "env"].append(p["name"] + "==" + p["version"])
print(json.dumps(res))
"#;

pub struct PythonSource;

impl PythonSource {
    pub fn check(cancel: &Cancel) -> SourceResult {
        let Some(py) = tools::python() else { return SourceResult::absent(UpdateSource::Python) };
        let script = storage::root().join("outdated.py");
        let _ = std::fs::create_dir_all(storage::root());
        if std::fs::write(&script, OUTDATED_SCRIPT).is_err() {
            return SourceResult::failed(UpdateSource::Python, "pip could not check: the check script couldn't be written");
        }
        let r = proc::run(&py, [script.as_os_str()], minutes(4), None, Some(cancel));
        let Some(report) = Self::parse_outdated(&r.output) else {
            return SourceResult::failed(UpdateSource::Python, format!("pip could not check: {}", r.last_line().unwrap_or_default()));
        };
        let items = report
            .items
            .iter()
            .map(|o| {
                let mut i = UpdateItem::new(UpdateSource::Python, &o.name, &o.name).versions(&o.version, &o.latest);
                i.is_major = versions::is_major_jump(&o.version, &o.latest);
                i.blockers = o.blockers.clone();
                i.scope = Some(o.scope.clone());
                i.unsafe_reason = Self::unsafe_reason(report.managed_by.as_deref(), o);
                i
            })
            .collect();
        SourceResult::items(UpdateSource::Python, items, "All Python packages are up to date")
    }

    /// Why pip must not touch this package on this PC, or None when it may.
    pub fn unsafe_reason(managed_by: Option<&str>, o: &Outdated) -> Option<String> {
        if let Some(m) = managed_by {
            let tool = if m == "conda" { "conda" } else { "the system's package manager" };
            return Some(format!("This Python is managed by {m}. Changing its packages with pip can break it, so update them with {tool} instead."));
        }
        if o.installer.eq_ignore_ascii_case("conda") {
            return Some("Installed by conda. Changing it with pip can break the conda environment.".into());
        }
        if !o.writable {
            return Some(format!("Installed for all users in {}, which needs admin to change. Update it from an admin terminal if you need to.", o.location));
        }
        None
    }

    pub fn parse_outdated(output: &str) -> Option<Report> {
        let json = proc::last_json_line(output, true)?;
        let root: Value = serde_json::from_str(json).ok()?;
        if root.get("error").is_some() {
            return None;
        }
        let managed_by = root.get("managedBy").and_then(Value::as_str).map(str::to_string);
        let list = if root.is_array() { &root } else { root.get("items")? };
        let mut items = Vec::new();
        for e in list.as_array()? {
            items.push(Outdated {
                name: e.get("name")?.as_str()?.to_string(),
                version: e.get("version")?.as_str()?.to_string(),
                latest: e.get("latest")?.as_str()?.to_string(),
                blockers: e
                    .get("blockers")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect())
                    .unwrap_or_default(),
                scope: e.get("scope").and_then(Value::as_str).unwrap_or("user").to_string(),
                writable: e.get("writable") != Some(&Value::Bool(false)),
                installer: str_of(e, "installer"),
                location: str_of(e, "location"),
            });
        }
        Some(Report { managed_by, items })
    }

    pub fn parse_snapshot(output: &str) -> Option<(Vec<String>, Vec<String>)> {
        let json = output.split('\n').map(str::trim).filter(|l| l.starts_with('{')).last()?;
        let doc: Value = serde_json::from_str(json).ok()?;
        let read = |k: &str| -> Option<Vec<String>> {
            Some(doc.get(k)?.as_array()?.iter().filter_map(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string).collect())
        };
        Some((read("user")?, read("env")?))
    }

    /// Packages "pip install --dry-run --report -" would install, as (name, version).
    pub fn parse_plan(output: &str) -> Option<Vec<(String, String)>> {
        let start = output.find('{')?;
        let end = output.rfind('}')?;
        if end < start {
            return None;
        }
        let doc: Value = serde_json::from_str(&output[start..=end]).ok()?;
        doc.get("install")?
            .as_array()?
            .iter()
            .map(|e| {
                let m = e.get("metadata")?;
                Some((m.get("name")?.as_str()?.to_string(), m.get("version")?.as_str()?.to_string()))
            })
            .collect()
    }

    /// Changes in a pip install plan that nobody picked and that could break things: a major jump or a downgrade of
    /// an installed package pulled in as a dependency. New packages and routine updates of dependencies are fine.
    pub fn risky_plan_changes(plan: &[(String, String)], picked: &HashSet<String>, installed: &BTreeMap<String, (String, String)>) -> Vec<String> {
        let mut risky = Vec::new();
        for (name, version) in plan {
            let key = normalize(name);
            if picked.contains(&key) {
                continue;
            }
            let Some((old_name, old_version)) = installed.get(&key) else { continue };
            if versions::is_major_jump(old_version, version) || versions::compare(version, old_version).is_lt() {
                risky.push(format!("{old_name} {old_version} → {version}"));
            }
        }
        risky
    }

    /// Lines like "kokoro-onnx 0.4.7 has requirement phonemizer-fork==3.3.1, but you have phonemizer-fork 3.3.2."
    pub fn parse_pip_check(output: &str) -> Vec<Conflict> {
        static LINE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^(?<pkg>\S+) \S+ has requirement (?<req>[^,]+), but you have (?<have>\S+) (?<hv>\S+?)\.?$").unwrap());
        output
            .split('\n')
            .filter_map(|l| LINE.captures(l.trim()))
            .map(|m| Conflict {
                package: m["pkg"].to_string(),
                requirement: m["req"].trim().to_string(),
                have: m["have"].to_string(),
                have_version: m["hv"].to_string(),
            })
            .collect()
    }

    /// "name==version" lines, keyed by normalised name.
    pub fn parse_freeze(text: &str) -> BTreeMap<String, (String, String)> {
        let mut map = BTreeMap::new();
        for line in text.split('\n').map(str::trim) {
            if let Some(i) = line.find("==") {
                if i > 0 && !line.starts_with('#') {
                    let name = &line[..i];
                    map.insert(normalize(name), (name.to_string(), line[i + 2..].to_string()));
                }
            }
        }
        map
    }
}

/// Global npm packages.
pub struct NodeSource;

impl NodeSource {
    pub fn check(cancel: &Cancel) -> SourceResult {
        let Some(npm) = tools::npm() else { return SourceResult::absent(UpdateSource::Node) };
        // npm exits with 1 when something is outdated, so the exit code says nothing about success.
        let r = proc::run(&npm, ["outdated", "-g", "--json"], minutes(3), None, Some(cancel));
        let Some(mut items) = Self::parse_outdated(&r.output) else {
            return SourceResult::failed(UpdateSource::Node, format!("npm could not check: {}", r.last_line().unwrap_or_default()));
        };
        // On some PCs the global folder is under Program Files; updating there needs an admin terminal.
        let prefix = proc::run(&npm, ["prefix", "-g"], minutes(1), None, Some(cancel)).last_line().unwrap_or_default().trim().to_string();
        if !items.is_empty() && !prefix.is_empty() && std::path::Path::new(&prefix).is_dir() && !tools::can_write(std::path::Path::new(&prefix)) {
            for i in &mut items {
                i.unsafe_reason = Some(format!("npm's global folder ({prefix}) needs admin on this PC. Update it from an admin terminal with npm install -g {}.", i.id));
            }
        }
        SourceResult::items(UpdateSource::Node, items, "All global npm packages are up to date")
    }

    pub fn parse_outdated(output: &str) -> Option<Vec<UpdateItem>> {
        let (Some(start), Some(end)) = (output.find('{'), output.rfind('}')) else {
            return output.trim().is_empty().then(Vec::new);
        };
        if end < start {
            return output.trim().is_empty().then(Vec::new);
        }
        let doc: Value = serde_json::from_str(&output[start..=end]).ok()?;
        let mut items = Vec::new();
        for (name, v) in doc.as_object()? {
            let current = str_of(v, "current");
            let latest = str_of(v, "latest");
            if latest.is_empty() || current == latest {
                continue;
            }
            let mut i = UpdateItem::new(UpdateSource::Node, name, name).versions(&current, &latest);
            i.is_major = versions::is_major_jump(&current, &latest);
            items.push(i);
        }
        Some(items)
    }
}

/// rustup and its toolchains.
pub struct RustSource;

impl RustSource {
    pub fn check(cancel: &Cancel) -> SourceResult {
        let Some(rustup) = tools::rustup() else { return SourceResult::absent(UpdateSource::Rust) };
        let r = proc::run(&rustup, ["check"], minutes(2), None, Some(cancel));
        SourceResult::items(UpdateSource::Rust, Self::parse_check(&r.output), "Rust is up to date")
    }

    pub fn parse_check(output: &str) -> Vec<UpdateItem> {
        static LINE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(?<name>\S+) - [Uu]pdate available\s*:\s*(?<cur>\d[\w.\-]*)(?:\s*\([^)]*\))?\s*->\s*(?<new>\d[\w.\-]*)").unwrap()
        });
        let mut items: Vec<UpdateItem> = output
            .split('\n')
            .filter_map(|l| LINE.captures(l.trim()))
            .map(|m| {
                let name = &m["name"];
                let title = if name == "rustup" {
                    "rustup (the installer)".to_string()
                } else {
                    format!("Rust {} toolchain", name.split('-').next().unwrap_or(name))
                };
                UpdateItem::new(UpdateSource::Rust, name, title).versions(&m["cur"], &m["new"])
            })
            .collect();
        items.sort_by_key(|i| if i.id == "rustup" { 0 } else { 1 });
        items
    }
}

/// The Claude Code command-line tool (the desktop app carries its own copy and updates itself).
pub struct ClaudeCodeSource;

impl ClaudeCodeSource {
    pub fn check(cancel: &Cancel) -> SourceResult {
        let Some(claude) = tools::claude() else { return SourceResult::absent(UpdateSource::ClaudeCode) };
        let current = Self::parse_version(&proc::run(&claude, ["--version"], minutes(1), None, Some(cancel)).output);
        let latest = tools::npm()
            .and_then(|npm| Self::parse_version(&proc::run(&npm, ["view", "@anthropic-ai/claude-code", "version"], minutes(1), None, Some(cancel)).output));
        match (current, latest) {
            (None, _) => SourceResult::message(UpdateSource::ClaudeCode, "Couldn't read the Claude Code version"),
            (Some(c), None) => SourceResult::message(UpdateSource::ClaudeCode, format!("Claude Code {c} (couldn't reach npm to compare)")),
            (Some(c), Some(l)) if versions::compare(&l, &c).is_le() => {
                SourceResult::message(UpdateSource::ClaudeCode, format!("Claude Code {c} is the newest"))
            }
            (Some(c), Some(l)) => SourceResult {
                source: Some(UpdateSource::ClaudeCode),
                items: vec![UpdateItem::new(UpdateSource::ClaudeCode, "claude-code", "Claude Code").versions(c, l)],
                ..Default::default()
            },
        }
    }

    pub fn parse_version(output: &str) -> Option<String> {
        static SEMVER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+\.\d+\.\d+)").unwrap());
        SEMVER.captures(output).map(|c| c[1].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_report() {
        let out = "warning\n{\"managedBy\": null, \"items\": [{\"name\": \"av\", \"version\": \"14.0\", \"latest\": \"19.0\", \"scope\": \"user\", \"writable\": true, \"installer\": \"pip\", \"location\": \"C:\\\\x\", \"blockers\": [\"faster-whisper needs av<15\"]}]}";
        let r = PythonSource::parse_outdated(out).unwrap();
        assert!(r.managed_by.is_none());
        assert_eq!(r.items[0].blockers, ["faster-whisper needs av<15"]);
        assert!(PythonSource::parse_outdated("{\"error\": \"boom\"}").is_none());
        assert!(PythonSource::parse_outdated("nothing").is_none());
    }

    #[test]
    fn pip_unsafe_reasons() {
        let o = Outdated { name: "x".into(), version: "1".into(), latest: "2".into(), blockers: vec![], scope: "env".into(), writable: false, installer: "pip".into(), location: r"C:\Program Files\Python".into() };
        assert!(PythonSource::unsafe_reason(Some("conda"), &o).unwrap().contains("with conda instead"));
        assert!(PythonSource::unsafe_reason(None, &o).unwrap().contains("needs admin"));
        let mut ok = o.clone();
        ok.writable = true;
        assert!(PythonSource::unsafe_reason(None, &ok).is_none());
        ok.installer = "conda".into();
        assert!(PythonSource::unsafe_reason(None, &ok).unwrap().starts_with("Installed by conda"));
    }

    #[test]
    fn pip_plan_and_risk() {
        let plan = PythonSource::parse_plan(r#"noise {"install": [{"metadata": {"name": "av", "version": "19.0"}}, {"metadata": {"name": "numpy", "version": "1.26.4"}}, {"metadata": {"name": "new-pkg", "version": "1.0"}}]} tail"#).unwrap();
        let mut installed = BTreeMap::new();
        installed.insert("av".to_string(), ("av".to_string(), "14.0".to_string()));
        installed.insert("numpy".to_string(), ("numpy".to_string(), "2.1.0".to_string()));
        let picked: HashSet<String> = ["faster-whisper".to_string()].into();
        let risky = PythonSource::risky_plan_changes(&plan, &picked, &installed);
        assert_eq!(risky, ["av 14.0 → 19.0", "numpy 2.1.0 → 1.26.4"]);
    }

    #[test]
    fn pip_check_and_freeze() {
        let c = PythonSource::parse_pip_check("kokoro-onnx 0.4.7 has requirement phonemizer-fork==3.3.1, but you have phonemizer-fork 3.3.2.\nNo broken requirements.");
        assert_eq!(c, [Conflict { package: "kokoro-onnx".into(), requirement: "phonemizer-fork==3.3.1".into(), have: "phonemizer-fork".into(), have_version: "3.3.2".into() }]);
        let f = PythonSource::parse_freeze("# comment\nFaster_Whisper==1.1.0\n\nnumpy==2.1.0\n");
        assert_eq!(f["faster-whisper"], ("Faster_Whisper".to_string(), "1.1.0".to_string()));
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn pip_snapshot() {
        let (u, e) = PythonSource::parse_snapshot("{\"user\": [\"a==1\"], \"env\": [\"b==2\", \"\"]}").unwrap();
        assert_eq!(u, ["a==1"]);
        assert_eq!(e, ["b==2"]);
    }

    #[test]
    fn npm_outdated() {
        let items = NodeSource::parse_outdated(r#"{"npm": {"current": "10.0.0", "latest": "11.1.0"}, "same": {"current": "1.0.0", "latest": "1.0.0"}}"#).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].is_major);
        assert_eq!(NodeSource::parse_outdated("").unwrap().len(), 0);
        assert!(NodeSource::parse_outdated("npm ERR! boom").is_none());
    }

    #[test]
    fn rustup_check() {
        let items = RustSource::parse_check("stable-x86_64-pc-windows-msvc - Update available : 1.99.0 (abc 2026-10-01) -> 1.100.0 (def 2026-11-09)\nrustup - Update available : 1.29.2 -> 1.29.3\nnightly - Up to date : 1.101.0");
        assert_eq!(items[0].id, "rustup");
        assert_eq!(items[1].name, "Rust stable toolchain");
        assert_eq!((items[1].current.as_str(), items[1].available.as_str()), ("1.99.0", "1.100.0"));
    }

    #[test]
    fn claude_version() {
        assert_eq!(ClaudeCodeSource::parse_version("2.1.31 (Claude Code)").as_deref(), Some("2.1.31"));
        assert_eq!(ClaudeCodeSource::parse_version("nope"), None);
    }
}
