using System.Diagnostics;
using System.Diagnostics.Eventing.Reader;
using System.Globalization;
using System.Management;
using System.Xml.Linq;

namespace SafeToolbox.Core;

public enum CrashKind { BlueScreen, UnexpectedShutdown, PowerLoss }

/// <summary>One time Windows went down without shutting down properly.</summary>
public sealed record Crash(DateTime When, CrashKind Kind, uint? StopCode, string Source)
{
    public string StopCodeHex => StopCode is uint c ? $"0x{c:X}" : "";
    public string? StopName => StopCode is uint c ? StopCodes.Name(c) : null;
}

public sealed record AppCrash(string App, int Count, DateTime Last);

public sealed record DriveHealth(string Name, string Media, string Bus, string Health, string Status, long Size)
{
    public bool Healthy => Health == "Healthy";
}

/// <summary>Admin-only SMART-style counters from MSFT_StorageReliabilityCounter. Null fields were not reported.</summary>
public sealed record DriveWear(string DeviceId, int? Wear, int? Temperature, int? TemperatureMax, long? PowerOnHours, long? ReadErrors);

public sealed record BatteryHealth(string Name, long DesignCapacity, long FullChargeCapacity, int? CycleCount)
{
    /// <summary>How much of its new capacity the battery still holds, in percent.</summary>
    public double Percent => DesignCapacity > 0 ? Math.Round(100.0 * FullChargeCapacity / DesignCapacity, 0) : 0;
}

public sealed record AntivirusState(string Name, bool On, bool UpToDate);

/// <summary>
/// Read-only health checks that work without admin. Sources: the System log (Kernel-Power 41, BugCheck 1001,
/// EventLog 6008), the Application log (Application Error 1000), MSFT_PhysicalDisk, powercfg /batteryreport
/// and Windows Security Center. Event 41 stores its stop code in decimal; it is shown in hex (159 is 0x9F).
/// </summary>
public static class Health
{
    public static List<Crash> Crashes(int days = 90)
    {
        var list = new List<Crash>();
        long ms = (long)TimeSpan.FromDays(days).TotalMilliseconds;
        string xpath = $"*[System[(EventID=41 or EventID=1001 or EventID=6008) and TimeCreated[timediff(@SystemTime) <= {ms}]]]";
        foreach (EventRecord e in Read("System", xpath))
        {
            using (e)
            {
                DateTime when = e.TimeCreated ?? DateTime.MinValue;
                string provider = e.ProviderName ?? "";
                if (e.Id == 41 && provider.Contains("Kernel-Power", StringComparison.OrdinalIgnoreCase))
                {
                    uint? code = Data(e, "BugcheckCode") is string s && uint.TryParse(s, out uint c) && c != 0 ? c : null;
                    list.Add(new Crash(when, code is null ? CrashKind.PowerLoss : CrashKind.BlueScreen, code, "Kernel-Power 41"));
                }
                else if (e.Id == 1001 && provider.Contains("WER-SystemErrorReporting", StringComparison.OrdinalIgnoreCase))
                {
                    list.Add(new Crash(when, CrashKind.BlueScreen, ParseBugcheckText(FirstData(e)), "BugCheck 1001"));
                }
                else if (e.Id == 6008 && provider.Equals("EventLog", StringComparison.OrdinalIgnoreCase))
                {
                    list.Add(new Crash(when, CrashKind.UnexpectedShutdown, null, "EventLog 6008"));
                }
            }
        }
        return Merge(list);
    }

    /// <summary>
    /// One crash often leaves two or three events (41 at the next boot, 1001 with the stop code, 6008).
    /// Events within ten minutes of each other count once, keeping the most informative.
    /// </summary>
    public static List<Crash> Merge(IEnumerable<Crash> events)
    {
        var merged = new List<Crash>();
        foreach (Crash c in events.OrderBy(c => c.When))
        {
            Crash? last = merged.Count > 0 ? merged[^1] : null;
            if (last is not null && (c.When - last.When).Duration() <= TimeSpan.FromMinutes(10))
            {
                if (Rank(c) > Rank(last))
                {
                    merged[^1] = c with { When = last.When };
                }
                continue;
            }
            merged.Add(c);
        }
        merged.Reverse();
        return merged;

        static int Rank(Crash c) => c.StopCode is not null ? 3 : c.Kind == CrashKind.PowerLoss ? 2 : 1;
    }

