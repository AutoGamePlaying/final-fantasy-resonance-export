using System.Diagnostics;
using System.Globalization;
using System.Text;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace FfrExport;

internal delegate Task<int> BuildDbRunner(string[] buildDbArgs);

/// <summary>
/// Interactive Chinese console wizard launched when ffr-export is started without arguments
/// (for example, by double-clicking ffr-export.exe in Windows Explorer).
/// Delegates the actual database build to the exact same build-db entry point.
/// </summary>
internal static class InteractiveWizard
{
    private const string BuildMetadataFileName = "ffr.sqlite.build.json";
    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    /// <summary>
    /// Trims surrounding whitespace and matched single or double quotes from user-pasted paths
    /// (such as Windows Explorer's "Copy as path" output).
    /// </summary>
    internal static string CleanUserPathInput(string? rawInput)
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

    /// <summary>
    /// Computes the default output directory under the unified agp layout:
    /// %LOCALAPPDATA%\agp\ffr-export\output\&lt;gameCode&gt;-&lt;steamBuildId&gt;.
    /// </summary>
    internal static string BuildDefaultOutputDirectory(
        string gameCode,
        string? steamBuildId,
        string? agpHomeOverride = null)
    {
        return AgpPaths.GetDefaultOutputDirectory(
            gameCode,
            steamBuildId,
            agpHomeEnvOverride: agpHomeOverride);
    }

    /// <summary>
    /// Auto-discovers the newest .usmap file across %LOCALAPPDATA%\agp\ffr-export\usmap\,
    /// FFRS\Binaries\Win64\ue4ss\, and FFRS\Binaries\Win64\.
    /// </summary>
    internal static (string FullPath, string SourceLabel)? TryDiscoverLatestUsmap(
        string gameDirectory,
        string? agpHomeOverride = null)
    {
        var agpUsmapDir = AgpPaths.GetUsmapDirectory(agpHomeEnvOverride: agpHomeOverride);
        var ue4ssDir = Ue4ssRuntimeSupport.GetUe4ssDirectory(gameDirectory);
        var win64Dir = Ue4ssRuntimeSupport.GetWin64Directory(gameDirectory);

        var primaryCandidates = new List<(string FullPath, DateTime LastWriteUtc, int Priority, string SourceLabel)>();
        CollectUsmapCandidates(agpUsmapDir, priority: 0, "agp/usmap", primaryCandidates);
        CollectUsmapCandidates(ue4ssDir, priority: 1, "ue4ss", primaryCandidates);
        if (primaryCandidates.Count > 0)
        {
            var chosen = primaryCandidates
                .OrderByDescending(item => item.LastWriteUtc)
                .ThenBy(item => item.Priority)
                .ThenByDescending(item => Path.GetFileName(item.FullPath), StringComparer.OrdinalIgnoreCase)
                .First();
            return (chosen.FullPath, chosen.SourceLabel);
        }

        var fallbackCandidates = new List<(string FullPath, DateTime LastWriteUtc, int Priority, string SourceLabel)>();
        CollectUsmapCandidates(win64Dir, priority: 2, "Win64", fallbackCandidates);
        if (fallbackCandidates.Count > 0)
        {
            var chosen = fallbackCandidates
                .OrderByDescending(item => item.LastWriteUtc)
                .ThenByDescending(item => Path.GetFileName(item.FullPath), StringComparer.OrdinalIgnoreCase)
                .First();
            return (chosen.FullPath, chosen.SourceLabel);
        }

        return null;
    }

