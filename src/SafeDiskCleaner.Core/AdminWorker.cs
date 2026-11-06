using System.ComponentModel;
using System.Diagnostics;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace SafeDiskCleaner.Core;

/// <summary>What the window asks the elevated helper to do with the admin-only rules.</summary>
public sealed class AdminJob
{
    /// <summary>"scan" to size the admin rules, "clean" to clean the listed ones.</summary>
    public string Mode { get; set; } = "scan";
    public List<string> RuleIds { get; set; } = [];
    public string ResultFile { get; set; } = "";
    public string ProgressFile { get; set; } = "";

    /// <summary>The requesting user's runs.csv, so one log holds the whole clean.</summary>
    public string? LogFile { get; set; }
    public string? StopFile { get; set; }
}

public sealed class AdminItem
{
    public string Label { get; set; } = "";
    public string? Path { get; set; }
    public long? Size { get; set; }
    public bool Handler { get; set; }
    public bool Action { get; set; }
    public string? Result { get; set; }
}

public sealed class AdminRule
{
    public string Id { get; set; } = "";
    public string? Blocked { get; set; }
    public List<AdminItem> Items { get; set; } = [];
    public List<Skip> Skipped { get; set; } = [];
}

public sealed class AdminResult
{
    public List<AdminRule> Rules { get; set; } = [];
    public long Freed { get; set; }
    public int Items { get; set; }
    public int Problems { get; set; }
    public int Stopped { get; set; }
    public string? Error { get; set; }
}

/// <summary>
/// The elevated half of the cleaner: "SafeToolbox.exe --clean-worker job.json". It only ever touches rules marked
/// admin (the Windows group and the memory dump), which are machine-wide, so whichever admin account approves the
/// prompt, nothing in a user profile is affected. Every path still goes through the guard twice.
/// </summary>
public static class AdminWorker
{
    private static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        Converters = { new JsonStringEnumConverter() },
    };

    public static int Run(string jobPath)
    {
        AdminJob? job = null;
        var result = new AdminResult();
        try
        {
            job = JsonSerializer.Deserialize<AdminJob>(File.ReadAllText(jobPath), Options) ?? throw new InvalidDataException("empty job");
            var wanted = new HashSet<string>(job.RuleIds, StringComparer.Ordinal);
            List<Rule> rules = RuleCatalog.Build().Where(r => r.NeedsAdmin && wanted.Contains(r.Id)).ToList();
            var ctx = new ScanContext(ProcessSnapshot.Take());
            Guard guard = Guard.CreateDefault();
            foreach (Rule r in rules)
            {
                Scanner.ScanRule(r, ctx, guard, admin: true);
            }
            if (job.Mode == "clean")
            {
                using var stop = new CancellationTokenSource();
                using var watch = job.StopFile is null ? null : new Timer(_ =>
                {
                    if (File.Exists(job.StopFile))
                    {
                        stop.Cancel();
                    }
                }, null, 500, 500);
                var progress = new Progress<CleanProgress>(p => Append(job.ProgressFile, $"{p.Done}/{p.Total}\t{p.Current}"));
                CleanResult cr = Cleaner.Clean(rules, progress, guard: guard, logFile: job.LogFile, admin: true, stop: stop.Token);
                result.Freed = cr.Freed;
                result.Items = cr.Items;
                result.Problems = cr.Problems;
                result.Stopped = cr.Stopped;
            }
            result.Rules = rules.Select(ToAdmin).ToList();
        }
        catch (Exception ex)
        {
            result.Error = ex.Message;
            RunLog.WriteError("Admin worker failed: " + ex);
        }
        if (job is not null)
        {
            File.WriteAllText(job.ResultFile, JsonSerializer.Serialize(result, Options));
        }
        return result.Error is null ? 0 : 1;
    }

    private static AdminRule ToAdmin(Rule r) => new()
    {
        Id = r.Id,
        Blocked = r.Blocked,
        Skipped = r.Skipped.ToList(),
        Items = r.Items.Select(i => new AdminItem
        {
            Label = i.Label, Path = i.Path, Size = i.Size, Handler = i.Handler is not null, Action = i.Action is not null, Result = i.Result,
        }).ToList(),
    };

    /// <summary>
    /// Copies the helper's findings into the window's rules, so the Windows group shows sizes and paths.
    /// The copies are display-only: the window never deletes these itself, it hands the rule ids back to the helper.
    /// </summary>
    public static void Apply(AdminResult result, IEnumerable<Rule> rules)
    {
        Dictionary<string, AdminRule> byId = result.Rules.ToDictionary(r => r.Id);
        foreach (Rule r in rules.Where(r => r.NeedsAdmin))
        {
            if (!byId.TryGetValue(r.Id, out AdminRule? a))
            {
                continue;
            }
            r.Items.Clear();
            r.Skipped.Clear();
            r.Blocked = a.Blocked;
            r.Skipped.AddRange(a.Skipped);
            foreach (AdminItem i in a.Items)
            {
                Item it = i.Handler ? Item.ForHandler(i.Label, "elevated", i.Size)
                    : i.Path is not null ? new Item(i.Path, i.Label, i.Size)
                    : new Item(i.Label, () => throw new InvalidOperationException("Runs in the admin helper"), i.Size);
                it.Result = i.Result;
                r.Items.Add(it);
            }
            r.ScannedElevated = true;
        }
    }

    public static string Serialize(AdminJob job) => JsonSerializer.Serialize(job, Options);

    public static AdminResult? ReadResult(string file)
    {
        try
        {
            return File.Exists(file) ? JsonSerializer.Deserialize<AdminResult>(File.ReadAllText(file), Options) : null;
        }
        catch (Exception ex) when (ex is IOException or JsonException)
        {
            return null;
        }
    }

    /// <summary>Starts this exe elevated in clean-worker mode. Null when the UAC prompt was declined.</summary>
    public static Process? Start(string jobPath)
    {
        string exe = Environment.ProcessPath ?? throw new InvalidOperationException("Unknown executable path");
        try
        {
            return Process.Start(new ProcessStartInfo(exe)
            {
                UseShellExecute = true,
                Verb = "runas",
                Arguments = $"--clean-worker \"{jobPath}\"",
                WindowStyle = ProcessWindowStyle.Hidden,
                WorkingDirectory = AppContext.BaseDirectory,
            });
        }
        catch (Win32Exception ex) when (ex.NativeErrorCode == 1223)
        {
            return null;
        }
    }

    private static void Append(string file, string line)
    {
        try
        {
            File.AppendAllText(file, line + Environment.NewLine);
        }
        catch (IOException)
        {
        }
    }
}
