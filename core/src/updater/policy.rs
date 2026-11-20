//! Decides, for every update, one of three things:
//! HIDDEN (unsafe or redundant: not listed, so it can't be ticked, and shown only with its reason),
//! LISTED BUT UNTICKED (safe, but your call), or LISTED AND TICKED (routine).
//! The hide rules run in a fixed order and the first match gives the reason; see README for the table.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use chrono::{Datelike, Local, NaiveDate};
use regex::Regex;

use super::model::{ItemKind, UpdateItem, UpdateSource};
use super::sources::ChocolateySource;
use super::storage::Settings;
use super::versions;
use crate::tools::eq_ic;

pub const HIDDEN_BY_YOU: &str = "Hidden by you";
pub const SKIPPED_BY_YOU: &str = "Version skipped";
const MAJORS_HINT: &str = " Turn on Include major versions to list it.";

/// A driver older than this is hidden whatever the device has (the 2017 Intel chipset driver Windows keeps offering).
const OLD_DRIVER_YEARS: i32 = 3;

/// Runtimes and database servers: a major version can break projects, global packages or data files.
const RUNTIME_WORDS: [&str; 16] = [
    "nodejs", "python", "openjdk", "temurin", "corretto", "zulu", "jdk", "postgresql", "mysql", "mariadb", "mongodb", "ruby", "php",
    "erlang", "elixir", "golang",
];

/// Packages GPU Whisper depends on. They are tested after updating; nothing more.
pub const WHISPER_STACK: [&str; 7] = ["faster-whisper", "ctranslate2", "av", "onnxruntime", "tokenizers", "huggingface-hub", "numpy"];

static NON_ALNUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]").unwrap());

pub fn squash(s: &str) -> String {
    NON_ALNUM.replace_all(&s.to_lowercase(), "").into_owned()
}

/// What the rules need to know besides the item itself.
#[derive(Clone, Debug)]
pub struct PolicyContext {
    pub include_majors: bool,
    /// "Source|Id" keys, compared without case.
    pub ignored: Vec<String>,
    /// Versions you skipped, as "Source|Id" to version.
    pub skipped: BTreeMap<String, String>,
    pub today: NaiveDate,
    /// Packages Chocolatey manages; winget must not update the same apps.
    pub chocolatey_packages: Vec<String>,
    /// rustup is on this PC, so winget's own Rustup entry is redundant.
    pub rustup_present: bool,
}

impl Default for PolicyContext {
    fn default() -> Self {
        PolicyContext {
            include_majors: false,
            ignored: Vec::new(),
            skipped: BTreeMap::new(),
            today: Local::now().date_naive(),
            chocolatey_packages: Vec::new(),
            rustup_present: false,
        }
    }
}

impl PolicyContext {
    pub fn from(s: &Settings, chocolatey: Vec<String>, rustup: bool) -> Self {
        PolicyContext {
            include_majors: s.include_majors,
            ignored: s.ignored.iter().cloned().collect(),
            skipped: s.skipped.clone(),
            chocolatey_packages: chocolatey,
            rustup_present: rustup,
            ..Default::default()
        }
    }

    fn is_ignored(&self, key: &str) -> bool {
        self.ignored.iter().any(|k| eq_ic(k, key))
    }

    fn skipped_version(&self, key: &str) -> Option<&String> {
        self.skipped.iter().find(|(k, _)| eq_ic(k, key)).map(|(_, v)| v)
    }
}

/// A language runtime or database server, judged by its id or name, from any app source.
pub fn is_runtime_name(id: &str, name: &str) -> bool {
    let a = squash(id);
    let b = squash(name);
    RUNTIME_WORDS.iter().any(|w| a.contains(w) || b.contains(w))
}

pub fn is_runtime(i: &UpdateItem) -> bool {
    matches!(i.source, UpdateSource::Apps | UpdateSource::Chocolatey | UpdateSource::Scoop) && is_runtime_name(&i.id, &i.name)
}

/// Applies the rules to every item; some rules look across groups (Chocolatey owning a winget app).
pub fn apply_all(items: &mut [UpdateItem], ctx: &PolicyContext) {
    let claude_listed = items.iter().any(|i| i.source == UpdateSource::ClaudeCode);
    for i in items.iter_mut() {
        apply(i, ctx, claude_listed);
    }
}

