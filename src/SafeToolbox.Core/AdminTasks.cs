using System.ComponentModel;
using System.Diagnostics;

namespace SafeToolbox.Core;

/// <summary>
/// Single admin reads and actions the pages need, each a fixed script (never text from the page), run by
/// "SafeToolbox.exe --admin-task name output" after one UAC prompt.
/// </summary>
public static class AdminTasks
{
    private static readonly Dictionary<string, string> Scripts = new(StringComparer.Ordinal)
    {
        ["drive-wear"] = Health.DriveWearScript,
        ["restore-point"] = SafeUpdater.Core.Preflight.RestorePointScript,
    };

    public static int Run(string name, string output)
    {
        if (!Scripts.TryGetValue(name, out string? script))
        {
            File.WriteAllText(output, "unknown task");
            return 2;
        }
        (int code, string text) = Health.RunHidden(
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "WindowsPowerShell", "v1.0", "powershell.exe"),
            "-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -",
            TimeSpan.FromMinutes(5), script);
        File.WriteAllText(output, text);
        return code;
    }

    /// <summary>Runs this exe elevated with the given arguments and waits. Null when the prompt was declined.</summary>
    public static async Task<int?> RunElevatedAsync(string arguments, CancellationToken ct = default)
    {
        string exe = Environment.ProcessPath ?? throw new InvalidOperationException("Unknown executable path");
        Process? p;
        try
        {
            p = Process.Start(new ProcessStartInfo(exe)
            {
                UseShellExecute = true,
                Verb = "runas",
                Arguments = arguments,
                WindowStyle = ProcessWindowStyle.Hidden,
                WorkingDirectory = AppContext.BaseDirectory,
            });
        }
        catch (Win32Exception ex) when (ex.NativeErrorCode == 1223)
        {
            return null;
        }
        if (p is null)
        {
            return null;
        }
        using (p)
        {
            await p.WaitForExitAsync(ct);
            return p.ExitCode;
        }
    }

    /// <summary>Runs a named task elevated and returns what it printed, or null when admin was declined.</summary>
    public static async Task<string?> RunTaskAsync(string name, CancellationToken ct = default)
    {
        string output = Path.Combine(ToolboxPaths.Jobs, $"{name}-{DateTime.Now:yyyyMMddHHmmss}.txt");
        int? code = await RunElevatedAsync($"--admin-task {name} \"{output}\"", ct);
        if (code is null)
        {
            return null;
        }
        try
        {
            return File.Exists(output) ? await File.ReadAllTextAsync(output, ct) : "";
        }
        finally
        {
            try
            {
                File.Delete(output);
            }
            catch (IOException)
            {
            }
        }
    }
}