    /// <summary>"0x00000154 (0xffff..., ...)" from a BugCheck event, as 0x154.</summary>
    public static uint? ParseBugcheckText(string? text)
    {
        if (string.IsNullOrEmpty(text))
        {
            return null;
        }
        int i = text.IndexOf("0x", StringComparison.OrdinalIgnoreCase);
        if (i < 0)
        {
            return null;
        }
        string hex = new(text[(i + 2)..].TakeWhile(Uri.IsHexDigit).ToArray());
        return uint.TryParse(hex, NumberStyles.HexNumber, CultureInfo.InvariantCulture, out uint v) && v != 0 ? v : null;
    }

    public static List<AppCrash> AppCrashes(int days = 14)
    {
        long ms = (long)TimeSpan.FromDays(days).TotalMilliseconds;
        string xpath = $"*[System[Provider[@Name='Application Error'] and EventID=1000 and TimeCreated[timediff(@SystemTime) <= {ms}]]]";
        var byApp = new Dictionary<string, (int Count, DateTime Last)>(StringComparer.OrdinalIgnoreCase);
        foreach (EventRecord e in Read("Application", xpath))
        {
            using (e)
            {
                string app = FirstData(e) ?? "unknown";
                DateTime when = e.TimeCreated ?? DateTime.MinValue;
                byApp[app] = byApp.TryGetValue(app, out var v) ? (v.Count + 1, when > v.Last ? when : v.Last) : (1, when);
            }
        }
        return byApp.Select(kv => new AppCrash(kv.Key, kv.Value.Count, kv.Value.Last))
            .OrderByDescending(a => a.Count).ThenByDescending(a => a.Last).ToList();
    }

    public static List<DriveHealth> Drives()
    {
        var list = new List<DriveHealth>();
        try
        {
            using var searcher = new ManagementObjectSearcher(@"root\Microsoft\Windows\Storage",
                "SELECT FriendlyName, MediaType, BusType, HealthStatus, OperationalStatus, Size FROM MSFT_PhysicalDisk");
            foreach (ManagementBaseObject o in searcher.Get())
            {
                using (o)
                {
                    string health = Convert.ToUInt16(o["HealthStatus"] ?? 5, CultureInfo.InvariantCulture) switch
                    {
                        0 => "Healthy",
                        1 => "Warning",
                        2 => "Unhealthy",
                        _ => "Unknown",
                    };
                    string media = Convert.ToUInt16(o["MediaType"] ?? 0, CultureInfo.InvariantCulture) switch
                    {
                        3 => "Hard drive",
                        4 => "SSD",
                        5 => "Storage class memory",
                        _ => "Drive",
                    };
                    string bus = Convert.ToUInt16(o["BusType"] ?? 0, CultureInfo.InvariantCulture) switch
                    {
                        17 => "NVMe",
                        11 => "SATA",
                        7 => "USB",
                        8 => "RAID",
                        10 => "SAS",
                        _ => "",
                    };
                    string status = o["OperationalStatus"] is ushort[] ops && ops.Length > 0
                        ? string.Join(", ", ops.Select(OperationalName))
                        : "OK";
                    long size = o["Size"] is ulong s ? (long)s : 0;
                    list.Add(new DriveHealth((o["FriendlyName"] as string ?? "Drive").Trim(), media, bus, health, status, size));
                }
            }
        }
        catch (Exception ex) when (ex is ManagementException or UnauthorizedAccessException or System.Runtime.InteropServices.COMException)
        {
            ToolboxPaths.WriteError("Drive health read failed: " + ex.Message);
        }
        return list;
    }

    private static string OperationalName(ushort v) => v switch
    {
        2 => "OK",
        3 => "Degraded",
        5 => "Predictive failure",
        6 => "Error",
        0xD00D => "In service",
        0xD00E => "Transient error",
        _ => "Status " + v,
    };

