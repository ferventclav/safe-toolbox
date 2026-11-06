using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using SafeToolbox.Core;

namespace SafeToolbox.Pages;

/// <summary>What a page may ask of the window.</summary>
public interface IShell
{
    ToolboxSettings Settings { get; }
    XamlRoot XamlRoot { get; }
    ElementTheme ActualTheme { get; }
    Microsoft.UI.WindowId WindowId { get; }
    void Navigate(string tag);
    void SetNavBadge(string tag, int? count);
    void ApplyAppearance();
    Task<ContentDialogResult> AskAsync(string title, object content, string primary, string? close = "Cancel", bool primaryIsDefault = false);
    T Page<T>() where T : PageBase;
}

/// <summary>
/// The frame every page shares: a header (title, what the page does, a summary on the right), a stack of
/// message bars, the page body, and a footer with a status line and the page's commands.
/// </summary>
public abstract class PageBase : UserControl
{
    private readonly StackPanel _bars = new() { Spacing = 8, Padding = new Thickness(24, 0, 24, 0) };
    private readonly InfoBar _error = new() { Severity = InfoBarSeverity.Error, IsClosable = true };
    private readonly InfoBar _notice = new() { Severity = InfoBarSeverity.Informational, IsClosable = true };
    private readonly InfoBar _result = new() { Severity = InfoBarSeverity.Success, IsClosable = true };
    private readonly Grid _footer = new() { Padding = new Thickness(24, 4, 16, 12), ColumnSpacing = 12 };
    private bool _loaded;

    protected IShell Shell { get; }
    protected Grid Root { get; } = new();
    protected StackPanel HeaderRight { get; } = new() { Spacing = 4, VerticalAlignment = VerticalAlignment.Center, MinWidth = 260 };
    protected TextBlock StatusText { get; } = new() { Style = Ui.S("MutedCaptionStyle"), TextTrimming = TextTrimming.CharacterEllipsis };
    protected ProgressBar WorkBar { get; } = new() { Minimum = 0, Maximum = 1, Visibility = Visibility.Collapsed };
    protected CommandBar Commands { get; } = new()
    {
        DefaultLabelPosition = CommandBarDefaultLabelPosition.Right,
        Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent),
        VerticalAlignment = VerticalAlignment.Center,
        OverflowButtonVisibility = CommandBarOverflowButtonVisibility.Auto,
    };

    protected PageBase(IShell shell, string title, params string[] lines)
    {
        Shell = shell;
        Root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        Root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        Root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        Root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        var titles = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        titles.Children.Add(new TextBlock { Text = title, Style = Ui.S("TitleTextBlockStyle") });
        foreach (string l in lines)
        {
            titles.Children.Add(Ui.Muted(l, caption: false));
        }
        Grid header = Ui.Columns((titles, Ui.Star), (HeaderRight, Ui.Auto));
        header.Padding = new Thickness(24, 20, 24, 12);
        header.ColumnSpacing = 24;
        Root.Children.Add(header);

        _bars.Children.Add(_error);
        _bars.Children.Add(_notice);
        _bars.Children.Add(_result);
        foreach (InfoBar b in new[] { _error, _notice, _result })
        {
            b.IsOpen = false;
        }
        Grid.SetRow(_bars, 1);
        Root.Children.Add(_bars);

        _footer.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        _footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var status = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        status.Children.Add(StatusText);
        status.Children.Add(WorkBar);
        _footer.Children.Add(status);
        Grid.SetColumn(Commands, 1);
        _footer.Children.Add(Commands);
        Grid.SetRow(_footer, 3);
        Root.Children.Add(_footer);

        Content = Root;
        Loaded += async (_, _) =>
        {
            if (!_loaded)
            {
                _loaded = true;
                await OnFirstShownAsync();
            }
        };
    }

    /// <summary>The body goes in row 2.</summary>
    protected void SetBody(FrameworkElement body)
    {
        Grid.SetRow(body, 2);
        Root.Children.Add(body);
    }

    protected void HideFooter() => _footer.Visibility = Visibility.Collapsed;

    protected virtual Task OnFirstShownAsync() => Task.CompletedTask;

    /// <summary>Called every time the page is shown, after the first time too.</summary>
    public virtual void OnShown()
    {
    }

    public void ShowError(string title, string message) => Bar(_error, InfoBarSeverity.Error, title, message);

    public void ShowNotice(string title, string message, InfoBarSeverity severity = InfoBarSeverity.Informational) =>
        Bar(_notice, severity, title, message);

    protected InfoBar ShowResult(string title, string message, InfoBarSeverity severity = InfoBarSeverity.Success, ButtonBase? action = null)
    {
        Bar(_result, severity, title, message);
        _result.ActionButton = action;
        return _result;
    }

    protected void CloseBars()
    {
        _error.IsOpen = _notice.IsOpen = _result.IsOpen = false;
    }

    protected void AddBar(InfoBar bar) => _bars.Children.Insert(0, bar);

    private static void Bar(InfoBar bar, InfoBarSeverity severity, string title, string message)
    {
        bar.Severity = severity;
        bar.Title = title;
        bar.Message = message;
        bar.IsOpen = true;
    }

    protected AppBarButton AddCommand(string label, string glyph, RoutedEventHandler click, bool secondary = false,
        Windows.System.VirtualKey? key = null)
    {
        var b = new AppBarButton { Label = label, Icon = new FontIcon { Glyph = glyph } };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(b, label);
        b.Click += click;
        if (key is Windows.System.VirtualKey k)
        {
            b.KeyboardAccelerators.Add(new Microsoft.UI.Xaml.Input.KeyboardAccelerator { Key = k });
        }
        (secondary ? Commands.SecondaryCommands : Commands.PrimaryCommands).Add(b);
        return b;
    }

    protected AppBarToggleButton AddToggle(string label, string glyph, bool on, RoutedEventHandler click, bool secondary = false)
    {
        var b = new AppBarToggleButton { Label = label, Icon = new FontIcon { Glyph = glyph }, IsChecked = on };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(b, label);
        b.Click += click;
        (secondary ? Commands.SecondaryCommands : Commands.PrimaryCommands).Add(b);
        return b;
    }

    /// <summary>The accent "do it" button at the end of the command bar.</summary>
    protected Button AddPrimary(string text, RoutedEventHandler click)
    {
        var b = new Button { Content = text, Style = Ui.S("AccentButtonStyle"), MinWidth = 160, Margin = new Thickness(8, 0, 4, 0), IsEnabled = false };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(b, text);
        b.Click += click;
        Commands.PrimaryCommands.Add(new AppBarElementContainer { Content = b, VerticalContentAlignment = VerticalAlignment.Center, IsTabStop = false });
        return b;
    }

    protected void AddSeparator(bool secondary = true) =>
        (secondary ? Commands.SecondaryCommands : Commands.PrimaryCommands).Add(new AppBarSeparator());

    /// <summary>A scrolling body column with page padding.</summary>
    protected static ScrollViewer Scroller(UIElement content) => new()
    {
        Content = content,
        VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
        HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        Padding = new Thickness(24, 8, 24, 0),
    };
}
