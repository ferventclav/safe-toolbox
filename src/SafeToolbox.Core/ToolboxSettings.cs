using System.Text.Json;
using System.Text.Json.Serialization;

namespace SafeToolbox.Core;

public enum ThemeMode { System, Light, Dark }

/// <summary>
/// When the app colours buttons, ticks and progress bars with the Windows accent colour.
/// FollowWindows uses it only while "Show accent colour on title bars and window borders" is on.
/// </summary>
public enum AccentMode { FollowWindows, Always, Never }

public enum BackdropMode { MicaAlt, Mica, Solid }

public enum Frequency { Off, Daily, Weekly }

/// <summary>Everything the toolbox writes of its own lives in %LOCALAPPDATA%\SafeToolbox.</summary>
public static class ToolboxPaths
{
    public static string Root { get; } =
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SafeToolbox");

    public static string SettingsFile => Path.Combine(Ensure(Root), "settings.json");
    public static string ErrorLog => Path.Combine(Ensure(Root), "error.log");
    public static string LastBackground => Path.Combine(Ensure(Root), "last-check.json");
    public static string Jobs => Ensure(Path.Combine(Root, "jobs"));
    public static string Exports => Ensure(Path.Combine(Root, "exports"));

    internal static string Ensure(string dir)
    {
        Directory.CreateDirectory(dir);
        return dir;
    }

    public static void WriteError(string text)
    {
        try
        {
            File.AppendAllText(ErrorLog, $"{DateTime.Now:yyyy-MM-dd HH:mm:ss} {text}{Environment.NewLine}");
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
        }
    }
}

/// <summary>
/// The toolbox's own settings. The update rules keep theirs in %LOCALAPPDATA%\SafeUpdater and the
/// cleaner keeps protect.txt in %APPDATA%\SafeDiskCleaner, so nothing from the separate apps is lost.
/// </summary>
public sealed class ToolboxSettings
{
    private static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter() },
    };

    public ThemeMode Theme { get; set; } = ThemeMode.System;
    public AccentMode Accent { get; set; } = AccentMode.FollowWindows;
    public BackdropMode Backdrop { get; set; } = BackdropMode.MicaAlt;

    /// <summary>Background check for updates, through Task Scheduler. Off until you turn it on.</summary>
    public Frequency UpdateCheck { get; set; } = Frequency.Off;

    /// <summary>Background scan that reminds you when there is space to free. Off until you turn it on.</summary>
    public Frequency CleanupReminder { get; set; } = Frequency.Off;

    /// <summary>The reminder fires only when at least this much can be cleaned safely.</summary>
    public int ReminderThresholdMb { get; set; } = 1024;

    /// <summary>Keep the cleanup ticks you change between runs.</summary>
    public bool RememberCleanupTicks { get; set; } = true;

    /// <summary>Cleanup rule id to tick state, only for rules where you differ from the default.</summary>
    public Dictionary<string, bool> CleanupChoices { get; set; } = new(StringComparer.Ordinal);

    public string? LastPage { get; set; }

    /// <summary>Start the update check when the window opens, as Safe Updater did.</summary>
    public bool CheckUpdatesOnOpen { get; set; } = true;

    /// <summary>Show a Windows notification when a background check finds something.</summary>
    public bool Notify { get; set; } = true;

    /// <summary>Where the window was when it last closed: x, y, width, height in screen pixels.</summary>
    public int[]? WindowBounds { get; set; }

    public bool WindowMaximized { get; set; }

    public static ToolboxSettings Load(string? file = null)
    {
        file ??= ToolboxPaths.SettingsFile;
        try
        {
            if (File.Exists(file))
            {
                return JsonSerializer.Deserialize<ToolboxSettings>(File.ReadAllText(file), Options) ?? new ToolboxSettings();
            }
        }
        catch (Exception ex) when (ex is IOException or JsonException or UnauthorizedAccessException)
        {
            ToolboxPaths.WriteError("settings.json unreadable, using defaults: " + ex.Message);
        }
        return new ToolboxSettings();
    }

    public void Save(string? file = null)
    {
        file ??= ToolboxPaths.SettingsFile;
        try
        {
            string tmp = file + ".tmp";
            File.WriteAllText(tmp, JsonSerializer.Serialize(this, Options));
            File.Move(tmp, file, overwrite: true);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            ToolboxPaths.WriteError("Could not save settings: " + ex.Message);
        }
    }

    /// <summary>The tick for a cleanup rule: your remembered choice, else the rule's default.</summary>
    public bool CleanupTick(string ruleId, bool defaultChecked) =>
        RememberCleanupTicks && CleanupChoices.TryGetValue(ruleId, out bool on) ? on : defaultChecked;

    /// <summary>Remembers a tick, storing it only when it differs from the default.</summary>
    public void RememberTick(string ruleId, bool on, bool defaultChecked)
    {
        if (!RememberCleanupTicks)
        {
            return;
        }
        if (on == defaultChecked)
        {
            CleanupChoices.Remove(ruleId);
        }
        else
        {
            CleanupChoices[ruleId] = on;
        }
    }
}