    /// <summary>The PowerShell that reads wear and temperature. It needs admin, so the page runs it elevated.</summary>
    public const string DriveWearScript =
        "Get-PhysicalDisk | ForEach-Object { $d = $_; $r = $d | Get-StorageReliabilityCounter; [pscustomobject]@{ DeviceId = $d.FriendlyName; Wear = $r.Wear; Temperature = $r.Temperature; TemperatureMax = $r.TemperatureMax; PowerOnHours = $r.PowerOnHours; ReadErrors = $r.ReadErrorsUncorrected } } | ConvertTo-Json -Compress";

    public static List<DriveWear> ParseDriveWear(string json)
    {
        var list = new List<DriveWear>();
        try
        {
            using var doc = System.Text.Json.JsonDocument.Parse(json.Trim().StartsWith('[') ? json : "[" + json + "]");
            foreach (System.Text.Json.JsonElement e in doc.RootElement.EnumerateArray())
            {
                list.Add(new DriveWear(Str(e, "DeviceId") ?? "Drive", Int(e, "Wear"), Int(e, "Temperature"), Int(e, "TemperatureMax"),
                    Long(e, "PowerOnHours"), Long(e, "ReadErrors")));
            }
        }
        catch (System.Text.Json.JsonException)
        {
        }
        return list;

        static string? Str(System.Text.Json.JsonElement e, string n) => e.TryGetProperty(n, out var v) && v.ValueKind == System.Text.Json.JsonValueKind.String ? v.GetString() : null;
        static int? Int(System.Text.Json.JsonElement e, string n) => e.TryGetProperty(n, out var v) && v.ValueKind == System.Text.Json.JsonValueKind.Number && v.TryGetInt32(out int i) ? i : null;
        static long? Long(System.Text.Json.JsonElement e, string n) => e.TryGetProperty(n, out var v) && v.ValueKind == System.Text.Json.JsonValueKind.Number && v.TryGetInt64(out long i) ? i : null;
    }

    /// <summary>From "powercfg /batteryreport /xml", which works without admin. Empty on a desktop PC.</summary>
    public static List<BatteryHealth> Batteries()
    {
        string xml = Path.Combine(ToolboxPaths.Jobs, "battery.xml");
        try
        {
            (int code, _) = RunHidden("powercfg.exe", $"/batteryreport /xml /output \"{xml}\"", TimeSpan.FromSeconds(30));
            return code == 0 && File.Exists(xml) ? ParseBatteryReport(File.ReadAllText(xml)) : [];
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException or System.ComponentModel.Win32Exception)
        {
            return [];
        }
        finally
        {
            try
            {
                File.Delete(xml);
            }
            catch (IOException)
            {
            }
        }
    }

    public static List<BatteryHealth> ParseBatteryReport(string xml)
    {
        var list = new List<BatteryHealth>();
        XDocument doc;
        try
        {
            doc = XDocument.Parse(xml);
        }
        catch (System.Xml.XmlException)
        {
            return list;
        }
        foreach (XElement b in doc.Descendants().Where(e => e.Name.LocalName == "Battery"))
        {
            string? V(string n) => b.Elements().FirstOrDefault(e => e.Name.LocalName == n)?.Value;
            if (!long.TryParse(V("DesignCapacity"), out long design) || !long.TryParse(V("FullChargeCapacity"), out long full) || design <= 0)
            {
                continue;
            }
            int? cycles = int.TryParse(V("CycleCount"), out int c) && c > 0 ? c : null;
            string name = string.Join(' ', new[] { V("Manufacturer"), V("Id") }.Where(s => !string.IsNullOrWhiteSpace(s)));
            list.Add(new BatteryHealth(name.Length > 0 ? name : "Battery", design, full, cycles));
        }
        return list;
    }

