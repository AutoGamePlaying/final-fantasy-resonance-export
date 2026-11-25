using System.Runtime.InteropServices;
using System.Text;

namespace FfrExport;

internal sealed record SupportedSteamGame(
    int AppId,
    string Code,
    string DefaultDisplayName,
    string RequiredRelativeDir);

internal sealed record DiscoveredSteamGame(
    SupportedSteamGame Spec,
    string DisplayName,
    string GameDirectory,
    string? SteamBuildId,
    string LibraryPath,
    string ManifestPath);

/// <summary>
/// Locates Steam installations and library folders across Windows and Linux, parses libraryfolders.vdf
/// and appmanifest_*.acf manifests, and discovers installed copies of supported games.
/// </summary>
internal static class SteamLocator
{
    private const string SteamRegistrySubKey = @"Software\Valve\Steam";
    private const string SteamRegistryValueName = "SteamPath";

    internal static readonly SupportedSteamGame ResonanceGame = new(
        AppId: GameVersionDetector.SteamAppId,
        Code: "ffr",
        DefaultDisplayName: "FINAL FANTASY RESONANCE",
        RequiredRelativeDir: GameVersionDetector.RelativePaksSubpath);

    internal static readonly IReadOnlyList<SupportedSteamGame> SupportedGames = [ResonanceGame];

    /// <summary>
    /// Discovers supported games across all Steam libraries on the current machine.
    /// </summary>
    internal static IReadOnlyList<DiscoveredSteamGame> DiscoverGames(
        IReadOnlyList<SupportedSteamGame>? supportedGames = null,
        bool? isWindowsOverride = null,
        Func<string?>? windowsRegistryReader = null,
        Func<string?>? homeDirectoryProvider = null,
        IEnumerable<string>? defaultWindowsCandidatesOverride = null)
    {
        var roots = FindSteamRoots(
            isWindowsOverride,
            windowsRegistryReader,
            homeDirectoryProvider,
            defaultWindowsCandidatesOverride);
        return DiscoverGamesInRoots(roots, supportedGames, isWindowsOverride);
    }

    /// <summary>
    /// Discovers supported games starting from an explicit set of Steam root directories.
    /// </summary>
    internal static IReadOnlyList<DiscoveredSteamGame> DiscoverGamesInRoots(
        IEnumerable<string> steamRoots,
        IReadOnlyList<SupportedSteamGame>? supportedGames = null,
        bool? isWindowsOverride = null)
    {
        var libraries = CollectLibraryFolders(steamRoots, isWindowsOverride);
        return DiscoverGamesInLibraries(libraries, supportedGames, isWindowsOverride);
    }

    /// <summary>
    /// Locates candidate Steam installation root directories for the target operating system.
    /// On Windows, checks HKCU\Software\Valve\Steam (SteamPath) first, then falls back to the default
    /// 32-bit Program Files Steam directory. On Linux, checks ~/.steam/steam and ~/.local/share/Steam.
    /// </summary>
    internal static IReadOnlyList<string> FindSteamRoots(
        bool? isWindowsOverride = null,
        Func<string?>? windowsRegistryReader = null,
        Func<string?>? homeDirectoryProvider = null,
        IEnumerable<string>? defaultWindowsCandidatesOverride = null)
    {
        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        var results = new List<string>();
        var seen = new HashSet<string>(StringComparer.Ordinal);

        if (isWindows)
        {
            var registryPath = (windowsRegistryReader ?? ReadWindowsRegistrySteamPath)();
            if (!TryAddExistingDirectory(registryPath, isWindows, seen, results))
            {
                var fallbacks = defaultWindowsCandidatesOverride ?? GetDefaultWindowsSteamRoots();
                foreach (var candidate in fallbacks)
                {
                    TryAddExistingDirectory(candidate, isWindows, seen, results);
                }
            }

            return results;
        }

        var homeDir = homeDirectoryProvider?.Invoke();
        if (string.IsNullOrWhiteSpace(homeDir))
        {
            homeDir = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        }

        if (string.IsNullOrWhiteSpace(homeDir))
        {
            homeDir = Environment.GetEnvironmentVariable("HOME");
        }

        if (!string.IsNullOrWhiteSpace(homeDir))
        {
            TryAddExistingDirectory(Path.Combine(homeDir, ".steam", "steam"), isWindows, seen, results);
            TryAddExistingDirectory(Path.Combine(homeDir, ".local", "share", "Steam"), isWindows, seen, results);
        }

        return results;
    }

