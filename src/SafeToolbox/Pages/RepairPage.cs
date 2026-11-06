using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using SafeToolbox.Core;
using static SafeToolbox.Ui;

namespace SafeToolbox.Pages;

/// <summary>Documented Windows fixes, one at a time, with every line of output kept on screen.</summary>
internal sealed class RepairPage : PageBase
{
    private readonly StackPanel _list = new() { Spacing = 8, Padding = new Thickness(0, 0, 12, 16) };
    private readonly TextBlock _outputTitle = Text("Output", "SubtitleTextBlockStyle");
    private readonly TextBlock _output = new() { Style = S("MonoCaptionStyle") };
    private readonly ScrollViewer _outputScroll;
    private readonly Grid _body = new() { Padding = new Thickness(24, 8, 24, 0) };
    private readonly Border _outputCard;
    private readonly List<Button> _runButtons = [];
    private bool _busy;
    private bool? _wide;

    public RepairPage(IShell shell) : base(shell, "Repair",
        "Fixes for common Windows problems. Each one is a documented Windows command, and you see everything it prints.",
        "For damaged system files, Microsoft's order is used: DISM repairs the Windows image first, then SFC checks the files.")
    {
        foreach (Repair r in Repairs.All)
        {
            _list.Children.Add(BuildCard(r));
        }
        _output.Text = "Pick a repair. Its output shows here as it runs.";
        _outputScroll = new ScrollViewer { Content = _output, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        var g = new Grid { RowSpacing = 8 };
        g.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        g.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        g.Children.Add(_outputTitle);
        Grid.SetRow(_outputScroll, 1);
        g.Children.Add(_outputScroll);
        _outputCard = Card(g);
        _outputCard.Margin = new Thickness(0, 0, 0, 16);

        _body.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        _body.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(420) });
        _body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        _body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(0) });
        var scroll = new ScrollViewer { Content = _list, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        _body.Children.Add(scroll);
        _body.Children.Add(_outputCard);
        _body.SizeChanged += (_, e) => ApplyLayout(e.NewSize.Width, e.NewSize.Height);
        SetBody(_body);

        AddCommand("Copy the output", "", (_, _) =>
        {
            var dp = new Windows.ApplicationModel.DataTransfer.DataPackage();
            dp.SetText(_output.Text);
            Windows.ApplicationModel.DataTransfer.Clipboard.SetContent(dp);
        });
        AddCommand("Windows troubleshooters", "", (_, _) => Open("ms-settings:troubleshoot"), secondary: true);
        AddCommand("Recovery options", "", (_, _) => Open("ms-settings:recovery"), secondary: true);
    }

    private Border BuildCard(Repair r)
    {
        var titleRow = Row(8, Text(r.Title, "BodyStrongTextBlockStyle", wrap: false));
        if (r.NeedsAdmin)
        {
            titleRow.Children.Add(Badge("Asks for admin"));
        }
        if (r.RestartAfter)
        {
            titleRow.Children.Add(Badge("Restart after", "caution"));
        }
        var text = Stack(2, titleRow, Text(r.What), Muted(r.Heads));
        var run = new Button { Content = "Run" };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(run, "Run " + r.Title);
        run.Click += async (_, _) => await RunAsync(r);
        run.VerticalAlignment = VerticalAlignment.Center;
        _runButtons.Add(run);
        Border card = Card(Columns((Icon(r.Glyph, 20), Auto), (text, Star), (run, Auto)));
        card.Padding = new Thickness(16, 14, 16, 14);
        return card;
    }

    private async Task RunAsync(Repair r)
    {
        if (_busy)
        {
            return;
        }
        string body = r.What + "\n\n" + r.Heads + (r.NeedsAdmin ? "\n\nWindows will ask for admin once." : "");
        if (await Shell.AskAsync(r.Title + "?", body, "Run") != ContentDialogResult.Primary)
        {
            return;
        }
        _busy = true;
        foreach (Button b in _runButtons)
        {
            b.IsEnabled = false;
        }
        CloseBars();
        _outputTitle.Text = r.Title;
        _output.Text = "";
        WorkBar.Visibility = Visibility.Visible;
        WorkBar.IsIndeterminate = true;
        StatusText.Text = r.NeedsAdmin && !SafeDiskCleaner.Core.Tools.IsAdmin() ? "Waiting for you to allow admin access…" : "Running…";
        bool? ok;
        try
        {
            ok = r.NeedsAdmin && !SafeDiskCleaner.Core.Tools.IsAdmin() ? await RunElevatedAsync(r) : await Repairs.RunAsync(r, line => DispatcherQueue.TryEnqueue(() => Append(line)));
        }
        catch (Exception ex)
        {
            Append("!! " + ex.Message);
            ok = false;
        }
        WorkBar.IsIndeterminate = false;
        WorkBar.Visibility = Visibility.Collapsed;
        _busy = false;
        foreach (Button b in _runButtons)
        {
            b.IsEnabled = true;
        }
        if (ok is null)
        {
            StatusText.Text = "Admin access was declined, so nothing ran.";
            return;
        }
        StatusText.Text = $"Finished at {DateTime.Now:HH:mm}.";
        ShowResult(ok == true ? $"{r.Title}: done" : $"{r.Title}: didn't finish",
            ok == true
                ? (r.RestartAfter ? "Restart Windows to finish." : "The output is on the right. Copy the output keeps it for later.")
                : "The output on the right says which step stopped and why.",
            ok == true ? InfoBarSeverity.Success : InfoBarSeverity.Warning);
    }

    /// <summary>Runs the repair in the elevated helper and follows its log file.</summary>
    private async Task<bool?> RunElevatedAsync(Repair r)
    {
        string log = Path.Combine(ToolboxPaths.Jobs, $"repair-{r.Id}-{DateTime.Now:yyyyMMdd-HHmmss}.log");
        File.WriteAllText(log, "");
        using var cts = new CancellationTokenSource();
        Task<int?> run = AdminTasks.RunElevatedAsync($"--repair {r.Id} \"{log}\"", cts.Token);
        long read = 0;
        while (!run.IsCompleted)
        {
            await Task.WhenAny(run, Task.Delay(500));
            read = Follow(log, read);
        }
        Follow(log, read);
        int? code = await run;
        if (code is null)
        {
            return null;
        }
        bool ok = File.ReadAllText(log).Contains("##EXIT ok", StringComparison.Ordinal);
        try
        {
            File.Move(log, Path.Combine(ToolboxPaths.Exports, Path.GetFileName(log)), overwrite: true);
        }
        catch (IOException)
        {
        }
        return ok;
    }

    private long Follow(string file, long from)
    {
        try
        {
            using var fs = new FileStream(file, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
            fs.Seek(from, SeekOrigin.Begin);
            using var sr = new StreamReader(fs);
            string rest = sr.ReadToEnd();
            foreach (string line in rest.Split('\n', StringSplitOptions.RemoveEmptyEntries))
            {
                if (!line.StartsWith("##EXIT", StringComparison.Ordinal))
                {
                    Append(line.TrimEnd('\r'));
                }
            }
            return from + System.Text.Encoding.UTF8.GetByteCount(rest);
        }
        catch (IOException)
        {
            return from;
        }
    }

    private void Append(string line)
    {
        _output.Text = _output.Text.Length == 0 ? line : _output.Text + "\n" + line;
        _outputScroll.ChangeView(null, _outputScroll.ScrollableHeight + 1000, null, disableAnimation: true);
        if (line.StartsWith("## ", StringComparison.Ordinal))
        {
            StatusText.Text = line[3..] + "…";
        }
    }

    private void ApplyLayout(double width, double height)
    {
        bool wide = width >= 980;
        if (!wide)
        {
            _body.RowDefinitions[1].Height = new GridLength(Math.Clamp(height * 0.38, 180, 360));
        }
        if (_wide == wide)
        {
            return;
        }
        _wide = wide;
        _body.ColumnDefinitions[1].Width = new GridLength(wide ? 420 : 0);
        _body.ColumnSpacing = wide ? 16 : 0;
        _body.RowSpacing = wide ? 0 : 12;
        if (wide)
        {
            _body.RowDefinitions[1].Height = new GridLength(0);
        }
        Grid.SetRow(_outputCard, wide ? 0 : 1);
        Grid.SetColumn(_outputCard, wide ? 1 : 0);
    }
}
