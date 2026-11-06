using System.Text.Json;
using SafeUpdater.Core;

namespace SafeToolbox.Core;

public sealed class UpdateFinding
{
    public string Source { get; set; } = "";
    public string Name { get; set; } = "";
    public string Current { get; set; } = "";
    public string Available { get; set; } = "";
}

/// <summary>What the last background or command-line check found. Saved to last-check.json for the Home page.</summary>
public sealed class CheckSummary
{
    public DateTime When { get; set; }

    /// <summary>When each part last ran. Kept here, not in settings.json, so the background never races the window.</summary>
    public DateTime? UpdatesWhen { get; set; }
    public DateTime? CleanupWhen { get; set; }

    /// <summary>The update set the last notification was about, so the same updates don't notify twice.</summary>
    public string? NotifiedUpdates { get; set; }
    public bool UpdatesChecked { get; set; }
    public List<UpdateFinding> Updates { get; set; } = [];
    public int LeftOut { get; set; }
    public List<string> FailedSources { get; set; } = [];
    public bool CleanupChecked { get; set; }
    public long Cleanable { get; set; }
    public long FreeBytes { get; set; }
    public long TotalBytes { get; set; }

    /// <summary>The update set as one string, so the same updates don't notify twice.</summary>
    public string UpdatesKey => string.Join('|', Updates.Select(u => $"{u.Source}:{u.Name}:{u.Available}").Order(StringComparer.Ordinal));

    private static readonly JsonSerializerOptions Options = new() { WriteIndented = true, PropertyNamingPolicy = JsonNamingPolicy.CamelCase };

    public string ToJson() => JsonSerializer.Serialize(this, Options);

    public static CheckSummary? Load()
    {
        try
        {
            return File.Exists(ToolboxPaths.LastBackground)
                ? JsonSerializer.Deserialize<CheckSummary>(File.ReadAllText(ToolboxPaths.LastBackground), Options)
                : null;
        }
        catch (Exception ex) when (ex is IOException or JsonException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    public void Save()
    {
        try
        {
            File.WriteAllText(ToolboxPaths.LastBackground, ToJson());
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            ToolboxPaths.WriteError("Could not save last-check.json: " + ex.Message);
        }
    }
}

/// <summary>
/// The scheduled task's work, also used by "SafeToolbox.exe --check". It only looks: the same update rules and
/// cleanup guard as the window, no installs, no deletes.
/// </summary>
public static class Background
{
    public static bool UpdatesDue(ToolboxSettings s, DateTime? last, DateTime now) => Due(s.UpdateCheck, last, now);

    public static bool CleanupDue(ToolboxSettings s, DateTime? last, DateTime now) => Due(s.CleanupReminder, last, now);

    private static bool Due(Frequency f, DateTime? last, DateTime now) => f switch
    {
        Frequency.Daily => last is null || now - last.Value >= TimeSpan.FromHours(20),
        Frequency.Weekly => last is null || now - last.Value >= TimeSpan.FromDays(6.5),
        _ => false,
    };

    public static async Task<CheckSummary> RunAsync(bool updates, bool cleanup, ToolboxSettings toolbox, CancellationToken ct = default)
    {
        var summary = CheckSummary.Load() ?? new CheckSummary();
        summary.When = DateTime.Now;
        if (updates)
        {
            await CheckUpdatesAsync(summary, ct);
            summary.UpdatesWhen = DateTime.Now;
        }
        if (cleanup)
        {
            ScanCleanup(summary, toolbox, ct);
            summary.CleanupWhen = DateTime.Now;
        }
        if (SafeDiskCleaner.Core.Tools.SystemDriveUsage() is (long total, long free))
        {
            summary.TotalBytes = total;
            summary.FreeBytes = free;
        }
        summary.Save();
        return summary;
    }

    private static async Task CheckUpdatesAsync(CheckSummary summary, CancellationToken ct)
    {
        var settings = SafeUpdater.Core.Settings.Load();
        SourceResult[] results = await Task.WhenAll(Checker.Sources.Select(async s =>
        {
            try
            {
                return await Task.Run(() => s.Check(ct), ct);
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            {
                RunLog.WriteError("Background check failed: " + ex.Message);
                return new SourceResult { Source = s.Source, Failed = true, Message = ex.Message };
            }
        }));
        var items = results.SelectMany(r => r.Items).ToList();
        IReadOnlyCollection<string> choco = results.FirstOrDefault(r => r.Source == UpdateSource.Chocolatey)?.Managed ?? [];
        Policy.ApplyAll(items, PolicyContext.From(settings, choco, Tools.Rustup() is not null));
        summary.UpdatesChecked = true;
        summary.Updates = items
            .Where(i => !i.Hidden && i.Selectable && i.Recommended && i.Kind != ItemKind.StoreScan)
            .Select(i => new UpdateFinding { Source = Checker.Title(i.Source), Name = i.DisplayName, Current = i.Current, Available = i.TargetVersion ?? i.Available })
            .ToList();
        summary.LeftOut = items.Count(i => i.Hidden);
        summary.FailedSources = results.Where(r => r.Failed).Select(r => Checker.Title(r.Source)).ToList();
        settings.LastChecked = DateTime.Now;
        settings.Save();
    }

    private static void ScanCleanup(CheckSummary summary, ToolboxSettings toolbox, CancellationToken ct)
    {
        SafeDiskCleaner.Core.ScanResult scan = SafeDiskCleaner.Core.Scanner.Scan(admin: SafeDiskCleaner.Core.Tools.IsAdmin(), ct: ct);
        summary.CleanupChecked = true;
        summary.Cleanable = scan.Rules
            .Where(r => r.CanClean && toolbox.CleanupTick(r.Id, r.DefaultChecked))
            .Sum(r => r.Total);
    }

    /// <summary>The notification to show, or null when there is nothing new worth a notification.</summary>
    public static (string Title, string Body)? Notification(CheckSummary s, ToolboxSettings toolbox, bool updatesRan, bool cleanupRan)
    {
        var lines = new List<string>();
        string? title = null;
        if (updatesRan && s.Updates.Count > 0 && s.UpdatesKey != s.NotifiedUpdates)
        {
            title = s.Updates.Count == 1 ? "1 safe update is ready" : $"{s.Updates.Count} safe updates are ready";
            IEnumerable<string> names = s.Updates.Select(u => u.Name).Take(3);
            lines.Add(string.Join(", ", names) + (s.Updates.Count > 3 ? $" and {s.Updates.Count - 3} more" : "") + ".");
        }
        long threshold = (long)toolbox.ReminderThresholdMb * 1024 * 1024;
        bool lowSpace = s.TotalBytes > 0 && s.FreeBytes < s.TotalBytes / 10;
        if (cleanupRan && (s.Cleanable >= threshold || lowSpace && s.Cleanable > 0))
        {
            string text = $"{SafeDiskCleaner.Core.SizeFormat.Format(s.Cleanable)} can be freed safely"
                + (lowSpace ? $", and C: has only {SafeDiskCleaner.Core.SizeFormat.FormatGb(s.FreeBytes)} free." : ".");
            title ??= lowSpace ? "C: is getting full" : "Space to free";
            lines.Add(text);
        }
        return title is null ? null : (title, string.Join(" ", lines));
    }
}
