namespace SafeToolbox.Core;

/// <summary>
/// Safe Toolbox ships as one exe. .NET unpacks its DLLs to %TEMP%\.net\&lt;exe name&gt;\&lt;hash&gt; and makes a new
/// folder for every new build, about 220 MB each, and never removes the old ones. Because the cleaner protects
/// %TEMP%\.net, the app tidies its own: every unpack folder except the one it runs from, unless a copy is
/// still running from it.
/// </summary>
public static class SingleFileUnpack
{
    // Every running copy has these two open, so a locked one means "in use, leave it".
    private static readonly string[] Probes = ["SafeToolbox.dll", "Microsoft.ui.xaml.dll"];

    /// <summary>
    /// Removes stale unpack folders next to <paramref name="current"/>. <paramref name="exeName"/> is the exe's
    /// file name without ".exe": .NET names the folder after it, so a renamed copy ("Safe Toolbox.exe") unpacks
    /// to .net\Safe Toolbox. Returns how many folders went.
    /// </summary>
    public static int RemoveStale(string current, string exeName)
    {
        string mine = Path.TrimEndingDirectorySeparator(Path.GetFullPath(current));
        string? parent = Path.GetDirectoryName(mine);
        // only when running unpacked under ...\.net\<exe name>\<hash>, never from a build or install folder
        if (parent is null || exeName.Length == 0 || !string.Equals(Path.GetFileName(parent), exeName, StringComparison.OrdinalIgnoreCase)
            || !string.Equals(Path.GetFileName(Path.GetDirectoryName(parent)), ".net", StringComparison.OrdinalIgnoreCase))
        {
            return 0;
        }
        int removed = 0;
        foreach (string dir in Directory.EnumerateDirectories(parent))
        {
            if (string.Equals(Path.TrimEndingDirectorySeparator(dir), mine, StringComparison.OrdinalIgnoreCase) || InUse(dir))
            {
                continue;
            }
            try
            {
                Directory.Delete(dir, recursive: true);
                removed++;
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                // a file came into use after the check; next start tries again
            }
        }
        return removed;
    }

    private static bool InUse(string dir)
    {
        foreach (string probe in Probes)
        {
            string file = Path.Combine(dir, probe);
            if (!File.Exists(file))
            {
                continue;
            }
            try
            {
                using var _ = new FileStream(file, FileMode.Open, FileAccess.ReadWrite, FileShare.None);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                return true;
            }
        }
        return false;
    }
}
