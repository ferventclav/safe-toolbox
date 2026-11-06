using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using SafeToolbox.Core;
using static SafeToolbox.Ui;

namespace SafeToolbox.Pages;

internal sealed class SettingsPage : PageBase
{
    private readonly TextBlock _accentNote = Muted("");
    private readonly TextBlock _taskStatus = Muted("");
    private readonly StackPanel _panel = new() { Spacing = 4, Padding = new Thickness(0, 0, 0, 24), MaxWidth = 1000, HorizontalAlignment = HorizontalAlignment.Left };
    private bool _loading = true;

    public SettingsPage(IShell shell) : base(shell, "Settings", "How Safe Toolbox looks, and what it checks by itself.")
    {
        HideFooter();
        ToolboxSettings s = Shell.Settings;

        Section("Appearance");
        _panel.Children.Add(SettingRow("", "Theme", "Light, dark, or the same as Windows.",
            Combo(["Use my Windows setting", "Light", "Dark"], (int)s.Theme, i => { s.Theme = (ThemeMode)i; Changed(); })));
        var accent = Combo(["Follow Windows", "Always", "Never"], (int)s.Accent, i => { s.Accent = (AccentMode)i; Changed(); });
        _panel.Children.Add(SettingRow("", "Accent colour", "Colour on buttons, ticks, switches and progress bars.", accent, _accentNote));
        _panel.Children.Add(SettingRow("", "Background", "Mica Alt is tinted by your wallpaper, Mica is subtler, Solid has no transparency.",
            Combo(["Mica Alt", "Mica", "Solid"], (int)s.Backdrop, i => { s.Backdrop = (BackdropMode)i; Changed(); })));

        Section("When Safe Toolbox opens");
        _panel.Children.Add(SettingRow("", "Check for updates", "Starts the update check as soon as the window opens, as Safe Updater did.",
            Switch(s.CheckUpdatesOnOpen, on => { s.CheckUpdatesOnOpen = on; s.Save(); })));

        Section("Background checks");
        _panel.Children.Add(Muted("A Task Scheduler task runs Safe Toolbox without a window, as you, never as admin. It only looks and tells you. Nothing is installed or deleted in the background.", caption: false));
        _panel.Children.Add(SettingRow("", "Check for updates", "Notifies you when safe updates are ready, the same list the Updates page shows.",
            Combo(["Off", "Every day", "Every week"], (int)s.UpdateCheck, i => { s.UpdateCheck = (Frequency)i; SyncTask(); })));
        _panel.Children.Add(SettingRow("", "Remind me to clean up", "Scans like the Clean up page and notifies you when there is a lot to free, or when C: is nearly full.",
            Combo(["Off", "Every day", "Every week"], (int)s.CleanupReminder, i => { s.CleanupReminder = (Frequency)i; SyncTask(); })));
        int[] mb = [500, 1024, 2048, 5120];
        _panel.Children.Add(SettingRow("", "Remind me from", "The cleanup reminder waits until at least this much can be freed.",
            Combo(["500 MB", "1 GB", "2 GB", "5 GB"], Math.Max(0, Array.IndexOf(mb, s.ReminderThresholdMb)), i => { s.ReminderThresholdMb = mb[i]; s.Save(); })));
        _panel.Children.Add(SettingRow("", "Notifications", "Show a Windows notification when a background check finds something.",
            Switch(s.Notify, on => { s.Notify = on; s.Save(); })));
        var test = Ui.Button("Send a test notification", (_, _) =>
        {
            bool ok = Notifier.Show("Safe Toolbox", "Notifications from Safe Toolbox are working.", "home");
            ShowNotice(ok ? "Notification sent" : "Notifications aren't available",
                ok ? "If nothing appeared, check that notifications are on for Safe Toolbox in Windows Settings, System, Notifications."
                   : "Windows didn't accept it. Background checks still save their result, and the Home page shows it.",
                ok ? InfoBarSeverity.Success : InfoBarSeverity.Warning);
        });
        var runNow = Ui.Button("Check now in the background", async (_, _) => await RunBackgroundNowAsync());
        var taskRow = Stack(6, _taskStatus, Row(8, test, runNow));
        taskRow.Margin = new Thickness(0, 4, 0, 8);
        _panel.Children.Add(taskRow);

        Section("Updates");
        SafeUpdater.Core.Settings us = Shell.Page<UpdatesPage>().UpdaterSettings;
        _panel.Children.Add(SettingRow("", "Make a restore point before admin updates", "Skipped when Windows made one in the last 24 hours.",
            Switch(us.RestorePoint, on => { us.RestorePoint = on; us.Save(); })));
        _panel.Children.Add(SettingRow("", "Test my tools after a Python update", "Runs the tool check before and after; a check that passed before and fails after rolls the update back.",
            Switch(us.TestAfterUpdate, on => { us.TestAfterUpdate = on; us.Save(); })));
        int hidden = us.Ignored.Count + us.Skipped.Count;
        _panel.Children.Add(SettingRow("", "Hidden and skipped updates", $"{Plural(hidden, "update")} hidden by you. Each one can also be shown again from the Updates page.",
            Ui.Button("Show them all again", (_, _) =>
            {
                us.Ignored.Clear();
                us.Skipped.Clear();
                us.Save();
                ShowNotice("Done", "Every hidden or skipped update is listed again after the next check.");
            })));

        Section("Clean up");
        _panel.Children.Add(SettingRow("", "Remember my ticks", "Keeps the rows you tick or untick for the next scan.",
            Switch(s.RememberCleanupTicks, on => { s.RememberCleanupTicks = on; s.Save(); })));
        _panel.Children.Add(SettingRow("", "Forget my ticks", $"{Plural(s.CleanupChoices.Count, "row")} differ from the defaults now.",
            Ui.Button("Reset to defaults", (_, _) =>
            {
                s.CleanupChoices.Clear();
                s.Save();
                ShowNotice("Done", "Rescan the Clean up page to see the default ticks.");
            })));

        Section("About");
        var about = Stack(6,
            Text($"Safe Toolbox {typeof(App).Assembly.GetName().Version?.ToString(3)}", "BodyStrongTextBlockStyle"),
            Muted("Safe Updater and Safe Disk Cleaner in one app, plus health checks, repairs, a PATH doctor, a big-files view and an app-list backup.", caption: false),
            Muted("What it never does: install or delete anything in the background, change Windows' services or privacy settings, clean the registry, or touch your documents without you asking for that file.", caption: false),
            Row(8,
                Ui.Button("Open the settings folder", (_, _) => Open(ToolboxPaths.Root)),
                Ui.Button("Open the update logs", (_, _) => Open(SafeUpdater.Core.AppPaths.Logs)),
                Ui.Button("Notification settings", (_, _) => Open("ms-settings:notifications"))));
        _panel.Children.Add(Card(about));

        SetBody(Scroller(_panel));
        _loading = false;
    }