    internal static async Task<int> RunAsync(
        TextReader input,
        TextWriter output,
        TextWriter error,
        BuildDbRunner buildDbRunner,
        Func<IReadOnlyList<DiscoveredSteamGame>> discoverGames,
        Func<string> resolveFfrDb,
        string executableDirectory,
        bool? isWindowsOverride = null,
        string? agpHomeOverride = null)
    {
        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        var exeDir = string.IsNullOrWhiteSpace(executableDirectory)
            ? Directory.GetCurrentDirectory()
            : PathSupport.TrimTrailingSeparators(Path.GetFullPath(executableDirectory));

        var effectiveAgpHome = agpHomeOverride;
        if (string.IsNullOrWhiteSpace(effectiveAgpHome))
        {
            var agpResolution = AgpPaths.ResolveWritableAgpRootOrThrow(
                noticeOutput: output,
                isWindowsOverride: isWindows,
                executableDirectoryOverride: exeDir);
            effectiveAgpHome = agpResolution.AgpRoot;
        }

        output.WriteLine("============================================================");
        output.WriteLine(L.Get("Wizard_Title"));
        output.WriteLine("============================================================");
        output.WriteLine(L.Get("Wizard_ReadOnlyNotice"));
        output.WriteLine(L.Get("Wizard_QuoteTip"));

        // Pre-check ffr-db binary so double-click users get an immediate localized message if it is missing.
        string resolvedFfrDb;
        try
        {
            resolvedFfrDb = resolveFfrDb();
        }
        catch (Exception exception)
        {
            output.WriteLine();
            PrintMissingFfrDbError(error, exception, exeDir, isWindows);
            WaitForExit(input, output);
            return 2;
        }

        // Step 1: Locate Steam games or prompt for manual game directory.
        output.WriteLine();
        output.WriteLine(L.Get("Wizard_Step1Header"));
        var gameSelection = SelectGameDirectory(input, output, discoverGames);
        if (gameSelection is null)
        {
            output.WriteLine(L.Get("Wizard_InputClosedCancelled"));
            return 2;
        }

        // Step 2: Auto-discover or prompt for .usmap (required) and .jmap (optional, multiple).
        output.WriteLine();
        output.WriteLine(L.Get("Wizard_Step2Header"));
        output.WriteLine(L.Get("Wizard_UsmapNote"));
        var discoveredUsmap = TryDiscoverLatestUsmap(gameSelection.GameDirectory, effectiveAgpHome);
        var usmapPath = PromptUsmapFile(input, output, discoveredUsmap);
        if (usmapPath is null)
        {
            output.WriteLine(L.Get("Wizard_InputClosedCancelled"));
            return 2;
        }

        output.WriteLine();
        output.WriteLine(L.Get("Wizard_JmapNote"));
        var jmapPaths = PromptOptionalFiles(input, output, isWindows);
        if (jmapPaths is null)
        {
            output.WriteLine(L.Get("Wizard_InputClosedCancelled"));
            return 2;
        }

        // Step 3: Output directory (defaults to <agp>/ffr-export/output/<gameCode>-<buildId>).
        output.WriteLine();
        output.WriteLine(L.Get("Wizard_Step3Header"));
        var defaultOutputDir = BuildDefaultOutputDirectory(
            gameSelection.GameCode,
            gameSelection.SteamBuildId,
            effectiveAgpHome);
        var outputSelection = PromptOutputDirectory(input, output, gameSelection.GameDirectory, defaultOutputDir);
        if (outputSelection is null)
        {
            output.WriteLine(L.Get("Wizard_InputClosedCancelled"));
            return 2;
        }

        // Step 4: Summary, confirmation, and invoke the exact build-db entry point.
        output.WriteLine();
        output.WriteLine(L.Get("Wizard_Step4Header"));
        output.WriteLine(L.Format("Wizard_SummaryGameName", gameSelection.DisplayName));
        output.WriteLine(L.Format("Wizard_SummaryGameDir", gameSelection.GameDirectory));
        output.WriteLine(L.Format("Wizard_SummarySteamBuild", gameSelection.SteamBuildId ?? L.Get("Common_NotDetected")));
        output.WriteLine(L.Format("Wizard_SummaryUsmap", usmapPath));
        if (jmapPaths.Count == 0)
        {
            output.WriteLine(L.Get("Wizard_SummaryJmapNone"));
        }
        else
        {
            output.WriteLine(L.Format("Wizard_SummaryJmapCount", jmapPaths.Count));
            for (var i = 0; i < jmapPaths.Count; i++)
            {
                output.WriteLine($"    [{i + 1}] {jmapPaths[i]}");
            }
        }

        output.WriteLine(outputSelection.Force
            ? L.Format("Wizard_SummaryOutputForce", outputSelection.OutputDirectory)
            : L.Format("Wizard_SummaryOutput", outputSelection.OutputDirectory));

        var confirmed = PromptConfirmation(input, output);
        if (confirmed is null)
        {
            output.WriteLine(L.Get("Wizard_InputClosedCancelled"));
            return 2;
        }

        if (!confirmed.Value)
        {
            output.WriteLine(L.Get("Wizard_CancelledNoChanges"));
            WaitForExit(input, output);
            return 0;
        }

        var buildDbArgs = BuildCliArguments(
            gameSelection.GameDirectory,
            usmapPath,
            jmapPaths,
            outputSelection.OutputDirectory,
            gameSelection.SteamBuildId,
            resolvedFfrDb,
            outputSelection.Force);

        output.WriteLine();
        output.WriteLine(L.Get("Wizard_StartingBuild"));
        output.Flush();

        var stopwatch = Stopwatch.StartNew();
        try
        {
            var exitCode = await buildDbRunner(buildDbArgs);
            stopwatch.Stop();

            if (exitCode is 0 or 1)
            {
                PrintCompletionSummary(output, outputSelection.OutputDirectory, exitCode, stopwatch.Elapsed);
            }
            else
            {
                error.WriteLine();
                error.WriteLine(L.Format("Wizard_BuildFailedExitCode", exitCode));
                PrintFailureAdvice(error, null, exeDir, isWindows);
            }

            WaitForExit(input, output);
            return exitCode;
        }
        catch (Exception exception)
        {
            stopwatch.Stop();
            error.WriteLine();
            error.WriteLine(L.Format("Wizard_BuildFailedException", exception.Message));
            PrintFailureAdvice(error, exception, exeDir, isWindows);
            WaitForExit(input, output);
            return 2;
        }
    }

