using Microsoft.Windows.AppLifecycle;
using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;
using SafeToolbox.Core;

namespace SafeToolbox;

/// <summary>
/// Windows notifications through the Windows App SDK. Unpackaged apps register themselves on first use
/// (Register() sets up the click handler with the shell). When the PC can't show them, the check result
/// still lands on the Home page.
/// </summary>
internal static class Notifier
{
    private static bool s_registered;

    public static bool Supported
    {
        get
        {
            try
            {
                return AppNotificationManager.IsSupported();
            }
            catch (Exception ex) when (ex is System.Runtime.InteropServices.COMException or TypeInitializationException or DllNotFoundException)
            {
                return false;
            }
        }
    }

    /// <summary>Clicks on a notification while the window is open come here instead of starting a second copy.</summary>
    public static void RegisterForClicks(Action<string?> onClick)
    {
        try
        {
            if (!Supported)
            {
                return;
            }
            // The handler must be attached before Register, or Windows starts a new process for each click.
            AppNotificationManager.Default.NotificationInvoked += (_, a) => onClick(PageFrom(a.Arguments));
            EnsureRegistered();
        }
        catch (Exception ex)
        {
            ToolboxPaths.WriteError("Notification registration failed: " + ex.Message);
        }
    }

    public static bool Show(string title, string body, string page)
    {
        try
        {
            if (!Supported)
            {
                ToolboxPaths.WriteError("Notifications are not supported here; the result is on the Home page.");
                return false;
            }
            EnsureRegistered();
            AppNotification n = new AppNotificationBuilder()
                .AddArgument("page", page)
                .AddText(title)
                .AddText(body)
                .AddButton(new AppNotificationButton("Open Safe Toolbox").AddArgument("page", page))
                .BuildNotification();
            AppNotificationManager.Default.Show(n);
            return n.Id != 0;
        }
        catch (Exception ex)
        {
            ToolboxPaths.WriteError("Could not show a notification: " + ex.Message);
            return false;
        }
    }

    private static void EnsureRegistered()
    {
        if (!s_registered)
        {
            AppNotificationManager.Default.Register();
            s_registered = true;
        }
    }

    public static string? PageFromActivation(AppActivationArguments a)
    {
        try
        {
            return a.Kind == ExtendedActivationKind.AppNotification && a.Data is AppNotificationActivatedEventArgs n ? PageFrom(n.Arguments) : null;
        }
        catch (Exception ex) when (ex is InvalidCastException or System.Runtime.InteropServices.COMException)
        {
            return null;
        }
    }

    private static string? PageFrom(IDictionary<string, string> args) => args.TryGetValue("page", out string? p) ? p : null;
}
