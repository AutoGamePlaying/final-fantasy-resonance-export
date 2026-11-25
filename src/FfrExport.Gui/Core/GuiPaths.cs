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

/// <summary>
/// Pure path calculations, CLI executable lookup, and output directory safety checks for the GUI.
/// </summary>
internal static class GuiPaths
{
    internal const string DefaultExportFolderName = "FF-Export";
    internal const string BuildMetadataFileName = "ffr.sqlite.build.json";
    internal const string CliExecutableWindows = "ffr-export.exe";
    internal const string CliExecutableUnix = "ffr-export";

    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    /// <summary>
    /// Resolves the default export root directory (%USERPROFILE%\Documents\FF-Export).
    /// </summary>
    internal static string ResolveDefaultOutputRoot(
        string? documentsDirectoryOverride = null,
        string? userProfileOverride = null)
    {
        var documentsDir = documentsDirectoryOverride;
        if (string.IsNullOrWhiteSpace(documentsDir))
        {
            documentsDir = Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments);
        }

        if (string.IsNullOrWhiteSpace(documentsDir))
        {
            var profile = userProfileOverride;
            if (string.IsNullOrWhiteSpace(profile))
            {
                profile = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
            }

            if (!string.IsNullOrWhiteSpace(profile))
            {
                documentsDir = Path.Combine(profile, "Documents");
            }
        }

        if (string.IsNullOrWhiteSpace(documentsDir))
        {
            documentsDir = Directory.GetCurrentDirectory();
        }

        return PathSupport.TrimTrailingSeparators(
            Path.GetFullPath(Path.Combine(documentsDir, DefaultExportFolderName)));
    }

    /// <summary>
    /// Computes the default output directory: &lt;outputRoot&gt;\&lt;gameCode&gt;-&lt;steamBuildId&gt;.
    /// </summary>
    internal static string BuildDefaultOutputDirectory(
        string? outputRootOverride,
        string gameCode,
        string? steamBuildId,
        string? documentsDirectoryOverride = null,
        string? userProfileOverride = null)
    {
        var baseRoot = string.IsNullOrWhiteSpace(outputRootOverride)
            ? ResolveDefaultOutputRoot(documentsDirectoryOverride, userProfileOverride)
            : PathSupport.TrimTrailingSeparators(Path.GetFullPath(outputRootOverride.Trim()));
        var normalizedCode = SanitizePathSegment(gameCode) ?? "ffr";
        var buildSegment = SanitizePathSegment(steamBuildId) ?? "unknown";
        return PathSupport.TrimTrailingSeparators(
            Path.GetFullPath(Path.Combine(baseRoot, $"{normalizedCode}-{buildSegment}")));
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
        var normalizedBaseDir = PathSupport.TrimTrailingSeparators(Path.GetFullPath(baseDir));
        var expectedPath = Path.Combine(normalizedBaseDir, fileName);

        if (File.Exists(PathSupport.ToIoPath(expectedPath)))
        {
            return new CliExecutableLookup(true, expectedPath, fileName, null);
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