pub fn apply(item: &mut UpdateItem, ctx: &PolicyContext, claude_code_listed: bool) {
    item.badge = None;
    item.note = None;
    item.hidden = false;
    item.ignored_by_user = false;

    if let Some((label, reason)) = hide_reason(item, ctx, claude_code_listed) {
        item.hidden = true;
        item.ignored_by_user = label == HIDDEN_BY_YOU || label == SKIPPED_BY_YOU;
        item.badge = Some(label);
        item.note = Some(reason);
        item.recommended = false;
        item.selected = false;
        return;
    }

    let mut tick = true;
    if item.source == UpdateSource::Nvidia {
        tick = false;
        item.badge = Some("In NVIDIA app".into());
        item.note = Some("Install it from the NVIDIA app. Graphics drivers can't be installed from here.".into());
    } else if item.target_version.is_some() {
        tick = !ctx.include_majors;
        item.badge = Some("Same family".into());
        item.note = Some(format!("The newest {} release, listed instead of the new major version.", versions::family(&item.current)));
    } else if item.is_major {
        item.badge = Some("Major".into());
        item.note = Some(if matches!(item.source, UpdateSource::Python | UpdateSource::Node) || is_runtime(item) {
            let mut s = String::from("A new major version, listed because you turned on Include major versions.");
            if item.source == UpdateSource::Python {
                s.push_str(" Your tools are tested afterwards, and it is rolled back if a test that passed before now fails.");
            }
            s
        } else {
            "A new major version of an app. Apps handle their own upgrades, so it is picked.".into()
        });
    } else if item.unknown_version {
        tick = false;
        item.badge = Some("Unknown version".into());
        item.note = Some("winget can't read the installed version, so this reinstalls the newest one. Safe, but apps like this usually update themselves when you open them.".into());
    }

    if item.source == UpdateSource::Windows {
        tick = false;
        let what = if item.kind == ItemKind::Driver {
            item.badge = Some("Driver".into());
            let installed = item.installed_driver_date.map(|d| format!(" (yours is from {})", day(d))).unwrap_or_default();
            format!("A driver update Windows recommends, newer than the one you have{installed}.")
        } else {
            "An update Windows Update offers.".to_string()
        };
        item.note = Some(format!("{what} Windows Update installs it: press Open Windows Update, then Download and install."));
    }

    if item.source == UpdateSource::Python
        && item.note.is_none()
        && WHISPER_STACK.iter().any(|w| eq_ic(w, &super::dev::normalize(&item.id)))
    {
        item.note = Some("GPU Whisper uses this. It is tested after updating and rolled back if Whisper stops working.".into());
    }

    item.recommended = tick && item.selectable;
    item.selected = item.recommended;
}

/// "21 Jun 2006"
pub fn day(d: NaiveDate) -> String {
    d.format("%-d %b %Y").to_string()
}

fn inbox_driver_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2006, 6, 21).unwrap()
}

