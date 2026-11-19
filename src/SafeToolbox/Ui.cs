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

    /// <summary>A list row: 40 high, a hover fill, and the accent pill when selected. Padding never changes with state.</summary>
    internal sealed class ListRow
    {
        private readonly Border _pill = new() { Style = S("RowPillStyle"), Visibility = Visibility.Collapsed };
        private bool _hover;
        private bool _selected;

        public Border Root { get; }

        public ListRow(FrameworkElement content)
        {
            content.Margin = new Thickness(12, 6, 12, 6);
            var host = new Grid();
            host.Children.Add(content);
            host.Children.Add(_pill);
            Root = new Border { Style = S("RowStyle"), Child = host };
            Root.PointerEntered += (_, _) => { _hover = true; Apply(); };
            Root.PointerExited += (_, _) => { _hover = false; Apply(); };
        }

        public bool Selected
        {
            get => _selected;
            set
            {
                _selected = value;
                _pill.Visibility = value ? Visibility.Visible : Visibility.Collapsed;
                Apply();
            }
        }

        private void Apply() => Root.Style = S(_selected ? "SelectedRowStyle" : _hover ? "RowHoverStyle" : "RowStyle");
    }

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

    /// <summary>A badge: "neutral", "caution", "critical" or "success". A status colour always comes with an icon.</summary>
    public static Border Badge(string text, string tone = "neutral")
    {
        var badge = new Border { Child = Row(4, new FontIcon { FontSize = 12, VerticalAlignment = VerticalAlignment.Center }, new TextBlock()) };
        SetBadge(badge, text, tone);
        return badge;
    }

    public static void SetBadge(Border badge, string? text, string tone = "neutral")
    {
        string t = char.ToUpperInvariant(tone[0]) + tone[1..];
        badge.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;
        badge.Style = S("Badge" + t);
        if (badge.Child is StackPanel row && row.Children[0] is FontIcon icon && row.Children[1] is TextBlock label)
        {
            label.Text = text ?? "";
            label.Style = S("Badge" + t + "Text");
            string glyph = tone switch { "success" => "\uE73E", "caution" => "\uE7BA", "critical" => "\uE711", _ => "" };
            icon.Visibility = glyph.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
            if (glyph.Length > 0)
            {
                icon.Glyph = glyph;
                icon.Style = S("Icon" + t);
            }
        }
    }

    public static Button Button(string text, RoutedEventHandler click, bool accent = false, string? glyph = null)
    {
        object content = glyph is null ? text : Row(8, Icon(glyph, 16, accent ? "IconPlain" : "IconPlain"), new TextBlock { Text = text });
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
        return Columns((Icon(glyph, 20), Auto), (titles, Star), (right ?? new Border(), Auto));
    }

    /// <summary>"label: value" lines in a details pane.</summary>
    public static StackPanel Field(string label, string value, bool mono = false) => Stack(4,
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
