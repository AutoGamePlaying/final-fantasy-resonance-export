using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace FfrExport.Gui.Core;

internal sealed record GuiSettings(
    [property: JsonPropertyName("output_root_directory")] string? OutputRootDirectory,
    [property: JsonPropertyName("include_jmap")] bool IncludeJmap)
{
    internal static GuiSettings Default => new(null, false);
}

/// <summary>
/// Persists GUI user preferences (last output root and .jmap checkbox state) in
/// %APPDATA%\ffr-export\settings.json without touching the Windows Registry.
/// </summary>
internal static class GuiSettingsStore
{
    internal const string AppFolderName = "ffr-export";
    internal const string SettingsFileName = "settings.json";

    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true,
        PropertyNameCaseInsensitive = true,
    };

    internal static string GetDefaultSettingsPath(string? appDataDirectoryOverride = null)
    {
        var appData = appDataDirectoryOverride;
        if (string.IsNullOrWhiteSpace(appData))
        {
            appData = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
        }

        if (string.IsNullOrWhiteSpace(appData))
        {
            appData = AppContext.BaseDirectory;
        }

        return Path.Combine(appData, AppFolderName, SettingsFileName);
    }

    internal static GuiSettings Load(string? settingsPath = null)
    {
        var path = string.IsNullOrWhiteSpace(settingsPath)
            ? GetDefaultSettingsPath()
            : settingsPath;

        var ioPath = PathSupport.ToIoPath(path);
        if (!File.Exists(ioPath))
        {
            return GuiSettings.Default;
        }

        try
        {
            var json = File.ReadAllText(ioPath, Utf8WithoutBom);
            var parsed = JsonSerializer.Deserialize<GuiSettings>(json, JsonOptions);
            if (parsed is null)
            {
                return GuiSettings.Default;
            }

            var outputRoot = string.IsNullOrWhiteSpace(parsed.OutputRootDirectory)
                ? null
                : parsed.OutputRootDirectory.Trim();
            return new GuiSettings(outputRoot, parsed.IncludeJmap);
        }
        catch
        {
            return GuiSettings.Default;
        }
    }

    internal static void Save(GuiSettings settings, string? settingsPath = null)
    {
        ArgumentNullException.ThrowIfNull(settings);

        var path = string.IsNullOrWhiteSpace(settingsPath)
            ? GetDefaultSettingsPath()
            : settingsPath;

        var fullPath = Path.GetFullPath(path);
        var dir = Path.GetDirectoryName(fullPath);
        if (!string.IsNullOrWhiteSpace(dir))
        {
            Directory.CreateDirectory(PathSupport.ToIoPath(dir));
        }

        var json = JsonSerializer.Serialize(settings, JsonOptions) + "\n";
        var tempPath = $"{fullPath}.{Environment.ProcessId}.{Guid.NewGuid():N}.tmp";
        try
        {
            File.WriteAllText(PathSupport.ToIoPath(tempPath), json, Utf8WithoutBom);
            File.Move(PathSupport.ToIoPath(tempPath), PathSupport.ToIoPath(fullPath), overwrite: true);
        }
        catch
        {
            try
            {
                if (File.Exists(PathSupport.ToIoPath(tempPath)))
                {
                    File.Delete(PathSupport.ToIoPath(tempPath));
                }
            }
            catch
            {
                // Ignore temp cleanup errors.
            }

            throw;
        }
    }
}
