using System.Text;
using System.Text.Json;

namespace FfrExport.Gui.Core;

internal sealed record CliExecutableLookup(
    bool Found,
    string ExpectedPath,
    string FileName,
    string? ErrorMessage);

internal enum OutputDirectoryStatus
{
    InvalidPath,
    OverlapsGameDirectory,
    TargetIsFile,
    NotExists,
    ExistsEmpty,
    ExistsPreviousBuild,
    ExistsForeignNonEmpty,
}

internal sealed record OutputDirectoryInspection(
    OutputDirectoryStatus Status,
    string NormalizedPath,
    int EntryCount,
    bool IsProtectedDirectory,
    string? ErrorMessage);

internal sealed record OutputRootResolution(
    string Path,
    bool UsedFallback,
    string? WarningMessage);

internal sealed record OutputDirectoryBuildResult(
    string Path,
    bool UsedFallback,
    string? WarningMessage);

/// <summary>
/// Pure path calculations, CLI executable lookup, and output directory safety checks for the GUI.
/// Uses the unified AgpPaths layout (%LOCALAPPDATA%\agp\ffr-export\output\).
/// </summary>
internal static class GuiPaths
{
    internal const string BuildMetadataFileName = "ffr.sqlite.build.json";
    internal const string CliExecutableWindows = "ffr-export.exe";
    internal const string CliExecutableUnix = "ffr-export";

    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    /// <summary>
    /// Resolves the default export root directory (%LOCALAPPDATA%\agp\ffr-export\output).
    /// </summary>
    internal static string ResolveDefaultOutputRoot(
        string? agpHomeOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        return AgpPaths.GetDefaultOutputRoot(
            agpHomeEnvOverride: agpHomeOverride,
            isWindowsOverride: isWindowsOverride,
            localAppDataOverride: localAppDataOverride,
            xdgDataHomeOverride: xdgDataHomeOverride,
            userProfileOverride: userProfileOverride);
    }

    /// <summary>
    /// Resolves the default export root directory safely, detecting blocked or inaccessible
    /// primary roots and falling back cleanly while reporting a Chinese warning for the UI.
    /// </summary>
    internal static OutputRootResolution ResolveDefaultOutputRootSafe(
        string? agpHomeOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null,
        string? tempDirectoryOverride = null,
        Func<string, bool>? directoryUsableCheck = null)
    {
        var isUsable = directoryUsableCheck ?? IsTargetDirectoryPathUsable;
        var primaryRoot = ResolveDefaultOutputRoot(
            agpHomeOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);

        string? primaryReason = null;
        if (TryNormalizeDirectoryPath(primaryRoot, out var normalizedPrimary, out var primaryPathError))
        {
            if (isUsable(normalizedPrimary))
            {
                return new OutputRootResolution(normalizedPrimary, UsedFallback: false, WarningMessage: null);
            }

            primaryReason = $"默认数据目录不可用（{normalizedPrimary}）";
        }
        else
        {
            primaryReason = $"默认数据目录路径无效（{primaryRoot}：{primaryPathError}）";
        }

        var fallbackBases = new (string? BasePath, string[] SubSegments)[]
        {
            (
                tempDirectoryOverride is not null ? tempDirectoryOverride : SafeGetTempPath(),
                [AgpPaths.AgpFolderName, AgpPaths.ToolFolderName, AgpPaths.OutputSubfolderName]),
            (
                AppContext.BaseDirectory,
                [AgpPaths.OutputSubfolderName]),
            (
                SafeGetCurrentDirectory(),
                [AgpPaths.OutputSubfolderName]),
        };

        foreach (var (basePath, subSegments) in fallbackBases)
        {
            if (string.IsNullOrWhiteSpace(basePath)
                || !TryNormalizeDirectoryPath(basePath, out var normalizedBase, out _))
            {
                continue;
            }

            var combined = normalizedBase;
            foreach (var segment in subSegments)
            {
                combined = Path.Combine(combined, segment);
            }

            if (TryNormalizeDirectoryPath(combined, out var normalizedFallback, out _)
                && isUsable(normalizedFallback))
            {
                return new OutputRootResolution(
                    normalizedFallback,
                    UsedFallback: true,
                    WarningMessage: $"{primaryReason}，默认输出位置已改用：{normalizedFallback}（可随时点击「浏览…」手动修改）。");
            }
        }

        return new OutputRootResolution(
            AgpPaths.OutputSubfolderName,
            UsedFallback: true,
            WarningMessage: $"{primaryReason}，且备选目录均不可用，请点击「浏览…」手动选择输出目录。");
    }

