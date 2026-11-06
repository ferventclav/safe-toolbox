using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using SafeToolbox.Core;
using static SafeToolbox.Ui;

namespace SafeToolbox.Pages;

internal sealed class HealthSnapshot
{
    public List<Crash> Crashes { get; init; } = [];
    public List<AppCrash> AppCrashes { get; init; } = [];
    public List<DriveHealth> Drives { get; init; } = [];
    public List<BatteryHealth> Batteries { get; init; } = [];
    public List<AntivirusState> Antivirus { get; init; } = [];
    public TimeSpan Uptime { get; init; }
    public bool RestartPending { get; init; }
    public DateTime Taken { get; init; }
}

/// <summary>
/// Read-only health checks. Everything here works without admin except drive wear and temperature, which
/// Windows only gives an administrator, and making a restore point.
/// </summary>
internal sealed class HealthPage : PageBase
{
    private readonly StackPanel _panel = new() { Spacing = 16, Padding = new Thickness(0, 0, 0, 24) };
    private readonly TextBlock _summary = Text("", "SubtitleTextBlockStyle", wrap: false);
    private readonly TextBlock _summarySub = Muted("");
    private Task<HealthSnapshot>? _snapshot;
    private List<DriveWear>? _wear;

    public HealthPage(IShell shell) : base(shell, "Health",
        "Crashes and blue screens, drive health, battery wear and security, from what Windows already records.",
        "Nothing here changes your PC.")
    {
        HeaderRight.Children.Add(_summary);
        HeaderRight.Children.Add(_summarySub);
        AddCommand("Check again", "", async (_, _) => { _snapshot = null; await RenderAsync(); }, key: Windows.System.VirtualKey.F5);
        AddCommand("Reliability Monitor", "", (_, _) =>
        {
            try
            {
                System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo("perfmon.exe", "/rel") { UseShellExecute = true });
            }
            catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or InvalidOperationException)
            {
                ShowError("Couldn't open Reliability Monitor", ex.Message);
            }
        });
        AddCommand("Windows Security", "", (_, _) => Open("windowsdefender:"), secondary: true);
        AddCommand("System information", "", (_, _) => Open("msinfo32.exe"), secondary: true);
        SetBody(Scroller(_panel));
    }

    /// <summary>One read shared by Home and this page; read again after ten minutes or on Check again.</summary>
    public Task<HealthSnapshot> GetSnapshotAsync()
    {
        if (_snapshot is null || (_snapshot.IsCompleted && DateTime.Now - _snapshot.Result.Taken > TimeSpan.FromMinutes(10)))
        {
            _snapshot = Task.Run(() => new HealthSnapshot
            {
                Crashes = Health.Crashes(90),
                AppCrashes = Health.AppCrashes(14),
                Drives = Health.Drives(),
                Batteries = Health.Batteries(),
                Antivirus = Health.Antivirus(),
                Uptime = Health.Uptime(),
                RestartPending = Health.RestartPending(),
                Taken = DateTime.Now,
            });
        }
        return _snapshot;
    }

    protected override Task OnFirstShownAsync() => RenderAsync();

    private async Task RenderAsync()
    {
        StatusText.Text = "Reading Windows' records…";
        WorkBar.Visibility = Visibility.Visible;
        WorkBar.IsIndeterminate = true;
        HealthSnapshot h = await GetSnapshotAsync();
        WorkBar.IsIndeterminate = false;
        WorkBar.Visibility = Visibility.Collapsed;
        StatusText.Text = $"Read {Ago(h.Taken)}.";
        _panel.Children.Clear();

        int recent = h.Crashes.Count(c => c.When >= DateTime.Now.AddDays(-30));
        bool drivesOk = h.Drives.All(d => d.Healthy);
        int issues = (recent > 0 ? 1 : 0) + (drivesOk ? 0 : 1) + (h.Antivirus.Any(a => a.On) ? 0 : 1);
        _summary.Text = issues == 0 ? "Looks healthy" : $"{Plural(issues, "thing")} to look at";
        _summarySub.Text = $"Up for {Uptime(h.Uptime)}" + (h.RestartPending ? ", restart pending" : "");

        _panel.Children.Add(StabilityCard(h));
        _panel.Children.Add(DrivesCard(h));
        if (h.Batteries.Count > 0)
        {
            _panel.Children.Add(BatteryCard(h));
        }
        _panel.Children.Add(SystemCard(h));
    }

    private Border StabilityCard(HealthSnapshot h)
    {
        var body = Stack(10);
        if (h.Crashes.Count == 0)
        {
            body.Children.Add(Row(8, Icon("", 16, "IconSuccess"), Text("No blue screens or unexpected shutdowns in the last 90 days.")));
        }
        foreach (Crash c in h.Crashes.Take(12))
        {
            string what = c.Kind switch
            {
                CrashKind.BlueScreen => c.StopCode is null ? "Blue screen" : $"Blue screen {c.StopCodeHex}" + (c.StopName is null ? "" : $" {c.StopName}"),
                CrashKind.PowerLoss => "Lost power or was forced off (no stop code)",
                _ => "Shut down unexpectedly",
            };
            var lines = Stack(2, Text(what, "BodyStrongTextBlockStyle"), Muted($"{c.When:ddd d MMM yyyy, HH:mm} · {c.Source}"));
            if (c.StopCode is uint code)
            {
                if (StopCodes.Hint(code) is string hint)
                {
                    lines.Children.Add(Muted(hint, caption: false));
                }
                lines.Children.Add(Link("What Microsoft says about this stop code", StopCodes.Link(code)));
            }
            body.Children.Add(Columns((Icon("", 16, c.Kind == CrashKind.BlueScreen ? "IconCritical" : "IconCaution"), Auto), (lines, Star)));
        }
        if (h.Crashes.Count > 12)
        {
            body.Children.Add(Muted($"and {h.Crashes.Count - 12} earlier ones. Reliability Monitor shows them all."));
        }
        if (h.AppCrashes.Count > 0)
        {
            var apps = Stack(4, Text("Apps that crashed in the last 14 days", "BodyStrongTextBlockStyle"));
            apps.Margin = new Thickness(0, 8, 0, 0);
            foreach (AppCrash a in h.AppCrashes.Take(6))
            {
                apps.Children.Add(Columns((Text(a.App, wrap: false), Star), (Muted($"{Plural(a.Count, "time")}, last {Ago(a.Last)}"), Auto)));
            }
            body.Children.Add(apps);
        }
        return Section("", "Stability", "Blue screens (stop codes), power cuts and unexpected shutdowns from the System log.", body);
    }

    private Border DrivesCard(HealthSnapshot h)
    {
        var body = Stack(10);
        if (h.Drives.Count == 0)
        {
            body.Children.Add(Muted("Windows didn't report any drives.", caption: false));
        }
        foreach (DriveHealth d in h.Drives)
        {
            string tone = d.Healthy ? "success" : d.Health == "Warning" ? "caution" : d.Health == "Unknown" ? "neutral" : "critical";
            var lines = Stack(2, Text(d.Name, "BodyStrongTextBlockStyle"),
                Muted($"{d.Media}{(d.Bus.Length > 0 ? " · " + d.Bus : "")} · {SafeDiskCleaner.Core.SizeFormat.FormatGb(d.Size)} · status {d.Status}"));
            if (_wear?.FirstOrDefault(w => w.DeviceId == d.Name) is DriveWear w)
            {
                var parts = new List<string>();
                parts.Add(w.Wear is int wear ? $"{wear}% of its rated wear used" : "wear not reported");
                if (w.Temperature is int t && t > 0)
                {
                    parts.Add($"{t} °C now" + (w.TemperatureMax is int tm && tm > 0 ? $", rated up to {tm} °C" : ""));
                }
                if (w.PowerOnHours is long hours && hours > 0)
                {
                    parts.Add($"{hours:N0} hours powered on");
                }
                if (w.ReadErrors is long re && re > 0)
                {
                    parts.Add($"{re} uncorrected read errors");
                }
                lines.Children.Add(Text(string.Join(" · ", parts)));
            }
            body.Children.Add(Columns((lines, Star), (Badge(d.Health, tone), Auto)));
        }
        if (_wear is null)
        {
            var read = Ui.Button("Read wear and temperature", async (_, _) => await ReadWearAsync());
            ToolTipService.SetToolTip(read, "Windows gives these counters to administrators only, so it asks for admin once.");
            body.Children.Add(Stack(4, Muted("SSD wear and temperature need admin. Windows asks once, and only these counters are read."), read));
        }
        return Section("", "Drives", "Health as the drive reports it to Windows.", body);
    }

    private async Task ReadWearAsync()
    {
        StatusText.Text = "Waiting for you to allow admin access…";
        string? json = await AdminTasks.RunTaskAsync("drive-wear");
        if (json is null)
        {
            StatusText.Text = "Admin access was declined.";
            return;
        }
        _wear = Health.ParseDriveWear(json);
        if (_wear.Count == 0)
        {
            ShowNotice("No counters", "The drive didn't report wear or temperature to Windows. Many USB and some SATA drives don't.");
        }
        await RenderAsync();
    }

    private Border BatteryCard(HealthSnapshot h)
    {
        var body = Stack(10);
        foreach (BatteryHealth b in h.Batteries)
        {
            string tone = b.Percent >= 80 ? "success" : b.Percent >= 60 ? "caution" : "critical";
            var lines = Stack(2,
                Text($"Holds {b.Percent:0}% of the charge it held new", "BodyStrongTextBlockStyle"),
                Muted($"{b.Name} · {b.FullChargeCapacity:N0} of {b.DesignCapacity:N0} mWh" + (b.CycleCount is int c ? $" · {c} charge cycles" : "")),
                Muted(b.Percent >= 80
                    ? "Normal wear."
                    : b.Percent >= 60
                        ? "Worn. It runs for less time than new; that is normal after a few years, and nothing needs doing yet."
                        : "Heavily worn. If it drains fast or the PC shuts down on battery, a replacement battery helps.", caption: false));
            var bar = new ProgressBar { Minimum = 0, Maximum = 100, Value = b.Percent, Margin = new Thickness(0, 4, 0, 0) };
            body.Children.Add(Columns((Stack(4, lines, bar), Star), (Badge($"{b.Percent:0}%", tone), Auto)));
        }
        body.Children.Add(Ui.Button("Open the full battery report", async (_, _) =>
        {
            StatusText.Text = "Making the battery report…";
            string? file = await Task.Run(Health.BatteryReport);
            StatusText.Text = file is null ? "Windows couldn't make the report." : "Opened the battery report.";
            if (file is not null)
            {
                Open(file);
            }
        }));
        return Section("", "Battery", "From Windows' battery report (powercfg).", body);
    }

    private Border SystemCard(HealthSnapshot h)
    {
        var body = Stack(10);
        if (h.Antivirus.Count == 0)
        {
            body.Children.Add(Row(8, Icon("", 16, "IconCaution"), Text("Windows Security didn't report an antivirus.")));
        }
        foreach (AntivirusState a in h.Antivirus)
        {
            body.Children.Add(Row(8, Icon(a.On ? "" : "", 16, a.On ? "IconSuccess" : "IconCritical"),
                Text($"{a.Name}: {(a.On ? "on" : "off")}{(a.On && !a.UpToDate ? ", definitions out of date" : "")}")));
        }
        body.Children.Add(Row(8, Icon(h.RestartPending ? "" : "", 16, h.RestartPending ? "IconCaution" : "IconSuccess"),
            Text(h.RestartPending ? "Windows is waiting for a restart to finish installing something." : "No restart pending.")));
        body.Children.Add(Row(8, Icon("", 16, "IconMuted"), Text($"Up for {Uptime(h.Uptime)} since the last restart or full shutdown.")));
        var rp = Ui.Button("Make a restore point now", async (_, _) =>
        {
            StatusText.Text = "Waiting for you to allow admin access…";
            string? output = await AdminTasks.RunTaskAsync("restore-point");
            StatusText.Text = output is null ? "Admin access was declined." : SafeUpdater.Core.Preflight.DescribeRestorePoint(output);
        });
        ToolTipService.SetToolTip(rp, "Asks for admin once. Windows skips it if it made one in the last 24 hours.");
        body.Children.Add(rp);
        return Section("", "Security and system", "Antivirus as Windows Security reports it, pending restarts and restore points.", body);
    }

    private static Border Section(string glyph, string title, string sub, UIElement body) =>
        Card(Stack(12, CardHeader(glyph, title, sub), body), 20);

    private static string Uptime(TimeSpan t) => t.TotalDays >= 1
        ? $"{(int)t.TotalDays} day{((int)t.TotalDays == 1 ? "" : "s")} {t.Hours} h"
        : $"{t.Hours} h {t.Minutes} min";
}
