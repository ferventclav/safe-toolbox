using System.Diagnostics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;

namespace SafeToolbox;

/// <summary>Small builders so the pages read like a layout, plus the shared page frame.</summary>
internal static class Ui
{
    public static Style S(string key) => (Style)Application.Current.Resources[key];

    public static TextBlock Text(string text, string style = "BodyTextBlockStyle", bool wrap = true, bool select = false) => new()
    {
        Text = text,
        Style = S(style),
        TextWrapping = wrap ? TextWrapping.Wrap : TextWrapping.NoWrap,
        TextTrimming = wrap ? TextTrimming.None : TextTrimming.CharacterEllipsis,
        IsTextSelectionEnabled = select,
    };

    public static TextBlock Muted(string text, bool caption = true) => Text(text, caption ? "MutedCaptionStyle" : "MutedBodyStyle");

    public static FontIcon Icon(string glyph, double size = 16, string style = "IconPlain") => new()
    {
        Glyph = glyph,
        FontSize = size,
        Style = S(style),
        VerticalAlignment = VerticalAlignment.Center,
    };

    public static Border Card(UIElement child, double padding = 16) => new()
    {
        Style = S("CardStyle"),
        Padding = new Thickness(padding),
        Child = child,
    };

    public static StackPanel Stack(double spacing = 8, params UIElement[] children)
    {
        var p = new StackPanel { Spacing = spacing };
        foreach (UIElement c in children)
        {
            p.Children.Add(c);
        }
        return p;
    }

    public static StackPanel Row(double spacing = 8, params UIElement[] children)
    {
        var p = new StackPanel { Orientation = Orientation.Horizontal, Spacing = spacing };
        foreach (UIElement c in children)
        {
            p.Children.Add(c);
        }
        return p;
    }

    /// <summary>A badge: "neutral", "caution", "critical" or "success".</summary>
    public static Border Badge(string text, string tone = "neutral")
    {
        string t = char.ToUpperInvariant(tone[0]) + tone[1..];
        return new Border { Style = S("Badge" + t), Child = new TextBlock { Text = text, Style = S("Badge" + t + "Text") } };
    }

    public static void SetBadge(Border badge, string? text, string tone = "neutral")
    {
        string t = char.ToUpperInvariant(tone[0]) + tone[1..];
        badge.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;
        badge.Style = S("Badge" + t);
        if (badge.Child is TextBlock tb)
        {
            tb.Text = text ?? "";
            tb.Style = S("Badge" + t + "Text");
        }
    }

    public static Button Button(string text, RoutedEventHandler click, bool accent = false, string? glyph = null)
    {
        object content = glyph is null ? text : Row(8, Icon(glyph, 14, accent ? "IconPlain" : "IconPlain"), new TextBlock { Text = text });
        var b = new Button { Content = content };
        if (accent)
        {
            b.Style = S("AccentButtonStyle");
            b.Content = text;
        }
        AutomationProperties.SetName(b, text);
        b.Click += click;
        return b;
    }

    public static HyperlinkButton Link(string text, string url)
    {
        var l = new HyperlinkButton { Content = text, Padding = new Thickness(0, 2, 0, 2) };
        if (Uri.TryCreate(url, UriKind.Absolute, out Uri? u) && (u.Scheme == "https" || u.Scheme == "http" || u.Scheme == "ms-settings"))
        {
            l.NavigateUri = u;
        }
        ToolTipService.SetToolTip(l, url);
        return l;
    }

    public static HyperlinkButton LinkButton(string text, RoutedEventHandler click)
    {
        var l = new HyperlinkButton { Content = text, Padding = new Thickness(0, 2, 0, 2) };
        l.Click += click;
        return l;
    }

    public static Grid Columns(params (UIElement Element, GridLength Width)[] cols)
    {
        var g = new Grid { ColumnSpacing = 12 };
        for (int i = 0; i < cols.Length; i++)
        {
            g.ColumnDefinitions.Add(new ColumnDefinition { Width = cols[i].Width });
            if (cols[i].Element is FrameworkElement fe)
            {
                Grid.SetColumn(fe, i);
            }
            g.Children.Add(cols[i].Element);
        }
        return g;
    }

    public static GridLength Star => new(1, GridUnitType.Star);
    public static GridLength Auto => GridLength.Auto;

    public static GridLength Px(double v) => new(v);

    /// <summary>A heading with its count or size on the right, used at the top of each card.</summary>
    public static Grid CardHeader(string glyph, string title, string? sub, UIElement? right = null)
    {
        var titles = Stack(0, Text(title, "BodyStrongTextBlockStyle"));
        if (sub is not null)
        {
            titles.Children.Add(Muted(sub));
        }
        return Columns((Icon(glyph, 18), Auto), (titles, Star), (right ?? new Border(), Auto));
    }

    /// <summary>"label: value" lines in a details pane.</summary>
    public static StackPanel Field(string label, string value, bool mono = false) => Stack(2,
        Muted(label),
        new TextBlock
        {
            Text = value,
            Style = S(mono ? "MonoCaptionStyle" : "BodyTextBlockStyle"),
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = true,
        });

    public static TextBlock Rich(params (string Text, bool Strong)[] parts)
    {
        var tb = new TextBlock { Style = S("BodyTextBlockStyle"), TextWrapping = TextWrapping.Wrap };
        foreach ((string text, bool strong) in parts)
        {
            tb.Inlines.Add(new Run { Text = text, FontWeight = strong ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal });
        }
        return tb;
    }

    public static void Open(string target)
    {
        try
        {
            Process.Start(new ProcessStartInfo(target) { UseShellExecute = true });
        }
        catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or InvalidOperationException)
        {
            Core.ToolboxPaths.WriteError($"Could not open {target}: {ex.Message}");
        }
    }

    /// <summary>Opens Explorer with the file selected.</summary>
    public static void ShowInFolder(string path) =>
        Process.Start(new ProcessStartInfo("explorer.exe", $"/select,\"{path}\"") { UseShellExecute = true });

    public static string Ago(DateTime t)
    {
        TimeSpan d = DateTime.Now - t;
        if (t.Date == DateTime.Today)
        {
            return $"today at {t:HH:mm}";
        }
        if (t.Date == DateTime.Today.AddDays(-1))
        {
            return $"yesterday at {t:HH:mm}";
        }
        return d.TotalDays < 7 ? $"{t:dddd} at {t:HH:mm}" : $"{t:d MMM} at {t:HH:mm}";
    }

    public static string Plural(int n, string one, string? many = null) => $"{n} {(n == 1 ? one : many ?? one + "s")}";
}
