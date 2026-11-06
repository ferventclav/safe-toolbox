using System.Runtime.InteropServices;
using Microsoft.Win32;

namespace SafeToolbox.Core;

public enum PathScope { Machine, User }

public enum PathProblem { None, Missing, Duplicate, Empty, NotAFolder }

public sealed record PathEntry(PathScope Scope, int Index, string Raw, string Expanded, PathProblem Problem, string? DuplicateOf)
{
    public bool Fixable => Scope == PathScope.User && Problem != PathProblem.None;
}

public sealed record CommandHit(string Command, List<string> Locations)
{
    /// <summary>The one that runs when you type the command: the first match in PATH order.</summary>
    public string Runs => Locations[0];
    public bool Shadowed => Locations.Count > 1;
}

public sealed record BrokenLink(string Link, string Target);

/// <summary>
/// Finds PATH entries that point nowhere, repeat an earlier entry or are empty; winget command links whose
/// target was deleted; and which copy of common commands actually runs. It edits only your user PATH, after
/// saving the old value so it can be put back.
/// </summary>
public static class PathDoctor
{
    private const string MachineKey = @"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

    public static readonly string[] CommonCommands =
        ["python", "py", "pip", "node", "npm", "npx", "git", "winget", "code", "ffmpeg", "yt-dlp", "rg", "uv", "cargo", "rustc", "dotnet", "java", "claude"];

    public static string RawPath(PathScope scope)
    {
        using RegistryKey? key = scope == PathScope.User
            ? Registry.CurrentUser.OpenSubKey("Environment")
            : Registry.LocalMachine.OpenSubKey(MachineKey);
        return key?.GetValue("Path", "", RegistryValueOptions.DoNotExpandEnvironmentNames) as string ?? "";
    }

    /// <summary>Both PATHs in the order Windows joins them (machine first, then user), each entry judged.</summary>
    public static List<PathEntry> Analyze() => Analyze(RawPath(PathScope.Machine), RawPath(PathScope.User), Directory.Exists, File.Exists);

    public static List<PathEntry> Analyze(string machine, string user, Func<string, bool> dirExists, Func<string, bool> fileExists)
    {
        var list = new List<PathEntry>();
        var seen = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach ((PathScope scope, string raw) in new[] { (PathScope.Machine, machine), (PathScope.User, user) })
        {
            string[] parts = raw.Split(';');
            for (int i = 0; i < parts.Length; i++)
            {
                string r = parts[i];
                // A trailing ";" is harmless and common; only empty entries in the middle are flagged.
                if (r.Trim().Length == 0)
                {
                    if (i < parts.Length - 1)
                    {
                        list.Add(new PathEntry(scope, i, r, "", PathProblem.Empty, null));
                    }
                    continue;
                }
                string expanded = Environment.ExpandEnvironmentVariables(r.Trim().Trim('"'));
                string key = Normalize(expanded);
                PathProblem problem = PathProblem.None;
                string? dupOf = null;
                if (seen.TryGetValue(key, out string? first))
                {
                    problem = PathProblem.Duplicate;
                    dupOf = first;
                }
                else if (!dirExists(expanded))
                {
                    problem = fileExists(expanded) ? PathProblem.NotAFolder : PathProblem.Missing;
                }
                seen.TryAdd(key, $"{(scope == PathScope.Machine ? "system" : "user")} PATH entry {i + 1}");
                list.Add(new PathEntry(scope, i, r, expanded, problem, dupOf));
            }
        }
        return list;
    }

    /// <summary>The user PATH with the fixable entries removed, keeping order and the rest exactly as written.</summary>
    public static string Cleaned(string userRaw, IEnumerable<PathEntry> entries)
    {
        var drop = entries.Where(e => e.Scope == PathScope.User && e.Problem != PathProblem.None).Select(e => e.Index).ToHashSet();
        return string.Join(';', userRaw.Split(';').Where((p, i) => !drop.Contains(i) && p.Trim().Length > 0));
    }

