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
    string? WarningMessage,
    string? MigratedFromPath = null);

/// <summary>
/// Persists GUI user preferences in %LOCALAPPDATA%\agp\ffr-export\settings.json
/// and migrates legacy %APPDATA%\ffr-export\settings.json on first startup by renaming it to .migrated.
/// </summary>
internal static class GuiSettingsStore
{
    internal const string AppFolderName = AgpPaths.ToolFolderName;
    internal const string SettingsFileName = AgpPaths.SettingsFileName;
    internal const string MigratedSuffix = ".migrated";

    private static readonly UTF8Encoding Utf8WithoutBom = TextOutput.Utf8WithoutBom;

    private static readonly JsonSerializerOptions JsonOptions = TextOutput.CreateSystemJsonOptions(
        writeIndented: true,
        propertyNameCaseInsensitive: true);

    internal static string GetDefaultSettingsPath(
        string? agpHomeOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        if (!string.IsNullOrWhiteSpace(agpHomeOverride)
            && AgpPaths.TryNormalizePath(agpHomeOverride, out var normalizedAgp))
        {
            return AgpPaths.GetSettingsFilePathForAgpRoot(normalizedAgp);
        }

        return AgpPaths.GetSettingsFilePath(
            agpHomeEnvOverride: agpHomeOverride,
            isWindowsOverride: isWindowsOverride,
            localAppDataOverride: localAppDataOverride,
            xdgDataHomeOverride: xdgDataHomeOverride,
            userProfileOverride: userProfileOverride);
    }

    internal static string? GetLegacySettingsPath(string? appDataDirectoryOverride = null)
    {
        var appData = appDataDirectoryOverride is not null
            ? appDataDirectoryOverride
            : SafeGetFolderPath(Environment.SpecialFolder.ApplicationData);
        if (string.IsNullOrWhiteSpace(appData))
        {
            return null;
        }

        try
        {
            return Path.GetFullPath(Path.Combine(appData.Trim(), AppFolderName, SettingsFileName));
        }
        catch
        {
            return null;
        }
    }

    internal static GuiSettings Load(string? settingsPath = null, string? legacySettingsPathOverride = null)
    {
        return LoadWithDiagnostics(
            settingsPath,
            legacySettingsPathOverride: legacySettingsPathOverride).Settings;
    }

    internal static GuiSettingsLoadResult LoadWithDiagnostics(
        string? settingsPath = null,
        Func<string, string>? fileReaderOverride = null,
        string? legacySettingsPathOverride = null,
        bool allowLegacyMigration = true)
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

        string? migratedFrom = null;
        string? migrationWarning = null;
        if (allowLegacyMigration && fileReaderOverride is null)
        {
            var effectiveLegacy = legacySettingsPathOverride;
            if (effectiveLegacy is null
                && string.IsNullOrWhiteSpace(settingsPath)
                && !AgpPaths.IsAgpHomeEnvironmentOverrideActive())
            {
                effectiveLegacy = GetLegacySettingsPath();
            }

            if (!string.IsNullOrWhiteSpace(effectiveLegacy))
            {
                (migratedFrom, migrationWarning) = TryMigrateLegacySettings(path, effectiveLegacy);
            }
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
                    return new GuiSettingsLoadResult(GuiSettings.Default, migrationWarning, migratedFrom);
                }

                json = File.ReadAllText(ioPath, Utf8WithoutBom);
            }

            var parsed = JsonSerializer.Deserialize<GuiSettings>(json, JsonOptions);
            if (parsed is null)
            {
                return new GuiSettingsLoadResult(
                    GuiSettings.Default,
                    $"配置文件内容为空或格式无效（{path}），已改用默认设置。",
                    migratedFrom);
            }

            var outputRoot = string.IsNullOrWhiteSpace(parsed.OutputRootDirectory)
                ? null
                : parsed.OutputRootDirectory.Trim();
            return new GuiSettingsLoadResult(
                new GuiSettings(outputRoot, parsed.IncludeJmap),
                migrationWarning,
                migratedFrom);
        }
        catch (JsonException exception)
        {
            return new GuiSettingsLoadResult(
                GuiSettings.Default,
                $"配置文件已损坏（{path}）：{exception.Message}；已改用默认设置。",
                migratedFrom);
        }
        catch (UnauthorizedAccessException exception)
        {
            return new GuiSettingsLoadResult(
                GuiSettings.Default,
                $"无权读取配置文件（{path}）：{exception.Message}；已改用默认设置。",
                migratedFrom);
        }
        catch (Exception exception)
        {
            return new GuiSettingsLoadResult(
                GuiSettings.Default,
                $"读取配置文件失败（{path}）：{exception.Message}；已改用默认设置。",
                migratedFrom);
        }
    }

    internal static void Save(GuiSettings settings, string? settingsPath = null)
    {
        ArgumentNullException.ThrowIfNull(settings);

        var path = string.IsNullOrWhiteSpace(settingsPath)
            ? GetDefaultSettingsPath()
            : settingsPath;

        var fullPath = Path.GetFullPath(path);
        var json = TextOutput.SerializeSystemJson(settings, JsonOptions, trailingNewline: true);
        TextOutput.WriteTextAtomic(fullPath, json);
    }

    internal static (string? MigratedFromPath, string? WarningMessage) TryMigrateLegacySettings(
        string targetSettingsPath,
        string legacySettingsPath)
    {
        try
        {
            if (targetSettingsPath.IndexOf('\0') >= 0 || legacySettingsPath.IndexOf('\0') >= 0)
            {
                return (null, null);
            }

            var fullTarget = Path.GetFullPath(targetSettingsPath);
            var fullLegacy = Path.GetFullPath(legacySettingsPath);
            var comparison = OperatingSystem.IsWindows()
                ? StringComparison.OrdinalIgnoreCase
                : StringComparison.Ordinal;
            if (string.Equals(fullTarget, fullLegacy, comparison))
            {
                return (null, null);
            }

            var ioTarget = PathSupport.ToIoPath(fullTarget);
            var ioLegacy = PathSupport.ToIoPath(fullLegacy);
            if (File.Exists(ioTarget) || !File.Exists(ioLegacy))
            {
                return (null, null);
            }

            var legacyJson = File.ReadAllText(ioLegacy, Utf8WithoutBom);
            var parsed = JsonSerializer.Deserialize<GuiSettings>(legacyJson, JsonOptions);
            if (parsed is null)
            {
                return (null, $"旧版配置文件内容无效（{fullLegacy}），跳过自动迁移。");
            }

            var normalized = new GuiSettings(
                string.IsNullOrWhiteSpace(parsed.OutputRootDirectory) ? null : parsed.OutputRootDirectory.Trim(),
                parsed.IncludeJmap);
            Save(normalized, fullTarget);

            var migratedPath = fullLegacy + MigratedSuffix;
            File.Move(ioLegacy, PathSupport.ToIoPath(migratedPath), overwrite: true);
            return (fullLegacy, null);
        }
        catch (Exception exception)
        {
            return (null, $"迁移旧版配置文件失败（{legacySettingsPath} -> {targetSettingsPath}）：{exception.Message}；已保留旧文件并继续使用默认设置。");
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
}