    /// <summary>
    /// Collects all existing Steam library folders from the given Steam roots and their
    /// steamapps/libraryfolders.vdf manifests.
    /// </summary>
    internal static IReadOnlyList<string> CollectLibraryFolders(
        IEnumerable<string> steamRoots,
        bool? isWindowsOverride = null)
    {
        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        var libraries = new List<string>();
        var seen = new HashSet<string>(StringComparer.Ordinal);

        foreach (var rawRoot in steamRoots)
        {
            if (!TryAddExistingDirectory(rawRoot, isWindows, seen, libraries))
            {
                continue;
            }

            var steamappsDir = FindSteamAppsDirectory(rawRoot);
            if (steamappsDir is null)
            {
                continue;
            }

            var vdfPath = Path.Combine(steamappsDir, "libraryfolders.vdf");
            if (!File.Exists(PathSupport.ToIoPath(vdfPath)))
            {
                continue;
            }

            string vdfContent;
            try
            {
                vdfContent = File.ReadAllText(PathSupport.ToIoPath(vdfPath), Encoding.UTF8);
            }
            catch
            {
                continue;
            }

            foreach (var parsedLibrary in ParseLibraryFoldersVdf(vdfContent))
            {
                TryAddExistingDirectory(parsedLibrary, isWindows, seen, libraries);
            }
        }

        return libraries;
    }

    /// <summary>
    /// Scans Steam library folders for supported games by reading appmanifest_&lt;appid&gt;.acf and
    /// verifying that each game's required content directory exists on disk.
    /// </summary>
    internal static IReadOnlyList<DiscoveredSteamGame> DiscoverGamesInLibraries(
        IEnumerable<string> libraryFolders,
        IReadOnlyList<SupportedSteamGame>? supportedGames = null,
        bool? isWindowsOverride = null)
    {
        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        var gamesToFind = supportedGames ?? SupportedGames;
        var discovered = new List<DiscoveredSteamGame>();
        var seenGameDirs = new HashSet<string>(StringComparer.Ordinal);

        foreach (var library in libraryFolders)
        {
            var steamappsDir = FindSteamAppsDirectory(library);
            if (steamappsDir is null)
            {
                continue;
            }

            foreach (var spec in gamesToFind)
            {
                var manifestPath = Path.Combine(steamappsDir, $"appmanifest_{spec.AppId}.acf");
                if (!File.Exists(PathSupport.ToIoPath(manifestPath)))
                {
                    continue;
                }

                string content;
                try
                {
                    content = File.ReadAllText(PathSupport.ToIoPath(manifestPath), Encoding.UTF8);
                }
                catch
                {
                    continue;
                }

                var fields = GameVersionDetector.ParseAppManifestVdf(content);
                if (!fields.TryGetValue("installdir", out var installDir)
                    || string.IsNullOrWhiteSpace(installDir))
                {
                    continue;
                }

                string gameDir;
                try
                {
                    gameDir = PathSupport.TrimTrailingSeparators(
                        Path.GetFullPath(Path.Combine(steamappsDir, "common", installDir.Trim())));
                }
                catch
                {
                    continue;
                }

                if (!Directory.Exists(PathSupport.ToIoPath(gameDir)))
                {
                    continue;
                }

                var requiredDir = CombineRelativeSubpath(gameDir, spec.RequiredRelativeDir);
                if (!Directory.Exists(PathSupport.ToIoPath(requiredDir)))
                {
                    continue;
                }

                var key = NormalizeDirectoryKey(gameDir, isWindows);
                if (!seenGameDirs.Add(key))
                {
                    continue;
                }

                fields.TryGetValue("name", out var manifestName);
                fields.TryGetValue("buildid", out var buildId);
                var displayName = string.IsNullOrWhiteSpace(manifestName)
                    ? spec.DefaultDisplayName
                    : manifestName.Trim();
                var normalizedBuildId = string.IsNullOrWhiteSpace(buildId) ? null : buildId.Trim();

                discovered.Add(new DiscoveredSteamGame(
                    spec,
                    displayName,
                    gameDir,
                    normalizedBuildId,
                    PathSupport.TrimTrailingSeparators(Path.GetFullPath(library)),
                    Path.GetFullPath(manifestPath)));
            }
        }

        return discovered;
    }