    internal static string[] BuildCliArguments(
        string gameDirectory,
        string usmapPath,
        IReadOnlyList<string> jmapPaths,
        string outputDirectory,
        string? steamBuildId,
        string? resolvedFfrDbPath,
        bool force)
    {
        var args = new List<string>
        {
            "--game",
            gameDirectory,
            "--usmap",
            usmapPath,
        };

        foreach (var jmap in jmapPaths)
        {
            args.Add("--jmap");
            args.Add(jmap);
        }

        args.Add("--out");
        args.Add(outputDirectory);

        if (!string.IsNullOrWhiteSpace(steamBuildId))
        {
            args.Add("--steam-buildid");
            args.Add(steamBuildId.Trim());
        }

        if (!string.IsNullOrWhiteSpace(resolvedFfrDbPath))
        {
            args.Add("--ffr-db");
            args.Add(resolvedFfrDbPath);
        }

        if (force)
        {
            args.Add("--force");
        }

        return [.. args];
    }

    private sealed record GameSelection(
        string GameCode,
        string DisplayName,
        string GameDirectory,
        string? SteamBuildId);

    private sealed record OutputSelection(
        string OutputDirectory,
        bool Force);

    private static GameSelection? SelectGameDirectory(
        TextReader input,
        TextWriter output,
        Func<IReadOnlyList<DiscoveredSteamGame>> discoverGames)
    {
        IReadOnlyList<DiscoveredSteamGame> discovered;
        try
        {
            discovered = discoverGames();
        }
        catch (Exception exception)
        {
            output.WriteLine(L.Format("Wizard_SteamScanError", exception.Message));
            discovered = [];
        }

        if (discovered.Count > 0)
        {
            output.WriteLine(L.Get("Wizard_SteamGamesFound"));
            for (var i = 0; i < discovered.Count; i++)
            {
                var game = discovered[i];
                output.WriteLine($"  [{i + 1}] {game.DisplayName}");
                output.WriteLine(L.Format("Wizard_SteamGameDirLine", game.GameDirectory));
                output.WriteLine(L.Format("Wizard_SteamGameBuildLine", game.SteamBuildId ?? L.Get("Common_Unknown")));
            }

            output.WriteLine(L.Get("Wizard_ManualGameOption"));

            while (true)
            {
                output.Write(L.Format("Wizard_SelectGamePrompt", discovered.Count));
                output.Flush();
                var raw = input.ReadLine();
                if (raw is null)
                {
                    return null;
                }

                var trimmed = raw.Trim();
                if (string.IsNullOrEmpty(trimmed))
                {
                    var chosen = discovered[0];
                    return new GameSelection(chosen.Spec.Code, chosen.DisplayName, chosen.GameDirectory, chosen.SteamBuildId);
                }

                if (int.TryParse(trimmed, NumberStyles.Integer, CultureInfo.InvariantCulture, out var number))
                {
                    if (number == 0)
                    {
                        break;
                    }

                    if (number >= 1 && number <= discovered.Count)
                    {
                        var chosen = discovered[number - 1];
                        return new GameSelection(chosen.Spec.Code, chosen.DisplayName, chosen.GameDirectory, chosen.SteamBuildId);
                    }
                }

                output.WriteLine(L.Format("Wizard_InvalidNumberRange", discovered.Count));
            }
        }
        else
        {
            output.WriteLine(L.Format(
                "Wizard_SteamGameNotFound",
                SteamLocator.ResonanceGame.DefaultDisplayName,
                SteamLocator.ResonanceGame.AppId));
        }

        while (true)
        {
            output.Write(L.Format("Wizard_PasteGameDirPrompt", GameVersionDetector.RelativePaksSubpath));
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var cleaned = CleanUserPathInput(raw);
            if (string.IsNullOrEmpty(cleaned))
            {
                output.WriteLine(L.Get("Wizard_GameDirEmpty"));
                continue;
            }

            if (!TryNormalizeFullPath(cleaned, out var fullDir, out var pathError))
            {
                output.WriteLine(L.Format("Wizard_InvalidPathRetry", pathError));
                continue;
            }

            if (!Directory.Exists(PathSupport.ToIoPath(fullDir)))
            {
                output.WriteLine(L.Format("Wizard_DirNotFoundRetry", cleaned));
                continue;
            }

            var paksDir = SteamLocator.CombineRelativeSubpath(fullDir, GameVersionDetector.RelativePaksSubpath);
            if (!Directory.Exists(PathSupport.ToIoPath(paksDir)))
            {
                output.WriteLine(L.Format("Wizard_GameDirMissingPaksRetry", GameVersionDetector.RelativePaksSubpath, fullDir));
                continue;
            }

            var steamInfo = GameVersionDetector.ReadSteamBuildInfo(fullDir, null, TextWriter.Null);
            var displayName = string.IsNullOrWhiteSpace(steamInfo.Name)
                ? SteamLocator.ResonanceGame.DefaultDisplayName
                : steamInfo.Name;
            return new GameSelection(
                SteamLocator.ResonanceGame.Code,
                displayName,
                fullDir,
                steamInfo.BuildId);
        }
    }

