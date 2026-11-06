using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using SafeToolbox.Core;
using static SafeToolbox.Ui;

namespace SafeToolbox.Pages;

/// <summary>Saves what is installed (winget apps, pip, npm, VS Code extensions) and restores apps with winget import.</summary>
internal sealed class AppListPage : PageBase
{
    private readonly StackPanel _panel = new() { Spacing = 16, Padding = new Thickness(0, 0, 0, 24), MaxWidth = 1000, HorizontalAlignment = HorizontalAlignment.Left };
    private readonly StackPanel _saveResult = new() { Spacing = 6 };
    private readonly StackPanel _restoreResult = new() { Spacing = 6 };
    private readonly CheckBox _versions = new() { Content = "Include the exact versions (import then installs those, not the newest)" };
    private readonly Button _save;

    public AppListPage(IShell shell) : base(shell, "App list",
        "Saves the list of what's installed, to set up a new PC or get back to normal after a reset.",
        "Saving only reads. Restoring runs winget import in a window you can watch.")
    {
        HideFooter();
        _save = Ui.Button("Save my app list", async (_, _) => await SaveAsync(), accent: true);
        _panel.Children.Add(Card(Stack(12,
            CardHeader("", "Save", "winget apps as winget-apps.json (the file winget import reads), plus plain lists of pip user packages, global npm packages and VS Code extensions, and a README with what to do."),
            _versions,
            _save,
            _saveResult), 20));

        _panel.Children.Add(Card(Stack(12,
            CardHeader("", "Restore", "Pick a winget-apps.json saved here or by winget export. You see the list before anything installs."),
            Ui.Button("Pick a saved list", async (_, _) => await PickAsync()),
            _restoreResult), 20));
        SetBody(Scroller(_panel));
    }

    private async Task SaveAsync()
    {
        string root = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments), "Safe Toolbox", "App lists");
        string folder = Path.Combine(root, $"{Environment.MachineName} {DateTime.Now:yyyy-MM-dd HHmm}");
        _save.IsEnabled = false;
        _saveResult.Children.Clear();
        var ring = Row(8, new ProgressRing { IsActive = true, Width = 16, Height = 16 }, Muted("Asking winget, pip, npm and VS Code what's installed. This takes about a minute.", caption: false));
        _saveResult.Children.Add(ring);
        ExportResult result;
        try
        {
            bool versions = _versions.IsChecked == true;
            result = await Task.Run(() => AppList.ExportAsync(folder, versions));
        }
        catch (Exception ex)
        {
            _saveResult.Children.Clear();
            ShowError("The list wasn't saved", ex.Message);
            _save.IsEnabled = true;
            return;
        }
        _save.IsEnabled = true;
        _saveResult.Children.Clear();
        _saveResult.Children.Add(Text("Saved to " + result.Folder, "BodyStrongTextBlockStyle", select: true));
        foreach (ExportPart p in result.Parts)
        {
            _saveResult.Children.Add(Columns((Icon("", 14, "IconSuccess"), Auto), (Stack(0, Text($"{p.Name}: {p.Count}"), Muted(p.Note ?? Path.GetFileName(p.File))), Star)));
        }
        if (result.NotInWinget.Count > 0)
        {
            var missing = Stack(2, Text($"{Plural(result.NotInWinget.Count, "app")} winget can't reinstall", "BodyStrongTextBlockStyle"),
                Muted("Reinstall these by hand on the new PC. The README lists them too.", caption: false));
            foreach (string m in result.NotInWinget.Take(40))
            {
                missing.Children.Add(Muted("• " + m));
            }
            if (result.NotInWinget.Count > 40)
            {
                missing.Children.Add(Muted($"and {result.NotInWinget.Count - 40} more in the README"));
            }
            missing.Margin = new Thickness(0, 6, 0, 0);
            _saveResult.Children.Add(missing);
        }
        _saveResult.Children.Add(Ui.Button("Open the folder", (_, _) => Open(result.Folder)));
    }

    private async Task PickAsync()
    {
        string? file;
        try
        {
            var picker = new Microsoft.Windows.Storage.Pickers.FileOpenPicker(Shell.WindowId);
            picker.FileTypeFilter.Add(".json");
            var picked = await picker.PickSingleFileAsync();
            file = picked?.Path;
        }
        catch (Exception ex)
        {
            ShowError("The file picker didn't open", ex.Message);
            return;
        }
        if (file is null)
        {
            return;
        }
        List<string> ids = AppList.WingetIds(await File.ReadAllTextAsync(file));
        _restoreResult.Children.Clear();
        if (ids.Count == 0)
        {
            _restoreResult.Children.Add(Text("That file has no winget packages in it.", "CautionBodyStyle"));
            return;
        }
        _restoreResult.Children.Add(Text($"{Plural(ids.Count, "app")} in {Path.GetFileName(file)}", "BodyStrongTextBlockStyle"));
        _restoreResult.Children.Add(Muted(string.Join(", ", ids.Take(60)) + (ids.Count > 60 ? $", and {ids.Count - 60} more" : ""), caption: false));
        _restoreResult.Children.Add(Muted("Apps you already have are skipped by winget. Some installers ask for admin themselves.", caption: false));
        _restoreResult.Children.Add(Ui.Button($"Install them with winget", async (_, _) =>
        {
            if (await Shell.AskAsync("Install these apps?", $"A terminal window opens and runs winget import for {Plural(ids.Count, "app")}. You can watch it and close it to stop.", "Open the terminal") == ContentDialogResult.Primary)
            {
                AppList.StartImport(file);
            }
        }, accent: true));
    }
}
