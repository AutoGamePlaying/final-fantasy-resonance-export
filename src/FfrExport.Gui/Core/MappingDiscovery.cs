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
    string? NotFoundGuidance);

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
    internal static UsmapDiscoveryResult DiscoverUsmap(string? gameDirectory)
    {
        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            return new UsmapDiscoveryResult(null, [], UsmapNotFoundGuidance);
        }

        var ue4ssDir = SteamLocator.CombineRelativeSubpath(gameDirectory, RelativeUe4ssSubpath);
        var ue4ssFiles = ScanDirectory(ue4ssDir, ".usmap", "ue4ss");
        if (ue4ssFiles.Count > 0)
        {
            var ordered = OrderNewestFirst(ue4ssFiles);
            return new UsmapDiscoveryResult(ordered[0], ordered, null);
        }

        var win64Dir = SteamLocator.CombineRelativeSubpath(gameDirectory, RelativeWin64Subpath);
        var win64Files = ScanDirectory(win64Dir, ".usmap", "Win64");
        if (win64Files.Count > 0)
        {
            var ordered = OrderNewestFirst(win64Files);
            return new UsmapDiscoveryResult(ordered[0], ordered, null);
        }

        return new UsmapDiscoveryResult(null, [], UsmapNotFoundGuidance);
    }

    /// <summary>
    /// Searches FFRS\Binaries\Win64\ue4ss\*.jmap and FFRS\Binaries\Win64\*.jmap and lists all discovered files.
    /// </summary>
    internal static IReadOnlyList<DiscoveredMappingFile> DiscoverJmaps(string? gameDirectory)
    {
        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            return [];
        }

        var ue4ssDir = SteamLocator.CombineRelativeSubpath(gameDirectory, RelativeUe4ssSubpath);
        var win64Dir = SteamLocator.CombineRelativeSubpath(gameDirectory, RelativeWin64Subpath);

        var results = new List<DiscoveredMappingFile>();
        var comparer = OperatingSystem.IsWindows() ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal;
        var seen = new HashSet<string>(comparer);

        foreach (var item in OrderOldestFirst(ScanDirectory(ue4ssDir, ".jmap", "ue4ss")))
        {
            if (seen.Add(item.FullPath))
            {
                results.Add(item);
            }
        }

        foreach (var item in OrderOldestFirst(ScanDirectory(win64Dir, ".jmap", "Win64")))
        {
            if (seen.Add(item.FullPath))
            {
                results.Add(item);
            }
        }

        return results;
    }

    internal static DiscoveredMappingFile CreateFromExistingFile(string filePath, string sourceLabel = "手动选择")
    {
        var fullPath = Path.GetFullPath(filePath);
        var info = new FileInfo(PathSupport.ToIoPath(fullPath));
        var modifiedUtc = info.Exists
            ? new DateTimeOffset(DateTime.SpecifyKind(info.LastWriteTimeUtc, DateTimeKind.Utc))
            : DateTimeOffset.MinValue;
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

        var targetZone = displayTimeZone ?? TimeZoneInfo.Local;
        var converted = TimeZoneInfo.ConvertTime(lastWriteTimeUtc, targetZone);
        return converted.ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture);
    }

    private static List<DiscoveredMappingFile> ScanDirectory(
        string directoryPath,
        string extension,
        string sourceLabel)
    {
        var results = new List<DiscoveredMappingFile>();
        var ioDir = PathSupport.ToIoPath(directoryPath);
        if (!Directory.Exists(ioDir))
        {
            return results;
        }

        try
        {
            foreach (var rawPath in Directory.EnumerateFiles(ioDir, "*", SearchOption.TopDirectoryOnly))
            {
                if (!string.Equals(Path.GetExtension(rawPath), extension, StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                var fullPath = Path.GetFullPath(rawPath);
                var info = new FileInfo(PathSupport.ToIoPath(fullPath));
                var modifiedUtc = new DateTimeOffset(DateTime.SpecifyKind(info.LastWriteTimeUtc, DateTimeKind.Utc));
                results.Add(new DiscoveredMappingFile(fullPath, Path.GetFileName(fullPath), modifiedUtc, sourceLabel));
            }
        }
        catch
        {
            // Ignore unreadable directories and treat as no files found.
        }

        return results;
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