    internal static bool TryNormalizeDirectoryPath(
        string? rawPath,
        out string normalizedPath,
        out string? errorMessage)
    {
        normalizedPath = string.Empty;
        errorMessage = null;

        var cleaned = CleanPathInput(rawPath);
        if (string.IsNullOrWhiteSpace(cleaned))
        {
            errorMessage = "路径为空";
            return false;
        }

        if (cleaned.IndexOf('\0') >= 0)
        {
            errorMessage = "路径包含非法空字符";
            return false;
        }

        try
        {
            var full = PathSupport.TrimTrailingSeparators(Path.GetFullPath(cleaned));
            if (string.IsNullOrWhiteSpace(full))
            {
                errorMessage = "路径无效";
                return false;
            }

            normalizedPath = full;
            return true;
        }
        catch (Exception exception)
        {
            errorMessage = exception.Message;
            return false;
        }
    }

    /// <summary>
    /// Computes the default output directory: &lt;outputRoot&gt;\&lt;gameCode&gt;-&lt;steamBuildId&gt;.
    /// </summary>
    internal static string BuildDefaultOutputDirectory(
        string? outputRootOverride,
        string gameCode,
        string? steamBuildId,
        string? agpHomeOverride = null)
    {
        string baseRoot;
        if (string.IsNullOrWhiteSpace(outputRootOverride)
            || !TryNormalizeDirectoryPath(outputRootOverride, out baseRoot, out _))
        {
            baseRoot = ResolveDefaultOutputRoot(agpHomeOverride: agpHomeOverride);
        }

        var normalizedCode = SanitizePathSegment(gameCode) ?? "ffr";
        var buildSegment = SanitizePathSegment(steamBuildId) ?? "unknown";
        var combined = Path.Combine(baseRoot, $"{normalizedCode}-{buildSegment}");
        return TryNormalizeDirectoryPath(combined, out var fullOutput, out _)
            ? fullOutput
            : combined;
    }

    /// <summary>
    /// Computes the default output directory safely and returns any fallback warning caused by a
    /// malformed saved root or inaccessible primary data root.
    /// </summary>
    internal static OutputDirectoryBuildResult BuildDefaultOutputDirectorySafe(
        string? outputRootOverride,
        string gameCode,
        string? steamBuildId,
        string? agpHomeOverride = null,
        string? tempDirectoryOverride = null,
        Func<string, bool>? directoryUsableCheck = null)
    {
        string baseRoot;
        var usedFallback = false;
        string? warning = null;

        if (!string.IsNullOrWhiteSpace(outputRootOverride))
        {
            if (TryNormalizeDirectoryPath(outputRootOverride, out var normalizedOverride, out var overrideError))
            {
                baseRoot = normalizedOverride;
            }
            else
            {
                var fallback = ResolveDefaultOutputRootSafe(
                    agpHomeOverride: agpHomeOverride,
                    tempDirectoryOverride: tempDirectoryOverride,
                    directoryUsableCheck: directoryUsableCheck);
                baseRoot = fallback.Path;
                usedFallback = true;
                warning = $"上次保存的输出目录无效（{outputRootOverride}：{overrideError}），已改用默认目录：{baseRoot}。";
            }
        }
        else
        {
            var resolved = ResolveDefaultOutputRootSafe(
                agpHomeOverride: agpHomeOverride,
                tempDirectoryOverride: tempDirectoryOverride,
                directoryUsableCheck: directoryUsableCheck);
            baseRoot = resolved.Path;
            usedFallback = resolved.UsedFallback;
            warning = resolved.WarningMessage;
        }

        var normalizedCode = SanitizePathSegment(gameCode) ?? "ffr";
        var buildSegment = SanitizePathSegment(steamBuildId) ?? "unknown";
        var combined = Path.Combine(baseRoot, $"{normalizedCode}-{buildSegment}");
        if (TryNormalizeDirectoryPath(combined, out var fullOutput, out _))
        {
            return new OutputDirectoryBuildResult(fullOutput, usedFallback, warning);
        }

        return new OutputDirectoryBuildResult(combined, usedFallback, warning);
    }