    public override void OnShown()
    {
        bool windows = Appearance.WindowsShowsAccent();
        _accentNote.Text = (windows
            ? "Windows shows the accent colour on title bars and window borders"
            : "Windows is set not to show the accent colour on title bars and window borders")
            + (Shell.Settings.Accent == AccentMode.FollowWindows ? $", so Safe Toolbox {(windows ? "uses it" : "uses neutral greys")}." : ".");
        RefreshTaskStatus();
    }

    private void Changed()
    {
        if (_loading)
        {
            return;
        }
        Shell.Settings.Save();
        Shell.ApplyAppearance();
        OnShown();
    }

    private void SyncTask()
    {
        if (_loading)
        {
            return;
        }
        Shell.Settings.Save();
        string exe = Environment.ProcessPath ?? "";
        string? error = Schedule.Sync(Shell.Settings, exe);
        if (error is not null)
        {
            ShowError("Task Scheduler didn't take the change", error);
        }
        RefreshTaskStatus();
    }

    private void RefreshTaskStatus()
    {
        Frequency? f = Schedule.TaskFrequency(Shell.Settings);
        bool exists = Schedule.Exists();
        CheckSummary? last = CheckSummary.Load();
        string lastText = last is null ? "No background check has run yet." : $"Last check {Ago(last.When)}.";
        _taskStatus.Text = f is null
            ? (exists ? "Both checks are off, but the task still exists. Change a setting above to remove it." : "Off. No task is set up.")
            : exists
                ? $"On: the task \"{Schedule.TaskName}\" runs {(f == Frequency.Daily ? "every day" : "every Sunday")} around 12:30, or at the next chance if the PC was off. {lastText}"
                : "The task is missing. Change a setting above to create it again.";
    }

    private async Task RunBackgroundNowAsync()
    {
        StatusText.Text = "Checking…";
        ShowNotice("Checking in the background", "This takes up to a minute. The Home page shows what it found.");
        ToolboxSettings s = Shell.Settings;
        CheckSummary summary = await Task.Run(() => SafeToolbox.Core.Background.RunAsync(updates: true, cleanup: true, s));
        if (SafeToolbox.Core.Background.Notification(summary, s, true, true) is (string title, string body))
        {
            Notifier.Show(title, body, summary.Updates.Count > 0 ? "updates" : "cleanup");
            ShowNotice(title, body, InfoBarSeverity.Success);
        }
        else
        {
            ShowNotice("Nothing to report", "No safe updates and not much to clean, so a background check would stay quiet.", InfoBarSeverity.Success);
        }
        RefreshTaskStatus();
    }

    // ------------------------------------------------------------------ builders

    private void Section(string title)
    {
        var t = Text(title, "BodyStrongTextBlockStyle");
        t.Margin = new Thickness(0, _panel.Children.Count == 0 ? 0 : 20, 0, 4);
        _panel.Children.Add(t);
    }

    private static Border SettingRow(string glyph, string title, string description, FrameworkElement control, TextBlock? note = null)
    {
        var text = Stack(0, Text(title), Muted(description));
        if (note is not null)
        {
            text.Children.Add(note);
        }
        text.VerticalAlignment = VerticalAlignment.Center;
        control.VerticalAlignment = VerticalAlignment.Center;
        AutomationProperties.SetName(control, title);
        Border card = Card(Columns((Icon(glyph, 18), Auto), (text, Star), (control, Auto)));
        card.Padding = new Thickness(16, 14, 16, 14);
        card.Margin = new Thickness(0, 0, 0, 2);
        return card;
    }

    private ComboBox Combo(string[] items, int selected, Action<int> changed)
    {
        var c = new ComboBox { MinWidth = 200 };
        foreach (string i in items)
        {
            c.Items.Add(i);
        }
        c.SelectedIndex = Math.Clamp(selected, 0, items.Length - 1);
        c.SelectionChanged += (_, _) =>
        {
            if (!_loading && c.SelectedIndex >= 0)
            {
                changed(c.SelectedIndex);
            }
        };
        return c;
    }

    private ToggleSwitch Switch(bool on, Action<bool> changed)
    {
        var t = new ToggleSwitch { IsOn = on, OnContent = "On", OffContent = "Off", MinWidth = 0 };
        t.Toggled += (_, _) =>
        {
            if (!_loading)
            {
                changed(t.IsOn);
            }
        };
        return t;
    }
}
