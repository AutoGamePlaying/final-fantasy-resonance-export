using System.Text;

namespace FfrExport;

/// <summary>
/// Single source of truth for the unified %LOCALAPPDATA%\agp\ffr-export\ directory layout
/// shared by both the CLI (ffr-export) and WinForms GUI (ffr-export-gui).
/// </summary>
internal static class AgpPaths
{
    internal const string AgpHomeEnvVar = "AGP_HOME";
    internal const string AgpFolderName = "agp";
    internal const string ToolFolderName = "ffr-export";
    internal const string OutputSubfolderName = "output";
    internal const string UsmapSubfolderName = "usmap";
    internal const string DownloadsSubfolderName = "downloads";
    internal const string LogsSubfolderName = "logs";
    internal const string SettingsFileName = "settings.json";

    internal static bool IsAgpHomeEnvironmentOverrideActive()
    {
        return !string.IsNullOrWhiteSpace(SafeGetEnvironmentVariable(AgpHomeEnvVar));
    }

    /// <summary>
    /// Resolves the root "agp" directory using the single agreed priority order:
    /// 1. Environment variable AGP_HOME (used directly as the agp layer);
    /// 2. Windows: %LOCALAPPDATA%\agp;
    /// 3. Linux/macOS: $XDG_DATA_HOME/agp, falling back to ~/.local/share/agp.
    /// </summary>
    internal static string ResolveAgpHome(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var rawAgpHome = agpHomeEnvOverride is not null
            ? agpHomeEnvOverride
            : SafeGetEnvironmentVariable(AgpHomeEnvVar);
        if (!string.IsNullOrWhiteSpace(rawAgpHome)
            && TryNormalizePath(rawAgpHome, out var normalizedEnvHome))
        {
            return normalizedEnvHome;
        }

        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        if (isWindows)
        {
            var localAppData = localAppDataOverride is not null
                ? localAppDataOverride
                : SafeGetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            if (string.IsNullOrWhiteSpace(localAppData) && localAppDataOverride is null)
            {
                localAppData = SafeGetEnvironmentVariable("LOCALAPPDATA");
            }

            if (!string.IsNullOrWhiteSpace(localAppData)
                && TryNormalizePath(Path.Combine(localAppData.Trim(), AgpFolderName), out var winAgpHome))
            {
                return winAgpHome;
            }
        }
        else
        {
            var xdgDataHome = xdgDataHomeOverride is not null
                ? xdgDataHomeOverride
                : SafeGetEnvironmentVariable("XDG_DATA_HOME");
            if (!string.IsNullOrWhiteSpace(xdgDataHome)
                && TryNormalizePath(Path.Combine(xdgDataHome.Trim(), AgpFolderName), out var xdgAgpHome))
            {
                return xdgAgpHome;
            }

            var homeDir = userProfileOverride is not null
                ? userProfileOverride
                : SafeGetFolderPath(Environment.SpecialFolder.UserProfile);
            if (string.IsNullOrWhiteSpace(homeDir) && userProfileOverride is null)
            {
                homeDir = SafeGetEnvironmentVariable("HOME");
            }

            if (!string.IsNullOrWhiteSpace(homeDir)
                && TryNormalizePath(Path.Combine(homeDir.Trim(), ".local", "share", AgpFolderName), out var unixAgpHome))
            {
                return unixAgpHome;
            }
        }

        var fallbackTemp = SafeGetTempPath();
        if (!string.IsNullOrWhiteSpace(fallbackTemp)
            && TryNormalizePath(Path.Combine(fallbackTemp.Trim(), AgpFolderName), out var tempAgpHome))
        {
            return tempAgpHome;
        }

        if (TryNormalizePath(Path.Combine(AppContext.BaseDirectory, AgpFolderName), out var baseAgpHome))
        {
            return baseAgpHome;
        }

        return Path.GetFullPath(AgpFolderName);
    }

    internal static string ResolveToolRoot(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var agpHome = ResolveAgpHome(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return PathSupport.TrimTrailingSeparators(Path.GetFullPath(Path.Combine(agpHome, ToolFolderName)));
    }

    internal static string GetDefaultOutputRoot(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var toolRoot = ResolveToolRoot(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return PathSupport.TrimTrailingSeparators(Path.GetFullPath(Path.Combine(toolRoot, OutputSubfolderName)));
    }

    internal static string GetDefaultOutputDirectory(
        string gameCode,
        string? steamBuildId,
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var outputRoot = GetDefaultOutputRoot(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        var normalizedCode = SanitizePathSegment(gameCode) ?? "ffr";
        var buildSegment = SanitizePathSegment(steamBuildId) ?? "unknown";
        return PathSupport.TrimTrailingSeparators(
            Path.GetFullPath(Path.Combine(outputRoot, $"{normalizedCode}-{buildSegment}")));
    }

    internal static string GetUsmapDirectory(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var toolRoot = ResolveToolRoot(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return PathSupport.TrimTrailingSeparators(Path.GetFullPath(Path.Combine(toolRoot, UsmapSubfolderName)));
    }

    internal static string GetDownloadsDirectory(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var toolRoot = ResolveToolRoot(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return PathSupport.TrimTrailingSeparators(Path.GetFullPath(Path.Combine(toolRoot, DownloadsSubfolderName)));
    }

    internal static string GetLogsDirectory(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var toolRoot = ResolveToolRoot(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return PathSupport.TrimTrailingSeparators(Path.GetFullPath(Path.Combine(toolRoot, LogsSubfolderName)));
    }

    internal static string GetSettingsFilePath(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var toolRoot = ResolveToolRoot(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return Path.GetFullPath(Path.Combine(toolRoot, SettingsFileName));
    }

    internal static string? SanitizePathSegment(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return null;
        }

        var sb = new StringBuilder(value.Length);
        foreach (var ch in value.Trim())
        {
            if (char.IsLetterOrDigit(ch) || ch is '-' or '_' or '.')
            {
                sb.Append(ch);
            }
        }

        return sb.Length > 0 ? sb.ToString() : null;
    }

    internal static bool TryNormalizePath(string? rawPath, out string normalizedPath)
    {
        normalizedPath = string.Empty;
        if (string.IsNullOrWhiteSpace(rawPath))
        {
            return false;
        }

        var trimmed = rawPath.Trim();
        if (trimmed.IndexOf('\0') >= 0)
        {
            return false;
        }

        try
        {
            var full = PathSupport.TrimTrailingSeparators(Path.GetFullPath(trimmed));
            if (string.IsNullOrWhiteSpace(full))
            {
                return false;
            }

            normalizedPath = full;
            return true;
        }
        catch
        {
            return false;
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

    private static string? SafeGetEnvironmentVariable(string name)
    {
        try
        {
            return Environment.GetEnvironmentVariable(name);
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
