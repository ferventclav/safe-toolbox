using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace SafeToolbox.Core;

public sealed record ExportPart(string Name, string File, int Count, string? Note);

public sealed record ExportResult(string Folder, List<ExportPart> Parts, List<string> NotInWinget);

/// <summary>
/// Saves the list of what is installed, for a new PC or after a reset: winget apps (winget export, the format
/// winget import reads), plus pip user packages, global npm packages and VS Code extensions as plain lists.
/// Nothing is installed here; restoring runs winget import in a window you can watch.
/// </summary>
public static partial class AppList
{
    [GeneratedRegex(@"^\s*(?:Installed package is not available from any source|Installed package is not available from any source:)\s*:?\s*(.+)$", RegexOptions.IgnoreCase | RegexOptions.Multiline)]
    private static partial Regex Unavailable();

    public static async Task<ExportResult> ExportAsync(string folder, bool includeVersions, CancellationToken ct = default)
    {
        Directory.CreateDirectory(folder);
        var parts = new List<ExportPart>();
        var missing = new List<string>();

        string winget = SafeUpdater.Core.Tools.Winget();
        if (File.Exists(winget))
        {
            string file = Path.Combine(folder, "winget-apps.json");
            string args = $"export -o \"{file}\" --accept-source-agreements --disable-interactivity --nowarn"
                + (includeVersions ? " --include-versions" : "");
            // --nowarn hides the "not available" lines, so run once more without it only to list them.
            SafeUpdater.Core.ProcResult r = await SafeUpdater.Core.ProcessRunner.RunAsync(winget, SplitArgs(args), TimeSpan.FromMinutes(5), ct: ct);
            int count = File.Exists(file) ? CountWingetPackages(File.ReadAllText(file)) : 0;
            SafeUpdater.Core.ProcResult warn = await SafeUpdater.Core.ProcessRunner.RunAsync(winget,
                SplitArgs($"export -o \"{Path.Combine(ToolboxPaths.Jobs, "winget-probe.json")}\" --accept-source-agreements --disable-interactivity"), TimeSpan.FromMinutes(5), ct: ct);
            missing.AddRange(ParseUnavailable(warn.Output));
            TryDelete(Path.Combine(ToolboxPaths.Jobs, "winget-probe.json"));
            parts.Add(new ExportPart("Apps (winget)", file, count, r.Ok ? null : "winget reported a problem: " + (r.Tail(1).FirstOrDefault() ?? "")));
        }

        if (SafeUpdater.Core.Tools.Python() is string py)
        {
            SafeUpdater.Core.ProcResult r = await SafeUpdater.Core.ProcessRunner.RunAsync(py, ["-m", "pip", "freeze", "--user", "--disable-pip-version-check"], TimeSpan.FromMinutes(2), ct: ct);
            if (r.Ok)
            {
                string[] lines = r.Lines.Where(l => l.Contains("==", StringComparison.Ordinal)).ToArray();
                string file = Path.Combine(folder, "pip-user-requirements.txt");
                await File.WriteAllLinesAsync(file, lines, ct);
                parts.Add(new ExportPart("Python packages (pip --user)", file, lines.Length, "Restore with: python -m pip install --user -r pip-user-requirements.txt"));
            }
        }

        if (SafeUpdater.Core.Tools.Npm() is string npm)
        {
            SafeUpdater.Core.ProcResult r = await SafeUpdater.Core.ProcessRunner.RunAsync(npm, ["ls", "-g", "--depth=0", "--json"], TimeSpan.FromMinutes(2), ct: ct);
            List<string> pkgs = ParseNpmGlobal(r.Output);
            if (pkgs.Count > 0)
            {
                string file = Path.Combine(folder, "npm-global.txt");
                await File.WriteAllLinesAsync(file, pkgs, ct);
                parts.Add(new ExportPart("Node packages (npm -g)", file, pkgs.Count, "Restore with: npm install -g (each line)"));
            }
        }

        if (SafeUpdater.Core.Tools.Find("code.cmd") is string code)
        {
            SafeUpdater.Core.ProcResult r = await SafeUpdater.Core.ProcessRunner.RunAsync(code, ["--list-extensions"], TimeSpan.FromMinutes(2), ct: ct);
            string[] ext = r.Lines.Where(l => l.Contains('.', StringComparison.Ordinal) && !l.Contains(' ', StringComparison.Ordinal)).ToArray();
            if (r.Ok && ext.Length > 0)
            {
                string file = Path.Combine(folder, "vscode-extensions.txt");
                await File.WriteAllLinesAsync(file, ext, ct);
                parts.Add(new ExportPart("VS Code extensions", file, ext.Length, "Restore with: code --install-extension (each line)"));
            }
        }

        await File.WriteAllTextAsync(Path.Combine(folder, "README.txt"), Readme(parts, missing), ct);
        return new ExportResult(folder, parts, missing);
    }

