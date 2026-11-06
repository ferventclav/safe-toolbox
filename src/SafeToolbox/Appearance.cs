using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.Win32;
using SafeToolbox.Core;
using Windows.UI;
using Windows.UI.ViewManagement;

namespace SafeToolbox;

/// <summary>
/// Theme, backdrop and accent colour. WinUI colours buttons, ticks, toggles, progress bars and the selected
/// sidebar item from seven accent shades. When the accent should not show, those shades are replaced with a
/// neutral graphite ramp, so the app looks like Windows does with "Show accent colour" switched off.
/// </summary>
internal static class Appearance
{
    private static readonly string[] ShadeKeys =
    [
        "SystemAccentColor", "SystemAccentColorLight1", "SystemAccentColorLight2", "SystemAccentColorLight3",
        "SystemAccentColorDark1", "SystemAccentColorDark2", "SystemAccentColorDark3",
    ];

    private static readonly UIColorType[] ShadeTypes =
    [
        UIColorType.Accent, UIColorType.AccentLight1, UIColorType.AccentLight2, UIColorType.AccentLight3,
        UIColorType.AccentDark1, UIColorType.AccentDark2, UIColorType.AccentDark3,
    ];

    // Dark theme fills use Light2 (black text on it) and links Light3; light theme fills use Dark1 (white text)
    // and links Dark2. A grey ramp keeps both readable: about 13:1 for text on fills in each theme.
    private static readonly Color[] Neutral =
    [
        Color.FromArgb(255, 0x80, 0x80, 0x80), // SystemAccentColor (selected text background)
        Color.FromArgb(255, 0xA8, 0xA8, 0xA8), // Light1
        Color.FromArgb(255, 0xD6, 0xD6, 0xD6), // Light2: dark-theme buttons, ticks, bars
        Color.FromArgb(255, 0xEE, 0xEE, 0xEE), // Light3: dark-theme links
        Color.FromArgb(255, 0x3A, 0x3A, 0x3A), // Dark1: light-theme buttons, ticks, bars
        Color.FromArgb(255, 0x2B, 0x2B, 0x2B), // Dark2: light-theme links
        Color.FromArgb(255, 0x1C, 0x1C, 0x1C), // Dark3
    ];

    private static readonly UISettings Ui = new();
    private static bool? s_neutral;

    /// <summary>
    /// "Show accent colour on title bars and window borders" (HKCU\...\DWM\ColorPrevalence). It is the switch
    /// Windows offers for accent colour in app windows; the Start and taskbar switch is about the shell.
    /// </summary>
    public static bool WindowsShowsAccent()
    {
        try
        {
            using RegistryKey? dwm = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\DWM");
            return dwm?.GetValue("ColorPrevalence") is int v && v == 1;
        }
        catch (Exception ex) when (ex is System.Security.SecurityException or UnauthorizedAccessException or IOException)
        {
            return true;
        }
    }

    public static bool UseAccent(AccentMode mode) => mode switch
    {
        AccentMode.Always => true,
        AccentMode.Never => false,
        _ => WindowsShowsAccent(),
    };

    public static ElementTheme ToElementTheme(ThemeMode t) => t switch
    {
        ThemeMode.Light => ElementTheme.Light,
        ThemeMode.Dark => ElementTheme.Dark,
        _ => ElementTheme.Default,
    };

    /// <summary>Applied in the App constructor (the only place Windows allows it), so pop-ups start in the chosen theme too.</summary>
    public static void ApplyStartupTheme(Application app, ToolboxSettings s)
    {
        if (s.Theme != ThemeMode.System)
        {
            app.RequestedTheme = s.Theme == ThemeMode.Light ? ApplicationTheme.Light : ApplicationTheme.Dark;
        }
    }

    /// <summary>
    /// The accent override. It has to wait for OnLaunched: Application.Resources throws E_UNEXPECTED while the
    /// App constructor is still running, and that stops the process before any window exists.
    /// </summary>
    public static void ApplyStartupShades(ToolboxSettings s) => SetShades(!UseAccent(s.Accent));