    /// <summary>
    /// Extracts the reusable output root directory from a chosen output path so settings remember
    /// the parent root without double-nesting &lt;gameCode&gt;-&lt;buildId&gt; segments on the next launch.
    /// </summary>
    internal static string? ExtractOutputRootDirectory(string? outputDirectory, string gameCode)
    {
        var cleaned = CleanPathInput(outputDirectory);
        if (string.IsNullOrWhiteSpace(cleaned))
        {
            return null;
        }

        string fullPath;
        try
        {
            fullPath = PathSupport.TrimTrailingSeparators(Path.GetFullPath(cleaned));
        }
        catch
        {
            return null;
        }

        var normalizedCode = SanitizePathSegment(gameCode) ?? "ffr";
        var leaf = Path.GetFileName(fullPath);
        if (!string.IsNullOrEmpty(leaf)
            && leaf.StartsWith($"{normalizedCode}-", StringComparison.OrdinalIgnoreCase))
        {
            var parent = Path.GetDirectoryName(fullPath);
            if (!string.IsNullOrWhiteSpace(parent))
            {
                return PathSupport.TrimTrailingSeparators(parent);
            }
        }

        return fullPath;
    }

    /// <summary>
    /// Locates the sibling ffr-export CLI executable next to the GUI executable.
    /// </summary>
    internal static CliExecutableLookup ResolveCliExecutable(
        string? executableDirectoryOverride = null,
        bool? isWindowsOverride = null)
    {
        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        var fileName = isWindows ? CliExecutableWindows : CliExecutableUnix;
        var baseDir = string.IsNullOrWhiteSpace(executableDirectoryOverride)
            ? AppContext.BaseDirectory
            : executableDirectoryOverride.Trim();

        string normalizedBaseDir;
        try
        {
            normalizedBaseDir = PathSupport.TrimTrailingSeparators(Path.GetFullPath(baseDir));
        }
        catch (Exception exception)
        {
            return new CliExecutableLookup(
                false,
                baseDir,
                fileName,
                $"定位命令行程序 {fileName} 失败（程序目录路径无效：{baseDir}）：{exception.Message}");
        }

        var expectedPath = Path.Combine(normalizedBaseDir, fileName);

        try
        {
            if (File.Exists(PathSupport.ToIoPath(expectedPath)))
            {
                return new CliExecutableLookup(true, expectedPath, fileName, null);
            }
        }
        catch (Exception exception)
        {
            return new CliExecutableLookup(
                false,
                expectedPath,
                fileName,
                $"检查命令行程序 {fileName} 失败（{expectedPath}）：{exception.Message}");
        }

        var dbFileName = isWindows ? "ffr-db.exe" : "ffr-db";
        var errorMessage =
            $"找不到命令行程序 {fileName}（期望路径：{expectedPath}）。" +
            $"请将压缩包完整解压到同一个文件夹，确保 ffr-export-gui.exe、{fileName}、{dbFileName} 与 known-builds.json 位于同一目录，不要单独移动窗口程序。";
        return new CliExecutableLookup(false, expectedPath, fileName, errorMessage);
    }

