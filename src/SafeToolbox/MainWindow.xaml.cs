using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;
using SafeToolbox.Core;
using SafeToolbox.Pages;
using Windows.UI.ViewManagement;

namespace SafeToolbox;

public sealed partial class MainWindow : Window, IShell
{
    private readonly Dictionary<string, PageBase> _pages = [];
    private readonly Dictionary<string, Func<PageBase>> _factories;
    private readonly UISettings _uiSettings = new();
    private PageBase? _current;

    public ToolboxSettings Settings { get; }

    public MainWindow(ToolboxSettings settings)
    {
        Settings = settings;
        InitializeComponent();

        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
        string ico = Path.Combine(AppContext.BaseDirectory, "Assets", "AppIcon.ico");
        if (File.Exists(ico))
        {
            AppWindow.SetIcon(ico);
        }
        string png = Path.Combine(AppContext.BaseDirectory, "Assets", "AppIcon.png");
        if (File.Exists(png))
        {
            TitleBarIcon.Source = new BitmapImage(new Uri(png));
        }
        IntPtr hwnd = WinRT.Interop.WindowNative.GetWindowHandle(this);
        WindowSizing.SetMinimumSize(hwnd, 960, 640);
        if (!WindowSizing.Restore(AppWindow, settings.WindowBounds))
        {
            WindowSizing.ResizeAndCenter(AppWindow, hwnd, 1440, 900);
        }
        if (settings.WindowMaximized && AppWindow.Presenter is OverlappedPresenter maximize)
        {
            maximize.Maximize();
        }
        // Remember the normal (not maximised, not minimised) bounds as the window moves, for the next start.
        AppWindow.Changed += (w, e) =>
        {
            if (!(e.DidPositionChange || e.DidSizeChange || e.DidPresenterChange) || w.Presenter is not OverlappedPresenter op)
            {
                return;
            }
            if (op.State != OverlappedPresenterState.Minimized)
            {
                Settings.WindowMaximized = op.State == OverlappedPresenterState.Maximized;
            }
            if (op.State == OverlappedPresenterState.Restored)
            {
                Settings.WindowBounds = [w.Position.X, w.Position.Y, w.Size.Width, w.Size.Height];
            }
        };

        _factories = new()
        {
            ["home"] = () => new HomePage(this),
            ["updates"] = () => new UpdatesPage(this),
            ["cleanup"] = () => new CleanupPage(this),
            ["storage"] = () => new StoragePage(this),
            ["health"] = () => new HealthPage(this),
            ["repair"] = () => new RepairPage(this),
            ["path"] = () => new PathPage(this),
            ["apps"] = () => new AppListPage(this),
            ["history"] = () => new HistoryPage(this),
            ["settings"] = () => new SettingsPage(this),
        };

        ApplyAppearance();
        // Windows' accent switches and colours can change while the app is open; check again on every activation.
        Activated += (_, e) =>
        {
            bool active = e.WindowActivationState != WindowActivationState.Deactivated;
            TitleBarText.Opacity = active ? 1.0 : 0.5;
            if (active)
            {
                ApplyAppearance();
            }
        };
        _uiSettings.ColorValuesChanged += (_, _) => DispatcherQueue.TryEnqueue(() => Appearance.OnSystemColorsChanged(RootGrid, Settings));
        RootGrid.ActualThemeChanged += (_, _) => Appearance.ApplyCaptionButtons(this, RootGrid.ActualTheme);

        RootGrid.Loaded += (_, _) =>
        {
            // The update check and the cleanup scan start with the window, as the separate apps did.
            if (Settings.CheckUpdatesOnOpen)
            {
                Page<UpdatesPage>().EnsureStarted();
            }
            Page<CleanupPage>().EnsureStarted();
            Navigate(StartPage ?? Settings.LastPage ?? "home");
        };
        Closed += (_, _) => Settings.Save();
    }

    /// <summary>Set when a notification opened the app on a specific page.</summary>
    public string? StartPage { get; set; }

    public ElementTheme ActualTheme => RootGrid.ActualTheme;

    public XamlRoot XamlRoot => RootGrid.XamlRoot;

    public Microsoft.UI.WindowId WindowId => AppWindow.Id;

    public void ApplyAppearance() => Appearance.Apply(this, RootGrid, SolidBackdrop, Settings);

    public T Page<T>() where T : PageBase => (T)GetPage(KeyOf(typeof(T)));

    private static string KeyOf(Type t) => t.Name switch
    {
        nameof(HomePage) => "home",
        nameof(UpdatesPage) => "updates",
        nameof(CleanupPage) => "cleanup",
        nameof(StoragePage) => "storage",
        nameof(HealthPage) => "health",
        nameof(RepairPage) => "repair",
        nameof(PathPage) => "path",
        nameof(AppListPage) => "apps",
        nameof(HistoryPage) => "history",
        _ => "settings",
    };

    private PageBase GetPage(string key)
    {
        if (!_pages.TryGetValue(key, out PageBase? page))
        {
            page = _factories[key]();
            _pages[key] = page;
        }
        return page;
    }

    public void Navigate(string tag)
    {
        if (!_factories.ContainsKey(tag))
        {
            tag = "home";
        }
        object? item = tag == "settings"
            ? Nav.SettingsItem
            : Nav.MenuItems.Concat(Nav.FooterMenuItems).OfType<NavigationViewItem>().FirstOrDefault(i => (string)i.Tag == tag);
        if (item is not null && !ReferenceEquals(Nav.SelectedItem, item))
        {
            Nav.SelectedItem = item;
        }
        else
        {
            Show(tag);
        }
    }

    private void Nav_SelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        string tag = args.IsSettingsSelected ? "settings" : (args.SelectedItem as NavigationViewItem)?.Tag as string ?? "home";
        Show(tag);
    }

    private void Show(string tag)
    {
        PageBase page = GetPage(tag);
        if (ReferenceEquals(_current, page))
        {
            return;
        }
        _current = page;
        PageHost.Content = page;
        Settings.LastPage = tag;
        page.OnShown();
    }

    public void SetNavBadge(string tag, int? count)
    {
        NavigationViewItem? item = Nav.MenuItems.Concat(Nav.FooterMenuItems).OfType<NavigationViewItem>().FirstOrDefault(i => (string)i.Tag == tag);
        if (item is null)
        {
            return;
        }
        item.InfoBadge = count is > 0 ? new InfoBadge { Value = count.Value } : null;
    }

    public async Task<ContentDialogResult> AskAsync(string title, object content, string primary, string? close = "Cancel", bool primaryIsDefault = false)
    {
        var dialog = new ContentDialog
        {
            XamlRoot = RootGrid.XamlRoot,
            Style = Application.Current.Resources["DefaultContentDialogStyle"] as Style,
            // Dialogs live outside the page tree, so they take the app's theme explicitly.
            RequestedTheme = RootGrid.ActualTheme,
            Title = title,
            Content = content is string s ? new TextBlock { Text = s, TextWrapping = TextWrapping.Wrap } : content,
            PrimaryButtonText = primary,
            CloseButtonText = close ?? "",
            DefaultButton = primaryIsDefault ? ContentDialogButton.Primary : ContentDialogButton.Close,
        };
        return await dialog.ShowAsync();
    }

    public void ShowError(string title, string message)
    {
        if (_current is not null)
        {
            _current.ShowError(title, message);
        }
    }

    /// <summary>Brings the window forward, for a second launch or a clicked notification.</summary>
    public void BringToFront(string? page)
    {
        if (page is not null)
        {
            Navigate(page);
        }
        if (AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized } p)
        {
            p.Restore();
        }
        Activate();
    }
}
