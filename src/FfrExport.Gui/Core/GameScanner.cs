using System.Text;

namespace FfrExport.Gui.Core;

internal sealed record GameEntry(
    string GameCode,
    string DisplayName,
    string GameDirectory,
    string? SteamBuildId,
    bool IsManual);

internal sealed record ManualGameValidationResult(
    bool IsValid,
    GameEntry? Entry,
    string? ErrorMessage);

internal sealed record GameScanResult(
    IReadOnlyList<GameEntry> Games,
    string? ErrorMessage);

/// <summary>
/// Scans Steam libraries via the shared SteamLocator and validates manually chosen game directories.
/// </summary>
internal static class GameScanner
{
    internal static IReadOnlyList<GameEntry> ScanSteamGames(
        bool? isWindowsOverride = null,
        Func<string?>? windowsRegistryReader = null,
        Func<string?>? homeDirectoryProvider = null,
        IEnumerable<string>? defaultWindowsCandidatesOverride = null)
    {
        var discovered = SteamLocator.DiscoverGames(
            supportedGames: null,
            isWindowsOverride: isWindowsOverride,
            windowsRegistryReader: windowsRegistryReader,
            homeDirectoryProvider: homeDirectoryProvider,
            defaultWindowsCandidatesOverride: defaultWindowsCandidatesOverride);

        return MapDiscoveredGames(discovered);
    }

    /// <summary>
    /// Fault-tolerant Steam scan for GUI startup and rescan actions.
    /// Captures registry and filesystem permission failures without throwing, so the main window always opens
    /// and displays actionable Chinese diagnostics.
    /// </summary>
    internal static GameScanResult ScanSteamGamesSafe(
        bool? isWindowsOverride = null,
        Func<string?>? windowsRegistryReader = null,
        Func<string?>? homeDirectoryProvider = null,
        IEnumerable<string>? defaultWindowsCandidatesOverride = null,
        Action<string>? steamDirectoryAccessProbe = null)
    {
        string? registryError = null;
        Func<string?>? safeRegistryReader = null;
        if (windowsRegistryReader is not null)
        {
            safeRegistryReader = () =>
            {
                try
                {
                    return windowsRegistryReader();
                }
                catch (Exception exception)
                {
                    registryError = $"{exception.GetType().Name}: {exception.Message}";
                    return null;
                }
            };
        }

        IReadOnlyList<GameEntry> games;
        try
        {
            games = ScanSteamGames(
                isWindowsOverride,
                safeRegistryReader,
                homeDirectoryProvider,
                defaultWindowsCandidatesOverride);
        }
        catch (Exception exception)
        {
            return new GameScanResult(
                [],
                L.Format("GuiCore_SteamScanFailed", exception.Message));
        }

        if (games.Count > 0)
        {
            return new GameScanResult(games, null);
        }

        var accessError = ProbeSteamRootsAccessError(
            isWindowsOverride,
            safeRegistryReader,
            homeDirectoryProvider,
            defaultWindowsCandidatesOverride,
            steamDirectoryAccessProbe);
        if (!string.IsNullOrWhiteSpace(accessError))
        {
            return new GameScanResult(
                [],
                L.Format("GuiCore_SteamLibraryUnreadable", accessError));
        }

        if (!string.IsNullOrWhiteSpace(registryError))
        {
            return new GameScanResult(
                [],
                L.Format("GuiCore_SteamRegistryFailed", registryError));
        }

        return new GameScanResult([], null);
    }

    internal static IReadOnlyList<GameEntry> ScanSteamGamesInRoots(
        IEnumerable<string> steamRoots,
        bool? isWindowsOverride = null)
    {
        var discovered = SteamLocator.DiscoverGamesInRoots(
            steamRoots,
            supportedGames: null,
            isWindowsOverride: isWindowsOverride);

        return MapDiscoveredGames(discovered);
    }