    /// <summary>Writes the full HTML battery report and returns its path.</summary>
    public static string? BatteryReport()
    {
        string html = Path.Combine(ToolboxPaths.Exports, $"battery-report-{DateTime.Now:yyyyMMdd-HHmm}.html");
        try
        {
            (int code, _) = RunHidden("powercfg.exe", $"/batteryreport /output \"{html}\"", TimeSpan.FromSeconds(60));
            return code == 0 && File.Exists(html) ? html : null;
        }
        catch (Exception ex) when (ex is InvalidOperationException or System.ComponentModel.Win32Exception or TimeoutException)
        {
            return null;
        }
    }

    /// <summary>The antivirus Windows Security reports as active (productState bit 12 on = enabled, bit 4 on = out of date).</summary>
    public static List<AntivirusState> Antivirus()
    {
        var list = new List<AntivirusState>();
        try
        {
            using var searcher = new ManagementObjectSearcher(@"root\SecurityCenter2", "SELECT displayName, productState FROM AntiVirusProduct");
            foreach (ManagementBaseObject o in searcher.Get())
            {
                using (o)
                {
                    uint state = Convert.ToUInt32(o["productState"] ?? 0u, CultureInfo.InvariantCulture);
                    list.Add(new AntivirusState(o["displayName"] as string ?? "Antivirus", (state & 0x1000) != 0, (state & 0x10) == 0));
                }
            }
        }
        catch (Exception ex) when (ex is ManagementException or UnauthorizedAccessException or System.Runtime.InteropServices.COMException)
        {
        }
        return list.OrderByDescending(a => a.On).ToList();
    }

    public static TimeSpan Uptime() => TimeSpan.FromMilliseconds(Environment.TickCount64);

    public static bool RestartPending() => SafeUpdater.Core.Preflight.RebootPending();

    // ------------------------------------------------------------------ helpers

    private static IEnumerable<EventRecord> Read(string log, string xpath)
    {
        EventLogReader reader;
        try
        {
            reader = new EventLogReader(new EventLogQuery(log, PathType.LogName, xpath) { ReverseDirection = true });
        }
        catch (Exception ex) when (ex is EventLogException or UnauthorizedAccessException)
        {
            ToolboxPaths.WriteError($"Reading the {log} log failed: {ex.Message}");
            yield break;
        }
        using (reader)
        {
            int n = 0;
            while (n++ < 2000)
            {
                EventRecord? e;
                try
                {
                    e = reader.ReadEvent();
                }
                catch (EventLogException)
                {
                    yield break;
                }
                if (e is null)
                {
                    yield break;
                }
                yield return e;
            }
        }
    }

    private static string? Data(EventRecord e, string name)
    {
        try
        {
            XElement x = XElement.Parse(e.ToXml());
            return x.Descendants().FirstOrDefault(d => d.Name.LocalName == "Data" && (string?)d.Attribute("Name") == name)?.Value;
        }
        catch (Exception ex) when (ex is System.Xml.XmlException or EventLogException)
        {
            return null;
        }
    }

    private static string? FirstData(EventRecord e)
    {
        try
        {
            return e.Properties.Count > 0 ? e.Properties[0].Value?.ToString() : null;
        }
        catch (EventLogException)
        {
            return null;
        }
    }

    internal static (int Code, string Output) RunHidden(string exe, string args, TimeSpan timeout, string? stdin = null)
    {
        var psi = new ProcessStartInfo(exe, args)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            RedirectStandardInput = stdin is not null,
        };
        using Process p = Process.Start(psi) ?? throw new InvalidOperationException("Could not start " + exe);
        if (stdin is not null)
        {
            p.StandardInput.Write(stdin);
            p.StandardInput.Close();
        }
        Task<string> o = p.StandardOutput.ReadToEndAsync();
        Task<string> err = p.StandardError.ReadToEndAsync();
        if (!p.WaitForExit(timeout))
        {
            p.Kill(entireProcessTree: true);
            throw new TimeoutException(exe + " did not finish");
        }
        return (p.ExitCode, o.Result + err.Result);
    }
}