    private static void CollectUsmapCandidates(
        string directoryPath,
        int priority,
        string sourceLabel,
        List<(string FullPath, DateTime LastWriteUtc, int Priority, string SourceLabel)> target)
    {
        try
        {
            var ioDir = PathSupport.ToIoPath(directoryPath);
            if (!Directory.Exists(ioDir))
            {
                return;
            }

            foreach (var rawFile in Directory.EnumerateFiles(ioDir, "*", SearchOption.TopDirectoryOnly))
            {
                if (!string.Equals(Path.GetExtension(rawFile), ".usmap", StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                var fullPath = Path.GetFullPath(rawFile);
                var writeUtc = DateTime.MinValue;
                try
                {
                    writeUtc = File.GetLastWriteTimeUtc(PathSupport.ToIoPath(fullPath));
                }
                catch
                {
                    // Treat unreadable mtime as oldest.
                }

                target.Add((fullPath, writeUtc, priority, sourceLabel));
            }
        }
        catch
        {
            // Ignore unreadable directories during best-effort wizard auto-discovery.
        }
    }

    private static string? PromptUsmapFile(
        TextReader input,
        TextWriter output,
        (string FullPath, string SourceLabel)? discoveredUsmap)
    {
        if (discoveredUsmap is null)
        {
            return PromptRequiredFile(input, output, L.Get("Wizard_UsmapLabel"), ".usmap");
        }

        output.WriteLine(L.Format("Wizard_UsmapAutoFound", discoveredUsmap.Value.SourceLabel, discoveredUsmap.Value.FullPath));
        while (true)
        {
            output.Write(L.Get("Wizard_UsmapAutoPrompt"));
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var cleaned = CleanUserPathInput(raw);
            if (string.IsNullOrEmpty(cleaned))
            {
                return discoveredUsmap.Value.FullPath;
            }

            if (!TryNormalizeFullPath(cleaned, out var fullPath, out var pathError))
            {
                output.WriteLine(L.Format("Wizard_InvalidPathRetry", pathError));
                continue;
            }

            var ioPath = PathSupport.ToIoPath(fullPath);
            if (Directory.Exists(ioPath))
            {
                output.WriteLine(L.Format("Wizard_PathIsDirNotFile", cleaned, ".usmap"));
                continue;
            }

            if (!File.Exists(ioPath))
            {
                output.WriteLine(L.Format("Wizard_FileNotFoundRetry", cleaned));
                continue;
            }

            return fullPath;
        }
    }

    private static string? PromptRequiredFile(
        TextReader input,
        TextWriter output,
        string promptLabel,
        string expectedExtension)
    {
        while (true)
        {
            output.Write(L.Format("Wizard_PasteRequiredFilePrompt", promptLabel));
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var cleaned = CleanUserPathInput(raw);
            if (string.IsNullOrEmpty(cleaned))
            {
                output.WriteLine(L.Format("Wizard_RequiredFieldEmpty", promptLabel));
                continue;
            }

            if (!TryNormalizeFullPath(cleaned, out var fullPath, out var pathError))
            {
                output.WriteLine(L.Format("Wizard_InvalidPathRetry", pathError));
                continue;
            }

            var ioPath = PathSupport.ToIoPath(fullPath);
            if (Directory.Exists(ioPath))
            {
                output.WriteLine(L.Format("Wizard_PathIsDirNotFile", cleaned, expectedExtension));
                continue;
            }

            if (!File.Exists(ioPath))
            {
                output.WriteLine(L.Format("Wizard_FileNotFoundRetry", cleaned));
                continue;
            }

            return fullPath;
        }
    }

    private static List<string>? PromptOptionalFiles(
        TextReader input,
        TextWriter output,
        bool isWindows)
    {
        var files = new List<string>();
        var comparer = isWindows ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal;
        var seen = new HashSet<string>(comparer);

        while (true)
        {
            var prompt = files.Count == 0
                ? L.Get("Wizard_JmapFirstPrompt")
                : L.Format("Wizard_JmapNextPrompt", files.Count + 1);
            output.Write(prompt);
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var cleaned = CleanUserPathInput(raw);
            if (string.IsNullOrEmpty(cleaned))
            {
                if (files.Count == 0)
                {
                    output.WriteLine(L.Get("Wizard_JmapSkipped"));
                }
                else
                {
                    output.WriteLine(L.Format("Wizard_JmapAddedCount", files.Count));
                }

                return files;
            }

            if (!TryNormalizeFullPath(cleaned, out var fullPath, out var pathError))
            {
                output.WriteLine(L.Format("Wizard_InvalidPathOrSkip", pathError));
                continue;
            }

            var ioPath = PathSupport.ToIoPath(fullPath);
            if (Directory.Exists(ioPath))
            {
                output.WriteLine(L.Format("Wizard_JmapIsDirOrSkip", cleaned));
                continue;
            }

            if (!File.Exists(ioPath))
            {
                output.WriteLine(L.Format("Wizard_FileNotFoundOrSkip", cleaned));
                continue;
            }

            if (!seen.Add(fullPath))
            {
                output.WriteLine(L.Format("Wizard_JmapDuplicate", fullPath));
                continue;
            }

            files.Add(fullPath);
            output.WriteLine(L.Format("Wizard_JmapAddedItem", fullPath));
        }
    }

    private static OutputSelection? PromptOutputDirectory(
        TextReader input,
        TextWriter output,
        string gameDirectory,
        string defaultOutputDir)
    {
        output.WriteLine(L.Format("Wizard_DefaultOutputDir", defaultOutputDir));

        while (true)
        {
            output.Write(L.Get("Wizard_OutputDirPrompt"));
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var cleaned = CleanUserPathInput(raw);
            string candidateDir;
            if (string.IsNullOrEmpty(cleaned))
            {
                candidateDir = defaultOutputDir;
            }
            else if (!TryNormalizeFullPath(cleaned, out candidateDir, out var pathError))
            {
                output.WriteLine(L.Format("Wizard_InvalidPathRetry", pathError));
                continue;
            }

            if (PathSupport.IsSameOrDescendantPath(gameDirectory, candidateDir)
                || PathSupport.IsSameOrDescendantPath(candidateDir, gameDirectory))
            {
                output.WriteLine(L.Format("Wizard_OutputOverlapsGame", candidateDir));
                continue;
            }

            var ioCandidate = PathSupport.ToIoPath(candidateDir);
            if (File.Exists(ioCandidate))
            {
                output.WriteLine(L.Format("Wizard_OutputIsExistingFile", candidateDir));
                continue;
            }

            if (Directory.Exists(ioCandidate) && Directory.EnumerateFileSystemEntries(ioCandidate).Any())
            {
                if (!HasOwnBuildDbMetadata(ioCandidate))
                {
                    output.WriteLine(L.Format("Wizard_OutputForeignNonEmpty", candidateDir));
                    continue;
                }

                output.WriteLine(L.Format("Wizard_OutputPreviousBuildExists", candidateDir));
                output.Write(L.Get("Wizard_OutputOverwritePrompt"));
                output.Flush();
                var overwriteRaw = input.ReadLine();
                if (overwriteRaw is null)
                {
                    return null;
                }

                var overwriteTrimmed = overwriteRaw.Trim();
                if (IsYes(overwriteTrimmed))
                {
                    return new OutputSelection(candidateDir, Force: true);
                }

                continue;
            }

            return new OutputSelection(candidateDir, Force: false);
        }
    }

    private static bool? PromptConfirmation(TextReader input, TextWriter output)
    {
        while (true)
        {
            output.Write(L.Get("Wizard_ConfirmStartPrompt"));
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var trimmed = raw.Trim();
            if (string.IsNullOrEmpty(trimmed) || IsYes(trimmed))
            {
                return true;
            }

            if (IsNo(trimmed))
            {
                return false;
            }

            output.WriteLine(L.Get("Wizard_ConfirmStartInvalid"));
        }
    }

    private static void PrintCompletionSummary(
        TextWriter output,
        string outputDirectory,
        int exitCode,
        TimeSpan wallElapsed)
    {
        var sqlitePath = Path.Combine(outputDirectory, "ffr.sqlite");
        var metadataPath = Path.Combine(outputDirectory, BuildMetadataFileName);

        output.WriteLine();
        output.WriteLine("============================================================");
        output.WriteLine(L.Get("Wizard_CompletionTitle"));
        output.WriteLine("============================================================");
        output.WriteLine(L.Format("Wizard_CompletionDbPath", sqlitePath));

        var ioMetaPath = PathSupport.ToIoPath(metadataPath);
        if (File.Exists(ioMetaPath))
        {
            try
            {
                var report = JObject.Parse(File.ReadAllText(ioMetaPath, Utf8WithoutBom));
                var db = report["database"] as JObject;
                var comparison = report["known_build_check"] as JObject;
                var timings = report["timings_ms"] as JObject;

                var none = L.Get("Common_None");
                var fileSha = db?["file_sha256"]?.Value<string>() ?? none;
                var contentSha = db?["content_sha256"]?.Value<string>() ?? none;
                var cmpMessage = comparison?["message"]?.Value<string>() ?? none;
                var cmpSource = comparison?["source"]?.Value<string>();
                var cmpStatus = comparison?["status"]?.Value<string>();
                var matchedDate = comparison?["matched_registered_at"]?.Value<string>();
                var fileShaOnly = comparison?["file_sha256_mismatch_only"]?.Value<bool>() ?? false;
                var tableDiffs = comparison?["table_diffs"] as JArray;

                output.WriteLine(L.Format("Wizard_CompletionFileSha", fileSha));
                output.WriteLine(L.Format("Wizard_CompletionContentSha", contentSha));
                output.WriteLine(string.IsNullOrWhiteSpace(cmpSource)
                    ? L.Format("Wizard_CompletionKnownBuilds", cmpMessage)
                    : L.Format("Wizard_CompletionKnownBuildsWithSource", cmpMessage, cmpSource));

                if (string.Equals(cmpStatus, "matched", StringComparison.Ordinal)
                    && !string.IsNullOrWhiteSpace(matchedDate))
                {
                    output.WriteLine(L.Format("Wizard_CompletionMatchedDate", matchedDate));
                }
                else if (string.Equals(cmpStatus, "hash_mismatch", StringComparison.Ordinal))
                {
                    if (fileShaOnly)
                    {
                        output.WriteLine(L.Get("Wizard_CompletionFileShaOnly"));
                    }
                    else if (tableDiffs is not null && tableDiffs.Count > 0)
                    {
                        output.WriteLine(L.Format("Wizard_CompletionTableDiffCount", tableDiffs.Count));
                    }
                }

                if (timings is not null)
                {
                    var totalMs = timings["total"]?.Value<long>();
                    var exportMs = (timings["export_datatable"]?.Value<long>() ?? 0)
                        + (timings["export_locres"]?.Value<long>() ?? 0)
                        + (timings["export_inventory"]?.Value<long>() ?? 0)
                        + (timings["export_assets"]?.Value<long>() ?? 0)
                        + (timings["export_maps"]?.Value<long>() ?? 0);
                    var dbBuildMs = timings["ffr_db_build"]?.Value<long>() ?? 0;
                    var dbHashMs = timings["ffr_db_hash"]?.Value<long>() ?? 0;
                    var displayTotalSec = totalMs.HasValue
                        ? totalMs.Value / 1000.0
                        : wallElapsed.TotalSeconds;
                    output.WriteLine(L.Format(
                        "Wizard_CompletionTimings",
                        (exportMs / 1000.0).ToString("F2", CultureInfo.InvariantCulture),
                        (dbBuildMs / 1000.0).ToString("F2", CultureInfo.InvariantCulture),
                        (dbHashMs / 1000.0).ToString("F2", CultureInfo.InvariantCulture),
                        displayTotalSec.ToString("F2", CultureInfo.InvariantCulture)));
                }
                else
                {
                    output.WriteLine(L.Format("Wizard_CompletionTotalTime", wallElapsed.TotalSeconds.ToString("F2", CultureInfo.InvariantCulture)));
                }

                output.WriteLine(L.Format("Wizard_CompletionReportPath", metadataPath));
            }
            catch (Exception exception)
            {
                output.WriteLine(L.Format("Wizard_CompletionReadReportFailed", metadataPath, exception.Message));
                output.WriteLine(L.Format("Wizard_CompletionTotalTime", wallElapsed.TotalSeconds.ToString("F2", CultureInfo.InvariantCulture)));
            }
        }
        else
        {
            output.WriteLine(L.Format("Wizard_CompletionTotalTime", wallElapsed.TotalSeconds.ToString("F2", CultureInfo.InvariantCulture)));
        }

        if (exitCode == 1)
        {
            output.WriteLine(L.Get("Wizard_CompletionPartialFailure"));
        }
    }

    private static void PrintMissingFfrDbError(
        TextWriter error,
        Exception exception,
        string executableDirectory,
        bool isWindows)
    {
        var dbExeName = isWindows ? "ffr-db.exe" : "ffr-db";
        var exportExeName = isWindows ? "ffr-export.exe" : "ffr-export";

        error.WriteLine(L.Format("Wizard_MissingFfrDbHeadline", dbExeName));
        error.WriteLine(L.Format("Wizard_ErrorDetails", exception.Message));
        error.WriteLine(L.Get("Wizard_NextStepsHeader"));
        error.WriteLine(L.Format("Wizard_MissingFfrDbStep1", dbExeName, exportExeName, executableDirectory));
        error.WriteLine(L.Format("Wizard_MissingFfrDbStep2", exportExeName));
        error.WriteLine(L.Format("Wizard_MissingFfrDbStep3", dbExeName));
    }

    private static void PrintFailureAdvice(
        TextWriter error,
        Exception? exception,
        string executableDirectory,
        bool isWindows)
    {
        var dbExeName = isWindows ? "ffr-db.exe" : "ffr-db";
        var message = exception?.Message ?? string.Empty;

        error.WriteLine(L.Get("Wizard_NextStepsHeader"));
        if (message.Contains("ffr-db", StringComparison.OrdinalIgnoreCase))
        {
            error.WriteLine(L.Format("Wizard_AdviceFfrDb1", dbExeName, executableDirectory));
            error.WriteLine(L.Get("Wizard_AdviceFfrDb2"));
            return;
        }

        if (message.Contains(".usmap", StringComparison.OrdinalIgnoreCase)
            || message.Contains("mapping", StringComparison.OrdinalIgnoreCase)
            || message.Contains("映射文件", StringComparison.Ordinal)
            || message.Contains("マッピング", StringComparison.Ordinal))
        {
            error.WriteLine(L.Get("Wizard_AdviceUsmap1"));
            error.WriteLine(L.Get("Wizard_AdviceUsmap2"));
            return;
        }

        if (message.Contains("game directory", StringComparison.OrdinalIgnoreCase)
            || message.Contains("游戏目录", StringComparison.Ordinal)
            || message.Contains("ゲームディレクトリ", StringComparison.Ordinal)
            || message.Contains("FFRS/", StringComparison.Ordinal)
            || message.Contains("FFRS-Win64-Shipping.exe", StringComparison.OrdinalIgnoreCase))
        {
            error.WriteLine(L.Get("Wizard_AdviceGameDir1"));
            error.WriteLine(L.Get("Wizard_AdviceGameDir2"));
            return;
        }

        error.WriteLine(L.Get("Wizard_AdviceGeneric1"));
        error.WriteLine(L.Get("Wizard_AdviceGeneric2"));
        error.WriteLine(L.Get("Wizard_AdviceGeneric3"));
    }

    private static void WaitForExit(TextReader input, TextWriter output)
    {
        output.WriteLine();
        output.Write(L.Get("Wizard_PressEnterToExit"));
        output.Flush();
        _ = input.ReadLine();
    }

    private static bool TryNormalizeFullPath(string cleanedPath, out string fullPath, out string errorMessage)
    {
        try
        {
            fullPath = PathSupport.TrimTrailingSeparators(Path.GetFullPath(cleanedPath));
            errorMessage = string.Empty;
            return true;
        }
        catch (Exception exception)
        {
            fullPath = string.Empty;
            errorMessage = exception.Message;
            return false;
        }
    }

    private static bool HasOwnBuildDbMetadata(string ioOutputDir)
    {
        var metaPath = Path.Combine(ioOutputDir, BuildMetadataFileName);
        if (!File.Exists(metaPath))
        {
            return false;
        }

        try
        {
            var doc = JObject.Parse(File.ReadAllText(metaPath, Utf8WithoutBom));
            var command = doc.Property("command")?.Value.Value<string>();
            return string.Equals(command, "build-db", StringComparison.Ordinal)
                && doc.Property("cue4ParseVersion") is not null;
        }
        catch (JsonException)
        {
            return false;
        }
    }

    private static bool IsYes(string value)
    {
        return value.Equals("y", StringComparison.OrdinalIgnoreCase)
            || value.Equals("yes", StringComparison.OrdinalIgnoreCase)
            || value.Equals("是", StringComparison.Ordinal)
            || value.Equals("はい", StringComparison.Ordinal);
    }

    private static bool IsNo(string value)
    {
        return value.Equals("n", StringComparison.OrdinalIgnoreCase)
            || value.Equals("no", StringComparison.OrdinalIgnoreCase)
            || value.Equals("q", StringComparison.OrdinalIgnoreCase)
            || value.Equals("否", StringComparison.Ordinal)
            || value.Equals("いいえ", StringComparison.Ordinal);
    }
}

internal static partial class Program
{
    internal static BuildDbRunner DefaultBuildDbRunner => RunBuildDbAsync;

    private static Task<int> RunInteractiveWizardAsync()
    {
        TryEnableUtf8Console();
        return InteractiveWizard.RunAsync(
            Console.In,
            Console.Out,
            Console.Error,
            DefaultBuildDbRunner,
            () => SteamLocator.DiscoverGames(),
            () => FfrDbLocator.Resolve(null),
            AppContext.BaseDirectory);
    }

    private static void TryEnableUtf8Console()
    {
        try
        {
            if (!Console.IsOutputRedirected)
            {
                Console.OutputEncoding = new UTF8Encoding(false);
            }

            if (!Console.IsInputRedirected)
            {
                Console.InputEncoding = new UTF8Encoding(false);
            }
        }
        catch
        {
            // Ignore console encoding errors in headless or restricted terminal environments.
        }
    }
}
