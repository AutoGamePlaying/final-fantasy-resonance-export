using System.Globalization;

namespace FfrExport.Gui.Core;

internal sealed record DiscoveredMappingFile(
    string FullPath,
    string FileName,
    DateTimeOffset LastWriteTimeUtc,
    string SourceLabel);

internal sealed record UsmapDiscoveryResult(
    DiscoveredMappingFile? Selected,
    IReadOnlyList<DiscoveredMappingFile> Candidates,
    string? NotFoundGuidance,
    string? ScanError = null);

internal sealed record JmapDiscoveryResult(
    IReadOnlyList<DiscoveredMappingFile> Files,
    string? ScanError);

/// <summary>
/// Auto-discovers UE4SS .usmap and optional .jmap files inside a FINAL FANTASY RESONANCE game directory.
/// </summary>
internal static class MappingDiscovery
{
    internal const string RelativeUe4ssSubpath = "FFRS/Binaries/Win64/ue4ss";
    internal const string RelativeWin64Subpath = "FFRS/Binaries/Win64";

    internal const string UsmapNotFoundGuidance =
        "未自动找到 .usmap 文件。需要先用 UE4SS 在游戏运行时转储（默认保存在游戏目录的 FFRS\\Binaries\\Win64\\ue4ss\\ 下），或点击“选择 .usmap…”手动指定。";

    /// <summary>
    /// Searches FFRS\Binaries\Win64\ue4ss\*.usmap first, then FFRS\Binaries\Win64\*.usmap.
    /// When multiple files exist in the matched location, selects the one with the latest modification time.
    /// </summary>
    internal static UsmapDiscoveryResult DiscoverUsmap(
        string? gameDirectory,
        Func<string, IEnumerable<string>>? fileEnumeratorOverride = null)
    {
        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            return new UsmapDiscoveryResult(null, [], UsmapNotFoundGuidance);
        }

        var errors = new List<string>();

        var ue4ssDir = SteamLocator.CombineRelativeSubpath(gameDirectory, RelativeUe4ssSubpath);
        var ue4ssScan = ScanDirectory(ue4ssDir, ".usmap", "ue4ss", fileEnumeratorOverride);
        if (!string.IsNullOrWhiteSpace(ue4ssScan.ErrorMessage))
        {
            errors.Add(ue4ssScan.ErrorMessage);
        }

        if (ue4ssScan.Files.Count > 0)
        {
            var ordered = OrderNewestFirst(ue4ssScan.Files);
            var partialError = errors.Count > 0 ? string.Join("；", errors) : null;
            return new UsmapDiscoveryResult(ordered[0], ordered, null, partialError);
        }

        var win64Dir = SteamLocator.CombineRelativeSubpath(gameDirectory, RelativeWin64Subpath);
        var win64Scan = ScanDirectory(win64Dir, ".usmap", "Win64", fileEnumeratorOverride);
        if (!string.IsNullOrWhiteSpace(win64Scan.ErrorMessage))
        {
            errors.Add(win64Scan.ErrorMessage);
        }

        if (win64Scan.Files.Count > 0)
        {
            var ordered = OrderNewestFirst(win64Scan.Files);
            var partialError = errors.Count > 0 ? string.Join("；", errors) : null;
            return new UsmapDiscoveryResult(ordered[0], ordered, null, partialError);
        }

        if (errors.Count > 0)
        {
            var combinedError = string.Join("；", errors);
            var errorGuidance =
                $"自动查找 .usmap 失败（{combinedError}）。请检查游戏目录权限，或点击「选择 .usmap…」手动指定。";
            return new UsmapDiscoveryResult(null, [], errorGuidance, combinedError);
        }

