using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using SafeUpdater.Core;
using static SafeToolbox.Ui;

namespace SafeToolbox.Pages;

/// <summary>Safe Updater, as a page: every source checked at once, only safe updates listed, one admin prompt.</summary>
internal sealed class UpdatesPage : PageBase
{
    private static readonly string SmokeScript = Path.Combine(AppContext.BaseDirectory, "Assets", "smoke-test.py");

    private readonly Settings _settings = Settings.Load();
    private readonly Dictionary<UpdateSource, GroupView> _groups = [];
    private readonly List<UpdateItem> _items = [];
    private readonly Dictionary<UpdateSource, SourceResult> _results = [];
    private readonly Dictionary<string, bool> _manual = [];
    private readonly Dictionary<string, List<NotesLink>> _wingetNotes = new(StringComparer.OrdinalIgnoreCase);
    private readonly StackPanel _groupsPanel = new() { Spacing = 12, Padding = new Thickness(0, 0, 12, 16) };
    private readonly Expander _hiddenExpander;
    private readonly StackPanel _hiddenList = new() { Spacing = 12 };
    private readonly Grid _body = new() { Padding = new Thickness(24, 8, 24, 0) };
    private readonly Border _details;
    private readonly TextBlock _detailsTitle = Text("Details", "SubtitleTextBlockStyle");
    private readonly TextBlock _detailsWhy = Muted("", caption: false);
    private readonly StackPanel _detailsList = new() { Spacing = 10, Padding = new Thickness(0, 4, 12, 4) };
    private readonly TextBlock _summary = Text("Not checked yet", "SubtitleTextBlockStyle");
    private readonly TextBlock _selected = Text("");
    private readonly TextBlock _lastChecked = Muted("");
    private readonly InfoBar _adminBar = new()
    {
        Severity = InfoBarSeverity.Informational,
        IsClosable = false,
        IsOpen = false,
        Message = "Windows will ask for admin once, for the app and Windows updates. Everything else runs as you.",
    };
    private readonly AppBarButton _checkButton;
    private readonly AppBarToggleButton _majorsToggle;
    private readonly Button _updateButton;
    private bool _busy;
    private bool _started;
    private bool _rustupPresent;
    private int _pendingChecks;
    private RowView? _shownRow;
    private string? _lastRunLog;
    private UpdateRun? _run;
    private bool? _wide;

    public event Action? StateChanged;

    public bool Checking => _pendingChecks > 0;
    public bool Busy => _busy;
    public int SafeCount => _items.Count(i => i.Selectable && !i.Hidden);
    public int LeftOutCount => _items.Count(i => i.Hidden);
    public DateTime? LastChecked => _settings.LastChecked;
    public DateTime? LastUpdated => _settings.LastUpdated;
    public IEnumerable<UpdateItem> SafeItems => _items.Where(i => i.Selectable && !i.Hidden);
    public Settings UpdaterSettings => _settings;

