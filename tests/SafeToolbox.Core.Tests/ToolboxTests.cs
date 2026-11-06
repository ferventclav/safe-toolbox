using SafeToolbox.Core;
using SafeUpdater.Core;

namespace SafeToolbox.Core.Tests;

public sealed class TempDir : IDisposable
{
    public string Path { get; } = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "stb-test-" + Guid.NewGuid().ToString("N")[..8]);

    public TempDir() => Directory.CreateDirectory(Path);

    public string File(string name, string text = "x")
    {
        string f = System.IO.Path.Combine(Path, name);
        Directory.CreateDirectory(System.IO.Path.GetDirectoryName(f)!);
        System.IO.File.WriteAllText(f, text);
        return f;
    }

    public void Dispose()
    {
        try
        {
            Directory.Delete(Path, recursive: true);
        }
        catch (IOException)
        {
        }
    }
}

public class HistoryTests
{
    [Fact]
    public void Update_runs_group_by_time_and_count_outcomes()
    {
        using var t = new TempDir();
        string f = t.File("runs.csv", """
            when,source,id,name,from,to,status,result
            2026-11-03 17:56:00,Apps,Git.Git,Git,2.55.0,2.56.0,Done,Updated
            2026-11-03 17:56:00,Python,numpy,numpy,2.5.1,2.5.4,Failed,"Didn't install: line one
            line two, with a comma"
            2026-11-03 17:56:00,Node,docx,docx,9.8.0,9.15.2,Skipped,Stopped before it started
            2026-11-05 09:00:00,Apps,Ollama.Ollama,Ollama,0.40.14,0.40.15,Done,Updated
            """);

        List<HistoryRun> runs = History.ReadUpdates(f);

        Assert.Equal(2, runs.Count);
        HistoryRun first = runs.Single(r => r.When.Day == 3);
        Assert.Equal("1 updated, 1 didn't, 1 kept back", first.Summary);
        Assert.Equal(1, first.Problems);
        Assert.Equal("numpy", first.Lines[0].Title); // problems first
        Assert.Contains("line two, with a comma", first.Lines[0].Outcome);
        Assert.Equal("Python packages · 2.5.1 → 2.5.4", first.Lines[0].Detail);
    }

    [Fact]
    public void Cleanups_end_at_each_total_row_and_group_by_rule()
    {
        using var t = new TempDir();
        string f = t.File("runs.csv", """
            time,rule,path,bytes,result
            2026-11-03 17:40:01,temp,C:\a,1000,deleted
            2026-11-03 17:40:02,temp,C:\b,2000,deleted
            2026-11-03 17:40:03,cache-chrome,C:\c,500,skipped: Chrome is open
            2026-11-03 17:40:04,TOTAL,drive free-space change,2048,measured
            2026-11-04 10:00:00,recycle,Recycle Bin,0,done
            2026-11-04 10:00:01,TOTAL,drive free-space change,0,measured
            """);

        List<HistoryRun> runs = History.ReadCleanups(f, id => id == "temp" ? "Old temp files" : id);

        Assert.Equal(2, runs.Count);
        Assert.Equal(2048, runs[0].Freed);
        Assert.Equal("Freed 2 KB, 3 items, 1 left alone", runs[0].Summary);
        HistoryLine temp = runs[0].Lines.Single(l => l.Title == "Old temp files");
        Assert.False(temp.Problem);
        Assert.Contains("2 items", temp.Detail);
        Assert.True(runs[0].Lines.Single(l => l.Title == "cache-chrome").Problem);
        Assert.Equal("Nothing measurable freed, 1 item", runs[1].Summary);
    }

    [Fact]
    public void A_missing_log_reads_as_no_runs() => Assert.Empty(History.ReadUpdates(@"C:\nowhere\runs.csv"));
}

public class PathDoctorTests
{
    private static readonly HashSet<string> Dirs = new(StringComparer.OrdinalIgnoreCase) { @"C:\Windows", @"C:\Tools", @"C:\Users\x\bin" };