    public static int CountWingetPackages(string json)
    {
        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            return doc.RootElement.TryGetProperty("Sources", out JsonElement sources)
                ? sources.EnumerateArray().Sum(s => s.TryGetProperty("Packages", out JsonElement p) ? p.GetArrayLength() : 0)
                : 0;
        }
        catch (JsonException)
        {
            return 0;
        }
    }

    /// <summary>Package ids from a winget export file, for showing what an import would install.</summary>
    public static List<string> WingetIds(string json)
    {
        var ids = new List<string>();
        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.TryGetProperty("Sources", out JsonElement sources))
            {
                foreach (JsonElement s in sources.EnumerateArray())
                {
                    if (s.TryGetProperty("Packages", out JsonElement pk))
                    {
                        ids.AddRange(pk.EnumerateArray()
                            .Select(p => p.TryGetProperty("PackageIdentifier", out JsonElement id) ? id.GetString() : null)
                            .OfType<string>());
                    }
                }
            }
        }
        catch (JsonException)
        {
        }
        return ids;
    }

    public static List<string> ParseUnavailable(string output) =>
        Unavailable().Matches(output).Select(m => m.Groups[1].Value.Trim()).Where(s => s.Length > 0).Distinct().ToList();

    public static List<string> ParseNpmGlobal(string json)
    {
        var list = new List<string>();
        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.TryGetProperty("dependencies", out JsonElement deps))
            {
                foreach (JsonProperty p in deps.EnumerateObject())
                {
                    if (p.Name == "npm" || p.Name == "corepack")
                    {
                        continue;
                    }
                    list.Add(p.Value.TryGetProperty("version", out JsonElement v) ? $"{p.Name}@{v.GetString()}" : p.Name);
                }
            }
        }
        catch (JsonException)
        {
        }
        return list;
    }

    /// <summary>Opens a terminal window that runs winget import on the file, so every prompt is visible.</summary>
    public static void StartImport(string wingetJson)
    {
        string winget = SafeUpdater.Core.Tools.Winget();
        string cmd = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "cmd.exe");
        Process.Start(new ProcessStartInfo(cmd,
            $"/k \"\"{winget}\" import -i \"{wingetJson}\" --ignore-unavailable --accept-package-agreements --accept-source-agreements\"")
        {
            UseShellExecute = true,
        });
    }

    private static string Readme(List<ExportPart> parts, List<string> missing)
    {
        var sb = new StringBuilder();
        sb.AppendLine($"App list saved by Safe Toolbox on {DateTime.Now:d MMM yyyy, HH:mm}.");
        sb.AppendLine();
        foreach (ExportPart p in parts)
        {
            sb.AppendLine($"{Path.GetFileName(p.File)}: {p.Name}, {p.Count} items.{(p.Note is null ? "" : " " + p.Note)}");
        }
        sb.AppendLine();
        sb.AppendLine("Apps: open Safe Toolbox, App list, Restore, and pick winget-apps.json. Or run:");
        sb.AppendLine("  winget import -i winget-apps.json --ignore-unavailable");
        if (missing.Count > 0)
        {
            sb.AppendLine();
            sb.AppendLine("Not in winget, so reinstall these by hand:");
            foreach (string m in missing)
            {
                sb.AppendLine("  " + m);
            }
        }
        return sb.ToString();
    }

    private static List<string> SplitArgs(string args)
    {
        var list = new List<string>();
        var cur = new StringBuilder();
        bool q = false;
        foreach (char c in args)
        {
            if (c == '"')
            {
                q = !q;
            }
            else if (c == ' ' && !q)
            {
                if (cur.Length > 0)
                {
                    list.Add(cur.ToString());
                    cur.Clear();
                }
            }
            else
            {
                cur.Append(c);
            }
        }
        if (cur.Length > 0)
        {
            list.Add(cur.ToString());
        }
        return list;
    }

    private static void TryDelete(string f)
    {
        try
        {
            File.Delete(f);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
        }
    }
}
