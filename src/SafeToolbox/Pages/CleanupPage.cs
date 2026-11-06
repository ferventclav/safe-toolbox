using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using SafeDiskCleaner.Core;
using static SafeToolbox.Ui;

namespace SafeToolbox.Pages;

/// <summary>
/// Safe Disk Cleaner, as a page. The window runs as you; the Windows group is scanned and cleaned by an elevated
/// helper after one admin prompt each time, so the rest of the toolbox never runs as admin.
/// </summary>
internal sealed class CleanupPage : PageBase
{
    private const int MaxDetailLines = 300;

    private readonly bool _admin = Tools.IsAdmin();
    private readonly Dictionary<string, bool> _checked = new(StringComparer.Ordinal);
    private readonly Dictionary<Rule, (Border Container, CheckBox Box)> _rows = [];
    private readonly Dictionary<RuleGroup, CheckBox> _groupBoxes = [];
    private readonly StackPanel _groupsPanel = new() { Spacing = 12, Padding = new Thickness(0, 0, 12, 16) };
    private readonly StackPanel _scanning = new() { Spacing = 12, Width = 360, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, Visibility = Visibility.Collapsed };
    private readonly TextBlock _scanningText = Text("Scanning…", "BodyStrongTextBlockStyle", wrap: false);
    private readonly ProgressBar _scanBar = new() { Minimum = 0, Maximum = 1 };
    private readonly ScrollViewer _list;
    private readonly Grid _body = new() { Padding = new Thickness(24, 8, 24, 0) };
    private readonly Border _details;
    private readonly TextBlock _detailsTitle = Text("Details", "SubtitleTextBlockStyle");
    private readonly TextBlock _detailsWhy = Muted("", caption: false);
    private readonly StackPanel _detailsList = new() { Spacing = 6, Padding = new Thickness(0, 4, 12, 4) };
    private readonly TextBlock _driveText = Text("", wrap: false);
    private readonly ProgressBar _driveBar = new() { Minimum = 0, Maximum = 100 };
    private readonly TextBlock _lastCleaned = Muted("");
    private readonly AppBarButton _rescan;
    private readonly AppBarButton _protect;
    private readonly Button _clean;
    private ScanResult? _scan;
    private string? _selectedId;
    private bool _busy;
    private bool _started;
    private bool _adminScanned;
    private CancellationTokenSource? _stop;
    private bool? _wide;

    public event Action? StateChanged;

    public bool Scanning { get; private set; }
    public long Selected => SelectedRules().Sum(r => r.Total);
    public long Found => _scan?.Rules.Where(r => r.CanClean).Sum(r => r.Total) ?? 0;
    public bool Scanned => _scan is not null;

