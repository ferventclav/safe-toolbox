//! build_rules() and the finders from the cleaner's reference, in the same order, plus the app cache tables.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use regex::Regex;

use super::paths::{self, K, VersionKey, children, days_since, expand, file_name, g, glob, idle_days, is_dir, is_file, join, mtime_of, same_path, tree_size, tree_stats, under};
use super::{Action, Item, Rule, RuleGroup, ScanContext, Skip, skip, system};

fn f(func: fn(&ScanContext, &mut Vec<Skip>) -> Vec<Item>) -> super::Finder {
    Arc::new(func)
}

pub fn build() -> Vec<Rule> {
    use RuleGroup::*;
    let mut rules = vec![
        Rule::new("nvidia", Leftovers, "NVIDIA driver downloads", "Copies of drivers that are already installed, plus GeForce NOW installer leftovers.", f(nvidia)),
        Rule::new("updaters", Leftovers, "App update installers", "Installers apps downloaded for updates. They fetch a fresh one when needed.", f(updaters)),
        Rule::new("old-versions", Leftovers, "Old app versions", "Previous versions Discord, Wand and similar apps keep after updating.", f(squirrel)),
        Rule::new("claude-versions", Leftovers, "Old Claude Code versions", "Earlier Claude Code builds. The newest and any running one stay.", f(claude_versions)),
        Rule::new("codex", Leftovers, "Old Codex runtimes", "Superseded Codex runtimes, an older CLI release and Codex's temp folder.", f(codex)),
        Rule::new("pkg-caches", Leftovers, "Package download caches", "npm, pip, uv, NuGet and Comfy Desktop download caches. They refill on the next install.", f(pkg_caches)),
    ];
    rules.extend(APPS.iter().map(app_rule));
    rules.extend(LAUNCHERS.iter().map(launcher_rule));
    rules.extend([
        Rule::new("temp", Leftovers, "Old temp files", "Temp files, crash dumps and error reports untouched for a week, plus old agent and LibreOffice scratch.", f(temp_files)),
        Rule::new("thumbnails", Leftovers, "Thumbnail cache", "Picture previews Explorer saved. It redraws them as you browse.", f(thumbnails)),
        Rule::new("inet-cache", Leftovers, "Internet cache files", "Windows' own web cache, used by older apps and Office.", f(inet_cache)),
        Rule::new("partial", Leftovers, "Unfinished downloads", "Partial downloads (.crdownload, .part) left in Downloads or Desktop for 2+ days.", f(partial)),
        Rule::new("node-modules", Leftovers, "Idle node_modules", "Dependencies of projects untouched for 30 days. npm install brings them back.", f(node_modules)),
        Rule::new("recycle", YourCall, "Recycle Bin", "Empties it for good.", f(recycle)),
        Rule::new("shader-cache", YourCall, "DirectX shader cache", "Games rebuild it, but stutter for a while after.", f(shader_cache)),
        Rule::new("memory-dump", YourCall, "Blue-screen memory dump", "Full crash dump. Keep it while you are still fixing crashes.", f(memory_dump)).admin(),
        Rule::new("gradle", YourCall, "Gradle caches", "Libraries and build outputs Gradle downloaded. The next build downloads them again, so it is slower once.", f(gradle))
            .busy_when(&["studio64.exe", "idea64.exe"], vec![], "Android Studio or IntelliJ is open, close it to clear this".into()),
        Rule::new("win-temp", Windows, "Windows temp and update leftovers", "Windows Temp, old Windows Update downloads and error reports.", f(win_temp)).admin(),
        Rule::new("delivery-opt", Windows, "Delivery Optimization cache", "Update pieces Windows keeps to share with other PCs.", f(delivery_opt)).admin(),
        Rule::new("win-logs", Windows, "Old Windows log files", "Logs Windows wrote more than a week ago, plus its system temp folder. Folders stay.", f(win_logs)).admin(),
        Rule::new("win-diag", Windows, "Windows diagnostic leftovers", "Feedback Hub archives, old setup logs, Defender's non-critical files, ChkDsk fragments.", f(win_diag)).admin(),
        Rule::new("win-update", Windows, "Windows Update cleanup", "Superseded update files and setup leftovers, removed by Windows' own Disk Cleanup. Size shows after the run.", f(win_update)).admin(),
        Rule::new("old-drivers", Windows, "Old driver versions", "Older copies of drivers a newer version replaced. You can't roll those drivers back after.", f(old_drivers)).unchecked().admin(),
        Rule::new("win-old", Windows, "Previous Windows installation", "Lets you go back to your old Windows for 10 days after an upgrade.", f(win_old)).unchecked().admin(),
        Rule::new("win-esd", Windows, "Windows installation files", "Needed for Reset this PC without a download. Keep unless space is desperate.", f(win_esd)).unchecked().admin(),
        Rule::new(
            "wu-reset-backups",
            Windows,
            "Old Windows Update folders",
            "Copies a Windows Update reset kept (SoftwareDistribution and catroot2 with .bak or .old names). Safe once updates work again.",
            f(update_reset_backups),
        )
        .unchecked()
        .admin(),
    ]);
    rules
}

