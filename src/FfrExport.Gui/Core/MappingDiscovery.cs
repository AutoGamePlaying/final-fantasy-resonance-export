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

    internal static string UsmapNotFoundGuidance => L.Get("GuiCore_UsmapNotFoundGuidance");

    /// <summary>
    /// Searches %LOCALAPPDATA%\agp\ffr-export\usmap\*.usmap and FFRS\Binaries\Win64\ue4ss\*.usmap first
    /// (selecting the newest file across both, breaking ties in favor of agp/usmap),
    /// then falls back to FFRS\Binaries\Win64\*.usmap.
    /// </summary>
    internal static UsmapDiscoveryResult DiscoverUsmap(
        string? gameDirectory,
        Func<string, IEnumerable<string>>? fileEnumeratorOverride = null,
        string? agpUsmapDirectoryOverride = null)
    {
        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            return new UsmapDiscoveryResult(null, [], UsmapNotFoundGuidance);
        }

        var errors = new List<string>();
        var agpUsmapDir = agpUsmapDirectoryOverride is not null
            ? agpUsmapDirectoryOverride
            : AgpPaths.GetUsmapDirectory();

        var primaryFiles = new List<DiscoveredMappingFile>();
        var comparer = OperatingSystem.IsWindows() ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal;
        var seen = new HashSet<string>(comparer);

        if (!string.IsNullOrWhiteSpace(agpUsmapDir))
        {
            var agpScan = ScanDirectory(agpUsmapDir, ".usmap", "agp/usmap", fileEnumeratorOverride);
            if (!string.IsNullOrWhiteSpace(agpScan.ErrorMessage))
            {
                errors.Add(agpScan.ErrorMessage);
            }

            foreach (var file in agpScan.Files)
            {
                if (seen.Add(file.FullPath))
                {
                    primaryFiles.Add(file);
                }
            }
        }

        var ue4ssDir = SteamLocator.CombineRelativeSubpath(gameDirectory, RelativeUe4ssSubpath);
        var ue4ssScan = ScanDirectory(ue4ssDir, ".usmap", "ue4ss", fileEnumeratorOverride);
        if (!string.IsNullOrWhiteSpace(ue4ssScan.ErrorMessage))
        {
            errors.Add(ue4ssScan.ErrorMessage);
        }

        foreach (var file in ue4ssScan.Files)
        {
            if (seen.Add(file.FullPath))
            {
                primaryFiles.Add(file);
            }
        }

        if (primaryFiles.Count > 0)
        {
            var ordered = OrderNewestFirstPreferringAgp(primaryFiles);
            var partialError = errors.Count > 0 ? string.Join(L.Get("Common_SemicolonSeparator"), errors) : null;
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
            var ordered = OrderNewestFirstPreferringAgp(win64Scan.Files);
            var partialError = errors.Count > 0 ? string.Join(L.Get("Common_SemicolonSeparator"), errors) : null;
            return new UsmapDiscoveryResult(ordered[0], ordered, null, partialError);
        }

        if (errors.Count > 0)
        {
            var combinedError = string.Join(L.Get("Common_SemicolonSeparator"), errors);
            var errorGuidance = L.Format("GuiCore_UsmapDiscoveryErrorGuidance", combinedError);
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

        var combinedError = errors.Count > 0 ? string.Join(L.Get("Common_SemicolonSeparator"), errors) : null;
        return new JmapDiscoveryResult(results, combinedError);
    }

    internal static DiscoveredMappingFile CreateFromExistingFile(string filePath, string sourceLabel = "manual")
    {
        var fullPath = Path.GetFullPath(filePath);
        var modifiedUtc = TryReadLastWriteTimeUtc(fullPath);
        return new DiscoveredMappingFile(fullPath, Path.GetFileName(fullPath), modifiedUtc, sourceLabel);
    }

    internal static string FormatSourceLabel(string sourceLabel)
    {
        return sourceLabel switch
        {
            "manual" => L.Get("GuiCore_SourceManualSelect"),
            "manual-add" => L.Get("GuiCore_SourceManualAdd"),
            "fetched" => L.Get("GuiCore_SourceFetched"),
            _ => sourceLabel,
        };
    }

    internal static string FormatUsmapDisplay(
        DiscoveredMappingFile file,
        int candidateCount = 1,
        TimeZoneInfo? displayTimeZone = null)
    {
        var formattedDate = FormatModifiedDate(file.LastWriteTimeUtc, displayTimeZone);
        if (candidateCount > 1)
        {
            return L.Format("GuiCore_UsmapDisplayMultiple", file.FileName, formattedDate, candidateCount);
        }

        return L.Format("GuiCore_UsmapDisplaySingle", file.FileName, formattedDate);
    }

    internal static string FormatJmapDisplay(
        DiscoveredMappingFile file,
        TimeZoneInfo? displayTimeZone = null)
    {
        var formattedDate = FormatModifiedDate(file.LastWriteTimeUtc, displayTimeZone);
        return L.Format("GuiCore_JmapDisplay", file.FileName, formattedDate, FormatSourceLabel(file.SourceLabel), file.FullPath);
    }

    internal static string FormatModifiedDate(
        DateTimeOffset lastWriteTimeUtc,
        TimeZoneInfo? displayTimeZone = null)
    {
        if (lastWriteTimeUtc == DateTimeOffset.MinValue)
        {
            return L.Get("Common_Unknown");
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
            return (results, L.Format("GuiCore_ScanDirCannotAccess", directoryPath, exception.Message));
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
            return (results, L.Format("GuiCore_ScanDirAccessDenied", directoryPath, exception.Message));
        }
        catch (Exception exception)
        {
            return (results, L.Format("GuiCore_ScanDirFailed", directoryPath, exception.Message));
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

    private static List<DiscoveredMappingFile> OrderNewestFirstPreferringAgp(List<DiscoveredMappingFile> items)
    {
        return items
            .OrderByDescending(file => file.LastWriteTimeUtc)
            .ThenBy(file => string.Equals(file.SourceLabel, "agp/usmap", StringComparison.Ordinal) ? 0 : 1)
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
