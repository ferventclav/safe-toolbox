using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using SafeToolbox.Core;
using static SafeToolbox.Ui;

namespace SafeToolbox.Pages;

/// <summary>
/// Your largest files, like Windows' "Large or unused files" recommendation but across your whole profile.
/// Nothing is deleted here: a file can only go to the Recycle Bin, one at a time, when you ask.
/// </summary>
internal sealed class StoragePage : PageBase
{
    private static readonly (string Label, long Bytes)[] Sizes = [("50 MB", 50L << 20), ("100 MB", 100L << 20), ("500 MB", 500L << 20), ("1 GB", 1L << 30)];
    private static readonly (string Label, FileKind? Kind)[] Kinds =
    [
        ("All files", null), ("Videos", FileKind.Video), ("Disk images", FileKind.DiskImage), ("Archives", FileKind.Archive),
        ("Installers", FileKind.Installer), ("Audio", FileKind.Audio), ("AI models", FileKind.AiModel), ("Other", FileKind.Other),
    ];

    private readonly ComboBox _size = new() { MinWidth = 120 };
    private readonly ComboBox _kind = new() { MinWidth = 160 };
    private readonly StackPanel _rows = new() { Spacing = 2, Padding = new Thickness(0, 0, 12, 16) };
    private readonly TextBlock _summary = Text("", "SubtitleTextBlockStyle", wrap: false);
    private readonly TextBlock _summarySub = Muted("");
    private readonly Button _scan;
    private List<BigFile> _files = [];
    private IReadOnlyList<string> _inUse = [];
    private CancellationTokenSource? _cts;

