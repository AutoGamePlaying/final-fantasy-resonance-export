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
            return new ManualGameValidationResult(false, null, "请选择游戏安装根目录。");
        }

        string fullRoot;
        try
        {
            fullRoot = PathSupport.TrimTrailingSeparators(Path.GetFullPath(cleaned));
        }
        catch (Exception exception)
        {
            return new ManualGameValidationResult(false, null, $"游戏目录路径无效（{cleaned}）：{exception.Message}");
        }

        if (!Directory.Exists(PathSupport.ToIoPath(fullRoot)))
        {
            return new ManualGameValidationResult(false, null, $"所选游戏目录不存在：{fullRoot}");
        }

        var paksDir = SteamLocator.CombineRelativeSubpath(fullRoot, GameVersionDetector.RelativePaksSubpath);
        if (!Directory.Exists(PathSupport.ToIoPath(paksDir)))
        {
            return new ManualGameValidationResult(
                false,
                null,
                $"所选目录不是支持的游戏目录：缺少 {GameVersionDetector.RelativePaksSubpath}（检查路径：{paksDir}）。" +
                "请选择《FINAL FANTASY RESONANCE》的游戏安装根目录。");
        }

        var exePath = SteamLocator.CombineRelativeSubpath(fullRoot, GameVersionDetector.RelativeExeSubpath);
        if (!File.Exists(PathSupport.ToIoPath(exePath)))
        {
            return new ManualGameValidationResult(
                false,
                null,
                $"所选目录不是支持的游戏目录：缺少游戏可执行文件 {GameVersionDetector.RelativeExeSubpath}（检查路径：{exePath}）。" +
                "请选择《FINAL FANTASY RESONANCE》的游戏安装根目录。");
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
}