    /// <summary>Removes the problem entries from your user PATH. Saves the old value first and returns the backup path.</summary>
    public static string FixUserPath(List<PathEntry> entries)
    {
        string raw = RawPath(PathScope.User);
        string backup = Path.Combine(ToolboxPaths.Exports, $"user-path-backup-{DateTime.Now:yyyyMMdd-HHmmss}.txt");
        File.WriteAllText(backup, raw);
        WriteUserPath(Cleaned(raw, entries));
        return backup;
    }

    /// <summary>Puts back a user PATH saved by <see cref="FixUserPath"/>.</summary>
    public static void Restore(string backupFile) => WriteUserPath(File.ReadAllText(backupFile).Trim());

    public static string? LatestBackup() => Directory.GetFiles(ToolboxPaths.Exports, "user-path-backup-*.txt")
        .OrderByDescending(f => f, StringComparer.Ordinal).FirstOrDefault();

    private static void WriteUserPath(string value)
    {
        using RegistryKey env = Registry.CurrentUser.OpenSubKey("Environment", writable: true)
            ?? throw new IOException("Your Environment registry key could not be opened.");
        RegistryValueKind kind = env.GetValueNames().Contains("Path", StringComparer.OrdinalIgnoreCase) ? env.GetValueKind("Path") : RegistryValueKind.ExpandString;
        env.SetValue("Path", value, kind == RegistryValueKind.String ? RegistryValueKind.String : RegistryValueKind.ExpandString);
        Broadcast();
    }

    /// <summary>Where each common command is found, in PATH order, so a shadowed python or node shows up.</summary>
    public static List<CommandHit> Commands(IEnumerable<PathEntry> entries)
    {
        string[] exts = (Environment.GetEnvironmentVariable("PATHEXT") ?? ".COM;.EXE;.BAT;.CMD").Split(';', StringSplitOptions.RemoveEmptyEntries);
        List<string> dirs = entries.Where(e => e.Problem == PathProblem.None).Select(e => e.Expanded).ToList();
        var hits = new List<CommandHit>();
        foreach (string cmd in CommonCommands)
        {
            var found = new List<string>();
            foreach (string d in dirs)
            {
                foreach (string ext in exts)
                {
                    string f = Path.Combine(d, cmd + ext.ToLowerInvariant());
                    if (File.Exists(f))
                    {
                        found.Add(f);
                        break;
                    }
                }
            }
            if (found.Count > 0)
            {
                hits.Add(new CommandHit(cmd, found));
            }
        }
        return hits;
    }

    /// <summary>Command links in WinGet\Links whose target file was deleted (the yt-dlp FFmpeg case of 3 Nov 2026).</summary>
    public static List<BrokenLink> BrokenWingetLinks()
    {
        var list = new List<BrokenLink>();
        string links = SafeUpdater.Core.PathHealth.LinksDir;
        if (!Directory.Exists(links))
        {
            return list;
        }
        foreach (string f in Directory.GetFiles(links))
        {
            string? target = new FileInfo(f).LinkTarget;
            if (target is null)
            {
                continue;
            }
            string full = Path.GetFullPath(target, links);
            if (!File.Exists(full))
            {
                list.Add(new BrokenLink(f, full));
            }
        }
        return list;
    }

    /// <summary>Whether the Windows Store "python" alias (which opens the Store) runs before a real Python.</summary>
    public static bool IsStoreAlias(string path) =>
        path.Contains(@"\Microsoft\WindowsApps\", StringComparison.OrdinalIgnoreCase);

    private static string Normalize(string p)
    {
        try
        {
            return Path.GetFullPath(p).TrimEnd('\\');
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return p.TrimEnd('\\');
        }
    }

    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint msg, UIntPtr wParam, string lParam, uint flags, uint timeout, out UIntPtr result);

    /// <summary>Tells Explorer and new terminals that the environment changed.</summary>
    private static void Broadcast() => SendMessageTimeout((IntPtr)0xffff, 0x001A, UIntPtr.Zero, "Environment", 0x0002, 3000, out _);
}