    public StoragePage(IShell shell) : base(shell, "Big files",
        "Your largest files across Desktop, Documents, Downloads, Videos and the rest of your profile, so you can decide what to keep.",
        "Nothing is deleted. A file you choose goes to the Recycle Bin, so it can be restored.")
    {
        HeaderRight.Children.Add(_summary);
        HeaderRight.Children.Add(_summarySub);
        foreach (var s in Sizes)
        {
            _size.Items.Add("At least " + s.Label);
        }
        _size.SelectedIndex = 1;
        foreach (var k in Kinds)
        {
            _kind.Items.Add(k.Label);
        }
        _kind.SelectedIndex = 0;
        AutomationProperties.SetName(_size, "Smallest size to list");
        AutomationProperties.SetName(_kind, "Kind of file");
        _kind.SelectionChanged += (_, _) => Render();
        _size.SelectionChanged += async (_, _) => { if (_files.Count > 0 || _cts is not null) { await ScanAsync(); } };

        var filters = Row(12, _size, _kind);
        filters.Margin = new Thickness(0, 0, 0, 12);
        var body = new Grid { Padding = new Thickness(24, 8, 24, 0) };
        body.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        body.Children.Add(filters);
        var scroll = new ScrollViewer { Content = _rows, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        Grid.SetRow(scroll, 1);
        body.Children.Add(scroll);
        SetBody(body);

        AddCommand("Storage settings", "", (_, _) => Open("ms-settings:storagesense"), secondary: true);
        AddCommand("Open the Recycle Bin", "", (_, _) => Open("shell:RecycleBinFolder"), secondary: true);
        _scan = AddPrimary("Find big files", async (_, _) => await ScanAsync());
        _scan.IsEnabled = true;
    }

    protected override Task OnFirstShownAsync() => ScanAsync();

    private async Task ScanAsync()
    {
        _cts?.Cancel();
        var cts = new CancellationTokenSource();
        _cts = cts;
        long min = Sizes[Math.Max(0, _size.SelectedIndex)].Bytes;
        _scan.IsEnabled = false;
        WorkBar.Visibility = Visibility.Visible;
        WorkBar.IsIndeterminate = true;
        _summary.Text = "Looking…";
        var progress = new Progress<string>(dir => StatusText.Text = "Looking in " + dir);
        try
        {
            _inUse = await Task.Run(() => SafeDiskCleaner.Core.Guard.CreateDefault().InUsePaths);
            _files = await Task.Run(() => BigFiles.Find(BigFiles.DefaultRoots(), min, 300, cts.Token, progress), cts.Token);
            StatusText.Text = $"Looked through your profile at {DateTime.Now:HH:mm}.";
        }
        catch (OperationCanceledException)
        {
            return;
        }
        finally
        {
            if (_cts == cts)
            {
                _cts = null;
                _scan.IsEnabled = true;
                WorkBar.IsIndeterminate = false;
                WorkBar.Visibility = Visibility.Collapsed;
            }
        }
        Render();
    }

    private void Render()
    {
        _rows.Children.Clear();
        FileKind? kind = Kinds[Math.Max(0, _kind.SelectedIndex)].Kind;
        List<BigFile> shown = _files.Where(f => kind is null || f.Kind == kind).ToList();
        long total = shown.Sum(f => f.Size);
        _summary.Text = shown.Count == 0 ? "No big files" : SafeDiskCleaner.Core.SizeFormat.Format(total);
        _summarySub.Text = shown.Count == 0 ? "" : $"in {Plural(shown.Count, "file")}" + (_files.Count >= 300 ? ", the 300 largest" : "");
        if (shown.Count == 0)
        {
            _rows.Children.Add(Muted(_files.Count == 0 ? "Nothing that size in your profile." : "None of this kind.", caption: false));
            return;
        }
        string downloads = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), "Downloads");
        foreach (BigFile f in shown)
        {
            _rows.Children.Add(BuildRow(f, downloads));
        }
    }

    private UIElement BuildRow(BigFile f, string downloads)
    {
        var badges = Row(6, Badge(BigFiles.KindName(f.Kind)));
        if (f.Kind == FileKind.Installer && SafeDiskCleaner.Core.PathUtil.Under(f.Path, downloads))
        {
            badges.Children.Add(Badge("Installer in Downloads: you can usually download it again", "success"));
        }
        if (f.Modified < DateTime.Now.AddYears(-1))
        {
            badges.Children.Add(Badge("Not changed for over a year"));
        }
        if (f.Kind == FileKind.AiModel)
        {
            badges.Children.Add(Badge("A model an AI tool may still use", "caution"));
        }
        var names = Stack(2, Text(f.Name, "BodyStrongTextBlockStyle", wrap: false), Text(f.Folder, "MutedCaptionStyle", wrap: false), badges);
        var size = Stack(0, Text(SafeDiskCleaner.Core.SizeFormat.Format(f.Size), "BodyStrongTextBlockStyle", wrap: false), Muted(f.Modified.ToString("d MMM yyyy")));
        size.MinWidth = 96;
        var show = new Button { Content = new FontIcon { Glyph = "", FontSize = 14 } };
        AutomationProperties.SetName(show, "Show " + f.Name + " in its folder");
        ToolTipService.SetToolTip(show, "Show in folder");
        show.Click += (_, _) => ShowInFolder(f.Path);
        var recycle = new Button { Content = new FontIcon { Glyph = "", FontSize = 14 } };
        AutomationProperties.SetName(recycle, "Move " + f.Name + " to the Recycle Bin");
        ToolTipService.SetToolTip(recycle, "Move to the Recycle Bin");
        var row = new Border { Style = S("RowStyle") };
        recycle.Click += async (_, _) => await RecycleAsync(f, row);
        var actions = Row(6, show, recycle);
        actions.VerticalAlignment = VerticalAlignment.Center;
        row.Child = Columns((Icon(Glyph(f.Kind), 18, "IconMuted"), Auto), (names, Star), (size, Auto), (actions, Auto));
        return row;
    }

    private async Task RecycleAsync(BigFile f, Border row)
    {
        string msg = $"{f.Name} ({SafeDiskCleaner.Core.SizeFormat.Format(f.Size)}) goes to the Recycle Bin. You can restore it from there until the bin is emptied.";
        if (f.Size > 20L << 30)
        {
            msg += "\n\nIt is very large, so Windows may say it is too big for the Recycle Bin and ask whether to delete it for good. Say No if you're not sure.";
        }
        if (await Shell.AskAsync("Move to the Recycle Bin?", msg, "Move to Recycle Bin") != ContentDialogResult.Primary)
        {
            return;
        }
        string? why = await Task.Run(() => BigFiles.Recycle(f.Path, _inUse));
        if (why is null)
        {
            _files.Remove(f);
            _rows.Children.Remove(row);
            ShowResult("Moved to the Recycle Bin", $"{f.Name}, {SafeDiskCleaner.Core.SizeFormat.Format(f.Size)}. Restore it from the Recycle Bin if you need it back.");
            Render();
        }
        else
        {
            ShowError($"{f.Name} was left in place", why);
        }
    }

    private static string Glyph(FileKind k) => k switch
    {
        FileKind.Video => "",
        FileKind.DiskImage => "",
        FileKind.Archive => "",
        FileKind.Installer => "",
        FileKind.Audio => "",
        FileKind.AiModel => "",
        _ => "",
    };
}