    /// <summary>
    /// Validates a manually selected directory to ensure it is a supported FINAL FANTASY RESONANCE
    /// installation and attempts to read its Steam buildid from the parent steamapps manifest.
    /// </summary>
    internal static ManualGameValidationResult ValidateManualGameDirectory(string? rawDirectory)
    {
        var cleaned = GuiPaths.CleanPathInput(rawDirectory);
        if (string.IsNullOrWhiteSpace(cleaned))
        {
            return new ManualGameValidationResult(false, null, L.Get("GuiCore_SelectGameRootPrompt"));
        }

        string fullRoot;
        try
        {
            fullRoot = PathSupport.TrimTrailingSeparators(Path.GetFullPath(cleaned));
        }
        catch (Exception exception)
        {
            return new ManualGameValidationResult(false, null, L.Format("GuiCore_GameDirInvalidPath", cleaned, exception.Message));
        }

        if (!Directory.Exists(PathSupport.ToIoPath(fullRoot)))
        {
            return new ManualGameValidationResult(false, null, L.Format("GuiCore_GameDirNotFound", fullRoot));
        }

        var paksDir = SteamLocator.CombineRelativeSubpath(fullRoot, GameVersionDetector.RelativePaksSubpath);
        if (!Directory.Exists(PathSupport.ToIoPath(paksDir)))
        {
            return new ManualGameValidationResult(
                false,
                null,
                L.Format("GuiCore_GameDirMissingPaks", GameVersionDetector.RelativePaksSubpath, paksDir));
        }

        var exePath = SteamLocator.CombineRelativeSubpath(fullRoot, GameVersionDetector.RelativeExeSubpath);
        if (!File.Exists(PathSupport.ToIoPath(exePath)))
        {
            return new ManualGameValidationResult(
                false,
                null,
                L.Format("GuiCore_GameDirMissingExe", GameVersionDetector.RelativeExeSubpath, exePath));
        }

        var (displayName, buildId) = TryReadSteamManifestInfo(fullRoot);
        var entry = new GameEntry(
            SteamLocator.ResonanceGame.Code,
            displayName ?? SteamLocator.ResonanceGame.DefaultDisplayName,
            fullRoot,
            buildId,
            IsManual: true);
        return new ManualGameValidationResult(true, entry, null);
    }

    private static IReadOnlyList<GameEntry> MapDiscoveredGames(IReadOnlyList<DiscoveredSteamGame> discovered)
    {
        var results = new List<GameEntry>(discovered.Count);
        foreach (var game in discovered)
        {
            results.Add(new GameEntry(
                game.Spec.Code,
                game.DisplayName,
                game.GameDirectory,
                game.SteamBuildId,
                IsManual: false));
        }

        return results;
    }

    private static (string? DisplayName, string? BuildId) TryReadSteamManifestInfo(string gameRoot)
    {
        try
        {
            var steamappsDir = Directory.GetParent(gameRoot)?.Parent?.FullName;
            if (string.IsNullOrWhiteSpace(steamappsDir))
            {
                return (null, null);
            }

            var manifestPath = Path.Combine(steamappsDir, GameVersionDetector.AppManifestFileName);
            var ioManifestPath = PathSupport.ToIoPath(manifestPath);
            if (!File.Exists(ioManifestPath))
            {
                return (null, null);
            }

            var content = File.ReadAllText(ioManifestPath, Encoding.UTF8);
            var fields = GameVersionDetector.ParseAppManifestVdf(content);
            fields.TryGetValue("name", out var name);
            fields.TryGetValue("buildid", out var buildId);
            return (
                string.IsNullOrWhiteSpace(name) ? null : name.Trim(),
                string.IsNullOrWhiteSpace(buildId) ? null : buildId.Trim());
        }
        catch
        {
            return (null, null);
        }
    }

    private static string? ProbeSteamRootsAccessError(
        bool? isWindowsOverride,
        Func<string?>? windowsRegistryReader,
        Func<string?>? homeDirectoryProvider,
        IEnumerable<string>? defaultWindowsCandidatesOverride,
        Action<string>? steamDirectoryAccessProbe)
    {
        IReadOnlyList<string> roots;
        try
        {
            roots = SteamLocator.FindSteamRoots(
                isWindowsOverride,
                windowsRegistryReader,
                homeDirectoryProvider,
                defaultWindowsCandidatesOverride);
        }
        catch (Exception exception)
        {
            return exception.Message;
        }

        foreach (var root in roots)
        {
            foreach (var dirName in new[] { "steamapps", "SteamApps" })
            {
                var steamappsDir = Path.Combine(root, dirName);
                try
                {
                    if (steamDirectoryAccessProbe is not null)
                    {
                        steamDirectoryAccessProbe(steamappsDir);
                        continue;
                    }

                    var ioDir = PathSupport.ToIoPath(steamappsDir);
                    if (!Directory.Exists(ioDir))
                    {
                        continue;
                    }

                    using var enumerator = Directory.EnumerateFileSystemEntries(ioDir).GetEnumerator();
                    _ = enumerator.MoveNext();

                    var vdfPath = Path.Combine(steamappsDir, "libraryfolders.vdf");
                    var ioVdfPath = PathSupport.ToIoPath(vdfPath);
                    if (File.Exists(ioVdfPath))
                    {
                        using var stream = new FileStream(ioVdfPath, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
                    }
                }
                catch (UnauthorizedAccessException exception)
                {
                    return L.Format("GuiCore_AccessDeniedPath", steamappsDir, exception.Message);
                }
                catch (Exception exception)
                {
                    return L.Format("GuiCore_AccessFailedPath", steamappsDir, exception.Message);
                }
            }
        }

        return null;
    }
}