/// The first rule that makes an update unsafe or redundant, as (short label, reason), or None.
pub fn hide_reason(item: &UpdateItem, ctx: &PolicyContext, claude_code_listed: bool) -> Option<(String, String)> {
    let r = |l: &str, s: String| Some((l.to_string(), s));
    // 1. Your choice.
    if ctx.is_ignored(&item.ignore_key()) {
        return r(HIDDEN_BY_YOU, "You chose not to see this update. Use Show again to bring it back.".into());
    }
    // 1b. You skipped this exact version; a newer one is listed again.
    if let Some(skipped) = ctx.skipped_version(&item.ignore_key()) {
        let offered = item.target_version.as_deref().unwrap_or(&item.available);
        if eq_ic(skipped, offered) {
            return r(SKIPPED_BY_YOU, format!("You skipped {skipped}. The next version after it is listed again."));
        }
    }
    // 2. The source found it can't be done safely on this PC (needs admin, conda-managed, held, failed install).
    if let Some(reason) = &item.unsafe_reason {
        return r("Not safe here", reason.clone());
    }
    // 3. Someone pinned it on purpose.
    if item.pinned {
        return r(
            "Pinned",
            if item.source == UpdateSource::Chocolatey {
                "Pinned in Chocolatey (choco pin), so it stays at this version until the pin is removed.".into()
            } else {
                "Pinned in winget, so it only updates when you ask winget for it.".into()
            },
        );
    }
    // 4. Another tool owns it; updating it here too would update it twice or confuse that tool.
    if item.source == UpdateSource::Apps {
        if let Some(owner) = ChocolateySource::owner_of(item, &ctx.chocolatey_packages) {
            return r(
                "Managed by Chocolatey",
                format!("Chocolatey manages this app (package {owner}). Updating it with winget as well leaves Chocolatey's records wrong, so it updates in the Chocolatey group."),
            );
        }
    }
    if item.source == UpdateSource::Apps && ctx.rustup_present && eq_ic(&item.id, "Rustlang.Rustup") {
        return r("Handled by rustup", "rustup updates itself in the Rust group. winget's record of it is out of date, and updating it through winget could clash with rustup.".into());
    }
    if item.source == UpdateSource::Node && claude_code_listed && eq_ic(&item.id, "@anthropic-ai/claude-code") {
        return r("Handled by Claude Code", "This is Claude Code, which updates in its own group, so it isn't installed twice.".into());
    }
    // 5. Never go backwards.
    if versions::is_downgrade(&item.current, &item.available) {
        return r("Older version", format!("{} is older than the {} you have, so installing it would downgrade.", item.available, item.current));
    }
    // 6. Never move a stable install onto a test release.
    if versions::is_prerelease(&item.available) && !versions::is_prerelease(&item.current) {
        return r("Pre-release", format!("{} is a test release (alpha, beta, release candidate or preview), and you have a stable version.", item.available));
    }
    // 7. Another installed package needs the current version.
    if !item.blockers.is_empty() {
        return r(
            "Would break another package",
            format!("{}. Updating it would break that package, so it waits until that package allows the new version.", item.blockers.join("; ")),
        );
    }
    // 8. Windows Update: optional extras, and drivers that aren't clearly newer than the one on the device.
    if item.source == UpdateSource::Windows {
        if item.kind == ItemKind::WindowsUpdate && item.optional {
            return r("Optional", "An optional update, such as a preview. Windows installs the important updates by itself, and these fixes arrive in a later regular update.".into());
        }
        if item.kind == ItemKind::Driver {
            let Some(offered) = item.driver_date else {
                return r("Can't confirm", "Windows Update didn't say how new this driver is, so it can't be checked against the one you have.".into());
            };
            let cutoff = ctx.today.with_year(ctx.today.year() - OLD_DRIVER_YEARS).unwrap_or(ctx.today);
            if offered < cutoff {
                return r(
                    "Old driver",
                    format!("This driver is from {}. Drivers this old are almost always older than what Windows already uses for the device, so it is left out.", day(offered)),
                );
            }
            // Windows' own in-box drivers all carry the date 21 Jun 2006, which says nothing about how new they are.
            match item.installed_driver_date {
                Some(installed) if installed != inbox_driver_date() => {
                    if offered <= installed {
                        return r(
                            "Not newer",
                            format!(
                                "This driver is dated {}. The one already on the device is from {}, so installing it would replace a newer or equal driver.",
                                day(offered),
                                day(installed)
                            ),
                        );
                    }
                }
                other => {
                    let why = if other.is_none() {
                        "the device it's for wasn't found"
                    } else {
                        "the device uses Windows' own driver, whose 21 Jun 2006 date says nothing about its age"
                    };
                    return r("Can't confirm", format!("This driver is dated {}, but {why}, so it can't be confirmed as newer than the driver you have.", day(offered)));
                }
            }
            // Settings shows optional drivers only under Advanced options, Optional updates, and installs them only on request.
            if item.optional {
                return r("Optional", "Windows marks this driver optional, so Settings lists it only under Windows Update, Advanced options, Optional updates. Install it from there if this device has a problem.".into());
            }
        }
    }
    // 9. Major versions of libraries and runtimes wait for your OK.
    if item.is_major && !ctx.include_majors {
        if matches!(item.source, UpdateSource::Python | UpdateSource::Node) {
            return r(
                "Major version",
                format!("A new major version of a library. It can change how your scripts and tools work (av 19 broke faster-whisper this way).{MAJORS_HINT}"),
            );
        }
        if is_runtime(item) {
            let family = if item.source == UpdateSource::Apps { " When there is a newer release in your current family, that is listed instead." } else { "" };
            return r(
                "Major version",
                format!("A new major version of a runtime or database. Projects, global packages or data files can need changes to work with it.{family}{MAJORS_HINT}"),
            );
        }
    }
    None
}
