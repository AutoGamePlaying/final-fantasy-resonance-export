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

internal sealed record GuiSettingsLoadResult(
    GuiSettings Settings,
    string? WarningMessage);

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

    internal static string GetDefaultSettingsPath(
        string? appDataDirectoryOverride = null,
        string? localAppDataDirectoryOverride = null,
        string? tempDirectoryOverride = null)
    {
        var candidates = new[]
        {
            appDataDirectoryOverride,
            SafeGetFolderPath(Environment.SpecialFolder.ApplicationData),
            localAppDataDirectoryOverride,
            SafeGetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            tempDirectoryOverride,
            SafeGetTempPath(),
            AppContext.BaseDirectory,
        };

        foreach (var candidate in candidates)
        {
            if (string.IsNullOrWhiteSpace(candidate))
            {
                continue;
            }

            try
            {
                var combined = Path.Combine(candidate.Trim(), AppFolderName, SettingsFileName);
                _ = Path.GetFullPath(combined);
                return combined;
            }
            catch
            {
                // Skip invalid environment directories and try the next fallback.
            }
        }

        return Path.Combine(AppFolderName, SettingsFileName);
    }

    internal static GuiSettings Load(string? settingsPath = null)
    {
        return LoadWithDiagnostics(settingsPath).Settings;
    }

    internal static GuiSettingsLoadResult LoadWithDiagnostics(
        string? settingsPath = null,
        Func<string, string>? fileReaderOverride = null)
    {
        string path;
        try
        {
            path = string.IsNullOrWhiteSpace(settingsPath)
                ? GetDefaultSettingsPath()
                : settingsPath;
        }
        catch (Exception exception)
        {
            return new GuiSettingsLoadResult(
                GuiSettings.Default,
                $"解析配置文件路径失败：{exception.Message}；已改用默认设置。");
        }

        try
        {
            string json;
            if (fileReaderOverride is not null)
            {
                json = fileReaderOverride(path);
            }
            else
            {
                if (path.IndexOf('\0') >= 0)
                {
                    throw new ArgumentException("路径包含非法空字符。", nameof(settingsPath));
                }

                var fullPath = Path.GetFullPath(path);
                var ioPath = PathSupport.ToIoPath(fullPath);
                if (!File.Exists(ioPath))
                {
                    return new GuiSettingsLoadResult(GuiSettings.Default, null);
                }

                json = File.ReadAllText(ioPath, Utf8WithoutBom);
            }

            var parsed = JsonSerializer.Deserialize<GuiSettings>(json, JsonOptions);
            if (parsed is null)
            {
                return new GuiSettingsLoadResult(
                    GuiSettings.Default,
                    $"配置文件内容为空或格式无效（{path}），已改用默认设置。");
            }

            var outputRoot = string.IsNullOrWhiteSpace(parsed.OutputRootDirectory)
                ? null
                : parsed.OutputRootDirectory.Trim();
            return new GuiSettingsLoadResult(new GuiSettings(outputRoot, parsed.IncludeJmap), null);
        }
        catch (JsonException exception)
        {
            return new GuiSettingsLoadResult(
                GuiSettings.Default,
                $"配置文件已损坏（{path}）：{exception.Message}；已改用默认设置。");
        }
        catch (UnauthorizedAccessException exception)
        {
            return new GuiSettingsLoadResult(
                GuiSettings.Default,
                $"无权读取配置文件（{path}）：{exception.Message}；已改用默认设置。");
        }
        catch (Exception exception)
        {
            return new GuiSettingsLoadResult(
                GuiSettings.Default,
                $"读取配置文件失败（{path}）：{exception.Message}；已改用默认设置。");
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

    private static string? SafeGetFolderPath(Environment.SpecialFolder folder)
    {
        try
        {
            return Environment.GetFolderPath(folder);
        }
        catch
        {
            return null;
        }
    }

    private static string? SafeGetTempPath()
    {
        try
        {
            return Path.GetTempPath();
        }
        catch
        {
            return null;
        }
    }
}