    private static List<PathEntry> Analyze(string machine, string user) =>
        PathDoctor.Analyze(machine, user, d => Dirs.Contains(d.TrimEnd('\\')), f => f.EndsWith(".exe", StringComparison.OrdinalIgnoreCase));

    [Fact]
    public void Finds_missing_duplicate_empty_and_file_entries()
    {
        List<PathEntry> e = Analyze(@"C:\Windows;C:\Tools", @"C:\Users\x\bin;C:\Gone;C:\tools\;;C:\Tools\app.exe;");

        Assert.Equal(PathProblem.Missing, e.Single(x => x.Raw == @"C:\Gone").Problem);
        PathEntry dup = e.Single(x => x.Raw == @"C:\tools\");
        Assert.Equal(PathProblem.Duplicate, dup.Problem);
        Assert.Equal("system PATH entry 2", dup.DuplicateOf);
        Assert.Single(e, x => x.Problem == PathProblem.Empty);
        Assert.Equal(PathProblem.NotAFolder, e.Single(x => x.Raw.EndsWith("app.exe")).Problem);
        Assert.Equal(4, e.Count(x => x.Fixable));
        Assert.All(e.Where(x => x.Scope == PathScope.Machine), x => Assert.False(x.Fixable));
    }

    [Fact]
    public void Cleaning_keeps_good_entries_in_order_and_as_written()
    {
        string user = @"%USERPROFILE%\bin;C:\Gone;C:\Users\x\bin;;C:\tools";
        var entries = new List<PathEntry>
        {
            new(PathScope.User, 1, @"C:\Gone", @"C:\Gone", PathProblem.Missing, null),
            new(PathScope.User, 2, @"C:\Users\x\bin", @"C:\Users\x\bin", PathProblem.Duplicate, "user PATH entry 1"),
            new(PathScope.User, 3, "", "", PathProblem.Empty, null),
        };

        Assert.Equal(@"%USERPROFILE%\bin;C:\tools", PathDoctor.Cleaned(user, entries));
    }

    [Fact]
    public void A_trailing_semicolon_is_not_a_problem() =>
        Assert.DoesNotContain(Analyze("", @"C:\Tools;"), e => e.Problem != PathProblem.None);

    [Fact]
    public void The_store_python_alias_is_recognised() =>
        Assert.True(PathDoctor.IsStoreAlias(@"C:\Users\x\AppData\Local\Microsoft\WindowsApps\python.exe"));
}

public class HealthTests
{
    [Theory]
    [InlineData("0x00000154 (0xffffd60e2f5dc000, 0xffffa38b, 0x2, 0x0)", 0x154u)]
    [InlineData("The computer has rebooted from a bugcheck.  The bugcheck was: 0x0000009f (0x3, ...)", 0x9Fu)]
    [InlineData("no code here", null)]
    [InlineData("0x00000000", null)]
    public void Bugcheck_codes_are_read_as_hex(string text, uint? code) => Assert.Equal(code, Health.ParseBugcheckText(text));

    [Fact]
    public void Events_from_one_crash_count_once_and_keep_the_stop_code()
    {
        DateTime t = new(2026, 7, 1, 10, 0, 0);
        List<Crash> merged = Health.Merge(
        [
            new Crash(t.AddMinutes(2), CrashKind.PowerLoss, null, "Kernel-Power 41"),
            new Crash(t, CrashKind.UnexpectedShutdown, null, "EventLog 6008"),
            new Crash(t.AddMinutes(3), CrashKind.BlueScreen, 0x154, "BugCheck 1001"),
            new Crash(t.AddDays(2), CrashKind.PowerLoss, null, "Kernel-Power 41"),
        ]);

        Assert.Equal(2, merged.Count);
        Assert.Equal(t.AddDays(2), merged[0].When); // newest first
        Assert.Equal(0x154u, merged[1].StopCode);
        Assert.Equal("UNEXPECTED_STORE_EXCEPTION", merged[1].StopName);
        Assert.Equal("0x154", merged[1].StopCodeHex);
    }

    [Fact]
    public void Stop_code_links_follow_microsofts_page_names()
    {
        Assert.Equal("https://learn.microsoft.com/windows-hardware/drivers/debugger/bug-check-0x9f--driver-power-state-failure", StopCodes.Link(0x9F));
        Assert.EndsWith("bug-check-code-reference2", StopCodes.Link(0xDEAD));
    }

