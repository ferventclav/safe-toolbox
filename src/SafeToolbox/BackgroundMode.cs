using SafeToolbox.Core;

namespace SafeToolbox;

/// <summary>"--background" (from Task Scheduler) and "--check" (on demand). No window, no installs, no deletes.</summary>
internal static class BackgroundMode
{
    public static int Run(bool scheduled, bool json)
    {
        try
        {
            ToolboxSettings settings = ToolboxSettings.Load();
            CheckSummary? last = CheckSummary.Load();
            DateTime now = DateTime.Now;
            bool updates = !scheduled || Background.UpdatesDue(settings, last?.UpdatesWhen, now);
            bool cleanup = !scheduled || Background.CleanupDue(settings, last?.CleanupWhen, now);
            if (!updates && !cleanup)
            {
                return 0;
            }
            CheckSummary summary = Background.RunAsync(updates, cleanup, settings).GetAwaiter().GetResult();
            if (scheduled && settings.Notify && Background.Notification(summary, settings, updates, cleanup) is (string title, string body))
            {
                string page = updates && summary.Updates.Count > 0 && summary.UpdatesKey != summary.NotifiedUpdates ? "updates" : "cleanup";
                if (Notifier.Show(title, body, page) && updates)
                {
                    summary.NotifiedUpdates = summary.UpdatesKey;
                    summary.Save();
                }
            }
            if (json)
            {
                Console.Out.WriteLine(summary.ToJson());
                Console.Out.Flush();
            }
            return 0;
        }
        catch (Exception ex)
        {
            ToolboxPaths.WriteError("Background check failed: " + ex);
            return 1;
        }
    }
}