    public CleanupPage(IShell shell) : base(shell, "Clean up",
        "Covers what Windows Disk Cleanup does, plus app caches it never sees.",
        "Installed tools, models and your files are never touched.")
    {
        var drive = Columns((Text(KnownPaths.SystemDrive.TrimEnd('\\'), "BodyStrongTextBlockStyle", wrap: false), Star), (_driveText, Auto));
        HeaderRight.Children.Add(drive);
        HeaderRight.Children.Add(_driveBar);
        HeaderRight.Children.Add(_lastCleaned);
        HeaderRight.Width = 300;

        _scanning.Children.Add(new ProgressRing { IsActive = true, Width = 40, Height = 40, HorizontalAlignment = HorizontalAlignment.Center });
        _scanningText.HorizontalAlignment = HorizontalAlignment.Center;
        _scanning.Children.Add(_scanningText);
        _scanning.Children.Add(_scanBar);
        AutomationProperties.SetName(_scanBar, "Scan progress");

        _list = new ScrollViewer { Content = _groupsPanel, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        var detailsGrid = new Grid { RowSpacing = 8 };
        detailsGrid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        detailsGrid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        detailsGrid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        detailsGrid.Children.Add(_detailsTitle);
        Grid.SetRow(_detailsWhy, 1);
        detailsGrid.Children.Add(_detailsWhy);
        var ds = new ScrollViewer { Content = _detailsList, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        Grid.SetRow(ds, 2);
        detailsGrid.Children.Add(ds);
        _details = Card(detailsGrid);
        _details.Margin = new Thickness(0, 0, 0, 16);

        _body.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        _body.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(380) });
        _body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        _body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(0) });
        _body.Children.Add(_list);
        _body.Children.Add(_scanning);
        _body.Children.Add(_details);
        _body.SizeChanged += (_, e) => ApplyLayout(e.NewSize.Width, e.NewSize.Height);
        SetBody(_body);

        _rescan = AddCommand("Rescan", "\uE72C", async (_, _) => await ScanAsync(), key: Windows.System.VirtualKey.F5);
        _protect = AddCommand("Protect a folder", "\uE72E", Protect_Click);
        _clean = AddPrimary("Clean selected", Clean_Click);
        AddCommand("Open the cleanup log", "\uE81C", (_, _) => OpenLog(), secondary: true);
        AddCommand("Edit protected folders (protect.txt)", "\uE70F", (_, _) => Open(UserProtection.EnsureFile()), secondary: true);
        AddCommand("Windows storage settings", "\uE713", (_, _) => Open("ms-settings:storagesense"), secondary: true);

        UpdateDriveHeader();
        UpdateLastCleaned();
        ShowDetails(null);
    }

    public void EnsureStarted()
    {
        if (!_started)
        {
            _started = true;
            _ = ScanAsync();
        }
    }

    protected override Task OnFirstShownAsync()
    {
        EnsureStarted();
        return Task.CompletedTask;
    }

    public override void OnShown() => UpdateDriveHeader();

    // ------------------------------------------------------------------ scan and clean

    public async Task ScanAsync()
    {
        if (_busy)
        {
            return;
        }
        _started = true;
        Scanning = true;
        _groupsPanel.Visibility = Visibility.Collapsed;
        _scanning.Visibility = Visibility.Visible;
        SetBusy(true, "Scanning…");
        _scanBar.Value = 0;
        var progress = new Progress<ScanProgress>(p =>
        {
            _scanBar.Maximum = Math.Max(1, p.Total);
            _scanBar.Value = p.Done;
            _scanningText.Text = $"Scanning… {p.Current}";
        });
        try
        {
            bool admin = _admin;
            _scan = await Task.Run(() => Scanner.Scan(admin, progress));
            _adminScanned = _admin;
            foreach (Rule r in _scan.Rules)
            {
                _checked[r.Id] = Shell.Settings.CleanupTick(r.Id, r.DefaultChecked);
            }
            StatusText.Text = $"Found {SizeFormat.Format(Found)} in {_scan.Elapsed.TotalSeconds:0.0} s.";
        }
        catch (Exception ex)
        {
            RunLog.WriteError(ex.ToString());
            ShowError("The scan failed", ex.Message);
            StatusText.Text = "";
        }
        finally
        {
            Scanning = false;
            _scanning.Visibility = Visibility.Collapsed;
            _groupsPanel.Visibility = Visibility.Visible;
            SetBusy(false);
            Render();
            UpdateDriveHeader();
            StateChanged?.Invoke();
        }
    }

    /// <summary>Sizes the Windows group through the elevated helper (one admin prompt).</summary>
    private async Task ScanAdminAsync()
    {
        if (_scan is null || _busy)
        {
            return;
        }
        SetBusy(true, "Waiting for you to allow admin access…");
        WorkBar.IsIndeterminate = true;
        AdminResult? result = await RunAdminJobAsync("scan", _scan.Rules.Where(r => r.NeedsAdmin).Select(r => r.Id).ToList());
        WorkBar.IsIndeterminate = false;
        if (result is null)
        {
            SetBusy(false, "Admin access was declined, so the Windows group wasn't scanned.");
            return;
        }
        if (result.Error is not null)
        {
            ShowError("The Windows scan failed", result.Error);
        }
        AdminWorker.Apply(result, _scan.Rules);
        _adminScanned = true;
        foreach (Rule r in _scan.Rules.Where(r => r.NeedsAdmin))
        {
            _checked[r.Id] = Shell.Settings.CleanupTick(r.Id, r.DefaultChecked);
        }
        SetBusy(false, $"Windows group scanned: {SizeFormat.Format(_scan.Rules.Where(r => r.NeedsAdmin && r.CanClean).Sum(r => r.Total))} found.");
        Render();
        StateChanged?.Invoke();
    }

    private async Task<AdminResult?> RunAdminJobAsync(string mode, List<string> ruleIds, Action<string>? progress = null)
    {
        string stamp = DateTime.Now.ToString("yyyyMMdd-HHmmss");
        string dir = Core.ToolboxPaths.Jobs;
        var job = new AdminJob
        {
            Mode = mode,
            RuleIds = ruleIds,
            ResultFile = Path.Combine(dir, $"clean-result-{stamp}.json"),
            ProgressFile = Path.Combine(dir, $"clean-progress-{stamp}.txt"),
            LogFile = RunLog.FilePath,
            StopFile = Path.Combine(dir, $"clean-stop-{stamp}.flag"),
        };
        string jobFile = Path.Combine(dir, $"clean-job-{stamp}.json");
        await File.WriteAllTextAsync(jobFile, AdminWorker.Serialize(job));
        System.Diagnostics.Process? p = AdminWorker.Start(jobFile);
        if (p is null)
        {
            return null;
        }
        using (p)
        {
            long read = 0;
            while (!p.HasExited)
            {
                await Task.Delay(400);
                if (_stop?.IsCancellationRequested == true && !File.Exists(job.StopFile))
                {
                    await File.WriteAllTextAsync(job.StopFile, "stop");
                }
                read = Follow(job.ProgressFile, read, progress);
            }
            Follow(job.ProgressFile, read, progress);
        }
        AdminResult? result = AdminWorker.ReadResult(job.ResultFile);
        foreach (string f in new[] { jobFile, job.ResultFile, job.ProgressFile, job.StopFile })
        {
            try
            {
                File.Delete(f);
            }
            catch (IOException)
            {
            }
        }
        return result ?? new AdminResult { Error = "The admin helper didn't report back. Details may be in error.log." };
    }

    private static long Follow(string file, long from, Action<string>? progress)
    {
        if (progress is null || !File.Exists(file))
        {
            return from;
        }
        try
        {
            using var fs = new FileStream(file, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
            fs.Seek(from, SeekOrigin.Begin);
            using var sr = new StreamReader(fs);
            string? line, last = null;
            while ((line = sr.ReadLine()) is not null)
            {
                last = line;
            }
            if (last is not null)
            {
                progress(last);
            }
            return fs.Position;
        }
        catch (IOException)
        {
            return from;
        }
    }

    private async void Clean_Click(object sender, RoutedEventArgs e)
    {
        if (_stop is not null)
        {
            _stop.Cancel();
            _clean.IsEnabled = false;
            _clean.Content = "Stopping…";
            return;
        }
        if (_scan is null || _busy)
        {
            return;
        }
        List<Rule> pick = SelectedRules();
        if (pick.Count == 0)
        {
            return;
        }
        int count = pick.Sum(r => r.Items.Count);
        long bytes = pick.Sum(r => r.Total);
        string text = $"Delete {count} item{(count == 1 ? "" : "s")}, about {SizeFormat.Format(bytes)}? This can't be undone.";
        List<Rule> adminRules = _admin ? [] : pick.Where(r => r.NeedsAdmin).ToList();
        if (adminRules.Count > 0)
        {
            text += "\n\nWindows will ask for admin once, for the Windows items.";
        }
        if (pick.Any(r => r.Items.Any(i => i.Handler is not null)))
        {
            text += "\n\nWindows' own Disk Cleanup runs too and can take 10+ minutes.";
        }
        if (await Shell.AskAsync("Clean selected?", text, "Delete") != ContentDialogResult.Primary)
        {
            return;
        }

        CloseBars();
        _stop = new CancellationTokenSource();
        SetBusy(true, "Cleaning…");
        _clean.IsEnabled = true;
        _clean.Content = "Stop";
        AutomationProperties.SetName(_clean, "Stop cleaning after the item in progress");

        long freed = 0;
        int items = 0, problems = 0, stopped = 0;
        string? error = null;
        try
        {
            if (adminRules.Count > 0)
            {
                StatusText.Text = "Waiting for you to allow admin access…";
                AdminResult? ar = await RunAdminJobAsync("clean", adminRules.Select(r => r.Id).ToList(),
                    line => DispatcherQueue.TryEnqueue(() => StatusText.Text = "Cleaning Windows items… " + line.Split('\t').Last()));
                if (ar is null)
                {
                    problems += adminRules.Sum(r => r.Items.Count);
                    error = "Admin access was declined, so the Windows items were left alone.";
                }
                else
                {
                    freed += ar.Freed;
                    items += ar.Items;
                    problems += ar.Problems;
                    stopped += ar.Stopped;
                    error = ar.Error;
                }
            }
            List<Rule> local = pick.Except(adminRules).ToList();
            if (local.Count > 0 && !_stop.IsCancellationRequested)
            {
                var progress = new Progress<CleanProgress>(p =>
                {
                    WorkBar.Maximum = Math.Max(1, p.Total);
                    WorkBar.Value = p.Done;
                    StatusText.Text = p.Current.Length > 0 ? $"Cleaning… {p.Current.Trim()}" : "Measuring freed space…";
                });
                CancellationToken token = _stop.Token;
                CleanResult result = await Task.Run(() => Cleaner.Clean(local, progress, stop: token));
                freed += result.Freed;
                items += result.Items;
                problems += result.Problems;
                stopped += result.Stopped;
            }
            else if (local.Count > 0)
            {
                stopped += local.Sum(r => r.Items.Count);
            }
        }
        catch (Exception ex)
        {
            RunLog.WriteError(ex.ToString());
            error = ex.Message;
        }
        _stop = null;
        AutomationProperties.SetName(_clean, "Clean selected");
        SetBusy(false);

        var openLog = new HyperlinkButton { Content = "Open the log" };
        openLog.Click += (_, _) => OpenLog();
        string title = freed > 0 ? $"Freed {SizeFormat.Format(freed)} (measured on the drive)" : "Done. The drive's free space did not grow measurably.";
        var msg = new List<string>();
        msg.Add(problems > 0 ? $"{problems} of {items} items were skipped, refused or in use. The log lists each one." : $"{items} item{(items == 1 ? "" : "s")} cleaned.");
        if (stopped > 0)
        {
            msg.Add($"Stopped: {stopped} not started.");
        }
        if (error is not null)
        {
            msg.Add(error);
        }
        ShowResult(title, string.Join(" ", msg), problems > 0 || error is not null ? InfoBarSeverity.Warning : InfoBarSeverity.Success, openLog);
        UpdateLastCleaned();
        bool hadAdmin = _adminScanned;
        await ScanAsync();
        if (hadAdmin && !_admin && adminRules.Count > 0)
        {
            StatusText.Text += " The Windows group needs another admin scan to show what's left.";
        }
    }

    private List<Rule> SelectedRules() => _scan?.Rules
        .Where(r => r.CanClean && _checked.GetValueOrDefault(r.Id))
        .ToList() ?? [];

    private void SetBusy(bool busy, string? status = null)
    {
        _busy = busy;
        _rescan.IsEnabled = !busy;
        _protect.IsEnabled = !busy;
        _list.IsEnabled = !busy;
        WorkBar.Visibility = busy && _scanning.Visibility != Visibility.Visible ? Visibility.Visible : Visibility.Collapsed;
        WorkBar.Value = 0;
        if (status is not null)
        {
            StatusText.Text = status;
        }
        UpdateCleanButton();
    }

    // ------------------------------------------------------------------ list

    private void Render()
    {
        _groupsPanel.Children.Clear();
        _rows.Clear();
        _groupBoxes.Clear();
        if (_scan is null)
        {
            return;
        }
        foreach (RuleGroup g in RuleGroups.Order)
        {
            List<Rule> rules = _scan.Rules
                .Where(r => r.Group == g && r.Visible)
                .OrderByDescending(r => r.CanClean)
                .ThenByDescending(r => r.Total)
                .ToList();
            if (rules.Count > 0 || g == RuleGroup.Windows)
            {
                _groupsPanel.Children.Add(BuildGroup(g, rules));
            }
        }
        _groupsPanel.Children.Add(BuildKept());
        ShowDetails(_scan.Rules.FirstOrDefault(r => r.Id == _selectedId));
        UpdateCleanButton();
    }

    private UIElement BuildGroup(RuleGroup g, List<Rule> rules)
    {
        long total = rules.Where(r => r.CanClean).Sum(r => r.Total);
        string title = RuleGroups.Title(g);

        var box = new CheckBox { MinWidth = 0, Padding = new Thickness(0), VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetName(box, $"Select everything in {title}");
        box.Click += (_, _) => OnGroupBoxClick(g);
        _groupBoxes[g] = box;

        string blurb = g == RuleGroup.Windows && !_adminScanned
            ? "What Disk Cleanup's system-file mode clears. Scanning it needs admin."
            : g == RuleGroup.Windows ? "What Disk Cleanup's system-file mode clears" : RuleGroups.Blurb(g, true);
        var text = Stack(0, Text(title, "BodyStrongTextBlockStyle"), Muted(blurb));
        text.VerticalAlignment = VerticalAlignment.Center;
        UIElement right = Text(total > 0 ? SizeFormat.Format(total) : "", "BodyStrongTextBlockStyle", wrap: false);
        if (g == RuleGroup.Windows && !_adminScanned)
        {
            var scan = new Button { Content = "Scan with admin" };
            AutomationProperties.SetName(scan, "Scan the Windows group with admin");
            ToolTipService.SetToolTip(scan, "Windows asks for admin once. Only Windows' own folders are scanned this way.");
            scan.Click += async (_, _) => await ScanAdminAsync();
            right = scan;
        }
        Grid header = Columns((box, Auto), (Icon(GroupGlyph(g), 20), Auto), (text, Star), (right, Auto));
        header.Padding = new Thickness(0, 10, 0, 10);

        var rowsPanel = new StackPanel { Spacing = 2 };
        foreach (Rule r in rules)
        {
            rowsPanel.Children.Add(BuildRow(r));
        }
        var expander = new Expander
        {
            Header = header,
            Content = rowsPanel,
            IsExpanded = rules.Count > 0,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
        };
        AutomationProperties.SetName(expander, total > 0 ? $"{title}, {SizeFormat.Format(total)}" : title);
        RefreshGroupBox(g);
        return expander;
    }

    private UIElement BuildRow(Rule r)
    {
        bool can = r.CanClean;
        string sizeText = SizeText(r);
        string description = Describe(r);

        var box = new CheckBox
        {
            MinWidth = 0,
            Padding = new Thickness(0),
            VerticalAlignment = VerticalAlignment.Center,
            IsEnabled = can,
            IsChecked = can && _checked.GetValueOrDefault(r.Id),
        };
        AutomationProperties.SetName(box, $"{r.Title}, {sizeText}");
        AutomationProperties.SetHelpText(box, description);
        box.Click += (_, _) =>
        {
            bool on = box.IsChecked == true;
            _checked[r.Id] = on;
            Shell.Settings.RememberTick(r.Id, on, r.DefaultChecked);
            Shell.Settings.Save();
            RefreshGroupBox(r.Group);
            UpdateCleanButton();
        };

        var lines = Stack(0,
            Text(r.Title, can ? "BodyTextBlockStyle" : "DisabledBodyStyle", wrap: false),
            Text(description, can ? "MutedCaptionStyle" : r.Blocked is not null && r.Blocked != Scanner.NeedsAdmin ? "CautionCaptionStyle" : "DisabledCaptionStyle"));
        lines.VerticalAlignment = VerticalAlignment.Center;
        var size = Text(sizeText, can ? "BodyStrongTextBlockStyle" : "DisabledBodyStyle", wrap: false);
        size.VerticalAlignment = VerticalAlignment.Center;
        size.TextAlignment = TextAlignment.Right;
        size.MinWidth = 72;

        var container = new Border { Child = Columns((box, Auto), (lines, Star), (size, Auto)), Style = S(r.Id == _selectedId ? "SelectedRowStyle" : "RowStyle") };
        container.Tapped += (_, _) => Select(r);
        var host = new ContentControl
        {
            Content = container,
            IsTabStop = !can,
            UseSystemFocusVisuals = true,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
        };
        AutomationProperties.SetName(host, $"{r.Title}, {description}");
        host.GotFocus += (_, _) => Select(r);
        _rows[r] = (container, box);
        return host;
    }

    private UIElement BuildKept()
    {
        int folders = _scan?.Guard.InUsePaths.Count ?? 0;
        var text = Stack(0, Text("Kept on purpose", "BodyStrongTextBlockStyle"),
            Muted($"Never cleaned. Every delete is also checked against {folders} install and PATH folders on this PC."));
        text.VerticalAlignment = VerticalAlignment.Center;
        Grid header = Columns((Icon("\uEA18", 20), Auto), (text, Star));
        header.Padding = new Thickness(0, 10, 0, 10);
        var rows = new StackPanel { Spacing = 10, Padding = new Thickness(12, 4, 12, 4) };
        foreach (Kept.Entry k in Kept.List)
        {
            rows.Children.Add(Columns((Icon("\uE72E", 14), Auto), (Stack(0, Text(k.What), Muted(k.Why)), Star)));
        }
        var expander = new Expander { Header = header, Content = rows, IsExpanded = false, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        AutomationProperties.SetName(expander, "Kept on purpose");
        return expander;
    }

    private void OnGroupBoxClick(RuleGroup g)
    {
        List<Rule> rules = CleanableIn(g);
        bool allOn = rules.Count > 0 && rules.All(r => _checked.GetValueOrDefault(r.Id));
        foreach (Rule r in rules)
        {
            _checked[r.Id] = !allOn;
            Shell.Settings.RememberTick(r.Id, !allOn, r.DefaultChecked);
            if (_rows.TryGetValue(r, out var v))
            {
                v.Box.IsChecked = !allOn;
            }
        }
        Shell.Settings.Save();
        RefreshGroupBox(g);
        UpdateCleanButton();
    }

    private void RefreshGroupBox(RuleGroup g)
    {
        if (!_groupBoxes.TryGetValue(g, out CheckBox? box))
        {
            return;
        }
        List<Rule> rules = CleanableIn(g);
        int on = rules.Count(r => _checked.GetValueOrDefault(r.Id));
        box.IsEnabled = rules.Count > 0;
        box.IsChecked = rules.Count == 0 || on == 0 ? false : on == rules.Count ? true : null;
    }

    private List<Rule> CleanableIn(RuleGroup g) => _scan?.Rules.Where(r => r.Group == g && r.CanClean).ToList() ?? [];

    private void UpdateCleanButton()
    {
        if (_stop is not null)
        {
            return;
        }
        List<Rule> pick = SelectedRules();
        long total = pick.Sum(r => r.Total);
        bool unknown = pick.Any(r => r.HasUnknownSize);
        string label = pick.Count == 0 ? "Clean selected" : total > 0 ? $"Clean selected · {SizeFormat.Format(total)}{(unknown ? " +" : "")}" : "Clean selected";
        _clean.Content = label;
        _clean.IsEnabled = pick.Count > 0 && !_busy;
        AutomationProperties.SetName(_clean, label.Replace("·", ",", StringComparison.Ordinal));
        StateChanged?.Invoke();
    }

    // ------------------------------------------------------------------ details pane

    private void Select(Rule r)
    {
        if (_selectedId == r.Id)
        {
            return;
        }
        _selectedId = r.Id;
        foreach ((Rule rule, var v) in _rows)
        {
            v.Container.Style = S(rule.Id == r.Id ? "SelectedRowStyle" : "RowStyle");
        }
        ShowDetails(r);
    }

    private void ShowDetails(Rule? r)
    {
        _detailsList.Children.Clear();
        if (r is null)
        {
            _detailsTitle.Text = "Details";
            _detailsWhy.Text = "Select a row to see exactly which folders it covers, and anything the safety check refused.";
            _detailsList.Children.Add(Muted("Every run is logged to runs.csv, and the History page shows each one."));
            return;
        }
        _detailsTitle.Text = r.Title;
        _detailsWhy.Text = r.Blocked switch
        {
            null => r.Why,
            Scanner.NeedsAdmin => r.Why + " Needs admin: use Scan with admin on the Windows group.",
            string b => $"{r.Why} {b}.",
        };
        int shown = 0;
        foreach (Item it in r.Items.OrderByDescending(i => i.Size ?? -1))
        {
            if (shown++ == MaxDetailLines)
            {
                _detailsList.Children.Add(Muted($"…and {r.Items.Count - MaxDetailLines} more."));
                break;
            }
            string? path = it.Path is not null && !string.Equals(it.Path, it.Label, StringComparison.Ordinal) ? it.Path : null;
            AddDetailLine(it.Label.Trim(), path, it.Size is null ? "after the run" : SizeFormat.Format(it.Size));
        }
        if (r.Skipped.Count > 0)
        {
            var head = Text("Left alone", "BodyStrongTextBlockStyle");
            head.Margin = new Thickness(0, shown > 0 ? 12 : 0, 0, 0);
            _detailsList.Children.Add(head);
            foreach (Skip s in r.Skipped)
            {
                AddDetailLine(s.What, s.Why, "", muted: true);
            }
        }
        if (r.Items.Count == 0 && r.Skipped.Count == 0)
        {
            _detailsList.Children.Add(Muted("Nothing found."));
        }
    }

    private void AddDetailLine(string title, string? sub, string size, bool muted = false)
    {
        var lines = Stack(0, Text(title, muted ? "MutedBodyStyle" : "BodyTextBlockStyle", select: true));
        if (!string.IsNullOrEmpty(sub))
        {
            lines.Children.Add(Text(sub, "MutedCaptionStyle", select: true));
        }
        var s = Text(size, wrap: false);
        s.VerticalAlignment = VerticalAlignment.Top;
        _detailsList.Children.Add(Columns((lines, Star), (s, Auto)));
    }

    // ------------------------------------------------------------------ header

    private void UpdateDriveHeader()
    {
        if (Tools.SystemDriveUsage() is (long total, long free) && total > 0)
        {
            _driveText.Text = $"{SizeFormat.FormatGb(free)} free of {SizeFormat.FormatGb(total)}";
            double used = 100.0 * (total - free) / total;
            _driveBar.Value = used;
            AutomationProperties.SetName(_driveBar, $"Drive {KnownPaths.SystemDrive.TrimEnd('\\')}: {used:0} percent used");
        }
    }

    private void UpdateLastCleaned()
    {
        if (RunLog.ReadLast() is (DateTime when, long freed))
        {
            _lastCleaned.Text = freed > 0 ? $"Last cleaned {Ago(when)}, freed {SizeFormat.Format(freed)}" : $"Last cleaned {Ago(when)}";
        }
        else
        {
            _lastCleaned.Text = "Not cleaned with this app yet";
        }
    }

    private void ApplyLayout(double width, double height)
    {
        bool wide = width >= 900;
        if (!wide)
        {
            _body.RowDefinitions[1].Height = new GridLength(Math.Clamp(height * 0.3, 140, 280));
        }
        if (_wide == wide)
        {
            return;
        }
        _wide = wide;
        _body.ColumnDefinitions[1].Width = new GridLength(wide ? 380 : 0);
        _body.ColumnSpacing = wide ? 16 : 0;
        _body.RowSpacing = wide ? 0 : 12;
        if (wide)
        {
            _body.RowDefinitions[1].Height = new GridLength(0);
        }
        Grid.SetRow(_details, wide ? 0 : 1);
        Grid.SetColumn(_details, wide ? 1 : 0);
    }

    // ------------------------------------------------------------------ commands

    private async void Protect_Click(object sender, RoutedEventArgs e)
    {
        string? folder;
        try
        {
            var picker = new Microsoft.Windows.Storage.Pickers.FolderPicker(Shell.WindowId);
            var picked = await picker.PickSingleFolderAsync();
            if (picked is null)
            {
                return;
            }
            folder = picked.Path;
        }
        catch (Exception ex)
        {
            RunLog.WriteError("Folder picker failed: " + ex);
            var box = new TextBox { PlaceholderText = @"For example D:\Games\Mods or %USERPROFILE%\Mods" };
            AutomationProperties.SetName(box, "Folder to protect");
            folder = await Shell.AskAsync("Protect a folder", box, "Protect", primaryIsDefault: true) == ContentDialogResult.Primary
                ? Environment.ExpandEnvironmentVariables(box.Text.Trim().Trim('"'))
                : null;
        }
        if (string.IsNullOrWhiteSpace(folder))
        {
            return;
        }
        if (!Directory.Exists(folder))
        {
            ShowError("Folder not found", folder);
            return;
        }
        try
        {
            UserProtection.Add(folder);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or ArgumentException)
        {
            ShowError("Could not update protect.txt", ex.Message);
            return;
        }
        ShowNotice("Folder protected", $"{folder} and everything in it will be left alone. The list is in protect.txt.");
        await ScanAsync();
    }

    private void OpenLog()
    {
        if (!File.Exists(RunLog.FilePath))
        {
            ShowNotice("No log yet", $"The log is created the first time you clean: {RunLog.FilePath}");
            return;
        }
        Open(RunLog.FilePath);
    }

    // ------------------------------------------------------------------ helpers

    private static string SizeText(Rule r)
    {
        if (r.Items.Count == 0)
        {
            return "";
        }
        if (r.Total == 0)
        {
            return r.HasUnknownSize ? "Size shows after the run" : "";
        }
        return SizeFormat.Format(r.Total) + (r.HasUnknownSize ? " +" : "");
    }

    private static string Describe(Rule r)
    {
        if (r.Blocked == Scanner.NeedsAdmin)
        {
            return "Needs admin. Use Scan with admin on this group.";
        }
        if (r.Blocked is not null)
        {
            return r.Blocked;
        }
        string d = r.Group == RuleGroup.Apps ? $"{r.Items.Count} cache folder{(r.Items.Count == 1 ? "" : "s")}" : r.Why;
        if (r.Skipped.Count > 0)
        {
            d += $"  ({r.Skipped.Count} left alone)";
        }
        return d;
    }

    private static string GroupGlyph(RuleGroup g) => g switch
    {
        RuleGroup.Leftovers => "\uE7B8",
        RuleGroup.Apps => "\uE774",
        RuleGroup.YourCall => "\uE74D",
        RuleGroup.Windows => "\uE770",
        _ => "\uE8B7",
    };
}