static LEADING_VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\d.]+").unwrap());
static VERSION_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+(\.\d+)+$").unwrap());

// ------------------------------------------------------------------ leftovers

fn updaters(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    g(&[&K.local, "*-updater"]).into_iter().filter(|d| is_dir(d)).map(Item::path).collect()
}

fn nvidia(ctx: &ScanContext, sk: &mut Vec<Skip>) -> Vec<Item> {
    let mut out = Vec::new();
    // NVIDIA app: installers in grd\<id>\<version>-....exe, unpacked copies in grd\post-processing\<id>
    let grd = join(&K.program_data, r"NVIDIA Corporation\NVIDIA App\UpdateFramework\ota-artifacts\grd");
    let pp = join(&grd, "post-processing");
    if is_dir(&grd) {
        let inst = ctx.nvidia_driver();
        for d in g(&[&grd, "*"]) {
            if !is_dir(&d) || same_path(&d, &pp) {
                continue;
            }
            let ver = g(&[&d, "*.exe"]).first().and_then(|exe| {
                let name = file_name(exe);
                (name.starts_with(|c: char| c.is_ascii_digit())).then(|| LEADING_VERSION.find(name).map(|m| VersionKey::parse(m.as_str()))).flatten()
            });
            match (&inst, ver) {
                (Some(inst), Some(ver)) if ver <= *inst => {
                    out.push(Item::labelled(&d, format!("Driver {ver} installer (installed: {inst})")));
                    let unpacked = join(&pp, file_name(&d));
                    if is_dir(&unpacked) {
                        out.push(Item::labelled(unpacked, "  its unpacked copy"));
                    }
                }
                (Some(_), Some(_)) => sk.push(skip(&d, "newer than the installed driver, may be a pending update")),
                _ => sk.push(skip(&d, "could not compare with the installed driver")),
            }
        }
        for d in g(&[&pp, "*"]) {
            if is_dir(&d) && !is_dir(&join(&grd, file_name(&d))) && idle_days(&d, false) > 7.0 {
                out.push(Item::labelled(d, "Unpacked driver with no installer left"));
            }
        }
    }
    // A driver downloaded from nvidia.com unpacks itself to C:\NVIDIA\DisplayDriver\<version> and leaves the copy
    // behind. The files are only used during setup; running the installer again recreates them.
    let manual = format!(r"{}NVIDIA\DisplayDriver", K.system_drive);
    if is_dir(&manual) {
        let inst = ctx.nvidia_driver();
        for d in g(&[&manual, "*"]) {
            let name = file_name(&d);
            if !is_dir(&d) || !VERSION_NAME.is_match(name) {
                continue;
            }
            let ver = VersionKey::parse(name);
            match &inst {
                None => sk.push(skip(&d, "could not compare with the installed driver")),
                Some(i) if ver > *i => sk.push(skip(&d, "newer than the installed driver, may be a pending update")),
                Some(i) if idle_days(&d, false) > 1.0 => out.push(Item::labelled(&d, format!("Unpacked driver {ver} from a manual install (installed: {i})"))),
                _ => {}
            }
        }
    }
    for n in ["GeforceNowInstallerFiles", "GeForceNOW_tempNew"] {
        let d = join(&K.local, &format!(r"NVIDIA Corporation\{n}"));
        if is_dir(&d) && idle_days(&d, false) > 3.0 {
            out.push(Item::path(d));
        }
    }
    out
}