    /// <summary>
    /// Inspects a candidate output directory before starting an export so the GUI can prompt before
    /// overwriting an existing directory that was or was not created by this tool.
    /// </summary>
    internal static OutputDirectoryInspection InspectOutputDirectory(
        string? rawOutputDirectory,
        string? gameDirectory,
        IReadOnlyCollection<string>? additionalProtectedDirectories = null)
    {
        var cleaned = CleanPathInput(rawOutputDirectory);
        if (string.IsNullOrWhiteSpace(cleaned))
        {
            return new OutputDirectoryInspection(
                OutputDirectoryStatus.InvalidPath,
                string.Empty,
                0,
                false,
                "输出目录不能为空，请选择一个输出文件夹。");
        }

        string fullOutputPath;
        try
        {
            fullOutputPath = PathSupport.TrimTrailingSeparators(Path.GetFullPath(cleaned));
        }
        catch (Exception exception)
        {
            return new OutputDirectoryInspection(
                OutputDirectoryStatus.InvalidPath,
                cleaned,
                0,
                false,
                $"输出目录路径无效（{cleaned}）：{exception.Message}");
        }

        if (!string.IsNullOrWhiteSpace(gameDirectory))
        {
            try
            {
                var fullGamePath = PathSupport.TrimTrailingSeparators(Path.GetFullPath(gameDirectory));
                if (PathSupport.IsSameOrDescendantPath(fullGamePath, fullOutputPath)
                    || PathSupport.IsSameOrDescendantPath(fullOutputPath, fullGamePath))
                {
                    return new OutputDirectoryInspection(
                        OutputDirectoryStatus.OverlapsGameDirectory,
                        fullOutputPath,
                        0,
                        true,
                        $"输出目录不能与游戏安装目录重叠：{fullOutputPath}，请更换其他目录。");
                }
            }
            catch
            {
                // Ignore invalid gameDirectory here; game validation reports it separately.
            }
        }

        var ioPath = PathSupport.ToIoPath(fullOutputPath);
        if (File.Exists(ioPath))
        {
            return new OutputDirectoryInspection(
                OutputDirectoryStatus.TargetIsFile,
                fullOutputPath,
                0,
                false,
                $"输出路径指向一个已存在的文件而非文件夹：{fullOutputPath}，请更换路径。");
        }

        if (!Directory.Exists(ioPath))
        {
            return new OutputDirectoryInspection(
                OutputDirectoryStatus.NotExists,
                fullOutputPath,
                0,
                false,
                null);
        }

        int entryCount;
        try
        {
            entryCount = Directory.EnumerateFileSystemEntries(ioPath).Count();
        }
        catch (Exception exception)
        {
            return new OutputDirectoryInspection(
                OutputDirectoryStatus.InvalidPath,
                fullOutputPath,
                0,
                false,
                $"无法读取输出目录 {fullOutputPath}：{exception.Message}");
        }

        if (entryCount == 0)
        {
            return new OutputDirectoryInspection(
                OutputDirectoryStatus.ExistsEmpty,
                fullOutputPath,
                0,
                false,
                null);
        }

        if (IsOwnBuildDbOutputDirectory(ioPath))
        {
            return new OutputDirectoryInspection(
                OutputDirectoryStatus.ExistsPreviousBuild,
                fullOutputPath,
                entryCount,
                false,
                null);
        }

        var isProtected = IsProtectedDirectory(fullOutputPath, additionalProtectedDirectories);
        return new OutputDirectoryInspection(
            OutputDirectoryStatus.ExistsForeignNonEmpty,
            fullOutputPath,
            entryCount,
            isProtected,
            isProtected
                ? $"输出目录 {fullOutputPath} 是系统或用户根级目录且非空，为防止误删个人文件，拒绝清空该目录；请在其下新建或指定专用子文件夹。"
                : null);
    }

    /// <summary>
    /// Removes all files and subdirectories inside a user-confirmed non-protected directory.
    /// </summary>
    internal static void ClearDirectoryContents(
        string directoryPath,
        IReadOnlyCollection<string>? additionalProtectedDirectories = null)
    {
        var fullPath = PathSupport.TrimTrailingSeparators(Path.GetFullPath(directoryPath));
        if (IsProtectedDirectory(fullPath, additionalProtectedDirectories))
        {
            throw new InvalidOperationException($"拒绝清空受保护目录：{fullPath}");
        }

        var ioPath = PathSupport.ToIoPath(fullPath);
        if (!Directory.Exists(ioPath))
        {
            return;
        }

        foreach (var file in Directory.EnumerateFiles(ioPath))
        {
            File.Delete(file);
        }

        foreach (var subdir in Directory.EnumerateDirectories(ioPath))
        {
            Directory.Delete(subdir, recursive: true);
        }
    }

