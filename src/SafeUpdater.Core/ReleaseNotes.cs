using System.Text.RegularExpressions;

namespace SafeUpdater.Core;

public sealed record NotesLink(string Label, string Url);

/// <summary>
/// Where to read what changed before updating. Most sources have a fixed page per version; winget apps carry a
/// release-notes URL and a homepage in their manifest, read on demand with "winget show".
/// </summary>
public static partial class ReleaseNotes
{
    [GeneratedRegex(@"KB(\d{6,8})", RegexOptions.IgnoreCase)]
    private static partial Regex Kb();

    [GeneratedRegex(@"https?://\S+", RegexOptions.IgnoreCase)]
    private static partial Regex Url();

    public static List<NotesLink> For(UpdateItem i)
    {
        string v = Uri.EscapeDataString(i.TargetVersion ?? i.Available);
        string id = Uri.EscapeDataString(i.Id);
        var links = new List<NotesLink>();
        switch (i.Source)
        {
            case UpdateSource.Python:
                links.Add(new("PyPI page for this version", $"https://pypi.org/project/{id}/{v}/"));
                break;
            case UpdateSource.Node:
                links.Add(new("npm page for this version", $"https://www.npmjs.com/package/{i.Id}/v/{v}"));
                break;
            case UpdateSource.Chocolatey:
                links.Add(new("Chocolatey package page", $"https://community.chocolatey.org/packages/{id}/{v}"));
                break;
            case UpdateSource.Scoop:
                links.Add(new("Find it on scoop.sh", $"https://scoop.sh/#/apps?q={id}"));
                break;
            case UpdateSource.Rust:
                links.Add(i.Id == "rustup"
                    ? new("rustup changelog", "https://github.com/rust-lang/rustup/blob/master/CHANGELOG.md")
                    : new("Rust release notes", "https://doc.rust-lang.org/stable/releases.html"));
                break;
            case UpdateSource.ClaudeCode:
                links.Add(new("Claude Code changelog", "https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md"));
                break;
            case UpdateSource.Nvidia:
                links.Add(new("NVIDIA driver release notes", "https://www.nvidia.com/en-us/geforce/drivers/"));
                break;
            case UpdateSource.Windows when Kb().Match((i.Detail ?? "") + " " + i.Name) is { Success: true } m:
                links.Add(new($"Microsoft support article KB{m.Groups[1].Value}", $"https://support.microsoft.com/help/{m.Groups[1].Value}"));
                break;
            case UpdateSource.Apps:
                links.Add(new("winget manifest", $"https://github.com/microsoft/winget-pkgs/tree/master/manifests/{char.ToLowerInvariant(i.Id[0])}/{i.Id.Replace('.', '/')}"));
                break;
        }
        return links;
    }

    /// <summary>
    /// Release notes and homepage from "winget show" output. The labels are in the display language, so a line
    /// counts when its label mentions notes or home in English, else the first two URLs are used in order.
    /// </summary>
    public static List<NotesLink> ParseWingetShow(string output)
    {
        string? notes = null, home = null;
        var urls = new List<string>();
        foreach (string raw in output.Split('\n'))
        {
            string line = raw.Trim();
            Match m = Url().Match(line);
            if (!m.Success)
            {
                continue;
            }
            string url = m.Value.TrimEnd('.', ',', ')');
            string label = line[..m.Index].ToLowerInvariant();
            urls.Add(url);
            if (label.Contains("release notes url") || label.Contains("releasenotesurl"))
            {
                notes ??= url;
            }
            else if (label.StartsWith("homepage", StringComparison.Ordinal))
            {
                home ??= url;
            }
        }
        var links = new List<NotesLink>();
        if (notes is not null)
        {
            links.Add(new("Release notes", notes));
        }
        if (home is not null)
        {
            links.Add(new("Homepage", home));
        }
        if (links.Count == 0 && urls.Count > 0)
        {
            links.Add(new("Project page", urls[0]));
        }
        return links;
    }

    public static async Task<List<NotesLink>> WingetAsync(string id, CancellationToken ct = default)
    {
        if (!Tools.HasWinget())
        {
            return [];
        }
        ProcResult r = await ProcessRunner.RunAsync(Tools.Winget(),
            ["show", "--id", id, "-e", "--accept-source-agreements", "--disable-interactivity"], TimeSpan.FromSeconds(40), ct: ct);
        return r.Ok ? ParseWingetShow(r.Output) : [];
    }
}