    /// <summary>Applies everything to a running window. Cheap; called on activation to pick up Windows changes.</summary>
    public static void Apply(Window window, FrameworkElement root, Border solidBackdrop, ToolboxSettings s)
    {
        ElementTheme wanted = ToElementTheme(s.Theme);
        bool themeChanged = root.RequestedTheme != wanted;
        root.RequestedTheme = wanted;

        bool neutralChanged = SetShades(!UseAccent(s.Accent));
        if (neutralChanged && !themeChanged)
        {
            Refresh(root);
        }

        ApplyBackdrop(window, solidBackdrop, s.Backdrop);
        ApplyCaptionButtons(window, root.ActualTheme);
    }

    /// <summary>Overrides or restores the accent shades. Returns true when that changed anything.</summary>
    private static bool SetShades(bool neutral)
    {
        if (s_neutral == neutral)
        {
            return false;
        }
        s_neutral = neutral;
        ResourceDictionary res = Application.Current.Resources;
        for (int i = 0; i < ShadeKeys.Length; i++)
        {
            res[ShadeKeys[i]] = neutral ? Neutral[i] : Ui.GetColorValue(ShadeTypes[i]);
        }
        return true;
    }

    /// <summary>Windows changed its accent colour: take the new shades unless the app is neutral.</summary>
    public static void OnSystemColorsChanged(FrameworkElement root, ToolboxSettings s)
    {
        s_neutral = null;
        SetShades(!UseAccent(s.Accent));
        Refresh(root);
    }

    /// <summary>
    /// ThemeResource references are re-read only when the theme changes, so flipping the theme and back makes
    /// every brush pick up the new shades without restarting.
    /// </summary>
    private static void Refresh(FrameworkElement root)
    {
        ElementTheme current = root.RequestedTheme;
        root.RequestedTheme = root.ActualTheme == ElementTheme.Dark ? ElementTheme.Light : ElementTheme.Dark;
        root.RequestedTheme = current;
    }

    private static void ApplyBackdrop(Window window, Border solid, BackdropMode mode)
    {
        switch (mode)
        {
            case BackdropMode.Solid:
                window.SystemBackdrop = null;
                solid.Visibility = Visibility.Visible;
                break;
            case BackdropMode.Mica:
                if (window.SystemBackdrop is not MicaBackdrop { Kind: MicaKind.Base })
                {
                    window.SystemBackdrop = new MicaBackdrop { Kind = MicaKind.Base };
                }
                solid.Visibility = Visibility.Collapsed;
                break;
            default:
                if (window.SystemBackdrop is not MicaBackdrop { Kind: MicaKind.BaseAlt })
                {
                    window.SystemBackdrop = new MicaBackdrop { Kind = MicaKind.BaseAlt };
                }
                solid.Visibility = Visibility.Collapsed;
                break;
        }
    }

    /// <summary>The minimise, maximise and close glyphs follow the app's theme, not only Windows'.</summary>
    public static void ApplyCaptionButtons(Window window, ElementTheme theme)
    {
        AppWindowTitleBar bar = window.AppWindow.TitleBar;
        bool dark = theme == ElementTheme.Dark;
        Color fg = dark ? Colors(255, 255, 255, 255) : Colors(255, 0, 0, 0);
        bar.ButtonBackgroundColor = Colors(0, 0, 0, 0);
        bar.ButtonInactiveBackgroundColor = Colors(0, 0, 0, 0);
        bar.ButtonForegroundColor = fg;
        bar.ButtonHoverForegroundColor = fg;
        bar.ButtonPressedForegroundColor = fg;
        bar.ButtonInactiveForegroundColor = dark ? Colors(255, 0x80, 0x80, 0x80) : Colors(255, 0x90, 0x90, 0x90);
        bar.ButtonHoverBackgroundColor = dark ? Colors(0x15, 255, 255, 255) : Colors(0x0F, 0, 0, 0);
        bar.ButtonPressedBackgroundColor = dark ? Colors(0x0B, 255, 255, 255) : Colors(0x08, 0, 0, 0);
    }

    private static Color Colors(byte a, byte r, byte g, byte b) => Color.FromArgb(a, r, g, b);

    public static string Describe(AccentMode m) => m switch
    {
        AccentMode.Always => "Always",
        AccentMode.Never => "Never",
        _ => "Follow Windows",
    };
}