/// <summary>Names and a plain hint for the stop codes people actually meet. The rest link to Microsoft's reference.</summary>
public static class StopCodes
{
    private static readonly Dictionary<uint, (string Name, string Hint)> Known = new()
    {
        [0x0A] = ("IRQL_NOT_LESS_OR_EQUAL", "Usually a driver touching memory it shouldn't."),
        [0x1A] = ("MEMORY_MANAGEMENT", "Often memory (RAM) or a driver. A memory test is the next step."),
        [0x1E] = ("KMODE_EXCEPTION_NOT_HANDLED", "A driver crashed."),
        [0x3B] = ("SYSTEM_SERVICE_EXCEPTION", "Often a graphics, antivirus or other driver."),
        [0x50] = ("PAGE_FAULT_IN_NONPAGED_AREA", "Memory, a driver or antivirus software."),
        [0x7A] = ("KERNEL_DATA_INPAGE_ERROR", "Windows couldn't read from the drive. Check the drive and its cable or slot."),
        [0x7E] = ("SYSTEM_THREAD_EXCEPTION_NOT_HANDLED", "A driver crashed."),
        [0x7F] = ("UNEXPECTED_KERNEL_MODE_TRAP", "Often hardware: memory, overheating or overclocking."),
        [0x9C] = ("MACHINE_CHECK_EXCEPTION", "The processor reported a hardware error."),
        [0x9F] = ("DRIVER_POWER_STATE_FAILURE", "A driver didn't handle sleep or wake properly."),
        [0xA0] = ("INTERNAL_POWER_ERROR", "Power management, often sleep, hibernation or a driver."),
        [0xC2] = ("BAD_POOL_CALLER", "A driver misused memory."),
        [0xD1] = ("DRIVER_IRQL_NOT_LESS_OR_EQUAL", "A driver touched memory it shouldn't, often network or graphics."),
        [0xEF] = ("CRITICAL_PROCESS_DIED", "A core Windows process stopped. System file repair can help."),
        [0xF4] = ("CRITICAL_OBJECT_TERMINATION", "A core process stopped, sometimes because the drive failed to respond."),
        [0x101] = ("CLOCK_WATCHDOG_TIMEOUT", "A processor core stopped responding: overclocking, undervolting or heat."),
        [0x109] = ("CRITICAL_STRUCTURE_CORRUPTION", "Kernel memory was changed: a driver or memory fault."),
        [0x113] = ("VIDEO_DXGKRNL_FATAL_ERROR", "The graphics stack failed. Graphics driver."),
        [0x116] = ("VIDEO_TDR_FAILURE", "The graphics driver stopped responding and couldn't recover."),
        [0x117] = ("VIDEO_TDR_TIMEOUT_DETECTED", "The graphics driver stopped responding."),
        [0x119] = ("VIDEO_SCHEDULER_INTERNAL_ERROR", "Graphics driver or GPU."),
        [0x124] = ("WHEA_UNCORRECTABLE_ERROR", "A hardware error: processor, memory, heat or overclocking."),
        [0x133] = ("DPC_WATCHDOG_VIOLATION", "A driver took too long, often storage (SSD firmware) or network."),
        [0x139] = ("KERNEL_SECURITY_CHECK_FAILURE", "A driver or memory corrupted kernel data."),
        [0x13A] = ("KERNEL_MODE_HEAP_CORRUPTION", "A driver corrupted kernel memory."),
        [0x154] = ("UNEXPECTED_STORE_EXCEPTION", "The storage stack failed: often the SSD, its firmware or its driver."),
        [0x1E0] = ("ILLEGAL_ATS_INITIALIZATION", "A device or driver problem."),
    };

    public static string? Name(uint code) => Known.TryGetValue(code, out var k) ? k.Name : null;

    public static string? Hint(uint code) => Known.TryGetValue(code, out var k) ? k.Hint : null;

    /// <summary>Microsoft's page for the code, or the reference list when its page name isn't known.</summary>
    public static string Link(uint code) => Known.TryGetValue(code, out var k)
        ? $"https://learn.microsoft.com/windows-hardware/drivers/debugger/bug-check-0x{code:x}--{k.Name.ToLowerInvariant().Replace('_', '-')}"
        : "https://learn.microsoft.com/windows-hardware/drivers/debugger/bug-check-code-reference2";
}