        return new UsmapDiscoveryResult(null, [], UsmapNotFoundGuidance);
    }

    /// <summary>
    /// Searches FFRS\Binaries\Win64\ue4ss\*.jmap and FFRS\Binaries\Win64\*.jmap and lists all discovered files.
    /// </summary>
    internal static IReadOnlyList<DiscoveredMappingFile> DiscoverJmaps(string? gameDirectory)
    {
        return DiscoverJmapsWithDiagnostics(gameDirectory).Files;
    }

    /// <summary>
    /// Searches FFRS\Binaries\Win64\ue4ss\*.jmap and FFRS\Binaries\Win64\*.jmap while preserving any
    /// directory permission or I/O errors for UI reporting.
    /// </summary>
    internal static JmapDiscoveryResult DiscoverJmapsWithDiagnostics(
        string? gameDirectory,
        Func<string, IEnumerable<string>>? fileEnumeratorOverride = null)
    {
        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            return new JmapDiscoveryResult([], null);
        }

        var ue4ssDir = SteamLocator.CombineRelativeSubpath(gameDirectory, RelativeUe4ssSubpath);
        var win64Dir = SteamLocator.CombineRelativeSubpath(gameDirectory, RelativeWin64Subpath);

        var results = new List<DiscoveredMappingFile>();
        var errors = new List<string>();
        var comparer = OperatingSystem.IsWindows() ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal;
        var seen = new HashSet<string>(comparer);

        var ue4ssScan = ScanDirectory(ue4ssDir, ".jmap", "ue4ss", fileEnumeratorOverride);
        if (!string.IsNullOrWhiteSpace(ue4ssScan.ErrorMessage))
        {
            errors.Add(ue4ssScan.ErrorMessage);
        }

        foreach (var item in OrderOldestFirst(ue4ssScan.Files))
        {
            if (seen.Add(item.FullPath))
            {
                results.Add(item);
            }
        }

        var win64Scan = ScanDirectory(win64Dir, ".jmap", "Win64", fileEnumeratorOverride);
        if (!string.IsNullOrWhiteSpace(win64Scan.ErrorMessage))
        {
            errors.Add(win64Scan.ErrorMessage);
        }

        foreach (var item in OrderOldestFirst(win64Scan.Files))
        {
            if (seen.Add(item.FullPath))
            {
                results.Add(item);
            }
        }

        var combinedError = errors.Count > 0 ? string.Join("；", errors) : null;
        return new JmapDiscoveryResult(results, combinedError);
    }

    internal static DiscoveredMappingFile CreateFromExistingFile(string filePath, string sourceLabel = "手动选择")
    {
        var fullPath = Path.GetFullPath(filePath);
        var modifiedUtc = TryReadLastWriteTimeUtc(fullPath);
        return new DiscoveredMappingFile(fullPath, Path.GetFileName(fullPath), modifiedUtc, sourceLabel);
    }

    internal static string FormatUsmapDisplay(
        DiscoveredMappingFile file,
        int candidateCount = 1,
        TimeZoneInfo? displayTimeZone = null)
    {
        var formattedDate = FormatModifiedDate(file.LastWriteTimeUtc, displayTimeZone);
        if (candidateCount > 1)
        {
            return $"{file.FileName}（修改日期：{formattedDate}，共找到 {candidateCount} 份，已选最新）";
        }

        return $"{file.FileName}（修改日期：{formattedDate}）";
    }

    internal static string FormatJmapDisplay(
        DiscoveredMappingFile file,
        TimeZoneInfo? displayTimeZone = null)
    {
        var formattedDate = FormatModifiedDate(file.LastWriteTimeUtc, displayTimeZone);
        return $"{file.FileName}（修改日期：{formattedDate}，{file.SourceLabel}） — {file.FullPath}";
    }

    internal static string FormatModifiedDate(
        DateTimeOffset lastWriteTimeUtc,
        TimeZoneInfo? displayTimeZone = null)
    {
        if (lastWriteTimeUtc == DateTimeOffset.MinValue)
        {
            return "未知";
        }

        TimeZoneInfo targetZone;
        try
        {
            targetZone = displayTimeZone ?? TimeZoneInfo.Local;
        }
        catch
        {
            targetZone = TimeZoneInfo.Utc;
        }

        try
        {
            var converted = TimeZoneInfo.ConvertTime(lastWriteTimeUtc, targetZone);
            return converted.ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture);
        }
        catch (ArgumentOutOfRangeException)
        {
            return lastWriteTimeUtc.ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture) + " UTC";
        }
    }

    private static (List<DiscoveredMappingFile> Files, string? ErrorMessage) ScanDirectory(
        string directoryPath,
        string extension,
        string sourceLabel,
        Func<string, IEnumerable<string>>? fileEnumeratorOverride)
    {
        var results = new List<DiscoveredMappingFile>();
        string ioDir;
        try
        {
            ioDir = PathSupport.ToIoPath(directoryPath);
            if (fileEnumeratorOverride is null && !Directory.Exists(ioDir))
            {
                return (results, null);
            }
        }
        catch (Exception exception)
        {
            return (results, $"无法访问目录 {directoryPath}：{exception.Message}");
        }

        try
        {
            var files = fileEnumeratorOverride is not null
                ? fileEnumeratorOverride(directoryPath)
                : Directory.EnumerateFiles(ioDir, "*", SearchOption.TopDirectoryOnly);

            foreach (var rawPath in files)
            {
                try
                {
                    if (!string.Equals(Path.GetExtension(rawPath), extension, StringComparison.OrdinalIgnoreCase))
                    {
                        continue;
                    }

                    var fullPath = Path.GetFullPath(rawPath);
                    var modifiedUtc = TryReadLastWriteTimeUtc(fullPath);
                    results.Add(new DiscoveredMappingFile(fullPath, Path.GetFileName(fullPath), modifiedUtc, sourceLabel));
                }
                catch
                {
                    // Skip malformed or unreadable single file entries while preserving other valid files.
                }
            }
        }
        catch (UnauthorizedAccessException exception)
        {
            return (results, $"无权读取目录 {directoryPath}：{exception.Message}");
        }
        catch (Exception exception)
        {
            return (results, $"扫描目录 {directoryPath} 失败：{exception.Message}");
        }

        return (results, null);
    }

    private static DateTimeOffset TryReadLastWriteTimeUtc(string fullPath)
    {
        try
        {
            var info = new FileInfo(PathSupport.ToIoPath(fullPath));
            return info.Exists
                ? new DateTimeOffset(DateTime.SpecifyKind(info.LastWriteTimeUtc, DateTimeKind.Utc))
                : DateTimeOffset.MinValue;
        }
        catch
        {
            return DateTimeOffset.MinValue;
        }
    }

    private static List<DiscoveredMappingFile> OrderNewestFirst(List<DiscoveredMappingFile> items)
    {
        return items
            .OrderByDescending(file => file.LastWriteTimeUtc)
            .ThenByDescending(file => file.FileName, StringComparer.OrdinalIgnoreCase)
            .ThenBy(file => file.FullPath, StringComparer.Ordinal)
            .ToList();
    }

    private static List<DiscoveredMappingFile> OrderOldestFirst(List<DiscoveredMappingFile> items)
    {
        return items
            .OrderBy(file => file.LastWriteTimeUtc)
            .ThenBy(file => file.FileName, StringComparer.OrdinalIgnoreCase)
            .ThenBy(file => file.FullPath, StringComparer.Ordinal)
            .ToList();
    }
}