/// Old app-x.y.z versions and update packages kept by Squirrel apps (Discord, Wand...).
fn squirrel(ctx: &ScanContext, sk: &mut Vec<Skip>) -> Vec<Item> {
    let mut out = Vec::new();
    for upd in g(&[&K.local, "*", "Update.exe"]) {
        let Some(root) = paths::parent(&upd).map(str::to_string) else { continue };
        let root_name = file_name(&root).to_string();
        let mut apps: Vec<String> = g(&[&root, "app-*"]).into_iter().filter(|d| is_dir(d)).collect();
        apps.sort_by_key(|d| VersionKey::parse(file_name(d)));
        let old: Vec<&String> = apps.iter().take(apps.len().saturating_sub(1)).collect();
        let pkgs: Vec<String> = g(&[&root, "packages", "*.nupkg"]).into_iter().filter(|p| idle_days(p, false) > 3.0).collect();
        if old.is_empty() && pkgs.is_empty() {
            continue;
        }
        if ctx.running(&[], std::slice::from_ref(&root)) {
            sk.push(skip(&root, format!("{root_name} is running")));
            continue;
        }
        let current = file_name(apps.last().map(String::as_str).unwrap_or("")).to_string();
        for d in old {
            out.push(Item::labelled(d, format!("{root_name} {} (current: {current})", file_name(d))));
        }
        for p in pkgs {
            out.push(Item::labelled(&p, format!("{root_name} update package {}", file_name(&p))));
        }
    }
    out
}

/// Version-named entries in `base` except the newest and `keep_extra`.
pub fn old_versions(base: &str, keep_extra: &[String]) -> Vec<String> {
    let vers: Vec<String> = g(&[base, "*"]).into_iter().filter(|d| VERSION_NAME.is_match(file_name(d))).collect();
    if vers.len() < 2 {
        return Vec::new();
    }
    let newest = vers.iter().max_by_key(|d| VersionKey::parse(file_name(d))).cloned().unwrap_or_default();
    vers.into_iter().filter(|d| *d != newest && !keep_extra.iter().any(|k| k == file_name(d))).collect()
}

fn claude_versions(ctx: &ScanContext, sk: &mut Vec<Skip>) -> Vec<Item> {
    let mut out = Vec::new();
    let mut bases = g(&[&K.local, "Claude-Data", "*", "claude-code"]);
    bases.extend(g(&[&K.local, "Claude-Data", "*", "claude-code-vm"]));
    for b in bases {
        let sdk = std::fs::read_to_string(join(&b, ".sdk-version")).map(|s| s.trim().to_string()).unwrap_or_default();
        for d in old_versions(&b, &[sdk]) {
            if ctx.running(&[], std::slice::from_ref(&d)) {
                sk.push(skip(&d, "this version is running"));
            } else {
                out.push(Item::labelled(&d, format!("Claude Code {} (desktop app copy)", file_name(&d))));
            }
        }
    }
    let cli = join(&K.home, r".local\share\claude\versions");
    for v in old_versions(&cli, &[]) {
        if idle_days(&v, false) > 7.0 {
            out.push(Item::labelled(&v, format!("Claude CLI {}", file_name(&v))));
        }
    }
    out
}

