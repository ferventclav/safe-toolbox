using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using SafeToolbox.Core;
using static SafeToolbox.Ui;

namespace SafeToolbox.Pages;

/// <summary>Every update run and every clean, read from the logs both tools already keep.</summary>
internal sealed class HistoryPage : PageBase
{
    private readonly StackPanel _panel = new() { Spacing = 8, Padding = new Thickness(0, 0, 0, 24) };
    private readonly ComboBox _filter = new() { MinWidth = 180 };
    private readonly TextBlock _summary = Text("", "SubtitleTextBlockStyle", wrap: false);
    private readonly TextBlock _summarySub = Muted("");
    private List<HistoryRun> _runs = [];

    public HistoryPage(IShell shell) : base(shell, "History", "What each update run and each clean did, newest first. Open a run to see every item.")
    {
        HeaderRight.Children.Add(_summary);
        HeaderRight.Children.Add(_summarySub);
        foreach (string s in new[] { "Everything", "Update runs", "Cleans" })
        {
            _filter.Items.Add(s);
        }
        _filter.SelectedIndex = 0;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(_filter, "Show");
        _filter.SelectionChanged += (_, _) => Render();
        var body = new Grid { Padding = new Thickness(24, 8, 24, 0) };
        body.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        _filter.Margin = new Thickness(0, 0, 0, 12);
        body.Children.Add(_filter);
        var scroll = new ScrollViewer { Content = _panel, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        Grid.SetRow(scroll, 1);
        body.Children.Add(scroll);
        SetBody(body);
        AddCommand("Refresh", "", (_, _) => OnShown(), key: Windows.System.VirtualKey.F5);
        AddCommand("Open the update log (runs.csv)", "", (_, _) => OpenIfExists(History.UpdaterLog), secondary: true);
        AddCommand("Open the cleanup log (runs.csv)", "", (_, _) => OpenIfExists(History.CleanerLog), secondary: true);
    }

    public override void OnShown()
    {
        Dictionary<string, string> titles = SafeDiskCleaner.Core.RuleCatalog.Build().ToDictionary(r => r.Id, r => r.Title);
        _runs = History.ReadAll(id => titles.TryGetValue(id, out string? t) ? t : id);
        long freed = _runs.Where(r => r.Kind == RunKind.Cleanup).Sum(r => Math.Max(0, r.Freed));
        int updated = _runs.Where(r => r.Kind == RunKind.Updates).Sum(r => r.Lines.Count(l => !l.Problem && l.Outcome != "Skipped"));
        _summary.Text = _runs.Count == 0 ? "Nothing yet" : Plural(_runs.Count, "run");
        _summarySub.Text = _runs.Count == 0 ? "" : $"{SafeDiskCleaner.Core.SizeFormat.Format(freed)} freed in all";
        Render();
    }

    private void Render()
    {
        _panel.Children.Clear();
        IEnumerable<HistoryRun> runs = _filter.SelectedIndex switch
        {
            1 => _runs.Where(r => r.Kind == RunKind.Updates),
            2 => _runs.Where(r => r.Kind == RunKind.Cleanup),
            _ => _runs,
        };
        int n = 0;
        foreach (HistoryRun r in runs.Take(200))
        {
            n++;
            var header = Columns(
                (Icon(r.Kind == RunKind.Updates ? "" : "", 18, r.Problems > 0 ? "IconCaution" : "IconMuted"), Auto),
                (Stack(0, Text(r.Summary, "BodyStrongTextBlockStyle", wrap: false), Muted($"{(r.Kind == RunKind.Updates ? "Update run" : "Clean")} · {r.When:dddd d MMM yyyy, HH:mm}")), Star),
                (r.Problems > 0 ? Badge(Plural(r.Problems, "problem"), "caution") : new Border(), Auto));
            header.Padding = new Thickness(0, 8, 0, 8);
            var lines = Stack(6);
            foreach (HistoryLine l in r.Lines)
            {
                lines.Children.Add(Columns(
                    (Stack(0, Text(l.Title, wrap: false), Muted(l.Detail)), Star),
                    (Text(l.Outcome, l.Problem ? "CautionCaptionStyle" : "MutedCaptionStyle"), Px(260))));
            }
            _panel.Children.Add(new Expander
            {
                Header = header,
                Content = lines,
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch,
            });
        }
        if (n == 0)
        {
            _panel.Children.Add(Muted("Nothing here yet. Runs from Updates and Clean up show up as soon as they finish.", caption: false));
        }
    }

    private void OpenIfExists(string file)
    {
        if (File.Exists(file))
        {
            Open(file);
        }
        else
        {
            ShowNotice("No log yet", file);
        }
    }
}