    /// <summary>
    /// Parses a Steam libraryfolders.vdf document (both modern nested-object format and legacy
    /// direct index-to-path format), unescaping VDF string literals such as double backslashes.
    /// </summary>
    internal static IReadOnlyList<string> ParseLibraryFoldersVdf(string content)
    {
        var paths = new List<string>();
        var seen = new HashSet<string>(StringComparer.Ordinal);
        if (string.IsNullOrWhiteSpace(content))
        {
            return paths;
        }

        var depth = 0;
        var index = 0;
        string? pendingKey = null;
        string? depth1BlockKey = null;

        while (index < content.Length)
        {
            SkipWhitespaceAndComments(content, ref index);
            if (index >= content.Length)
            {
                break;
            }

            var ch = content[index];
            if (ch == '{')
            {
                if (depth == 1)
                {
                    depth1BlockKey = pendingKey;
                }

                depth++;
                pendingKey = null;
                index++;
                continue;
            }

            if (ch == '}')
            {
                if (depth > 0)
                {
                    depth--;
                }

                if (depth <= 1)
                {
                    depth1BlockKey = null;
                }

                pendingKey = null;
                index++;
                continue;
            }

            var token = ReadVdfToken(content, ref index);
            if (depth == 1)
            {
                if (pendingKey is null)
                {
                    pendingKey = token;
                }
                else
                {
                    // Legacy libraryfolders.vdf format: "1" "<library-path>"
                    if (IsAsciiDigits(pendingKey) && !string.IsNullOrWhiteSpace(token))
                    {
                        var trimmed = token.Trim();
                        if (seen.Add(trimmed))
                        {
                            paths.Add(trimmed);
                        }
                    }

                    pendingKey = null;
                }
            }
            else if (depth == 2)
            {
                if (pendingKey is null)
                {
                    pendingKey = token;
                }
                else
                {
                    // Modern libraryfolders.vdf format: "0" { "path" "<library-path>" ... }
                    if (IsAsciiDigits(depth1BlockKey)
                        && string.Equals(pendingKey, "path", StringComparison.OrdinalIgnoreCase)
                        && !string.IsNullOrWhiteSpace(token))
                    {
                        var trimmed = token.Trim();
                        if (seen.Add(trimmed))
                        {
                            paths.Add(trimmed);
                        }
                    }

                    pendingKey = null;
                }
            }
            else
            {
                pendingKey = null;
            }
        }

        return paths;
    }

    internal static string CombineRelativeSubpath(string rootDir, string slashRelativePath)
    {
        var segments = slashRelativePath.Split(['/', '\\'], StringSplitOptions.RemoveEmptyEntries);
        var current = rootDir;
        foreach (var segment in segments)
        {
            current = Path.Combine(current, segment);
        }

        return current;
    }

    private static string? ReadWindowsRegistrySteamPath()
    {
        return WindowsRegistry.TryReadCurrentUserString(SteamRegistrySubKey, SteamRegistryValueName);
    }

    private static IEnumerable<string> GetDefaultWindowsSteamRoots()
    {
        var programFilesX86 = Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86);
        if (!string.IsNullOrWhiteSpace(programFilesX86))
        {
            yield return Path.Combine(programFilesX86, "Steam");
        }

        var envProgramFilesX86 = Environment.GetEnvironmentVariable("ProgramFiles(x86)");
        if (!string.IsNullOrWhiteSpace(envProgramFilesX86))
        {
            yield return Path.Combine(envProgramFilesX86, "Steam");
        }

