using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using SafeToolbox.Core;
using static SafeToolbox.Ui;

namespace SafeToolbox.Pages;

/// <summary>Your PC at a glance. Each card says what matters now and opens its tool.</summary>
internal sealed class HomePage : PageBase
{
    private readonly Grid _grid = new() { ColumnSpacing = 16, RowSpacing = 16, Padding = new Thickness(0, 0, 0, 24) };
    private readonly List<Border> _cards = [];
    private readonly TextBlock _updNumber = Text("Checking…", "BigNumberStyle", wrap: false);
    private readonly TextBlock _updSub = Muted("", caption: false);
    private readonly StackPanel _updList = new() { Spacing = 2 };
    private readonly TextBlock _cleanNumber = Text("Scanning…", "BigNumberStyle", wrap: false);
    private readonly TextBlock _cleanSub = Muted("", caption: false);
    private readonly ProgressBar _drive = new() { Minimum = 0, Maximum = 100 };
    private readonly TextBlock _driveText = Muted("");
    private readonly StackPanel _health = new() { Spacing = 6 };
    private readonly TextBlock _bgStatus = Muted("", caption: false);
    private readonly StackPanel _recent = new() { Spacing = 8 };
    private int _columns = -1;

    public HomePage(IShell shell) : base(shell, "Home", "Your PC at a glance. Each card opens its tool.")
    {
        HideFooter();
        AddCard(CardHeader("", "Updates", "Apps, Windows, drivers and dev tools"),
            Stack(6, _updNumber, _updSub, _updList),
            Ui.Button("Review updates", (_, _) => Shell.Navigate("updates")));
        AddCard(CardHeader("", "Clean up", "Caches and leftovers, checked by the guard"),
            Stack(6, _cleanNumber, _cleanSub, _drive, _driveText),
            Ui.Button("Clean up", (_, _) => Shell.Navigate("cleanup")));
        AddCard(CardHeader("", "Health", "Crashes, drive, battery and security"),
            _health,
            Ui.Button("Open Health", (_, _) => Shell.Navigate("health")));
        AddCard(CardHeader("", "Background checks", "Notifications when something needs you"),
            _bgStatus,
            Ui.Button("Change in Settings", (_, _) => Shell.Navigate("settings")));
        AddCard(CardHeader("", "Recent activity", "Every update run and clean, from the logs"),
            _recent,
            Ui.Button("Open History", (_, _) => Shell.Navigate("history")));
        AddCard(CardHeader("", "More tools", null),
            Stack(10,
                Tool("Big files", "Your largest files, to decide what to keep.", "storage"),
                Tool("Repair", "Windows file repair, network reset, DNS flush, Explorer restart.", "repair"),
                Tool("PATH doctor", "Dead or repeated PATH entries and which python or node runs.", "path"),
                Tool("App list", "Save what's installed, to set up a new PC fast.", "apps")),
            null);

        var scroller = Scroller(_grid);
        scroller.SizeChanged += (_, e) => Layout(e.NewSize.Width);
        SetBody(scroller);

        Shell.Page<UpdatesPage>().StateChanged += () => DispatcherQueue.TryEnqueue(RefreshUpdates);
        Shell.Page<CleanupPage>().StateChanged += () => DispatcherQueue.TryEnqueue(RefreshCleanup);
    }

    public override void OnShown()
    {
        RefreshUpdates();
        RefreshCleanup();
        RefreshBackground();
        RefreshRecent();
        _ = RefreshHealthAsync();
    }

    private void AddCard(UIElement header, UIElement body, Button? action)
    {
        var content = Stack(12, header, body);
        if (action is not null)
        {
            action.HorizontalAlignment = HorizontalAlignment.Left;
            content.Children.Add(action);
        }
        Border card = Card(content, 20);
        _cards.Add(card);
        _grid.Children.Add(card);
    }

    private UIElement Tool(string title, string what, string page)
    {
        var link = LinkButton(title, (_, _) => Shell.Navigate(page));
        link.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        return Stack(0, link, Muted(what));
    }

    private void Layout(double width)
    {
        int cols = width >= 1100 ? 3 : width >= 720 ? 2 : 1;
        if (cols == _columns)
        {
            return;
        }
        _columns = cols;
        _grid.ColumnDefinitions.Clear();
        _grid.RowDefinitions.Clear();
        for (int c = 0; c < cols; c++)
        {
            _grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        }
        int rows = (_cards.Count + cols - 1) / cols;
        for (int r = 0; r < rows; r++)
        {
            _grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        }
        for (int i = 0; i < _cards.Count; i++)
        {
            Grid.SetColumn(_cards[i], i % cols);
            Grid.SetRow(_cards[i], i / cols);
        }
    }