    [Fact]
    public void Battery_report_gives_wear_percent()
    {
        const string xml = """
            <?xml version="1.0" encoding="utf-8"?>
            <BatteryReport xmlns="http://schemas.microsoft.com/battery/2012">
              <Batteries>
                <Battery><Id>BIF0_9</Id><Manufacturer>MSI</Manufacturer><DesignCapacity>63232</DesignCapacity><FullChargeCapacity>45478</FullChargeCapacity><CycleCount>0</CycleCount></Battery>
              </Batteries>
            </BatteryReport>
            """;
        BatteryHealth b = Assert.Single(Health.ParseBatteryReport(xml));
        Assert.Equal(72, b.Percent);
        Assert.Null(b.CycleCount); // 0 means the battery doesn't report it
        Assert.Equal("MSI BIF0_9", b.Name);
    }

    [Fact]
    public void Drive_wear_json_handles_one_object_or_a_list_and_nulls()
    {
        DriveWear one = Assert.Single(Health.ParseDriveWear("""{"DeviceId":"SAMSUNG MZVL21T0HCLR","Wear":3,"Temperature":41,"TemperatureMax":82,"PowerOnHours":5120,"ReadErrors":0}"""));
        Assert.Equal(3, one.Wear);
        Assert.Equal(41, one.Temperature);
        List<DriveWear> two = Health.ParseDriveWear("""[{"DeviceId":"A","Wear":null},{"DeviceId":"B","Wear":10}]""");
        Assert.Null(two[0].Wear);
        Assert.Equal(10, two[1].Wear);
        Assert.Empty(Health.ParseDriveWear("not json"));
    }
}

public class ReleaseNotesTests
{
    [Fact]
    public void Winget_show_gives_release_notes_and_homepage()
    {
        const string output = """
            Found Ollama [Ollama.Ollama]
            Version: 0.40.15
            Publisher: Ollama
            Homepage: https://ollama.com/
            Release Notes Url: https://github.com/ollama/ollama/releases/tag/v0.40.15
            """;
        List<NotesLink> links = ReleaseNotes.ParseWingetShow(output);
        Assert.Equal("Release notes", links[0].Label);
        Assert.Equal("https://github.com/ollama/ollama/releases/tag/v0.40.15", links[0].Url);
        Assert.Equal("https://ollama.com/", links[1].Url);
    }

    [Fact]
    public void Other_languages_still_get_a_project_link()
    {
        List<NotesLink> links = ReleaseNotes.ParseWingetShow("Startseite: https://example.org/tool");
        Assert.Equal("https://example.org/tool", Assert.Single(links).Url);
    }

    [Fact]
    public void Fixed_pages_per_source()
    {
        Assert.Equal("https://pypi.org/project/numpy/2.5.4/",
            ReleaseNotes.For(new UpdateItem { Source = UpdateSource.Python, Id = "numpy", Name = "numpy", Available = "2.5.4" })[0].Url);
        Assert.Equal("https://support.microsoft.com/help/5066835",
            ReleaseNotes.For(new UpdateItem { Source = UpdateSource.Windows, Kind = ItemKind.WindowsUpdate, Id = "x", Name = "2026-11 Cumulative Update (KB5066835)" })[0].Url);
        Assert.Equal("https://github.com/microsoft/winget-pkgs/tree/master/manifests/p/Python/Python/3/14",
            ReleaseNotes.For(new UpdateItem { Source = UpdateSource.Apps, Id = "Python.Python.3.14", Name = "Python" })[0].Url);
    }
}

public class BackgroundTests
{
    private static readonly DateTime Now = new(2026, 11, 8, 12, 30, 0);

