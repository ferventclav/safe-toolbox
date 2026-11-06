# Safe Toolbox

Safe Updater and Safe Disk Cleaner in one Windows app, plus the maintenance tools a developer's laptop needs
now and then: health checks, repairs, a PATH doctor, a big-files view and an app-list backup.

It is one file, `Safe Toolbox.exe`, with no installer and no files beside it.

## What each page does

| Page | What it does | Changes anything? |
| --- | --- | --- |
| Home | One card per tool: safe updates waiting, space to free, health warnings, background checks, recent runs. | No |
| Updates | winget apps, Chocolatey, Scoop, Windows Update and drivers, the NVIDIA driver, pip, npm, Rust and Claude Code, checked together. Updates that could break something are left out, and the list says why. You can skip one version or hide an update for good. | Only what you tick, after you press Update |
| Clean up | Everything Windows Disk Cleanup covers, plus app and browser caches it never sees. Every path goes through the guard first (below). | Only the rows you tick |
| Big files | The largest files in your profile, with a tag for models, videos, archives and installers. | Only a file you choose, and it goes to the Recycle Bin |
| Health | Blue screens with their stop codes, unexpected shutdowns, app crashes, drive health and wear, battery wear and the antivirus Windows Security reports. All read from what Windows already records. A button makes a restore point. | Only if you make a restore point |
| Repair | DISM then SFC, chkdsk /scan, DNS flush, network reset, Windows Update reset, winget source repair, Explorer restart, Store cache reset. Each one is a documented Windows command and you see its output. | Yes, the one you run |
| PATH doctor | Dead and repeated PATH entries, which copy of python, node or git runs, and winget command links an update broke. | Only your user PATH, after a backup you can undo |
| App list | Saves winget apps (as the file `winget import` reads), pip user packages, global npm packages and VS Code extensions to Documents\Safe Toolbox\App lists. Restores apps through winget import in a window you can watch. | Restoring installs apps |
| History | Every update run and every clean, from the logs both tools keep. | No |
| Settings | Theme, accent colour, backdrop, background checks, update and cleanup options. | Settings only |

## What it never does

- Install or delete anything in the background. Background checks only look and notify you.
- Clean the registry. Microsoft does not support registry cleaners (KB 2563254).
- Change Windows services, privacy settings or telemetry.
- Touch documents, pictures, videos, game saves, AI models, `.ssh`, installed tools or synced folders
  without you picking that exact file.
- Run as admin on its own. The window always runs as you. The parts that need admin (Windows Update, the
  Windows cleanup rows, repairs, drive wear) start a separate helper and Windows asks you each time.

## The cleaner's guard

Every path is checked before anything is deleted. A path is refused when it is outside the allowed roots,
inside or containing a protected folder, reached through a junction, holding a PATH entry, a registered
install location or a winget link target, or listed in `%APPDATA%\SafeDiskCleaner\protect.txt`. An open
app's cache is skipped and the row says which app to close. The reasoning for each rule is in
[docs/CLEANER-DECISIONS.md](docs/CLEANER-DECISIONS.md).

## Accent colour

WinUI apps normally paint buttons, ticks and switches in your accent colour whatever Windows is set to.
Safe Toolbox follows the Windows switch "Show accent colour on title bars and window borders": when it is
off, the app uses neutral greys. Settings can also force the accent on or off.

## First launch and disk use

The exe holds everything WinUI needs. On first launch .NET unpacks it once to
`%TEMP%\.net\<exe name>\<id>` (about 220 MB, two seconds). Later launches start from there in about a
second. A new version unpacks to a new folder. When Safe Toolbox starts, it removes its own older
folders, except any a running copy still uses. The cleaner never deletes `%TEMP%\.net`.

## Where it keeps things

| What | Where |
| --- | --- |
| Toolbox settings, background check result, error log | `%LOCALAPPDATA%\SafeToolbox` |
| Update settings, hidden and skipped updates, run logs, pip snapshots | `%LOCALAPPDATA%\SafeUpdater` |
| Folders you protected from cleaning | `%APPDATA%\SafeDiskCleaner\protect.txt` |
| Cleanup run log | `%LOCALAPPDATA%\SafeDiskCleaner` |
| Saved app lists | `Documents\Safe Toolbox\App lists` |

The update and cleaner folders are the ones the separate apps used, so nothing from them is lost.

## Background checks

Off by default. When you turn one on in Settings, a Task Scheduler task named "Safe Toolbox background
check" runs the exe with `--background`, as you and never as admin, daily or weekly around 12:30 (or at the
next chance if the PC was off). It notifies you when safe updates are ready or there is a lot to clean.
Turning both checks off removes the task.

## Command line

```
"Safe Toolbox.exe"                  the window
"Safe Toolbox.exe" --check --json   the background check once, printed as JSON, for scripts and agents
```

The exe also runs as its own elevated helpers (`--worker`, `--clean-worker`, `--repair`, `--admin-task`).
Those are started by the app, not by hand.

## Build

Needs the .NET 10 SDK on Windows.

```
dotnet test SafeToolbox.slnx -p:Platform=x64
dotnet publish src/SafeToolbox -c Release -p:Platform=x64 -o artifacts/publish
```

The publish output is one `SafeToolbox.exe` (about 92 MB, compressed). It can be renamed: the app's resource
index is `resources.pri`, which WinUI finds whatever the exe is called.

## Layout

```
src/SafeUpdater.Core       update sources, safety rules, the elevated update worker
src/SafeDiskCleaner.Core   cleanup rules, the guard, the elevated cleanup worker
src/SafeToolbox.Core       settings, health, repairs, PATH doctor, big files, app list, history, schedule
src/SafeToolbox            the WinUI 3 app
tests/                     one test project per core library
```

## Licence

MIT. See [LICENSE](LICENSE).