    internal static bool IsOwnBuildDbOutputDirectory(string ioOutputDirectory)
    {
        var metadataPath = Path.Combine(ioOutputDirectory, BuildMetadataFileName);
        if (!File.Exists(metadataPath))
        {
            return false;
        }

        try
        {
            using var doc = JsonDocument.Parse(File.ReadAllText(metadataPath, Utf8WithoutBom));
            if (doc.RootElement.ValueKind != JsonValueKind.Object)
            {
                return false;
            }

            if (!doc.RootElement.TryGetProperty("command", out var commandProp)
                || commandProp.ValueKind != JsonValueKind.String
                || !string.Equals(commandProp.GetString(), "build-db", StringComparison.Ordinal))
            {
                return false;
            }

            return doc.RootElement.TryGetProperty("cue4ParseVersion", out _);
        }
        catch (JsonException)
        {
            return false;
        }
        catch (IOException)
        {
            return false;
        }
    }

    internal static bool IsProtectedDirectory(
        string fullDirectoryPath,
        IReadOnlyCollection<string>? additionalProtectedDirectories = null)
    {
        var normalized = PathSupport.TrimTrailingSeparators(Path.GetFullPath(fullDirectoryPath));
        var pathRoot = Path.GetPathRoot(normalized);
        if (!string.IsNullOrEmpty(pathRoot))
        {
            var normalizedRoot = PathSupport.TrimTrailingSeparators(pathRoot);
            if (PathsEqual(normalized, normalizedRoot))
            {
                return true;
            }
        }

        var specialFolders = new[]
        {
            Environment.SpecialFolder.UserProfile,
            Environment.SpecialFolder.MyDocuments,
            Environment.SpecialFolder.DesktopDirectory,
            Environment.SpecialFolder.ApplicationData,
            Environment.SpecialFolder.LocalApplicationData,
            Environment.SpecialFolder.ProgramFiles,
            Environment.SpecialFolder.ProgramFilesX86,
            Environment.SpecialFolder.Windows,
        };

        foreach (var folder in specialFolders)
        {
            var folderPath = Environment.GetFolderPath(folder);
            if (!string.IsNullOrWhiteSpace(folderPath) && PathsEqual(normalized, folderPath))
            {
                return true;
            }
        }

        if (additionalProtectedDirectories is not null)
        {
            foreach (var extra in additionalProtectedDirectories)
            {
                if (!string.IsNullOrWhiteSpace(extra) && PathsEqual(normalized, extra))
                {
                    return true;
                }
            }
        }

        return false;
    }

    internal static string CleanPathInput(string? rawInput)
    {
        if (string.IsNullOrWhiteSpace(rawInput))
        {
            return string.Empty;
        }

        var cleaned = rawInput.Trim();
        while (cleaned.Length >= 2
            && ((cleaned[0] == '"' && cleaned[^1] == '"')
                || (cleaned[0] == '\'' && cleaned[^1] == '\'')))
        {
            cleaned = cleaned[1..^1].Trim();
        }

        return cleaned;
    }

    internal static string? SanitizePathSegment(string? value)
    {
        return AgpPaths.SanitizePathSegment(value);
    }

    private static bool IsTargetDirectoryPathUsable(string fullPath)
    {
        try
        {
            var current = fullPath;
            while (!string.IsNullOrEmpty(current))
            {
                var ioCurrent = PathSupport.ToIoPath(current);
                if (File.Exists(ioCurrent))
                {
                    return false;
                }

                if (Directory.Exists(ioCurrent))
                {
                    return true;
                }

                var parent = Path.GetDirectoryName(current);
                if (string.IsNullOrEmpty(parent) || string.Equals(parent, current, StringComparison.Ordinal))
                {
                    break;
                }

                current = parent;
            }

            return false;
        }
        catch
        {
            return false;
        }
    }

    private static bool PathsEqual(string left, string right)
    {
        var a = PathSupport.TrimTrailingSeparators(Path.GetFullPath(left));
        var b = PathSupport.TrimTrailingSeparators(Path.GetFullPath(right));
        var comparison = OperatingSystem.IsWindows()
            ? StringComparison.OrdinalIgnoreCase
            : StringComparison.Ordinal;
        return string.Equals(a, b, comparison);
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

    private static string? SafeGetCurrentDirectory()
    {
        try
        {
            return Directory.GetCurrentDirectory();
        }
        catch
        {
            return null;
        }
    }
}