    private void RefreshUpdates()
    {
        UpdatesPage u = Shell.Page<UpdatesPage>();
        _updList.Children.Clear();
        if (u.Checking)
        {
            _updNumber.Text = "Checking…";
            _updSub.Text = "Every source is checked at once. This takes about a minute.";
            return;
        }
        int n = u.SafeCount;
        CheckSummary? bg = CheckSummary.Load();
        if (u.LastChecked is null && bg?.UpdatesChecked != true)
        {
            _updNumber.Text = "Not checked yet";
            _updSub.Text = "Open Updates to check.";
            return;
        }
        _updNumber.Text = n == 0 ? "Up to date" : $"{n} safe update{(n == 1 ? "" : "s")}";
        _updSub.Text = (u.LastChecked is DateTime c ? $"Checked {Ago(c)}" : "") + (u.LeftOutCount > 0 ? $", {u.LeftOutCount} left out for safety" : "");
        foreach (var i in u.SafeItems.Take(4))
        {
            _updList.Children.Add(Text($"{i.DisplayName}  {i.Current} → {i.TargetVersion ?? i.Available}", "MutedCaptionStyle", wrap: false));
        }
        if (n > 4)
        {
            _updList.Children.Add(Muted($"and {n - 4} more"));
        }
    }

    private void RefreshCleanup()
    {
        CleanupPage c = Shell.Page<CleanupPage>();
        if (c.Scanning || !c.Scanned)
        {
            _cleanNumber.Text = "Scanning…";
            _cleanSub.Text = "";
        }
        else
        {
            _cleanNumber.Text = c.Selected > 0 ? $"{SafeDiskCleaner.Core.SizeFormat.Format(c.Selected)} to free" : "Nothing much to clean";
            _cleanSub.Text = c.Found > c.Selected ? $"{SafeDiskCleaner.Core.SizeFormat.Format(c.Found)} found, the rest is unticked or your call." : "Ticked by default, nothing you'd miss.";
        }
        if (SafeDiskCleaner.Core.Tools.SystemDriveUsage() is (long total, long free) && total > 0)
        {
            _drive.Value = 100.0 * (total - free) / total;
            _driveText.Text = $"C: {SafeDiskCleaner.Core.SizeFormat.FormatGb(free)} free of {SafeDiskCleaner.Core.SizeFormat.FormatGb(total)}";
        }
    }

    private async Task RefreshHealthAsync()
    {
        if (_health.Children.Count == 0)
        {
            _health.Children.Add(Muted("Reading…", caption: false));
        }
        HealthSnapshot h = await Shell.Page<HealthPage>().GetSnapshotAsync();
        _health.Children.Clear();
        int crashes = h.Crashes.Count(c => c.When >= DateTime.Now.AddDays(-30));
        _health.Children.Add(Line(crashes == 0 ? "" : "", crashes == 0 ? "IconSuccess" : "IconCaution",
            crashes == 0 ? "No crashes in the last 30 days" : $"{Plural(crashes, "crash", "crashes")} in the last 30 days"));
        foreach (DriveHealth d in h.Drives.Take(2))
        {
            _health.Children.Add(Line(d.Healthy ? "" : "", d.Healthy ? "IconSuccess" : "IconCritical", $"{d.Name}: {d.Health.ToLowerInvariant()}"));
        }
        foreach (BatteryHealth b in h.Batteries.Take(1))
        {
            _health.Children.Add(Line(b.Percent >= 80 ? "" : "", b.Percent >= 80 ? "IconSuccess" : "IconCaution", $"Battery holds {b.Percent:0}% of its new charge"));
        }
        if (h.Antivirus.FirstOrDefault() is AntivirusState av)
        {
            _health.Children.Add(Line(av.On ? "" : "", av.On ? "IconSuccess" : "IconCritical", $"{av.Name} is {(av.On ? "on" : "off")}"));
        }
        if (h.RestartPending)
        {
            _health.Children.Add(Line("", "IconCaution", "Windows is waiting for a restart"));
        }
    }

    private static UIElement Line(string glyph, string style, string text) => Row(8, Icon(glyph, 14, style), Text(text, wrap: false));

    private void RefreshBackground()
    {
        ToolboxSettings s = Shell.Settings;
        CheckSummary? last = CheckSummary.Load();
        string parts = Schedule.TaskFrequency(s) is null
            ? "Off. Turn it on to get a notification when safe updates are ready or there is a lot to clean."
            : $"On: updates {Describe(s.UpdateCheck)}, cleanup reminder {Describe(s.CleanupReminder)}.";
        if (last is not null)
        {
            parts += $" Last check {Ago(last.When)}" + (last.UpdatesChecked ? $": {Plural(last.Updates.Count, "safe update")}" : "") +
                (last.CleanupChecked ? $", {SafeDiskCleaner.Core.SizeFormat.Format(last.Cleanable)} to free" : "") + ".";
        }
        _bgStatus.Text = parts;

        static string Describe(Frequency f) => f switch { Frequency.Daily => "daily", Frequency.Weekly => "weekly", _ => "off" };
    }

    private void RefreshRecent()
    {
        _recent.Children.Clear();
        List<HistoryRun> runs = History.ReadAll().Take(4).ToList();
        if (runs.Count == 0)
        {
            _recent.Children.Add(Muted("Nothing yet. Update runs and cleans show up here.", caption: false));
            return;
        }
        foreach (HistoryRun r in runs)
        {
            _recent.Children.Add(Columns(
                (Icon(r.Kind == RunKind.Updates ? "" : "", 14, r.Problems > 0 ? "IconCaution" : "IconMuted"), Auto),
                (Stack(0, Text(r.Summary, wrap: false), Muted(Ago(r.When))), Star)));
        }
    }
}
