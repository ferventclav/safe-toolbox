using System.Globalization;
using System.Text;

namespace SafeToolbox.Core;

public enum RunKind { Updates, Cleanup }

public sealed record HistoryLine(string Title, string Detail, string Outcome, bool Problem);

public sealed class HistoryRun
{
    public required RunKind Kind { get; init; }
    public required DateTime When { get; init; }
    public required string Summary { get; init; }
    public List<HistoryLine> Lines { get; } = [];
    public int Problems => Lines.Count(l => l.Problem);

    /// <summary>Cleanup runs: the measured change in free space.</summary>
    public long Freed { get; init; }
}

/// <summary>
/// Reads the two run logs the tools already keep: %LOCALAPPDATA%\SafeUpdater\runs.csv (one row per update,
/// all rows of a run share a time) and %LOCALAPPDATA%\SafeDiskCleaner\runs.csv (one row per path, then a TOTAL row).
/// </summary>
public static class History
{
    private const string TimeFormat = "yyyy-MM-dd HH:mm:ss";

    public static string UpdaterLog => SafeUpdater.Core.AppPaths.RunsCsv;
    public static string CleanerLog => SafeDiskCleaner.Core.RunLog.FilePath;

    public static List<HistoryRun> ReadAll(Func<string, string>? ruleTitle = null) =>
        ReadUpdates(UpdaterLog).Concat(ReadCleanups(CleanerLog, ruleTitle)).OrderByDescending(r => r.When).ToList();

    public static List<HistoryRun> ReadUpdates(string file)
    {
        var runs = new List<HistoryRun>();
        foreach (IGrouping<string, List<string>> g in Rows(file).Skip(1).Where(f => f.Count >= 7).GroupBy(f => f[0]))
        {
            if (!TryTime(g.Key, out DateTime when))
            {
                continue;
            }
            int done = 0, failed = 0, other = 0;
            var lines = new List<HistoryLine>();
            foreach (List<string> f in g)
            {
                string status = f[6];
                string result = f.Count > 7 ? f[7] : "";
                bool problem = status == "Failed";
                done += status == "Done" ? 1 : 0;
                failed += problem ? 1 : 0;
                other += status is not ("Done" or "Failed") ? 1 : 0;
                string versions = f[4].Length > 0 || f[5].Length > 0 ? $"{(f[4].Length > 0 ? f[4] : "?")} → {f[5]}" : "";
                lines.Add(new HistoryLine(f[3], $"{SourceName(f[1])}{(versions.Length > 0 ? " · " + versions : "")}",
                    result.Length > 0 ? result : status, problem));
            }
            var parts = new List<string> { $"{done} updated" };
            if (failed > 0)
            {
                parts.Add($"{failed} didn't");
            }
            if (other > 0)
            {
                parts.Add($"{other} kept back");
            }
            var run = new HistoryRun { Kind = RunKind.Updates, When = when, Summary = string.Join(", ", parts) };
            run.Lines.AddRange(lines.OrderBy(l => l.Problem ? 0 : 1).ThenBy(l => l.Title, StringComparer.CurrentCultureIgnoreCase));
            runs.Add(run);
        }
        return runs;
    }

    public static List<HistoryRun> ReadCleanups(string file, Func<string, string>? ruleTitle = null)
    {
        ruleTitle ??= id => id;
        var runs = new List<HistoryRun>();
        var pending = new List<List<string>>();
        foreach (List<string> f in Rows(file).Skip(1))
        {
            if (f.Count < 5)
            {
                continue;
            }
            if (f[1] != "TOTAL")
            {
                pending.Add(f);
                continue;
            }
            if (!TryTime(f[0], out DateTime when))
            {
                pending.Clear();
                continue;
            }
            long freed = long.TryParse(f[3], NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out long b) ? b : 0;
            int items = pending.Count;
            int problems = pending.Count(p => !IsCleanOk(p[4]));
            string summary = freed > 0 ? $"Freed {SafeDiskCleaner.Core.SizeFormat.Format(freed)}" : "Nothing measurable freed";
            summary += $", {items} item{(items == 1 ? "" : "s")}" + (problems > 0 ? $", {problems} left alone" : "");
            var run = new HistoryRun { Kind = RunKind.Cleanup, When = when, Summary = summary, Freed = freed };
            foreach (IGrouping<string, List<string>> rule in pending.GroupBy(p => p[1]))
            {
                long bytes = rule.Sum(p => long.TryParse(p[3], NumberStyles.Integer, CultureInfo.InvariantCulture, out long v) ? v : 0);
                int bad = rule.Count(p => !IsCleanOk(p[4]));
                string outcome = bad == 0 ? "Cleaned" : bad == rule.Count() ? FirstWords(rule.First(p => !IsCleanOk(p[4]))[4]) : $"{bad} of {rule.Count()} left alone";
                run.Lines.Add(new HistoryLine(ruleTitle(rule.Key),
                    $"{rule.Count()} item{(rule.Count() == 1 ? "" : "s")}{(bytes > 0 ? " · " + SafeDiskCleaner.Core.SizeFormat.Format(bytes) : "")}",
                    outcome, bad > 0));
            }
            runs.Add(run);
            pending.Clear();
        }
        return runs;
    }

    private static bool IsCleanOk(string result) => result is "deleted" or "done" or "done by Windows" or "dry run";

    private static string FirstWords(string s) => s.Length > 80 ? s[..80] + "…" : s;

    private static string SourceName(string source) => Enum.TryParse(source, out SafeUpdater.Core.UpdateSource s)
        ? SafeUpdater.Core.Checker.Title(s)
        : source;

    private static bool TryTime(string s, out DateTime when) =>
        DateTime.TryParseExact(s, TimeFormat, CultureInfo.InvariantCulture, DateTimeStyles.AssumeLocal, out when);

    internal static IEnumerable<List<string>> Rows(string file)
    {
        string[] lines;
        try
        {
            if (!File.Exists(file))
            {
                yield break;
            }
            using var fs = new FileStream(file, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            using var sr = new StreamReader(fs, Encoding.UTF8);
            lines = sr.ReadToEnd().Split('\n');
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            yield break;
        }
        // A quoted field can hold a newline (pip output in a result), so rows are rebuilt across lines.
        var sb = new StringBuilder();
        foreach (string raw in lines)
        {
            sb.Append(raw.TrimEnd('\r'));
            if (sb.ToString().Count(c => c == '"') % 2 == 1)
            {
                sb.Append('\n');
                continue;
            }
            if (sb.Length > 0)
            {
                yield return Csv.ParseLine(sb.ToString());
            }
            sb.Clear();
        }
    }
}

public static class Csv
{
    public static List<string> ParseLine(string line)
    {
        var fields = new List<string>();
        var cur = new StringBuilder();
        bool quoted = false;
        for (int i = 0; i < line.Length; i++)
        {
            char c = line[i];
            if (quoted)
            {
                if (c == '"' && i + 1 < line.Length && line[i + 1] == '"')
                {
                    cur.Append('"');
                    i++;
                }
                else if (c == '"')
                {
                    quoted = false;
                }
                else
                {
                    cur.Append(c);
                }
            }
            else if (c == '"')
            {
                quoted = true;
            }
            else if (c == ',')
            {
                fields.Add(cur.ToString());
                cur.Clear();
            }
            else
            {
                cur.Append(c);
            }
        }
        fields.Add(cur.ToString());
        return fields;
    }
}