    [Theory]
    [InlineData(Frequency.Off, null, false)]
    [InlineData(Frequency.Daily, null, true)]
    [InlineData(Frequency.Daily, 19.0, false)]
    [InlineData(Frequency.Daily, 21.0, true)]
    [InlineData(Frequency.Weekly, 24.0 * 6, false)]
    [InlineData(Frequency.Weekly, 24.0 * 7, true)]
    public void Jobs_run_when_due(Frequency f, double? hoursAgo, bool due)
    {
        var s = new ToolboxSettings { UpdateCheck = f };
        Assert.Equal(due, Background.UpdatesDue(s, hoursAgo is double h ? Now.AddHours(-h) : null, Now));
    }

    [Fact]
    public void The_same_updates_notify_only_once()
    {
        var s = new ToolboxSettings();
        var summary = new CheckSummary { Updates = [new UpdateFinding { Source = "Apps", Name = "Git", Available = "2.56" }] };
        Assert.NotNull(Background.Notification(summary, s, updatesRan: true, cleanupRan: false));
        summary.NotifiedUpdates = summary.UpdatesKey;
        Assert.Null(Background.Notification(summary, s, updatesRan: true, cleanupRan: false));
    }

    [Fact]
    public void Cleanup_reminder_waits_for_the_threshold_unless_the_drive_is_nearly_full()
    {
        var s = new ToolboxSettings { ReminderThresholdMb = 1024 };
        var small = new CheckSummary { Cleanable = 300L << 20, TotalBytes = 1000L << 30, FreeBytes = 400L << 30 };
        Assert.Null(Background.Notification(small, s, false, true));
        var full = new CheckSummary { Cleanable = 300L << 20, TotalBytes = 1000L << 30, FreeBytes = 50L << 30 };
        (string title, string body) = Background.Notification(full, s, false, true)!.Value;
        Assert.Equal("C: is getting full", title);
        Assert.Contains("can be freed safely", body);
    }

    [Fact]
    public void Notification_names_the_first_three_updates()
    {
        var s = new ToolboxSettings();
        var summary = new CheckSummary
        {
            Updates = new[] { "Git", "Ollama", "Gemini", "Vortex", "Grammarly" }.Select(n => new UpdateFinding { Name = n, Available = "1" }).ToList(),
        };
        (string title, string body) = Background.Notification(summary, s, true, false)!.Value;
        Assert.Equal("5 safe updates are ready", title);
        Assert.Equal("Git, Ollama, Gemini and 2 more.", body);
    }
}

public class ScheduleTests
{
    [Fact]
    public void Task_runs_as_the_user_with_least_privilege_and_catches_up()
    {
        string xml = Schedule.BuildXml(@"C:\Apps\Safe Toolbox\SafeToolbox.exe", Frequency.Weekly, new DateTime(2026, 11, 6, 23, 0, 0), "S-1-5-21-1");
        Assert.Contains("<RunLevel>LeastPrivilege</RunLevel>", xml);
        Assert.Contains("<StartWhenAvailable>true</StartWhenAvailable>", xml);
        Assert.Contains("<Sunday />", xml);
        Assert.Contains("<StartBoundary>2026-11-07T12:30:00</StartBoundary>", xml);
        Assert.Contains("<Command>\"C:\\Apps\\Safe Toolbox\\SafeToolbox.exe\"</Command>", xml);
        Assert.Contains("<Arguments>--background</Arguments>", xml);
        System.Xml.Linq.XDocument.Parse(xml.Replace("encoding=\"UTF-16\"", ""));
    }

    [Theory]
    [InlineData(Frequency.Off, Frequency.Off, null)]
    [InlineData(Frequency.Weekly, Frequency.Off, Frequency.Weekly)]
    [InlineData(Frequency.Weekly, Frequency.Daily, Frequency.Daily)]
    public void One_task_runs_as_often_as_the_most_frequent_job(Frequency u, Frequency c, Frequency? task) =>
        Assert.Equal(task, Schedule.TaskFrequency(new ToolboxSettings { UpdateCheck = u, CleanupReminder = c }));
}

public class SettingsTests
{
    [Fact]
    public void Ticks_are_stored_only_when_they_differ_from_the_default()
    {
        var s = new ToolboxSettings();
        s.RememberTick("recycle", on: true, defaultChecked: false);
        s.RememberTick("temp", on: true, defaultChecked: true);
        Assert.Single(s.CleanupChoices);
        Assert.True(s.CleanupTick("recycle", false));
        s.RememberTick("recycle", on: false, defaultChecked: false);
        Assert.Empty(s.CleanupChoices);
    }