    public UpdatesPage(IShell shell) : base(shell, "Updates",
        "Apps, Windows, drivers and your dev tools, checked in one place.",
        "Only updates that are safe to install are listed. Anything that could break something is left out, and the end of the list says why.")
    {
        HeaderRight.Children.Add(_summary);
        HeaderRight.Children.Add(_selected);
        HeaderRight.Children.Add(_lastChecked);
        AddBar(_adminBar);

        foreach (SourceCheck s in Checker.Sources)
        {
            var g = new GroupView(this, s.Source);
            _groups[s.Source] = g;
            _groupsPanel.Children.Add(g.Card);
        }
        _hiddenExpander = new Expander
        {
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Content = _hiddenList,
            Visibility = Visibility.Collapsed,
        };
        _groupsPanel.Children.Add(_hiddenExpander);

        var list = new ScrollViewer { Content = _groupsPanel, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        var detailsGrid = new Grid { RowSpacing = 8 };
        detailsGrid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        detailsGrid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        detailsGrid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        detailsGrid.Children.Add(_detailsTitle);
        Grid.SetRow(_detailsWhy, 1);
        detailsGrid.Children.Add(_detailsWhy);
        var detailsScroll = new ScrollViewer { Content = _detailsList, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        Grid.SetRow(detailsScroll, 2);
        detailsGrid.Children.Add(detailsScroll);
        _details = Card(detailsGrid);
        _details.Margin = new Thickness(0, 0, 0, 16);

        _body.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        _body.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(380) });
        _body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        _body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(0) });
        _body.Children.Add(list);
        _body.Children.Add(_details);
        _body.SizeChanged += (_, e) => ApplyLayout(e.NewSize.Width, e.NewSize.Height);
        SetBody(_body);

        _checkButton = AddCommand("Check again", "", async (_, _) => await CheckAsync(), key: Windows.System.VirtualKey.F5);
        _majorsToggle = AddToggle("Include major versions", "", _settings.IncludeMajors, (_, _) => MajorsChanged());
        _updateButton = AddPrimary("Update selected", Update_Click);
        AddCommand("Roll back the last Python update", "", Rollback_Click, secondary: true);
        AddSeparator();
        AddCommand("Open the NVIDIA app", "", (_, _) =>
        {
            if (!NvidiaSource.OpenApp())
            {
                ShowNotice("NVIDIA app not found", "It isn't in the usual folder. Open it from the Start menu.");
            }
        }, secondary: true);
        AddCommand("Windows Update settings", "", (_, _) => Open("ms-settings:windowsupdate"), secondary: true);
        AddCommand("Open the update logs", "", (_, _) => Open(AppPaths.Logs), secondary: true);

        ShowDetailsIntro();
        UpdateSummary();
    }

    public void EnsureStarted()
    {
        if (!_started)
        {
            _started = true;
            _ = CheckAsync();
        }
    }

    protected override Task OnFirstShownAsync()
    {
        EnsureStarted();
        return Task.CompletedTask;
    }

    // ---------------------------------------------------------------- checking

    public async Task CheckAsync()
    {
        if (_busy)
        {
            return;
        }
        _started = true;
        SetBusy(true, "Checking apps, Windows, drivers and dev tools…");
        WorkBar.IsIndeterminate = true;
        CloseBars();
        _items.Clear();
        _results.Clear();
        _manual.Clear();
        _rustupPresent = Tools.Rustup() is not null;
        _pendingChecks = Checker.Sources.Count;
        foreach (GroupView g in _groups.Values)
        {
            g.SetChecking();
        }
        RebuildHidden();
        UpdateSummary();

        await Task.WhenAll(Checker.Sources.Select(async s =>
        {
            SourceResult res;
            try
            {
                res = await Task.Run(() => s.Check(CancellationToken.None));
            }
            catch (Exception ex)
            {
                RunLog.WriteError(ex.ToString());
                res = new SourceResult { Source = s.Source, Failed = true, Message = "The check failed: " + ex.Message };
            }
            _pendingChecks--;
            _results[res.Source] = res;
            Reapply();
        }));

        _settings.LastChecked = DateTime.Now;
        _settings.Save();
        WorkBar.IsIndeterminate = false;
        SetBusy(false, _items.Any(i => !i.Hidden && i.Selectable) ? "Untick anything you'd rather leave, then press Update selected." : "Nothing safe to update.");

        List<string> warnings = Preflight.Warnings();
        if (warnings.Count > 0)
        {
            ShowNotice("Before you update", string.Join(" ", warnings), InfoBarSeverity.Warning);
        }
    }

    /// <summary>Runs the rules over every group together, keeps the ticks you changed by hand, and redraws.</summary>
    private void Reapply()
    {
        _items.Clear();
        foreach (SourceResult res in _results.Values)
        {
            _items.AddRange(res.Items);
        }
        IReadOnlyCollection<string> choco = _results.TryGetValue(UpdateSource.Chocolatey, out SourceResult? c) ? c.Managed : [];
        Policy.ApplyAll(_items, PolicyContext.From(_settings, choco, _rustupPresent));
        foreach (UpdateItem i in _items.Where(i => !i.Hidden && i.Selectable))
        {
            if (_manual.TryGetValue(i.Key, out bool on))
            {
                i.Selected = on;
            }
        }
        foreach ((UpdateSource source, SourceResult res) in _results)
        {
            _groups[source].Show(res);
        }
        RebuildHidden();
        UpdateSummary();
    }

    private void RememberChoice(UpdateItem item) => _manual[item.Key] = item.Selected;

    private void SetIgnored(UpdateItem item, bool ignored)
    {
        if (ignored)
        {
            _settings.Ignored.Add(item.IgnoreKey);
        }
        else
        {
            _settings.Ignored.Remove(item.IgnoreKey);
            _settings.Skipped.Remove(item.IgnoreKey);
        }
        _settings.Save();
        _shownRow = null;
        ShowDetailsIntro();
        Reapply();
    }

    private void SkipVersion(UpdateItem item)
    {
        _settings.Skipped[item.IgnoreKey] = item.TargetVersion ?? item.Available;
        _settings.Save();
        _shownRow = null;
        ShowDetailsIntro();
        Reapply();
    }

    private void RebuildHidden()
    {
        List<UpdateItem> hidden = _items.Where(i => i.Hidden).ToList();
        _hiddenList.Children.Clear();
        foreach (UpdateItem i in hidden.OrderBy(i => i.Source))
        {
            string versions = i.Kind is ItemKind.Package && i.Current.Length > 0 ? $"   {i.Current}  →  {i.TargetVersion ?? i.Available}" : "";
            var head = Rich((i.DisplayName, true), (versions, false));
            var entry = Stack(2, head, Muted($"{Checker.Title(i.Source)} · {i.Badge}"), Muted(i.Note ?? "", caption: false));
            if (i.IgnoredByUser)
            {
                UpdateItem target = i;
                entry.Children.Add(LinkButton("Show again", (_, _) => SetIgnored(target, ignored: false)));
            }
            _hiddenList.Children.Add(entry);
        }
        _hiddenExpander.Header = $"Left out for safety ({hidden.Count}). These aren't listed above, so they can't be ticked by accident.";
        _hiddenExpander.Visibility = hidden.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    private void UpdateSummary()
    {
        int n = SafeCount;
        int sel = _items.Count(i => i.Selectable && !i.Hidden && i.Selected);
        int hidden = LeftOutCount;
        _summary.Text = _pendingChecks > 0
            ? $"Checking… {Checker.Sources.Count - _pendingChecks} of {Checker.Sources.Count} done"
            : !_started ? "Not checked yet"
            : n == 0 ? "Everything safe is up to date" : $"{n} safe update{(n == 1 ? "" : "s")}";
        _selected.Text = (n == 0 ? "" : $"{sel} selected") + (hidden > 0 ? $"{(n == 0 ? "" : ", ")}{hidden} left out for safety" : "");
        if (!_busy || _run is null)
        {
            _updateButton.IsEnabled = !_busy && sel > 0;
            _updateButton.Content = sel > 0 ? $"Update selected ({sel})" : "Update selected";
        }
        _adminBar.IsOpen = !_busy && _items.Any(i => i.Selectable && !i.Hidden && i.Selected && i.NeedsAdmin);

        var parts = new List<string>();
        if (_settings.LastChecked is DateTime c)
        {
            parts.Add($"Checked {Ago(c)}");
        }
        if (_settings.LastUpdated is DateTime u)
        {
            parts.Add($"last updated {Ago(u)}");
        }
        _lastChecked.Text = string.Join(", ", parts);
        Shell.SetNavBadge("updates", _pendingChecks > 0 ? null : n);
        StateChanged?.Invoke();
    }

    // ---------------------------------------------------------------- updating

    private async void Update_Click(object sender, RoutedEventArgs e)
    {
        if (_run is not null)
        {
            // The button reads "Stop after this update" while a run is going.
            _run.RequestStop();
            _updateButton.IsEnabled = false;
            _updateButton.Content = "Stopping after this update…";
            return;
        }
        if (_busy)
        {
            return;
        }
        List<UpdateItem> selected = _items.Where(i => i.Selectable && !i.Hidden && i.Selected).ToList();
        if (selected.Count == 0)
        {
            return;
        }
        List<string> open = RunningApps.Find(selected);
        if (open.Count > 0)
        {
            string msg = $"{string.Join(", ", open)} {(open.Count == 1 ? "is" : "are")} running. The installer may close {(open.Count == 1 ? "it" : "them")}, so save your work first.";
            if (await Shell.AskAsync(open.Count == 1 ? "One of these apps is open" : "Some of these apps are open", msg, "Update anyway") != ContentDialogResult.Primary)
            {
                return;
            }
        }

        CloseBars();
        SetBusy(true, "Starting…");
        WorkBar.IsIndeterminate = false;
        WorkBar.Maximum = selected.Count;
        WorkBar.Value = 0;

        var run = new UpdateRun(_settings, SmokeScript);
        _run = run;
        _updateButton.IsEnabled = true;
        _updateButton.Content = "Stop after this update";
        AutomationProperties.SetName(_updateButton, "Stop after the update that is installing now");
        run.Changed += item => DispatcherQueue.TryEnqueue(() => OnItemChanged(item, selected));
        run.Status += text => DispatcherQueue.TryEnqueue(() => StatusText.Text = text);
        RunSummary summary;
        try
        {
            summary = await Task.Run(() => run.RunAsync(selected));
        }
        catch (Exception ex)
        {
            RunLog.WriteError(ex.ToString());
            ShowError("The update run stopped", ex.Message);
            _run = null;
            SetBusy(false, "");
            return;
        }
        _run = null;
        AutomationProperties.SetName(_updateButton, "Update selected");

        _lastRunLog = summary.LogFile;
        var notes = new List<string>(summary.Notes);
        if (summary.Skipped > 0)
        {
            notes.Insert(0, run.StopRequested ? $"Stopped: {summary.Skipped} not started." : $"{summary.Skipped} kept back or skipped (see each row).");
        }
        var openLog = new HyperlinkButton { Content = "Open the log" };
        openLog.Click += (_, _) => Open(_lastRunLog is not null && File.Exists(_lastRunLog) ? _lastRunLog : AppPaths.Logs);
        ShowResult(summary.Failed > 0 ? $"{summary.Updated} updated, {summary.Failed} didn't" : $"{summary.Updated} updated",
            string.Join(" ", notes), summary.Failed > 0 ? InfoBarSeverity.Warning : InfoBarSeverity.Success, openLog);
        if (summary.RebootRecommended)
        {
            ShowNotice("Restart to finish", "Some updates finish installing when Windows restarts.");
        }
        SetBusy(false, $"Finished at {DateTime.Now:HH:mm}. Press Check again to see what's left.");
    }

    private void OnItemChanged(UpdateItem item, List<UpdateItem> run)
    {
        if (_groups.TryGetValue(item.Source, out GroupView? g))
        {
            g.Refresh(item);
        }
        WorkBar.Value = run.Count(i => i.Status is ItemStatus.Done or ItemStatus.Failed or ItemStatus.Skipped);
        if (_shownRow?.Item == item)
        {
            ShowDetails(_shownRow);
        }
    }

    private void SetBusy(bool busy, string status)
    {
        _busy = busy;
        _checkButton.IsEnabled = !busy;
        _majorsToggle.IsEnabled = !busy;
        WorkBar.Visibility = busy ? Visibility.Visible : Visibility.Collapsed;
        StatusText.Text = status;
        foreach (GroupView g in _groups.Values)
        {
            g.SetEnabled(!busy);
        }
        UpdateSummary();
    }

    private void MajorsChanged()
    {
        _settings.IncludeMajors = _majorsToggle.IsChecked == true;
        _settings.Save();
        _manual.Clear();
        _shownRow = null;
        ShowDetailsIntro();
        Reapply();
    }

    private async void Rollback_Click(object sender, RoutedEventArgs e)
    {
        if (_busy)
        {
            return;
        }
        string? snap = _settings.LastPipSnapshot;
        if (snap is null || !File.Exists(snap))
        {
            ShowNotice("Nothing to roll back", "No Python update has run from here yet, so there is no snapshot.");
            return;
        }
        string msg = $"Every package in your user site goes back to the version it had on {File.GetLastWriteTime(snap):d MMM 'at' HH:mm}.";
        if (await Shell.AskAsync("Roll back the last Python update?", msg, "Roll back") != ContentDialogResult.Primary)
        {
            return;
        }
        SetBusy(true, "Rolling back Python packages…");
        WorkBar.IsIndeterminate = true;
        ProcResult r = await Task.Run(() => UpdateRun.RestoreSnapshotAsync(snap));
        WorkBar.IsIndeterminate = false;
        SetBusy(false, "");
        ShowResult(r.Ok ? "Python packages rolled back" : "The rollback didn't finish", r.Tail(1).FirstOrDefault() ?? "",
            r.Ok ? InfoBarSeverity.Success : InfoBarSeverity.Error);
    }

    // ---------------------------------------------------------------- details pane

    private void ShowDetailsIntro()
    {
        _detailsTitle.Text = "Details";
        _detailsWhy.Text = "Pick an update to see what it is, why it is or isn't selected, what's new, and the exact command it runs.";
        _detailsList.Children.Clear();
        foreach (SourceCheck s in Checker.Sources)
        {
            _detailsList.Children.Add(Field(Checker.Title(s.Source), Checker.Describe(s.Source)));
        }
    }

    private void ShowDetails(RowView row)
    {
        if (_shownRow is not null && _shownRow != row)
        {
            _shownRow.SetShown(false);
        }
        _shownRow = row;
        row.SetShown(true);
        UpdateItem i = row.Item;
        _detailsTitle.Text = i.DisplayName;
        _detailsWhy.Text = i.Note ?? DefaultWhy(i);
        _detailsList.Children.Clear();
        if (i.Kind is ItemKind.Package or ItemKind.Info)
        {
            _detailsList.Children.Add(Field("Installed", string.IsNullOrEmpty(i.Current) ? "Unknown" : i.Current));
            _detailsList.Children.Add(Field(i.TargetVersion is null ? "Newest" : "Will install", i.TargetVersion ?? i.Available));
        }
        else if (i.Kind is ItemKind.WindowsUpdate or ItemKind.Driver)
        {
            if (!string.IsNullOrEmpty(i.Detail))
            {
                _detailsList.Children.Add(Field("Article", i.Detail));
            }
            if (i.DriverDate is DateTime d)
            {
                _detailsList.Children.Add(Field("Driver date", d.ToString("d MMM yyyy")));
            }
            if (!string.IsNullOrEmpty(i.Available))
            {
                _detailsList.Children.Add(Field("Download", i.Available));
            }
        }

        // What's new: fixed pages for most sources, winget's own release-notes link for apps.
        var links = Stack(0, Muted("What's new"));
        foreach (NotesLink l in ReleaseNotes.For(i))
        {
            links.Children.Add(Link(l.Label, l.Url));
        }
        if (i.Source == UpdateSource.Apps)
        {
            if (_wingetNotes.TryGetValue(i.Id, out List<NotesLink>? cached))
            {
                foreach (NotesLink l in cached)
                {
                    links.Children.Insert(1, Link(l.Label, l.Url));
                }
            }
            else
            {
                var loading = Muted("Looking up the release notes…");
                links.Children.Insert(1, loading);
                _ = LoadWingetNotesAsync(row, loading);
            }
        }
        if (links.Children.Count > 1)
        {
            _detailsList.Children.Add(links);
        }

        _detailsList.Children.Add(Field("Runs", SafeUpdater.Core.Commands.Describe(i), mono: true));
        if (i.NeedsAdmin)
        {
            _detailsList.Children.Add(Field("Admin", "Yes, in the one admin step of the run"));
        }
        if (i.Result is not null)
        {
            _detailsList.Children.Add(Field("Result", i.Result));
        }
        if (i.Output.Count > 0)
        {
            _detailsList.Children.Add(Field("Last lines of output", string.Join("\n", i.Output), mono: true));
        }
        if (i.Selectable && i.Kind != ItemKind.StoreScan && !_busy)
        {
            var skip = Ui.Button("Skip this version", (_, _) => SkipVersion(i));
            ToolTipService.SetToolTip(skip, $"Hides {i.TargetVersion ?? i.Available} only. The next version after it is listed again.");
            var hide = Ui.Button("Don't show this update", (_, _) => SetIgnored(i, ignored: true));
            ToolTipService.SetToolTip(hide, "Hides every version of it until you choose Show again in the left-out list.");
            var buttons = Row(8, skip, hide);
            buttons.Margin = new Thickness(0, 6, 0, 0);
            _detailsList.Children.Add(buttons);
        }
    }

    private async Task LoadWingetNotesAsync(RowView row, TextBlock placeholder)
    {
        List<NotesLink> links;
        try
        {
            links = await ReleaseNotes.WingetAsync(row.Item.Id);
        }
        catch (Exception ex) when (ex is IOException or InvalidOperationException or System.ComponentModel.Win32Exception)
        {
            links = [];
        }
        _wingetNotes[row.Item.Id] = links;
        if (_shownRow == row)
        {
            ShowDetails(row);
        }
        else
        {
            placeholder.Text = "";
        }
    }

    private static string DefaultWhy(UpdateItem i) => i.Kind switch
    {
        ItemKind.StoreScan => "Asks the Microsoft Store to update all your Store apps in the background.",
        ItemKind.WindowsUpdate => "An update offered by Windows Update.",
        ItemKind.Driver => "An optional driver offered by Windows Update.",
        _ when i.Recommended => "Picked: a routine update within the same version family.",
        _ => "Not picked by default.",
    };

    private void ApplyLayout(double width, double height)
    {
        bool wide = width >= 900;
        if (!wide)
        {
            _body.RowDefinitions[1].Height = new GridLength(Math.Clamp(height * 0.34, 160, 320));
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

    // ---------------------------------------------------------------- group card

    private sealed class GroupView
    {
        private readonly UpdatesPage _owner;
        private readonly CheckBox _all;
        private readonly TextBlock _count;
        private readonly TextBlock _status;
        private readonly ProgressRing _ring;
        private readonly StackPanel _rows;
        private readonly Dictionary<string, RowView> _views = [];

        public Border Card { get; }

        public GroupView(UpdatesPage owner, UpdateSource source)
        {
            _owner = owner;
            _all = new CheckBox { MinWidth = 0, Padding = new Thickness(0), VerticalAlignment = VerticalAlignment.Center, Visibility = Visibility.Collapsed };
            AutomationProperties.SetName(_all, "Select every update in " + Checker.Title(source));
            _all.Click += (_, _) =>
            {
                bool on = _all.IsChecked == true;
                foreach (RowView v in _views.Values.Where(v => v.Item.Selectable))
                {
                    v.Item.Selected = on;
                    _owner.RememberChoice(v.Item);
                    v.Refresh();
                }
                Sync();
                _owner.UpdateSummary();
            };

            var titles = Stack(0, Text(Checker.Title(source), "BodyStrongTextBlockStyle"), Muted(Checker.Describe(source)));
            titles.VerticalAlignment = VerticalAlignment.Center;
            _count = Text("", "BodyStrongTextBlockStyle", wrap: false);
            _count.VerticalAlignment = VerticalAlignment.Center;
            _ring = new ProgressRing { Width = 18, Height = 18, IsActive = false, Visibility = Visibility.Collapsed };
            Grid header = Columns((_all, Px(28)), (Icon(Glyph(source), 18), Auto), (titles, Star), (Row(8, _ring, _count), Auto));
            _status = Muted("", caption: false);
            _status.Margin = new Thickness(40, 0, 0, 0);
            _status.Visibility = Visibility.Collapsed;
            _rows = new StackPanel { Spacing = 2 };
            Card = Ui.Card(Stack(8, header, _status, _rows), 0);
            Card.Padding = new Thickness(16, 12, 16, 12);
        }

        public void SetChecking()
        {
            Card.Visibility = Visibility.Visible;
            _rows.Children.Clear();
            _views.Clear();
            _all.Visibility = Visibility.Collapsed;
            _count.Text = "";
            _ring.IsActive = true;
            _ring.Visibility = Visibility.Visible;
            SetStatus("Checking…", failed: false);
        }

        public void Show(SourceResult r)
        {
            Card.Visibility = r.Absent ? Visibility.Collapsed : Visibility.Visible;
            _ring.IsActive = false;
            _ring.Visibility = Visibility.Collapsed;
            _rows.Children.Clear();
            _views.Clear();
            List<UpdateItem> visible = r.Items.Where(i => !i.Hidden).ToList();
            foreach (UpdateItem i in visible)
            {
                var v = new RowView(_owner, this, i);
                _views[i.Key] = v;
                _rows.Children.Add(v.Host);
            }
            int n = visible.Count(i => i.Selectable);
            int hidden = r.Items.Count - visible.Count;
            _count.Text = n > 0 ? n.ToString() : "";
            _all.Visibility = n > 0 ? Visibility.Visible : Visibility.Collapsed;
            string? status = r.Message;
            if (status is null && visible.Count == 0)
            {
                status = hidden > 0 ? $"Nothing safe to update. {hidden} left out for safety, listed at the end." : "Up to date";
            }
            else if (hidden > 0 && !r.Failed)
            {
                status = (status is null ? "" : status + ". ") + $"{hidden} more left out for safety, listed at the end.";
            }
            SetStatus(status, r.Failed);
            Sync();
        }

        public void Refresh(UpdateItem item)
        {
            if (_views.TryGetValue(item.Key, out RowView? v))
            {
                v.Refresh();
            }
        }

        public void SetEnabled(bool on)
        {
            _all.IsEnabled = on;
            foreach (RowView v in _views.Values)
            {
                v.Check.IsEnabled = on && v.Item.Selectable;
            }
        }

        public void Sync()
        {
            List<UpdateItem> sel = _views.Values.Select(v => v.Item).Where(i => i.Selectable).ToList();
            int on = sel.Count(i => i.Selected);
            _all.IsChecked = on == 0 ? false : on == sel.Count ? true : null;
        }

        private void SetStatus(string? text, bool failed)
        {
            _status.Text = text ?? "";
            _status.Style = S(failed ? "CautionBodyStyle" : "MutedBodyStyle");
            _status.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;
        }

        private static string Glyph(UpdateSource s) => s switch
        {
            UpdateSource.Apps => "",
            UpdateSource.Chocolatey => "",
            UpdateSource.Scoop => "",
            UpdateSource.Windows => "",
            UpdateSource.Nvidia => "",
            UpdateSource.Python => "",
            UpdateSource.Node => "",
            UpdateSource.Rust => "",
            UpdateSource.ClaudeCode => "",
            _ => "",
        };
    }

    // ---------------------------------------------------------------- one row

    private sealed class RowView
    {
        private readonly UpdatesPage _owner;
        private readonly TextBlock _versions;
        private readonly Border _badge;
        private readonly FontIcon _statusIcon;
        private readonly ProgressRing _ring;
        private readonly TextBlock _statusText;
        private readonly Border _root;
        private bool _shown;
        private bool _hover;

        public UpdateItem Item { get; }
        public ContentControl Host { get; }
        public CheckBox Check { get; }

        public RowView(UpdatesPage owner, GroupView group, UpdateItem item)
        {
            _owner = owner;
            Item = item;

            Check = new CheckBox { MinWidth = 0, Padding = new Thickness(0), VerticalAlignment = VerticalAlignment.Center, IsEnabled = item.Selectable };
            AutomationProperties.SetName(Check, "Update " + item.Name);
            Check.Click += (_, _) =>
            {
                Item.Selected = Check.IsChecked == true;
                _owner.RememberChoice(Item);
                group.Sync();
                _owner.UpdateSummary();
            };

            string sub = item.Kind switch
            {
                ItemKind.WindowsUpdate or ItemKind.Driver => item.Detail ?? "",
                ItemKind.StoreScan => "Store apps update in the background",
                _ => item.Id,
            };
            var names = Stack(0, Text(item.DisplayName, wrap: false));
            if (sub.Length > 0)
            {
                names.Children.Add(Text(sub, "MutedCaptionStyle", wrap: false));
            }
            names.VerticalAlignment = VerticalAlignment.Center;

            _versions = Text("", wrap: false);
            _versions.VerticalAlignment = VerticalAlignment.Center;
            _versions.HorizontalAlignment = HorizontalAlignment.Right;
            _badge = Badge("");
            _statusIcon = Icon("", 14, "IconMuted");
            _ring = new ProgressRing { Width = 14, Height = 14, IsActive = false, Visibility = Visibility.Collapsed };
            _statusText = Text("", "MutedCaptionStyle", wrap: false);
            _statusText.VerticalAlignment = VerticalAlignment.Center;
            var status = Row(6, _ring, _statusIcon, _statusText);
            status.VerticalAlignment = VerticalAlignment.Center;

            Grid grid = Columns((Check, Px(28)), (names, Star), (_versions, Auto), (_badge, Auto), (status, Px(150)));
            grid.ColumnDefinitions[2].MinWidth = 150;
            _root = new Border { Style = S("RowStyle"), Padding = new Thickness(0, 6, 8, 6), Child = grid };
            _root.Tapped += (_, _) => _owner.ShowDetails(this);
            _root.PointerEntered += (_, _) => { _hover = true; ApplyStyle(); };
            _root.PointerExited += (_, _) => { _hover = false; ApplyStyle(); };

            // A keyboard stop for the row, so its details can be opened without a mouse.
            Host = new ContentControl
            {
                Content = _root,
                IsTabStop = true,
                UseSystemFocusVisuals = true,
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch,
            };
            AutomationProperties.SetName(Host, $"{item.Name}, details");
            Host.GotFocus += (_, e) =>
            {
                if (ReferenceEquals(e.OriginalSource, Host))
                {
                    _owner.ShowDetails(this);
                }
            };
            Refresh();
        }

        public void SetShown(bool shown)
        {
            _shown = shown;
            ApplyStyle();
        }

        private void ApplyStyle()
        {
            Style s = S(_shown ? "SelectedRowStyle" : _hover ? "RowHoverStyle" : "RowStyle");
            if (!ReferenceEquals(_root.Style, s))
            {
                _root.Style = s;
                _root.Padding = new Thickness(0, 6, 8, 6);
            }
        }

        public void Refresh()
        {
            UpdateItem i = Item;
            Check.IsChecked = i.Selectable && i.Selected;
            string target = i.TargetVersion ?? i.Available;
            _versions.Text = i.Kind switch
            {
                ItemKind.WindowsUpdate or ItemKind.Driver => i.Available,
                ItemKind.StoreScan => "",
                _ => $"{(i.UnknownVersion || i.Current.Length == 0 ? "?" : i.Current)}  →  {target}",
            };
            string tone = i.Badge switch
            {
                "Major" => "caution",
                "Held" or "Old driver" => "critical",
                "Same family" => "success",
                _ => "neutral",
            };
            SetBadge(_badge, i.Badge, tone);

            bool running = i.Status == ItemStatus.Running;
            _ring.IsActive = running;
            _ring.Visibility = running ? Visibility.Visible : Visibility.Collapsed;
            (string glyph, string style) = i.Status switch
            {
                ItemStatus.Done => ("", "IconSuccess"),
                ItemStatus.Failed => ("", "IconCritical"),
                ItemStatus.Skipped => ("", "IconCaution"),
                ItemStatus.Queued => ("", "IconMuted"),
                _ => ("", "IconMuted"),
            };
            _statusIcon.Glyph = glyph;
            _statusIcon.Style = S(style);
            _statusIcon.Visibility = glyph.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
            _statusText.Text = i.Status switch
            {
                ItemStatus.Idle => "",
                ItemStatus.Queued => "Waiting",
                ItemStatus.Running => "Installing",
                ItemStatus.Done => i.Result?.StartsWith("Already", StringComparison.Ordinal) == true ? "Up to date" : "Updated",
                ItemStatus.Failed => i.Result?.StartsWith("Rolled back", StringComparison.Ordinal) == true ? "Rolled back" : "Failed",
                ItemStatus.Skipped => i.Result?.StartsWith("Stopped", StringComparison.Ordinal) == true ? "Not started" : "Kept back",
                _ => "",
            };
            ToolTipService.SetToolTip(_root, i.Result);
        }
    }
}
