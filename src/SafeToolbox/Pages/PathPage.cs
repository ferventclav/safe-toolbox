using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using SafeToolbox.Core;
using static SafeToolbox.Ui;

namespace SafeToolbox.Pages;

/// <summary>
/// PATH entries that point nowhere or repeat, winget command links whose target was deleted, and which copy of
/// python, node or git actually runs. Only your user PATH is edited, and only after saving it.
/// </summary>
internal sealed class PathPage : PageBase
{
    private readonly StackPanel _panel = new() { Spacing = 16, Padding = new Thickness(0, 0, 0, 24) };
    private readonly TextBlock _summary = Text("", "SubtitleTextBlockStyle", wrap: false);
    private readonly TextBlock _summarySub = Muted("");
    private readonly Button _fix;
    private List<PathEntry> _entries = [];

    public PathPage(IShell shell) : base(shell, "PATH doctor",
        "Finds PATH entries that point to folders that no longer exist or repeat an earlier one, and command links an update broke.",
        "It edits only your user PATH, and saves the old value first so you can put it back.")
    {
        HeaderRight.Children.Add(_summary);
        HeaderRight.Children.Add(_summarySub);
        AddCommand("Check again", "", (_, _) => Render(), key: Windows.System.VirtualKey.F5);
        AddCommand("Undo the last fix", "", Undo_Click);
        AddCommand("Edit environment variables", "", (_, _) =>
        {
            try
            {
                System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo("rundll32.exe", "sysdm.cpl,EditEnvironmentVariables") { UseShellExecute = true });
            }
            catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or InvalidOperationException)
            {
                ShowError("Couldn't open Environment Variables", ex.Message);
            }
        }, secondary: true);
        _fix = AddPrimary("Fix my user PATH", Fix_Click);
        SetBody(Scroller(_panel));
    }

    protected override Task OnFirstShownAsync()
    {
        Render();
        return Task.CompletedTask;
    }

    private void Render()
    {
        _panel.Children.Clear();
        _entries = PathDoctor.Analyze();
        List<PathEntry> problems = _entries.Where(e => e.Problem != PathProblem.None).ToList();
        int fixable = problems.Count(e => e.Fixable);
        _summary.Text = problems.Count == 0 ? "PATH looks clean" : Plural(problems.Count, "problem");
        _summarySub.Text = $"{_entries.Count(e => e.Problem != PathProblem.Empty)} entries, system and user";
        _fix.IsEnabled = fixable > 0;
        _fix.Content = fixable > 0 ? $"Fix my user PATH ({fixable})" : "Fix my user PATH";

        // problems
        var pb = Stack(10);
        if (problems.Count == 0)
        {
            pb.Children.Add(Row(8, Icon("", 16, "IconSuccess"), Text("Every entry points to a folder that exists, and none repeats.")));
        }
        foreach (PathEntry e in problems)
        {
            string what = e.Problem switch
            {
                PathProblem.Missing => "The folder doesn't exist",
                PathProblem.Duplicate => $"Repeats {e.DuplicateOf}",
                PathProblem.Empty => "Empty entry (two semicolons in a row)",
                PathProblem.NotAFolder => "Points to a file, not a folder",
                _ => "",
            };
            var lines = Stack(2, Text(e.Raw.Length > 0 ? e.Raw : "(empty)", "BodyStrongTextBlockStyle", select: true), Muted(what));
            if (e.Raw != e.Expanded && e.Expanded.Length > 0)
            {
                lines.Children.Add(Text(e.Expanded, "MutedCaptionStyle", select: true));
            }
            if (e.Scope == PathScope.Machine)
            {
                lines.Children.Add(Muted("In the system PATH, which needs admin to change: use Edit environment variables in the … menu."));
            }
            pb.Children.Add(Columns((Badge(e.Scope == PathScope.User ? "User" : "System", e.Fixable ? "caution" : "neutral"), Auto), (lines, Star)));
        }
        _panel.Children.Add(Section("", "Problems", "Missing folders slow down every command lookup a little; repeats just add noise.", pb));

        // commands
        var cb = Stack(10);
        foreach (CommandHit h in PathDoctor.Commands(_entries))
        {
            var lines = Stack(2, Text(h.Command, "BodyStrongTextBlockStyle"), Text("Runs " + h.Runs, "MutedCaptionStyle", select: true));
            if (PathDoctor.IsStoreAlias(h.Runs) && h.Command is "python" or "python3")
            {
                lines.Children.Add(Text("This is Windows' shortcut that opens the Microsoft Store, not a real Python. Turn it off in Settings, Apps, Advanced app settings, App execution aliases.", "CautionCaptionStyle"));
            }
            if (h.Shadowed)
            {
                lines.Children.Add(Muted("Also on PATH, but not used: " + string.Join(", ", h.Locations.Skip(1))));
            }
            cb.Children.Add(Columns((Icon(h.Shadowed ? "" : "", 16, h.Shadowed ? "IconCaution" : "IconMuted"), Auto), (lines, Star)));
        }
        if (cb.Children.Count == 0)
        {
            cb.Children.Add(Muted("None of the common developer commands are on PATH.", caption: false));
        }
        _panel.Children.Add(Section("", "Which copy runs", "When two folders hold the same command, the first one in PATH wins.", cb));

        // broken links
        List<BrokenLink> broken = PathDoctor.BrokenWingetLinks();
        var lb = Stack(10);
        if (broken.Count == 0)
        {
            lb.Children.Add(Row(8, Icon("", 16, "IconSuccess"), Text("Every command link winget made still points to its program.")));
        }
        foreach (BrokenLink b in broken)
        {
            lb.Children.Add(Stack(2, Text(Path.GetFileName(b.Link), "BodyStrongTextBlockStyle"), Text("Points to " + b.Target + ", which is gone.", "MutedCaptionStyle", select: true),
                Muted("Reinstall the package with winget (winget install --id <id> --force) to bring the link back.")));
        }
        _panel.Children.Add(Section("", "winget command links", "Links in WinGet\\Links that make tools like ffmpeg runnable from any terminal.", lb));

        // full list
        var all = Stack(4);
        foreach (PathEntry e in _entries.Where(e => e.Problem != PathProblem.Empty))
        {
            all.Children.Add(Columns((Muted(e.Scope == PathScope.User ? "User" : "System"), Px(56)), (Text(e.Expanded, e.Problem == PathProblem.None ? "MutedCaptionStyle" : "CautionCaptionStyle", select: true), Star)));
        }
        _panel.Children.Add(new Expander
        {
            Header = "Every PATH entry, in the order Windows searches them",
            Content = all,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
        });
        StatusText.Text = PathDoctor.LatestBackup() is string bk ? $"Last fix saved the old PATH to {Path.GetFileName(bk)}." : "";
    }

    private async void Fix_Click(object sender, RoutedEventArgs e)
    {
        List<PathEntry> fix = _entries.Where(x => x.Fixable).ToList();
        if (fix.Count == 0)
        {
            return;
        }
        string list = string.Join("\n", fix.Select(x => "• " + (x.Raw.Length > 0 ? x.Raw : "(empty)")));
        if (await Shell.AskAsync("Remove these from your user PATH?", list + "\n\nThe old PATH is saved first, and Undo the last fix puts it back. Open terminals keep the old PATH until you reopen them.", "Remove") != ContentDialogResult.Primary)
        {
            return;
        }
        try
        {
            string backup = PathDoctor.FixUserPath(fix);
            ShowResult("User PATH fixed", $"{Plural(fix.Count, "entry", "entries")} removed. The old value is in {backup}.");
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or System.Security.SecurityException)
        {
            ShowError("PATH wasn't changed", ex.Message);
        }
        Render();
    }

    private async void Undo_Click(object sender, RoutedEventArgs e)
    {
        if (PathDoctor.LatestBackup() is not string backup)
        {
            ShowNotice("Nothing to undo", "No PATH fix has been made from here yet.");
            return;
        }
        if (await Shell.AskAsync("Put the old user PATH back?", $"Restores the user PATH saved in {Path.GetFileName(backup)}.", "Restore") != ContentDialogResult.Primary)
        {
            return;
        }
        try
        {
            PathDoctor.Restore(backup);
            File.Move(backup, backup + ".restored", overwrite: true);
            ShowResult("Old user PATH restored", "Reopen terminals to pick it up.");
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or System.Security.SecurityException)
        {
            ShowError("PATH wasn't changed", ex.Message);
        }
        Render();
    }

    private static Border Section(string glyph, string title, string sub, UIElement body) =>
        Card(Stack(12, CardHeader(glyph, title, sub), body), 20);
}