    [Fact]
    public void Turning_remembering_off_uses_the_defaults()
    {
        var s = new ToolboxSettings();
        s.RememberTick("temp", on: false, defaultChecked: true);
        s.RememberCleanupTicks = false;
        Assert.True(s.CleanupTick("temp", true));
    }

    [Fact]
    public void Settings_round_trip()
    {
        using var t = new TempDir();
        string f = System.IO.Path.Combine(t.Path, "settings.json");
        new ToolboxSettings { Accent = AccentMode.Never, Theme = ThemeMode.Dark, UpdateCheck = Frequency.Daily }.Save(f);
        ToolboxSettings back = ToolboxSettings.Load(f);
        Assert.Equal(AccentMode.Never, back.Accent);
        Assert.Equal(ThemeMode.Dark, back.Theme);
        Assert.Equal(Frequency.Daily, back.UpdateCheck);
        Assert.Contains("\"accent\": \"Never\"", File.ReadAllText(f));
    }

    [Fact]
    public void Defaults_follow_windows_and_schedule_nothing()
    {
        var s = new ToolboxSettings();
        Assert.Equal(AccentMode.FollowWindows, s.Accent);
        Assert.Null(Schedule.TaskFrequency(s));
    }
}

public class BigFilesTests
{
    [Fact]
    public void Finds_the_largest_files_and_skips_app_data_and_dot_folders()
    {
        using var t = new TempDir();
        t.File("Videos/trip.mp4", new string('v', 5000));
        t.File("Downloads/setup.exe", new string('i', 3000));
        t.File("Downloads/small.txt", "tiny");
        t.File("AppData/Local/cache.bin", new string('c', 9000));
        t.File(".cache/model.gguf", new string('m', 9000));
        t.File("Documents/deep/a/b/c/archive.zip", new string('z', 4000));

        List<BigFile> files = BigFiles.Find([t.Path], minBytes: 1000, limit: 2);

        Assert.Equal(["trip.mp4", "archive.zip"], files.Select(f => f.Name));
        Assert.Equal(FileKind.Video, files[0].Kind);
        Assert.Equal(FileKind.Archive, files[1].Kind);
    }

    [Fact]
    public void Recycling_refuses_files_inside_installed_programs()
    {
        using var t = new TempDir();
        string f = t.File("Tools/ffmpeg/ffmpeg.exe", "x");
        string? why = BigFiles.Recycle(f, [System.IO.Path.Combine(t.Path, "Tools")]);
        Assert.NotNull(why);
        Assert.True(File.Exists(f));
    }

    [Theory]
    [InlineData("a.MKV", FileKind.Video)]
    [InlineData("b.iso", FileKind.DiskImage)]
    [InlineData("c.safetensors", FileKind.AiModel)]
    [InlineData("d.docx", FileKind.Other)]
    public void Kinds_by_extension(string name, FileKind kind) => Assert.Equal(kind, BigFiles.KindOf(name));
}

public class AppListTests
{
    private const string Export = """
        {"$schema":"https://aka.ms/winget-packages.schema.2.0.json","Sources":[{"Packages":[{"PackageIdentifier":"Git.Git"},{"PackageIdentifier":"Ollama.Ollama"}],"SourceDetails":{"Name":"winget"}},{"Packages":[{"PackageIdentifier":"9NBLGGH4NNS1"}],"SourceDetails":{"Name":"msstore"}}]}
        """;

    [Fact]
    public void Counts_and_lists_packages_across_sources()
    {
        Assert.Equal(3, AppList.CountWingetPackages(Export));
        Assert.Equal(["Git.Git", "Ollama.Ollama", "9NBLGGH4NNS1"], AppList.WingetIds(Export));
        Assert.Equal(0, AppList.CountWingetPackages("{}"));
    }

