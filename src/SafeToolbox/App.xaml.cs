using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using SafeToolbox.Core;

namespace SafeToolbox;

public partial class App : Application
{
    private readonly ToolboxSettings _settings = ToolboxSettings.Load();
    private MainWindow? _window;

    public App()
    {
        UnhandledException += OnUnhandledException;
        try
        {
            InitializeComponent();
        }
        catch (Exception ex)
        {
            ToolboxPaths.WriteError("App.xaml didn't load: " + ex);
            throw;
        }
        Appearance.ApplyStartupTheme(this, _settings);
        TaskScheduler.UnobservedTaskException += (_, e) =>
        {
            ToolboxPaths.WriteError(e.Exception.ToString());
            e.SetObserved();
        };
    }

    public static MainWindow? Window => (Current as App)?._window;

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        try
        {
            Appearance.ApplyStartupShades(_settings);
            _window = new MainWindow(_settings) { StartPage = Notifier.PageFromActivation(AppInstance.GetCurrent().GetActivatedEventArgs()) };
        }
        catch (Exception ex)
        {
            ToolboxPaths.WriteError("The main window didn't open: " + ex);
            throw;
        }
        Notifier.RegisterForClicks(page => _window.DispatcherQueue.TryEnqueue(() => _window.BringToFront(page)));
        _window.Activate();
        _ = Task.Run(() =>
        {
            try
            {
                SingleFileUnpack.RemoveStale(AppContext.BaseDirectory, Path.GetFileNameWithoutExtension(Environment.ProcessPath ?? ""));
            }
            catch (Exception ex)
            {
                ToolboxPaths.WriteError("Removing old unpack folders failed: " + ex.Message);
            }
        });
    }

    /// <summary>A second launch (Start menu, shortcut, notification) lands here instead of opening a second window.</summary>
    public void OnRedirected(AppActivationArguments args) =>
        _window?.DispatcherQueue.TryEnqueue(() => _window.BringToFront(Notifier.PageFromActivation(args)));

    private void OnUnhandledException(object sender, Microsoft.UI.Xaml.UnhandledExceptionEventArgs e)
    {
        // Log it, say so, keep the window: nothing changes on the PC outside the guarded actions.
        ToolboxPaths.WriteError(e.Exception.ToString());
        if (_window is not null)
        {
            e.Handled = true;
            _window.ShowError("Something went wrong", e.Exception.Message + " Details are in error.log in %LOCALAPPDATA%\\SafeToolbox.");
        }
    }
}
