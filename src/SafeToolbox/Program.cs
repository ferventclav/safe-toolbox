using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using SafeToolbox.Core;

namespace SafeToolbox;

public static class Program
{
    [DllImport("Microsoft.ui.xaml.dll")]
    private static extern void XamlCheckProcessRequirements();

    [DllImport("kernel32.dll")]
    private static extern bool AttachConsole(int pid);

    /// <summary>
    /// One exe, several roles. Without arguments it is the window. The others run without any UI:
    ///   --worker job.json         elevated helper for app, Chocolatey and Windows updates
    ///   --clean-worker job.json   elevated helper for the Windows cleanup rules
    ///   --repair id log           elevated helper for a repair
    ///   --admin-task name out     elevated helper for a single admin read (drive wear, restore point)
    ///   --background              the scheduled check: looks, then notifies; never installs or deletes
    ///   --check [--json]          the same check on demand, for scripts and agents
    /// </summary>
    [STAThread]
    private static int Main(string[] args)
    {
        switch (args)
        {
            case ["--worker", string job]:
                return SafeUpdater.Core.Worker.Run(job);
            case ["--clean-worker", string job]:
                return SafeDiskCleaner.Core.AdminWorker.Run(job);
            case ["--repair", string id, string log]:
                return Repairs.RunElevated(id, log);
            case ["--admin-task", string name, string output]:
                return AdminTasks.Run(name, output);
            case ["--background", ..]:
                return BackgroundMode.Run(scheduled: true, json: false);
            case ["--check", ..]:
                if (args.Contains("--json"))
                {
                    AttachConsole(-1);
                }
                return BackgroundMode.Run(scheduled: false, json: args.Contains("--json"));
        }

        AppDomain.CurrentDomain.UnhandledException += (_, e) => ToolboxPaths.WriteError("Crashed: " + e.ExceptionObject);
        XamlCheckProcessRequirements();
        WinRT.ComWrappersSupport.InitializeComWrappers();

        // One window: a second launch hands its activation (a notification click, say) to the first one.
        if (RedirectedToRunningCopy())
        {
            return 0;
        }

        Application.Start(p =>
        {
            var context = new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread());
            SynchronizationContext.SetSynchronizationContext(context);
            _ = new App();
        });
        return 0;
    }

    private static bool RedirectedToRunningCopy()
    {
        try
        {
            AppInstance main = AppInstance.FindOrRegisterForKey("SafeToolbox.Main");
            if (main.IsCurrent)
            {
                main.Activated += (_, a) => (Application.Current as App)?.OnRedirected(a);
                return false;
            }
            AppActivationArguments a = AppInstance.GetCurrent().GetActivatedEventArgs();
            // Redirecting must not block the STA thread the activation came in on.
            Task.Run(() => main.RedirectActivationToAsync(a).AsTask().Wait()).Wait(TimeSpan.FromSeconds(10));
            return true;
        }
        catch (Exception ex)
        {
            ToolboxPaths.WriteError("Single-instance check failed, opening a window anyway: " + ex.Message);
            return false;
        }
    }
}
