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
/// Uses the unified AgpPaths writable root resolution without ever touching %TEMP%.
/// </summary>
internal static class GuiPaths
{
    internal const string BuildMetadataFileName = "ffr.sqlite.build.json";
    internal const string CliExecutableWindows = "ffr-export.exe";
    internal const string CliExecutableUnix = "ffr-export";

    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    /// <summary>
    /// Resolves the default export root directory (&lt;agpRoot&gt;\ffr-export\output).
    /// </summary>
    internal static string ResolveDefaultOutputRoot(
        string? agpHomeOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        if (!string.IsNullOrWhiteSpace(agpHomeOverride)
            && AgpPaths.TryNormalizePath(agpHomeOverride, out var normalizedAgp))
        {
            return AgpPaths.GetDefaultOutputRootForAgpRoot(normalizedAgp);
        }

        return AgpPaths.GetDefaultOutputRoot(
            agpHomeEnvOverride: agpHomeOverride,
            isWindowsOverride: isWindowsOverride,
            localAppDataOverride: localAppDataOverride,
            xdgDataHomeOverride: xdgDataHomeOverride,
            userProfileOverride: userProfileOverride);
    }

    /// <summary>
    /// Resolves the default export root directory safely through the AgpPaths writable candidate chain
    /// (AGP_HOME -> LocalAppData -> LocalLow -> exe\agp-data -> UserProfile -> agp-location.txt),
    /// never falling back to %TEMP%.
    /// </summary>
    internal static OutputRootResolution ResolveDefaultOutputRootSafe(
        string? agpHomeOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? localAppDataLowOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null,
        string? executableDirectoryOverride = null,
        Func<string, AgpWriteProbeResult>? writeProbeOverride = null,
        ProcessIntegrityInfo? integrityOverride = null)
    {
        var resolution = AgpPaths.ResolveWritableAgpRoot(
            agpHomeEnvOverride: agpHomeOverride,
            isWindowsOverride: isWindowsOverride,
            localAppDataOverride: localAppDataOverride,
            localAppDataLowOverride: localAppDataLowOverride,
            xdgDataHomeOverride: xdgDataHomeOverride,
            userProfileOverride: userProfileOverride,
            executableDirectoryOverride: executableDirectoryOverride,
            writeProbeOverride: writeProbeOverride,
            integrityOverride: integrityOverride);

        if (resolution.Success && !string.IsNullOrWhiteSpace(resolution.AgpRoot))
        {
            var outputRoot = AgpPaths.GetDefaultOutputRootForAgpRoot(resolution.AgpRoot);
            var warning = resolution.UsedFallback ? AgpPaths.FormatGuiBannerMessage(resolution) : null;
            return new OutputRootResolution(outputRoot, resolution.UsedFallback, warning);
        }

        return new OutputRootResolution(
            AgpPaths.OutputSubfolderName,
            UsedFallback: true,
            WarningMessage: resolution.ErrorMessage
                ?? L.Get("GuiCore_DefaultOutputAllUnwritable"));
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
            errorMessage = L.Get("GuiCore_PathEmpty");
            return false;
        }

        if (cleaned.IndexOf('\0') >= 0)
        {
            errorMessage = L.Get("GuiCore_PathContainsNull");
            return false;
        }

        try
        {
            var full = PathSupport.TrimTrailingSeparators(Path.GetFullPath(cleaned));
            if (string.IsNullOrWhiteSpace(full))
            {
                errorMessage = L.Get("Common_InvalidPath");
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
        Func<string, AgpWriteProbeResult>? writeProbeOverride = null)
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
                    writeProbeOverride: writeProbeOverride);
                baseRoot = fallback.Path;
                usedFallback = true;
                warning = L.Format("GuiCore_SavedOutputInvalidFallback", outputRootOverride, overrideError, baseRoot);
            }
        }
        else
        {
            var resolved = ResolveDefaultOutputRootSafe(
                agpHomeOverride: agpHomeOverride,
                writeProbeOverride: writeProbeOverride);
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
                L.Format("GuiCore_CliLookupInvalidBaseDir", fileName, baseDir, exception.Message));
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
                L.Format("GuiCore_CliLookupCheckFailed", fileName, expectedPath, exception.Message));
        }

        var dbFileName = isWindows ? "ffr-db.exe" : "ffr-db";
        var errorMessage = L.Format("GuiCore_CliNotFound", fileName, expectedPath, dbFileName);
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
                L.Get("GuiCore_OutputEmpty"));
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
                L.Format("GuiCore_OutputInvalidPath", cleaned, exception.Message));
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
                        L.Format("GuiCore_OutputOverlapsGame", fullOutputPath));
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
                L.Format("GuiCore_OutputIsFile", fullOutputPath));
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
                L.Format("GuiCore_OutputReadFailed", fullOutputPath, exception.Message));
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
                ? L.Format("GuiCore_OutputProtectedRefused", fullOutputPath)
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
            throw new InvalidOperationException(L.Format("GuiCore_RefuseClearProtected", fullPath));
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

    private static bool PathsEqual(string left, string right)
    {
        var a = PathSupport.TrimTrailingSeparators(Path.GetFullPath(left));
        var b = PathSupport.TrimTrailingSeparators(Path.GetFullPath(right));
        var comparison = OperatingSystem.IsWindows()
            ? StringComparison.OrdinalIgnoreCase
            : StringComparison.Ordinal;
        return string.Equals(a, b, comparison);
    }
}
