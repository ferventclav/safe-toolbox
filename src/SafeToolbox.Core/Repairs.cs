using System.Diagnostics;
using System.Text;

namespace SafeToolbox.Core;

public sealed record RepairStep(string Label, string Exe, string Args, int[] OkCodes, Encoding? OutputEncoding = null);

public sealed class Repair
{
    public required string Id { get; init; }
    public required string Title { get; init; }
    public required string What { get; init; }

    /// <summary>What to expect: time, a restart, windows that close.</summary>
    public required string Heads { get; init; }
    public required bool NeedsAdmin { get; init; }
    public bool RestartAfter { get; init; }
    public required IReadOnlyList<RepairStep> Steps { get; init; }
    public string Glyph { get; init; } = "";
}

/// <summary>
/// Fixes for common Windows problems, each a documented Windows command, run one step at a time with the output kept.
/// Order follows Microsoft's guidance for system files: DISM /RestoreHealth first, then sfc /scannow
/// (support.microsoft.com, "Use the System File Checker tool").
/// </summary>
public static class Repairs
{
    private static string Sys(string exe) => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), exe);

    private static string Ps => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "WindowsPowerShell", "v1.0", "powershell.exe");

    private static string PsArgs(string script) => "-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command \"" + script.Replace("\"", "\\\"") + "\"";

    public static IReadOnlyList<Repair> All { get; } =
    [
        new Repair
        {
            Id = "system-files", Title = "Repair Windows system files",
            What = "Checks Windows' own files and replaces any that are damaged, first from Windows Update (DISM), then on the PC (SFC).",
            Heads = "Takes 10 to 30 minutes. Your files and apps are not touched.",
            NeedsAdmin = true, Glyph = "",
            Steps =
            [
                new RepairStep("Repairing the Windows image (DISM)", Sys("dism.exe"), "/Online /Cleanup-Image /RestoreHealth", [0, 3010]),
                new RepairStep("Checking system files (SFC)", Sys("sfc.exe"), "/scannow", [0, 1], Encoding.Unicode),
            ],
        },
        new Repair
        {
            Id = "check-disk", Title = "Check the drive for errors",
            What = "Scans the C: drive's file system while Windows runs (chkdsk /scan) and fixes what it can without a restart.",
            Heads = "Takes a few minutes. If it needs more, it tells you to schedule a check at the next restart.",
            NeedsAdmin = true, Glyph = "",
            Steps = [new RepairStep("Scanning C:", Sys("chkdsk.exe"), "C: /scan", [0, 1, 2])],
        },
        new Repair
        {
            Id = "dns", Title = "Flush the DNS cache",
            What = "Forgets saved website addresses, so a site that moved or a changed hosts file takes effect.",
            Heads = "Instant. One admin prompt: this Windows build refuses it without one.",
            NeedsAdmin = true, Glyph = "",
            Steps = [new RepairStep("Flushing DNS", Sys("ipconfig.exe"), "/flushdns", [0])],
        },
        new Repair
        {
            Id = "network", Title = "Reset network settings",
            What = "Resets Winsock and TCP/IP to their defaults (netsh winsock reset, netsh int ip reset). Fixes many \"connected, no internet\" problems.",
            Heads = "Needs a restart. VPN adapters and static IP settings may need setting up again.",
            NeedsAdmin = true, RestartAfter = true, Glyph = "",
            Steps =
            [
                new RepairStep("Resetting Winsock", Sys("netsh.exe"), "winsock reset", [0, 1]),
                new RepairStep("Resetting TCP/IP", Sys("netsh.exe"), "int ip reset", [0, 1]),
            ],
        },
        new Repair
        {
            Id = "windows-update", Title = "Reset Windows Update",
            What = "Stops the update services, renames the SoftwareDistribution and catroot2 download folders, and starts the services again, the reset Microsoft documents for stuck updates.",
            Heads = "Takes a minute. The old folders are kept with a .bak name, so nothing is lost.",
            NeedsAdmin = true, Glyph = "",
            Steps =
            [
                new RepairStep("Stopping update services", Ps, PsArgs("foreach ($s in 'wuauserv','bits','cryptsvc','usosvc') { Stop-Service -Name $s -Force -ErrorAction SilentlyContinue }; 'stopped'"), [0]),
                new RepairStep("Renaming the download folders", Ps, PsArgs("$t = Get-Date -Format yyyyMMdd-HHmmss; foreach ($p in \"$env:SystemRoot\\SoftwareDistribution\", \"$env:SystemRoot\\System32\\catroot2\") { if (Test-Path $p) { Rename-Item -Path $p -NewName ((Split-Path $p -Leaf) + '.bak-' + $t) -ErrorAction Stop; \"renamed $p\" } }"), [0]),
                new RepairStep("Starting update services", Ps, PsArgs("foreach ($s in 'cryptsvc','bits','wuauserv','usosvc') { Start-Service -Name $s -ErrorAction SilentlyContinue }; 'started'"), [0]),
            ],
        },
        new Repair
        {
            Id = "winget-sources", Title = "Repair winget sources",
            What = "Resets winget's package sources (winget source reset --force), which fixes \"failed when searching source\" errors in Updates.",
            Heads = "Takes under a minute.",
            NeedsAdmin = true, Glyph = "",
            Steps =
            [
                new RepairStep("Resetting sources", SafeUpdater.Core.Tools.Winget(), "source reset --force --disable-interactivity", [0]),
                new RepairStep("Updating sources", SafeUpdater.Core.Tools.Winget(), "source update --disable-interactivity", [0]),
            ],
        },
        new Repair
        {
            Id = "explorer", Title = "Restart File Explorer",
            What = "Closes and reopens Explorer, the taskbar and the Start button. Fixes a frozen taskbar or a missing tray icon.",
            Heads = "Open Explorer windows close. No admin needed.",
            NeedsAdmin = false, Glyph = "",
            Steps = [new RepairStep("Restarting Explorer", Ps, PsArgs("Stop-Process -Name explorer -Force; Start-Sleep -Seconds 2; if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe }; 'restarted'"), [0])],
        },
        new Repair
        {
            Id = "store", Title = "Reset the Microsoft Store cache",
            What = "Clears the Store's cache (wsreset). Fixes Store apps that won't download or update.",
            Heads = "The Store opens when it's done. No admin needed.",
            NeedsAdmin = false, Glyph = "",
            Steps = [new RepairStep("Resetting the Store cache", Sys("wsreset.exe"), "", [0])],
        },
    ];

    public static Repair? Find(string id) => All.FirstOrDefault(r => r.Id == id);

    /// <summary>Runs every step, writing each output line to <paramref name="log"/>. Returns true when every step succeeded.</summary>
    public static async Task<bool> RunAsync(Repair repair, Action<string> log, CancellationToken ct = default)
    {
        bool ok = true;
        foreach (RepairStep step in repair.Steps)
        {
            log($"## {step.Label}");
            if (!File.Exists(step.Exe))
            {
                log($"!! {Path.GetFileName(step.Exe)} was not found on this PC.");
                return false;
            }
            int code = await RunStepAsync(step, log, ct);
            bool stepOk = step.OkCodes.Contains(code);
            log(stepOk ? $"== done (exit {code})" : $"!! failed (exit {code})");
            if (!stepOk)
            {
                ok = false;
                if (repair.Id != "windows-update")
                {
                    break;
                }
            }
        }
        return ok;
    }

    private static async Task<int> RunStepAsync(RepairStep step, Action<string> log, CancellationToken ct)
    {
        var psi = new ProcessStartInfo(step.Exe, step.Args)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        if (step.OutputEncoding is not null)
        {
            psi.StandardOutputEncoding = step.OutputEncoding;
        }
        using var p = new Process { StartInfo = psi, EnableRaisingEvents = true };
        var last = new StringBuilder();
        void Line(string? s)
        {
            if (s is null)
            {
                return;
            }
            // DISM and SFC redraw a progress bar with carriage returns; keep only the final state of each line.
            string clean = s.Split('\r').Select(x => x.Replace("\0", "", StringComparison.Ordinal).Trim()).LastOrDefault(x => x.Length > 0) ?? "";
            if (clean.Length > 0 && clean != last.ToString())
            {
                last.Clear().Append(clean);
                log(clean);
            }
        }
        p.OutputDataReceived += (_, e) => Line(e.Data);
        p.ErrorDataReceived += (_, e) => Line(e.Data);
        p.Start();
        p.BeginOutputReadLine();
        p.BeginErrorReadLine();
        await p.WaitForExitAsync(ct);
        p.WaitForExit();
        return p.ExitCode;
    }

    /// <summary>The elevated side: "SafeToolbox.exe --repair id log". Writes "##EXIT ok|fail" last.</summary>
    public static int RunElevated(string id, string logFile)
    {
        Repair? r = Find(id);
        void Log(string line)
        {
            try
            {
                File.AppendAllText(logFile, line + Environment.NewLine, Encoding.UTF8);
            }
            catch (IOException)
            {
            }
        }
        if (r is null)
        {
            Log("!! unknown repair " + id);
            Log("##EXIT fail");
            return 2;
        }
        bool ok;
        try
        {
            ok = RunAsync(r, Log).GetAwaiter().GetResult();
        }
        catch (Exception ex)
        {
            Log("!! " + ex.Message);
            ok = false;
        }
        Log(ok ? "##EXIT ok" : "##EXIT fail");
        return ok ? 0 : 1;
    }
}