    [Fact]
    public void Lists_what_winget_cannot_match()
    {
        const string output = """
            Installed package is not available from any source: Ubisoft Connect
            Installed package is not available from any source: FL Studio 2026
            Exported package requires license agreement to install: Something
            """;
        Assert.Equal(["Ubisoft Connect", "FL Studio 2026"], AppList.ParseUnavailable(output));
    }

    [Fact]
    public void Npm_globals_skip_npm_itself()
    {
        const string json = """{"dependencies":{"npm":{"version":"11.0.0"},"docx":{"version":"9.15.2"},"@anthropic-ai/claude-code":{"version":"2.1.317"}}}""";
        Assert.Equal(["docx@9.15.2", "@anthropic-ai/claude-code@2.1.317"], AppList.ParseNpmGlobal(json));
    }
}

public class RepairTests
{
    [Fact]
    public void System_file_repair_runs_dism_before_sfc()
    {
        Repair r = Repairs.Find("system-files")!;
        Assert.Contains("dism", r.Steps[0].Exe, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("/RestoreHealth", r.Steps[0].Args);
        Assert.Contains("sfc", r.Steps[1].Exe, StringComparison.OrdinalIgnoreCase);
        Assert.True(r.NeedsAdmin);
    }

    [Fact]
    public void Every_repair_has_a_unique_id_and_says_what_to_expect()
    {
        Assert.Equal(Repairs.All.Count, Repairs.All.Select(r => r.Id).Distinct().Count());
        Assert.All(Repairs.All, r =>
        {
            Assert.False(string.IsNullOrWhiteSpace(r.What));
            Assert.False(string.IsNullOrWhiteSpace(r.Heads));
            Assert.NotEmpty(r.Steps);
        });
    }

    [Fact]
    public void Dns_flush_asks_for_admin()
    {
        Repair dns = Repairs.Find("dns")!;
        Assert.True(dns.NeedsAdmin);
        Assert.Single(dns.Steps);
        Assert.Equal("/flushdns", dns.Steps[0].Args);
    }

    [Fact]
    public void Old_unpack_folders_go_but_the_current_and_busy_ones_stay()
    {
        string root = Path.Combine(Path.GetTempPath(), "stb-test-" + Guid.NewGuid().ToString("N"), ".net", "Safe Toolbox");
        string mine = Directory.CreateDirectory(Path.Combine(root, "current")).FullName;
        string old = Directory.CreateDirectory(Path.Combine(root, "old")).FullName;
        string busy = Directory.CreateDirectory(Path.Combine(root, "busy")).FullName;
        File.WriteAllText(Path.Combine(mine, "SafeToolbox.dll"), "x");
        File.WriteAllText(Path.Combine(old, "SafeToolbox.dll"), "x");
        File.WriteAllText(Path.Combine(busy, "SafeToolbox.dll"), "x");
        try
        {
            using (new FileStream(Path.Combine(busy, "SafeToolbox.dll"), FileMode.Open, FileAccess.Read, FileShare.Read))
            {
                Assert.Equal(0, SingleFileUnpack.RemoveStale(mine, "SafeToolbox"));
                Assert.Equal(1, SingleFileUnpack.RemoveStale(mine + Path.DirectorySeparatorChar, "Safe Toolbox"));
            }
            Assert.True(Directory.Exists(mine));
            Assert.True(Directory.Exists(busy));
            Assert.False(Directory.Exists(old));
        }
        finally
        {
            Directory.Delete(Path.GetDirectoryName(Path.GetDirectoryName(root))!, recursive: true);
        }
    }

    [Fact]
    public void Unpack_cleanup_does_nothing_outside_a_dotnet_unpack_folder()
    {
        string root = Path.Combine(Path.GetTempPath(), "stb-test-" + Guid.NewGuid().ToString("N"), "SafeToolbox");
        string mine = Directory.CreateDirectory(Path.Combine(root, "bin")).FullName;
        string sibling = Directory.CreateDirectory(Path.Combine(root, "other")).FullName;
        try
        {
            Assert.Equal(0, SingleFileUnpack.RemoveStale(mine, "SafeToolbox"));
            Assert.True(Directory.Exists(sibling));
        }
        finally
        {
            Directory.Delete(Path.GetDirectoryName(root)!, recursive: true);
        }
    }
}
