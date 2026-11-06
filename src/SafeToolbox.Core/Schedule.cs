using System.Diagnostics;
using System.Security;
using System.Security.Principal;
using System.Text;

namespace SafeToolbox.Core;

/// <summary>
/// One Task Scheduler task, "Safe Toolbox background check", that runs "SafeToolbox.exe --background"
/// as you (never elevated). It only checks and notifies; nothing is installed or deleted in the background.
/// A missed run (PC off or asleep) runs at the next chance, as Task Scheduler's StartWhenAvailable does.
/// </summary>
public static class Schedule
{
    public const string TaskName = "Safe Toolbox background check";

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
                  <Command>"{SecurityElement.Escape(exePath)}"</Command>
                  <Arguments>--background</Arguments>
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

    /// <summary>The program the existing task runs, so a moved app can re-point it.</summary>
    public static string? RegisteredExe()
    {
        (int code, string xml) = Schtasks($"/Query /TN \"{TaskName}\" /XML");
        if (code != 0)
        {
            return null;
        }
        int a = xml.IndexOf("<Command>", StringComparison.Ordinal);
        int b = xml.IndexOf("</Command>", StringComparison.Ordinal);
        return a >= 0 && b > a ? SecurityElementUnescape(xml[(a + 9)..b]).Trim('"') : null;
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