fn codex(ctx: &ScanContext, sk: &mut Vec<Skip>) -> Vec<Item> {
    let mut out = Vec::new();
    for d in g(&[&K.home, ".cache", "codex-runtimes", "*"]) {
        let n = file_name(&d);
        if n.contains(".previous-") || (n.starts_with("codex-runtime-install-") && idle_days(&d, false) > 1.0) {
            out.push(Item::path(d));
        }
    }
    let rel = join(&K.home, r".codex\packages\standalone\releases");
    let cur = join(&K.home, r".codex\packages\standalone\current");
    // Only when "current" is a link can the live release be told apart.
    if paths::exists(&cur) && paths::is_link(&cur) {
        if let Ok(live) = std::fs::canonicalize(&cur) {
            let live = live.to_string_lossy().trim_start_matches(r"\\?\").to_string();
            for d in g(&[&rel, "*"]) {
                if !same_path(&d, &live) {
                    out.push(Item::labelled(&d, format!("Codex CLI {}", file_name(&d))));
                }
            }
        }
    }
    let tmp = join(&K.home, r".codex\.tmp");
    if is_dir(&tmp) {
        if ctx.running(&["codex.exe"], &[]) {
            sk.push(skip(&tmp, "Codex is running"));
        } else {
            out.push(Item::path(tmp));
        }
    }
    out
}

fn pkg_caches(ctx: &ScanContext, sk: &mut Vec<Skip>) -> Vec<Item> {
    let l = &K.local;
    let mut out: Vec<Item> = [r"npm-cache\_cacache", r"npm-cache\_logs", r"pip\Cache", r"Yarn\Cache"].iter().map(|p| join(l, p)).filter(|p| is_dir(p)).map(Item::path).collect();
    out.extend(g(&[l, "npm-cache", "_npx", "*"]).into_iter().filter(|d| idle_days(d, false) > 14.0).map(Item::path));
    let uv = join(l, r"uv\cache");
    if is_dir(&uv) {
        if ctx.running(&["uv.exe"], &[]) { sk.push(skip(&uv, "uv is running")) } else { out.push(Item::path(uv)) }
    }
    let comfy = join(l, r"Comfy-Desktop\ComfyUI-Cache\download-cache");
    if is_dir(&comfy) && idle_days(&comfy, false) > 3.0 {
        out.push(Item::labelled(comfy, "Comfy Desktop download cache"));
    }
    // NuGet's HTTP cache only ("dotnet nuget locals http-cache --clear"); the global packages folder stays.
    for p in [join(l, r"NuGet\v3-cache"), join(l, r"NuGet\plugins-cache")] {
        if is_dir(&p) {
            if ctx.running(&["dotnet.exe", "msbuild.exe", "devenv.exe"], &[]) {
                sk.push(skip(&p, "dotnet or Visual Studio is running"));
            } else {
                out.push(Item::labelled(p, "NuGet download cache"));
            }
        }
    }
    out
}

/// ~\.gradle\caches, only when no Gradle daemon is using it (its lock files change while one runs).
fn gradle(_: &ScanContext, sk: &mut Vec<Skip>) -> Vec<Item> {
    let caches = join(&K.home, r".gradle\caches");
    if !is_dir(&caches) {
        return Vec::new();
    }
    if idle_days(&caches, true) < 1.0 {
        sk.push(skip(&caches, "used in the last day, so a build may be running"));
        return Vec::new();
    }
    vec![Item::path(caches)]
}

/// The folders "Reset Windows Update" renames (.bak-date or .old). Their contents go; the empty folder stays.
fn update_reset_backups(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    reset_backup_dirs().iter().flat_map(|d| g(&[d, "*"])).map(Item::path).collect()
}

pub fn reset_backup_dirs() -> Vec<String> {
    let w = &K.windows;
    let mut v = g(&[w, "SoftwareDistribution.bak-*"]);
    v.extend(g(&[w, "SoftwareDistribution.old"]));
    v.extend(g(&[w, "System32", "catroot2.bak-*"]));
    v.extend(g(&[w, "System32", "catroot2.old"]));
    v
}

fn short(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn rel(base: &str, p: &str) -> String {
    p.get(base.trim_end_matches('\\').len() + 1..).unwrap_or(p).to_string()
}

fn temp_files(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    let mut out = Vec::new();
    for e in g(&[&K.temp, "*"]) {
        if file_name(&e).eq_ignore_ascii_case("claude") || idle_days(&e, false) <= 7.0 {
            continue;
        }
        // A folder counts as idle only when nothing inside it changed for 7 days either.
        let (size, newest) = tree_stats(&e);
        if days_since(newest) > 7.0 {
            out.push(Item::sized(e, size));
        }
    }
    for d in g(&[&K.temp, "claude", "*", "*"]) {
        if is_dir(&d) && idle_days(&d, true) > 7.0 {
            out.push(Item::labelled(&d, format!("old agent session scratch {}", short(file_name(&d), 8))));
        }
    }
    for d in g(&[&K.drive_temp, "lo_*"]) {
        if idle_days(&d, false) > 1.0 {
            out.push(Item::labelled(&d, format!("LibreOffice scratch profile {}", file_name(&d))));
        }
    }
    // winget's own logs (one per command); a week is plenty for troubleshooting
    for f in g(&[&K.local, "Packages", "Microsoft.DesktopAppInstaller_8wekyb3d8bbwe", "LocalState", "DiagOutputDir", "*.log"]) {
        if idle_days(&f, false) > 7.0 {
            out.push(Item::labelled(&f, format!("winget log {}", file_name(&f))));
        }
    }
    out.extend(g(&[&K.local, "CrashDumps", "*"]).into_iter().filter(|f| idle_days(f, false) > 1.0).map(Item::path));
    // the user's own Windows Error Reporting folders (ReportArchive, ReportQueue)
    for d in g(&[&K.local, "Microsoft", "Windows", "WER", "*", "*"]) {
        if idle_days(&d, true) > 7.0 {
            out.push(Item::labelled(&d, format!("error report {}", short(file_name(&d), 40))));
        }
    }
    out
}

fn thumbnails(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    g(&[&K.local, "Microsoft", "Windows", "Explorer", "thumbcache_*.db"]).into_iter().map(|f| Item::labelled(&f, format!("Explorer {}", file_name(&f)))).collect()
}

/// Only the IE cache buckets, never Content.Outlook (it can hold the only copy of an edited attachment).
fn inet_cache(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    let b = join(&K.local, r"Microsoft\Windows\INetCache");
    let mut v = g(&[&b, "IE", "*"]);
    v.extend(g(&[&b, "Low", "IE", "*"]));
    v.into_iter().filter(|d| is_dir(d)).map(|d| Item::labelled(&d, format!("Internet cache {}", rel(&b, &d)))).collect()
}

const WALK_SKIP: [&str; 5] = [".git", "node_modules", "AppData", ".venv", "venv"];

/// Every folder below the roots, never through links, not descending into WALK_SKIP names.
fn walk_dirs(roots: &[String], visit: &mut dyn FnMut(&str)) {
    for root in roots {
        let mut stack = vec![root.clone()];
        while let Some(d) = stack.pop() {
            for (p, m) in children(&d, true).unwrap_or_default() {
                if !m.is_dir() {
                    continue;
                }
                visit(&p);
                if !WALK_SKIP.contains(&file_name(&p)) {
                    stack.push(p);
                }
            }
        }
    }
}

fn project_roots() -> Vec<String> {
    ["Downloads", "Documents", "Desktop", "source"].iter().map(|n| join(&K.home, n)).collect()
}

fn node_modules(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    let codex_docs = join(&K.home, r"Documents\Codex");
    let mut out = Vec::new();
    walk_dirs(&project_roots(), &mut |e| {
        if file_name(e) != "node_modules" || under(e, &codex_docs) {
            return;
        }
        let Some(parent) = paths::parent(e) else { return };
        if paths::parent(parent).map(file_name) == Some("node_modules") || !is_file(&join(parent, "package.json")) {
            return;
        }
        let mut newest = mtime_of(e).max(mtime_of(&join(e, ".package-lock.json")));
        match children(parent, true) {
            Some(kids) => {
                for (_, m) in kids.iter().filter(|(_, m)| !m.is_dir()) {
                    newest = newest.max(m.modified().unwrap_or_else(|_| std::time::SystemTime::now()));
                }
            }
            None => newest = std::time::SystemTime::now(),
        }
        if days_since(newest) > 30.0 {
            out.push(Item::labelled(e, rel(&K.home, e)));
        }
    });
    out
}

const PARTIAL_ENDS: [&str; 4] = [".crdownload", ".part", ".partial", ".opdownload"];

fn partial(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    let mut out = Vec::new();
    for root in [K.downloads(), K.desktop()] {
        let mut stack = vec![root];
        while let Some(d) = stack.pop() {
            for (p, m) in children(&d, true).unwrap_or_default() {
                let name = file_name(&p).to_lowercase();
                if m.is_dir() {
                    if name != "node_modules" {
                        stack.push(p);
                    }
                } else if PARTIAL_ENDS.iter().any(|x| name.ends_with(x)) && idle_days(&p, false) > 2.0 {
                    out.push(Item::labelled(&p, rel(&K.home, &p)));
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------ your call

fn recycle(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    match system::query_recycle_bin() {
        Some((bytes, items)) if items > 0 => vec![Item::action(format!("Recycle Bin ({items} items)"), Action::EmptyRecycleBin, Some(bytes))],
        _ => Vec::new(),
    }
}

fn shader_cache(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    g(&[&K.local, "D3DSCache", "*"]).into_iter().map(Item::path).collect()
}

/// Full memory dumps directly in C:\Windows. The Minidump folder stays protected.
fn memory_dump(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    g(&[&K.windows, "*.dmp"])
        .into_iter()
        .filter(|p| is_file(p))
        .map(|p| {
            let when: chrono::DateTime<chrono::Local> = mtime_of(&p).into();
            Item::labelled(&p, format!("{} from {}", file_name(&p), when.format("%-d %b %Y")))
        })
        .collect()
}

// ------------------------------------------------------------------ windows (admin)

fn win_temp(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    let (w, pd) = (&K.windows, &K.program_data);
    let mut out = Vec::new();
    for (base, days, deep) in [
        (join(w, "Temp"), 7.0, true),
        (join(w, r"SoftwareDistribution\Download"), 10.0, false),
        (join(pd, r"Microsoft\Windows\WER\ReportArchive"), 7.0, false),
        (join(pd, r"Microsoft\Windows\WER\ReportQueue"), 7.0, false),
    ] {
        for e in g(&[&base, "*"]) {
            if idle_days(&e, false) > days && (!deep || idle_days(&e, true) > days) {
                out.push(Item::path(e));
            }
        }
    }
    out
}

fn delivery_opt(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    let d = join(&K.windows, r"ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache");
    let size = tree_size(&d);
    if size > 0 { vec![Item::action("Delivery Optimization cache", Action::DeliveryOptimization, Some(size))] } else { Vec::new() }
}

/// Files under `base` untouched for `days`. Folders stay, so the services that write them still find them.
pub fn old_files(base: &str, days: f64, skip_dirs: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![base.to_string()];
    while let Some(d) = stack.pop() {
        for (p, m) in children(&d, true).unwrap_or_default() {
            if m.is_dir() {
                if !skip_dirs.iter().any(|s| s.eq_ignore_ascii_case(file_name(&p))) {
                    stack.push(p);
                }
            } else if days_since(m.modified().unwrap_or_else(|_| std::time::SystemTime::now())) > days {
                out.push(p);
            }
        }
    }
    out
}

fn win_logs(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    let (w, pd) = (&K.windows, &K.program_data);
    let mut out = Vec::new();
    // Measured-boot logs back TPM health checks and SystemRestore logs explain restore points: kept to be safe.
    for b in [join(w, "Logs"), join(w, r"System32\LogFiles")] {
        out.extend(old_files(&b, 7.0, &["MeasuredBoot", "SystemRestore"]).into_iter().map(|p| Item::labelled(&p, rel(w, &p))));
    }
    for b in [join(w, "SystemTemp"), join(pd, r"Microsoft\WDF"), join(pd, r"Microsoft\Windows\HiveUpload")] {
        out.extend(g(&[&b, "*"]).into_iter().filter(|e| idle_days(e, true) > 7.0).map(Item::path));
    }
    out
}

fn win_diag(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    let (w, pd) = (&K.windows, &K.program_data);
    let mut out = Vec::new();
    for b in [r"Microsoft\Diagnosis\FeedbackArchive", r"Microsoft\Diagnosis\EventTranscript", r"Microsoft\Windows\RetailDemo"] {
        out.extend(g(&[&join(pd, b), "*"]).into_iter().map(Item::path));
    }
    // Defender's non-critical files only; Quarantine is never an allowed root
    let defender = join(pd, r"Microsoft\Windows Defender");
    for b in ["LocalCopy", "Support"] {
        out.extend(old_files(&join(&defender, b), 7.0, &[]).into_iter().map(|p| Item::labelled(&p, format!("Defender {}", rel(&defender, &p)))));
    }
    for pat in ["setup*.log", "setup*.old", "setuplog.txt", "winnt32.log"] {
        out.extend(g(&[w, pat]).into_iter().filter(|p| is_file(p) && idle_days(p, false) > 30.0).map(Item::path));
    }
    for d in super::guard::found_dirs() {
        out.extend(g(&[&d, "*.CHK"]).into_iter().map(|f| Item::labelled(&f, format!("old ChkDsk fragment {}", rel(&K.system_drive, &f)))));
    }
    out
}

fn win_update(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    system::UPDATE_HANDLERS
        .iter()
        .filter(|(name, _)| system::handler_exists(name))
        .map(|(name, what)| Item::handler(format!("{what} ({name})"), *name, None))
        .collect()
}

fn win_old(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    let d = format!("{}Windows.old", K.system_drive);
    if is_dir(&d) && system::handler_exists(system::PREVIOUS_INSTALLATIONS) {
        vec![Item::handler("Windows.old, your previous Windows", system::PREVIOUS_INSTALLATIONS, Some(tree_size(&d)))]
    } else {
        Vec::new()
    }
}

fn win_esd(_: &ScanContext, _: &mut Vec<Skip>) -> Vec<Item> {
    let ds: Vec<String> = ["ESD", "$Windows.~WS"].iter().map(|n| format!("{}{n}", K.system_drive)).filter(|d| is_dir(d)).collect();
    if !ds.is_empty() && system::handler_exists(system::ESD_FILES) {
        vec![Item::handler("Windows installation files (ESD)", system::ESD_FILES, Some(ds.iter().map(|d| tree_size(d)).sum()))]
    } else {
        Vec::new()
    }
}

fn old_drivers(_: &ScanContext, sk: &mut Vec<Skip>) -> Vec<Item> {
    let packages = match system::list_third_party_drivers() {
        Ok(p) if !p.is_empty() => p,
        Ok(_) => {
            sk.push(skip("driver list", "no third-party driver packages found"));
            return Vec::new();
        }
        Err(e) => {
            sk.push(skip("driver list", format!("could not read the driver list: {e}")));
            return Vec::new();
        }
    };
    let (old, skipped) = system::select_old(&packages, &system::infs_in_use());
    for (p, why) in skipped {
        sk.push(skip(format!("{} {} ({})", p.original_name, p.version, p.published_name), why));
    }
    old.into_iter()
        .map(|(p, newest)| {
            let size = system::driver_package_size(&p.published_name);
            Item::action(format!("{} {} ({}; keeps {newest})", p.original_name, p.version, p.published_name), Action::DeleteDriver(p.published_name), size)
        })
        .collect()
}

// ------------------------------------------------------------------ app caches

/// An app whose web and GPU caches are cleared (label, user-data roots, process names, install dirs).
pub struct App(pub &'static str, pub &'static [&'static str], pub &'static [&'static str], pub &'static [&'static str]);

/// A launcher whose whole cache folder is cleared (label, cache folders, process names).
pub struct Launcher(pub &'static str, pub &'static [&'static str], pub &'static [&'static str]);

pub const APPS: &[App] = &[
    App("Claude app", &[r"{L}\Claude-Data\*", r"{R}\Claude"], &["claude.exe"], &[]),
    App("Chrome", &[r"{L}\Google\Chrome\User Data"], &["chrome.exe"], &[]),
    App("Chrome Beta", &[r"{L}\Google\Chrome Beta\User Data"], &["chrome.exe"], &[]),
    App("Edge", &[r"{L}\Microsoft\Edge\User Data"], &["msedge.exe"], &[]),
    App("Gemini app", &[r"{R}\gemini"], &["gemini.exe", "geminiapplauncher.exe"], &[]),
    App("Notion", &[r"{R}\Notion"], &["notion.exe"], &[r"{L}\Programs\Notion"]),
    App("Discord", &[r"{R}\discord"], &["discord.exe"], &[r"{L}\Discord"]),
    App("Antigravity", &[r"{R}\Antigravity"], &["antigravity.exe"], &[r"{L}\Programs\antigravity"]),
    App("VS Code", &[r"{R}\Code"], &["code.exe"], &[r"{L}\Programs\Microsoft VS Code"]),
    App("Obsidian", &[r"{R}\obsidian"], &["obsidian.exe"], &[r"{L}\Programs\Obsidian"]),
    App("Wand", &[r"{R}\WeMod", r"{R}\Wand"], &["wand.exe", "wemod.exe"], &[r"{L}\Wand"]),
    App("Codex app", &[r"{L}\Packages\OpenAI.Codex_*\LocalCache\Roaming\Codex", r"{R}\Codex"], &["codex.exe"], &[]),
    App("Comfy Desktop", &[r"{R}\Comfy Desktop"], &[], &[r"{L}\Programs\Comfy Desktop"]),
    App("Open Design", &[r"{R}\Open Design"], &[], &[r"{L}\Programs\Open Design"]),
    App("Power BI", &[r"{L}\Microsoft\Power BI Desktop\WebView2"], &["pbidesktop.exe"], &[]),
    // Same signature check, so only folders that look like a Chromium HTTP/code/GPU cache are ever picked.
    App("Brave", &[r"{L}\BraveSoftware\Brave-Browser\User Data"], &["brave.exe"], &[]),
    App("Opera", &[r"{R}\Opera Software\Opera Stable", r"{L}\Opera Software\Opera Stable", r"{R}\Opera Software\Opera GX Stable", r"{L}\Opera Software\Opera GX Stable"], &["opera.exe"], &[]),
    App("Slack", &[r"{R}\Slack", r"{L}\Packages\91750D7E.Slack_*\LocalCache\Roaming\Slack"], &["slack.exe"], &[r"{L}\slack"]),
    App("Teams", &[r"{L}\Packages\MSTeams_8wekyb3d8bbwe\LocalCache\Microsoft\MSTeams"], &["ms-teams.exe"], &[]),
    App("Spotify", &[r"{R}\Spotify", r"{L}\Spotify", r"{L}\Packages\SpotifyAB.SpotifyMusic_*\LocalCache\Spotify"], &["spotify.exe"], &[]),
    App("Cursor", &[r"{R}\Cursor"], &["cursor.exe"], &[r"{L}\Programs\cursor"]),
    App("Windsurf", &[r"{R}\Windsurf"], &["windsurf.exe"], &[r"{L}\Programs\Windsurf"]),
    App("Firefox", &[r"{L}\Mozilla\Firefox\Profiles", r"{L}\Packages\Mozilla.Firefox_*\LocalCache\Local\Mozilla\Firefox\Profiles"], &["firefox.exe"], &[]),
];

pub const LAUNCHERS: &[Launcher] = &[
    Launcher("Steam web cache", &[r"{L}\Steam\htmlcache"], &["steam.exe", "steamwebhelper.exe"]),
    Launcher("Ubisoft Connect cache", &[r"{L}\Ubisoft Game Launcher\cache"], &["upc.exe", "ubisoftconnect.exe", "uplaywebcore.exe"]),
    Launcher("Epic launcher web cache", &[r"{L}\EpicGamesLauncher\Saved\webcache*"], &["epicgameslauncher.exe"]),
    Launcher("Zoom installer downloads", &[r"{R}\Zoom\ZoomDownload"], &["zoom.exe"]),
];

const GPU_CACHE_NAMES: [&str; 6] = ["gpucache", "dawncache", "dawngraphitecache", "dawnwebgpucache", "grshadercache", "shadercache"];

/// Is this folder a browser cache, judged by its name and what is directly inside it?
pub fn is_chromium_cache(dir: &str) -> bool {
    let n = file_name(dir).to_lowercase();
    let Some(kids) = children(dir, false) else { return false };
    let kids: HashSet<String> = kids.iter().map(|(p, _)| file_name(p).to_lowercase()).collect();
    let has = |k: &str| kids.contains(k);
    match n.as_str() {
        "cache" => has("cache_data") || has("index"),
        "code cache" => has("js") || has("wasm") || has("webui_js"),
        "cache2" => has("entries"),
        _ if GPU_CACHE_NAMES.contains(&n.as_str()) => has("index") || has("data_0"),
        _ => false,
    }
}

/// Cache folders under an app's data root, up to `depth` levels down. Never follows links.
pub fn cache_dirs(root: &str, depth: usize) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![(root.to_string(), 0)];
    while let Some((d, lvl)) = stack.pop() {
        for (p, m) in children(&d, true).unwrap_or_default() {
            if !m.is_dir() {
                continue;
            }
            let low = file_name(&p).to_lowercase();
            if low == "optguideondevicemodel" {
                continue;
            }
            if is_chromium_cache(&p) {
                found.push(p);
            } else if low == "service worker" {
                for sub in ["CacheStorage", "ScriptCache"] {
                    let s = join(&p, sub);
                    if is_dir(&s) {
                        found.push(s);
                    }
                }
            } else if lvl < depth {
                stack.push((p, lvl + 1));
            }
        }
    }
    found
}

fn app_rule(app: &App) -> Rule {
    let roots: Vec<String> = app.1.iter().map(|s| s.to_string()).collect();
    let find: super::Finder = Arc::new(move |_, _| {
        roots.iter().flat_map(|pat| glob(&expand(pat))).filter(|r| is_dir(r)).flat_map(|r| cache_dirs(&r, 5)).map(Item::path).collect()
    });
    let dirs = app.3.iter().map(|d| expand(d)).collect();
    Rule::new(&format!("cache-{}", app.0.to_lowercase().replace(' ', "-")), RuleGroup::Apps, app.0, "Web and GPU cache folders", find)
        .busy_when(app.2, dirs, format!("{} is open, close it to clear this", app.0))
}

fn launcher_rule(l: &Launcher) -> Rule {
    let first = l.0.split(' ').next().unwrap_or(l.0);
    let pats: Vec<String> = l.1.iter().map(|s| s.to_string()).collect();
    let find: super::Finder = Arc::new(move |_, _| pats.iter().flat_map(|p| glob(&expand(p))).filter(|p| is_dir(p)).map(Item::path).collect());
    Rule::new(&format!("launcher-{}", first.to_lowercase()), RuleGroup::Apps, l.0, "Launcher cache, rebuilt on next start", find)
        .busy_when(l.2, vec![], format!("{first} is open, close it to clear this"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rule_ids_are_unique_and_ordered() {
        let rules = build();
        let mut ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids[0], "nvidia");
        assert!(ids.contains(&"cache-vs-code") && ids.contains(&"launcher-steam"));
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        assert!(rules.iter().find(|r| r.id == "recycle").is_some_and(|r| !r.default_checked));
        assert!(rules.iter().find(|r| r.id == "old-drivers").is_some_and(|r| r.needs_admin && !r.default_checked));
    }

    #[test]
    fn recognises_chromium_caches_and_keeps_the_newest_version() {
        let t = std::env::temp_dir().join(format!("st-rules-{}", std::process::id())).to_string_lossy().into_owned();
        for d in [r"Default\Cache\Cache_Data", r"Default\Code Cache\js", r"Default\Local Storage", r"Default\Service Worker\CacheStorage", r"v\1.2.3", r"v\1.10.0", r"v\notes"] {
            fs::create_dir_all(join(&t, d)).unwrap();
        }
        let mut found: Vec<String> = cache_dirs(&t, 5).iter().map(|p| rel(&t, p)).collect();
        found.sort();
        assert_eq!(found, [r"Default\Cache", r"Default\Code Cache", r"Default\Service Worker\CacheStorage"]);
        let old: Vec<String> = old_versions(&join(&t, "v"), &[]).iter().map(|p| file_name(p).to_string()).collect();
        assert_eq!(old, ["1.2.3"]);
        let _ = fs::remove_dir_all(&t);
    }
}