        // Standard default 32-bit Program Files location on drive C.
        var driveC = string.Concat("C:", Path.DirectorySeparatorChar);
        yield return Path.Combine(driveC, "Program Files (x86)", "Steam");
    }

    private static string? FindSteamAppsDirectory(string libraryRoot)
    {
        foreach (var dirName in new[] { "steamapps", "SteamApps" })
        {
            var candidate = Path.Combine(libraryRoot, dirName);
            if (Directory.Exists(PathSupport.ToIoPath(candidate)))
            {
                return candidate;
            }
        }

        return null;
    }

    private static bool TryAddExistingDirectory(
        string? rawPath,
        bool isWindows,
        HashSet<string> seen,
        List<string> results)
    {
        if (string.IsNullOrWhiteSpace(rawPath))
        {
            return false;
        }

        string fullPath;
        try
        {
            fullPath = PathSupport.TrimTrailingSeparators(Path.GetFullPath(rawPath.Trim()));
        }
        catch
        {
            return false;
        }

        if (!Directory.Exists(PathSupport.ToIoPath(fullPath)))
        {
            return false;
        }

        var key = NormalizeDirectoryKey(fullPath, isWindows);
        if (seen.Add(key))
        {
            results.Add(fullPath);
        }

        return true;
    }

    private static string NormalizeDirectoryKey(string fullPath, bool isWindows)
    {
        var normalized = PathSupport.TrimTrailingSeparators(Path.GetFullPath(fullPath));
        try
        {
            var resolved = Directory.ResolveLinkTarget(PathSupport.ToIoPath(normalized), returnFinalTarget: true);
            if (resolved is not null)
            {
                normalized = PathSupport.TrimTrailingSeparators(Path.GetFullPath(resolved.FullName));
            }
        }
        catch
        {
            // Keep the normalized full path if symlink resolution is unavailable.
        }

        return isWindows ? normalized.ToUpperInvariant() : normalized;
    }

    private static bool IsAsciiDigits(string? value)
    {
        if (string.IsNullOrEmpty(value))
        {
            return false;
        }

        foreach (var ch in value)
        {
            if (ch < '0' || ch > '9')
            {
                return false;
            }
        }

        return true;
    }

    private static void SkipWhitespaceAndComments(string content, ref int index)
    {
        while (index < content.Length)
        {
            var ch = content[index];
            if (char.IsWhiteSpace(ch))
            {
                index++;
                continue;
            }

            if (ch == '/' && index + 1 < content.Length && content[index + 1] == '/')
            {
                index += 2;
                while (index < content.Length && content[index] != '\n')
                {
                    index++;
                }

                continue;
            }

            break;
        }
    }

    private static string ReadVdfToken(string content, ref int index)
    {
        if (content[index] == '"')
        {
            index++;
            var sb = new StringBuilder();
            while (index < content.Length)
            {
                var ch = content[index++];
                if (ch == '"')
                {
                    break;
                }

                if (ch == '\\' && index < content.Length)
                {
                    var escaped = content[index++];
                    switch (escaped)
                    {
                        case 'n':
                            sb.Append('\n');
                            break;
                        case 'r':
                            sb.Append('\r');
                            break;
                        case 't':
                            sb.Append('\t');
                            break;
                        case '"':
                            sb.Append('"');
                            break;
                        case '\\':
                            sb.Append('\\');
                            break;
                        default:
                            sb.Append('\\');
                            sb.Append(escaped);
                            break;
                    }
                }
                else
                {
                    sb.Append(ch);
                }
            }

            return sb.ToString();
        }

        var start = index;
        while (index < content.Length
            && !char.IsWhiteSpace(content[index])
            && content[index] != '{'
            && content[index] != '}'
            && content[index] != '"')
        {
            index++;
        }

        return content[start..index];
    }

    private static class WindowsRegistry
    {
        private static readonly IntPtr HkeyCurrentUser = new(unchecked((int)0x80000001));
        private const uint KeyQueryValue = 0x0001;
        private const uint RegSz = 1;
        private const uint RegExpandSz = 2;
        private const int ErrorSuccess = 0;

        [DllImport("advapi32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
        private static extern int RegOpenKeyExW(
            IntPtr hKey,
            string lpSubKey,
            uint ulOptions,
            uint samDesired,
            out IntPtr phkResult);

        [DllImport("advapi32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
        private static extern int RegQueryValueExW(
            IntPtr hKey,
            string lpValueName,
            IntPtr lpReserved,
            out uint lpType,
            byte[]? lpData,
            ref uint lpcbData);

        [DllImport("advapi32.dll", ExactSpelling = true)]
        private static extern int RegCloseKey(IntPtr hKey);

        internal static string? TryReadCurrentUserString(string subKey, string valueName)
        {
            if (!OperatingSystem.IsWindows())
            {
                return null;
            }

            try
            {
                if (RegOpenKeyExW(HkeyCurrentUser, subKey, 0, KeyQueryValue, out var keyHandle) != ErrorSuccess
                    || keyHandle == IntPtr.Zero)
                {
                    return null;
                }

                try
                {
                    uint dataSize = 0;
                    var probeResult = RegQueryValueExW(
                        keyHandle,
                        valueName,
                        IntPtr.Zero,
                        out var valueType,
                        null,
                        ref dataSize);

                    if (probeResult != ErrorSuccess
                        || dataSize == 0
                        || (valueType != RegSz && valueType != RegExpandSz)
                        || dataSize > 65536)
                    {
                        return null;
                    }

                    var buffer = new byte[dataSize];
                    var readResult = RegQueryValueExW(
                        keyHandle,
                        valueName,
                        IntPtr.Zero,
                        out valueType,
                        buffer,
                        ref dataSize);

                    if (readResult != ErrorSuccess || dataSize == 0)
                    {
                        return null;
                    }

                    var byteCount = (int)(dataSize & ~1u);
                    var text = Encoding.Unicode.GetString(buffer, 0, byteCount).TrimEnd('\0');
                    if (valueType == RegExpandSz)
                    {
                        text = Environment.ExpandEnvironmentVariables(text);
                    }

                    return string.IsNullOrWhiteSpace(text) ? null : text.Trim();
                }
                finally
                {
                    _ = RegCloseKey(keyHandle);
                }
            }
            catch
            {
                return null;
            }
        }
    }
}
