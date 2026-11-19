using System.Diagnostics;
using System.Security;
using System.Security.Principal;
using System.Text;
using System.Text.RegularExpressions;

namespace SafeToolbox.Core;

/// <summary>
/// One Task Scheduler task, "Safe Toolbox background check", that runs "SafeToolbox.exe --background"
/// as you (never elevated). It only checks and notifies; nothing is installed or deleted in the background.
/// A missed run (PC off or asleep) runs at the next chance, as Task Scheduler's StartWhenAvailable does.
/// The exe starts through a windowless cmd with RunAsInvoker, so a "Run as administrator" compatibility
/// setting on the exe can't stop the task: without it Windows refuses the launch with 0x800702E4.
/// </summary>
public static partial class Schedule
{
    public const string TaskName = "Safe Toolbox background check";

    private const string RunAsInvoker = "__COMPAT_LAYER=RunAsInvoker";
    private const string BackgroundSuffix = "\" --background";
    private const int ElevationRequired = unchecked((int)0x800702E4);

    [GeneratedRegex("\"([^\"]*)\"")]
    private static partial Regex CsvField();

    /// <summary>What the task starts: conhost without a window, then cmd, then the exe as the current user.</summary>
    public static (string Command, string Arguments) TaskAction(string exePath) => (
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "conhost.exe"),
        $"--headless cmd.exe /d /c \"set \"{RunAsInvoker}\"&\"{exePath}{BackgroundSuffix}\"");

    /// <summary>Daily when either job is daily, weekly when both are weekly, null when both are off.</summary>
    public static Frequency? TaskFrequency(ToolboxSettings s) =>
        s.UpdateCheck == Frequency.Daily || s.CleanupReminder == Frequency.Daily ? Frequency.Daily
        : s.UpdateCheck == Frequency.Weekly || s.CleanupReminder == Frequency.Weekly ? Frequency.Weekly
        : null;

    /// <summary>The task XML. Weekly runs on Sunday; both start at 12:30 with up to 30 minutes of random delay.</summary>
    public static string BuildXml(string exePath, Frequency frequency, DateTime now, string? userId = null)
    {
        userId ??= WindowsIdentity.GetCurrent().User?.Value ?? "";
        DateTime start = now.Date.AddHours(12).AddMinutes(30);
        if (start <= now)
        {
            start = start.AddDays(1);
        }
        string schedule = frequency == Frequency.Daily
            ? "<ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>"
            : "<ScheduleByWeek><DaysOfWeek><Sunday /></DaysOfWeek><WeeksInterval>1</WeeksInterval></ScheduleByWeek>";
        string dir = Path.GetDirectoryName(exePath) ?? "";
        (string command, string arguments) = TaskAction(exePath);
        return $"""
            <?xml version="1.0" encoding="UTF-16"?>
            <Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
              <RegistrationInfo>
                <Author>Safe Toolbox</Author>
                <Description>Checks for safe updates and space to free, and shows a notification. It never installs or deletes anything by itself. Turn it off in Safe Toolbox, Settings.</Description>
              </RegistrationInfo>
              <Triggers>
                <CalendarTrigger>
                  <StartBoundary>{start:yyyy-MM-ddTHH:mm:ss}</StartBoundary>
                  <RandomDelay>PT30M</RandomDelay>
                  <Enabled>true</Enabled>
                  {schedule}
                </CalendarTrigger>
              </Triggers>
              <Principals>
                <Principal id="Author">
                  <UserId>{SecurityElement.Escape(userId)}</UserId>
                  <LogonType>InteractiveToken</LogonType>
                  <RunLevel>LeastPrivilege</RunLevel>
                </Principal>
              </Principals>
              <Settings>
                <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
                <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
                <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
                <AllowHardTerminate>true</AllowHardTerminate>
                <StartWhenAvailable>true</StartWhenAvailable>
                <RunOnlyIfNetworkAvailable>true</RunOnlyIfNetworkAvailable>
                <IdleSettings>
                  <StopOnIdleEnd>false</StopOnIdleEnd>
                  <RestartOnIdle>false</RestartOnIdle>
                </IdleSettings>
                <AllowStartOnDemand>true</AllowStartOnDemand>
                <Enabled>true</Enabled>
                <Hidden>false</Hidden>
                <RunOnlyIfIdle>false</RunOnlyIfIdle>
                <WakeToRun>false</WakeToRun>
                <ExecutionTimeLimit>PT1H</ExecutionTimeLimit>
                <Priority>7</Priority>
              </Settings>
              <Actions Context="Author">
                <Exec>
                  <Command>{SecurityElement.Escape(command)}</Command>
                  <Arguments>{SecurityElement.Escape(arguments)}</Arguments>
                  <WorkingDirectory>{SecurityElement.Escape(dir)}</WorkingDirectory>
                </Exec>
              </Actions>
            </Task>
            """;
    }

    /// <summary>Creates, updates or removes the task to match the settings. Returns an error message, or null.</summary>
    public static string? Sync(ToolboxSettings s, string exePath)
    {
        Frequency? f = TaskFrequency(s);
        if (f is null)
        {
            return Exists() ? Remove() : null;
        }
        string xmlFile = Path.Combine(ToolboxPaths.Jobs, "task.xml");
        try
        {
            File.WriteAllText(xmlFile, BuildXml(exePath, f.Value, DateTime.Now), Encoding.Unicode);
            (int code, string output) = Schtasks($"/Create /TN \"{TaskName}\" /XML \"{xmlFile}\" /F");
            return code == 0 ? null : "Task Scheduler refused the task: " + output.Trim();
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException or System.ComponentModel.Win32Exception)
        {
            return ex.Message;
        }
        finally
        {
            TryDelete(xmlFile);
        }
    }

    public static bool Exists() => Schtasks($"/Query /TN \"{TaskName}\"").Code == 0;

    /// <summary>The exe a registered task runs, and whether the task already starts it the current way.</summary>
    public static (string? Exe, bool Current) ParseRegistered(string xml)
    {
        string command = Element(xml, "Command").Trim('"');
        string arguments = Element(xml, "Arguments");
        int end = arguments.LastIndexOf(BackgroundSuffix, StringComparison.Ordinal);
        if (end > 0 && arguments.Contains(RunAsInvoker, StringComparison.Ordinal))
        {
            int start = arguments.LastIndexOf('"', end - 1);
            return (arguments[(start + 1)..end], true);
        }
        return (command.Length > 0 ? command : null, false);
    }

    /// <summary>
    /// Brings an existing task up to date when this exe owns it, or when the exe it names is gone. A copy run
    /// from somewhere else leaves a working task alone. Returns an error message, or null.
    /// </summary>
    public static string? Repair(ToolboxSettings s, string exePath)
    {
        if (TaskFrequency(s) is null)
        {
            return null;
        }
        (int code, string xml) = Schtasks($"/Query /TN \"{TaskName}\" /XML");
        if (code != 0)
        {
            return null;
        }
        (string? exe, bool current) = ParseRegistered(xml);
        bool mine = string.Equals(exe, exePath, StringComparison.OrdinalIgnoreCase);
        bool gone = exe is null || !File.Exists(exe);
        return mine && !current || !mine && gone ? Sync(s, exePath) : null;
    }

    /// <summary>The "Last Result" column of schtasks /Query /FO CSV /V /NH, as a signed HRESULT.</summary>
    public static int? ParseLastResult(string csv)
    {
        MatchCollection fields = CsvField().Matches(csv);
        return fields.Count > 6 && long.TryParse(fields[6].Groups[1].Value, out long v) ? unchecked((int)v) : null;
    }

    /// <summary>What went wrong last time in plain words, or null when the task ran, is running or hasn't run yet.</summary>
    public static string? DescribeLastResult(int? result) => result switch
    {
        null or 0 or (>= 0x41300 and <= 0x41303) or 0x41325 => null,
        ElevationRequired => "The last background check didn't start, because Windows asked for admin first.",
        int r => $"The last background check didn't finish (code 0x{r:X8}). Details may be in error.log.",
    };

    public static string? LastProblem()
    {
        (int code, string csv) = Schtasks($"/Query /TN \"{TaskName}\" /FO CSV /V /NH");
        return code == 0 ? DescribeLastResult(ParseLastResult(csv)) : null;
    }

    private static string Element(string xml, string name)
    {
        int a = xml.IndexOf($"<{name}>", StringComparison.Ordinal);
        int b = xml.IndexOf($"</{name}>", StringComparison.Ordinal);
        return a >= 0 && b > a ? SecurityElementUnescape(xml[(a + name.Length + 2)..b]) : "";
    }

    public static string? Remove()
    {
        (int code, string output) = Schtasks($"/Delete /TN \"{TaskName}\" /F");
        return code == 0 ? null : output.Trim();
    }

    /// <summary>Runs the task now, for "Check now in the background".</summary>
    public static bool RunNow() => Schtasks($"/Run /TN \"{TaskName}\"").Code == 0;

    private static (int Code, string Output) Schtasks(string args)
    {
        string exe = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "schtasks.exe");
        var psi = new ProcessStartInfo(exe, args)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        try
        {
            using Process p = Process.Start(psi)!;
            Task<string> o = p.StandardOutput.ReadToEndAsync();
            Task<string> e = p.StandardError.ReadToEndAsync();
            if (!p.WaitForExit(30_000))
            {
                p.Kill();
                return (-1, "schtasks did not answer");
            }
            return (p.ExitCode, o.Result + e.Result);
        }
        catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or InvalidOperationException)
        {
            return (-1, ex.Message);
        }
    }

    private static string SecurityElementUnescape(string s) => s
        .Replace("&quot;", "\"", StringComparison.Ordinal).Replace("&apos;", "'", StringComparison.Ordinal)
        .Replace("&lt;", "<", StringComparison.Ordinal).Replace("&gt;", ">", StringComparison.Ordinal)
        .Replace("&amp;", "&", StringComparison.Ordinal);

    private static void TryDelete(string f)
    {
        try
        {
            File.Delete(f);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
        }
    }
}
