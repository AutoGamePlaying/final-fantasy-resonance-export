using System.Diagnostics;
using System.Globalization;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace FfrExport;

internal sealed record BuildDbOptions(
    string GameDirectory,
    string UsmapPath,
    string OutputArgument,
    string? ExplicitFfrDbPath,
    string? SteamBuildId,
    bool KeepJson,
    int? Jobs,
    bool Force,
    bool HasExplicitOutput = true,
    bool ProgressJson = false);

internal sealed record ProcessExecutionResult(
    int ExitCode,
    string Stdout,
    string Stderr);

/// <summary>
/// Resolves the ffr-db executable in priority order: --ffr-db, executable sibling directory, then PATH.
/// </summary>
internal static class FfrDbLocator
{
    internal static string Resolve(
        string? explicitPath,
        string? executableDirectoryOverride = null,
        string? pathEnvironmentOverride = null,
        bool? isWindowsOverride = null)
    {
        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        if (!string.IsNullOrWhiteSpace(explicitPath))
        {
            var trimmed = explicitPath.Trim();
            var fullExplicit = Path.GetFullPath(trimmed);
            if (!File.Exists(PathSupport.ToIoPath(fullExplicit)))
            {
                throw new FileNotFoundException(L.Format("BuildDb_ExplicitFfrDbNotFound", explicitPath));
            }

            return fullExplicit;
        }

        string[] candidateNames = isWindows
            ? ["ffr-db.exe", "ffr-db.cmd", "ffr-db.bat"]
            : ["ffr-db"];

        var baseDir = executableDirectoryOverride ?? AppContext.BaseDirectory;
        if (!string.IsNullOrWhiteSpace(baseDir))
        {
            foreach (var name in candidateNames)
            {
                var sibling = Path.Combine(baseDir, name);
                if (File.Exists(PathSupport.ToIoPath(sibling)))
                {
                    return Path.GetFullPath(sibling);
                }
            }
        }

        var pathEnv = pathEnvironmentOverride ?? Environment.GetEnvironmentVariable("PATH");
        if (!string.IsNullOrWhiteSpace(pathEnv))
        {
            foreach (var rawDir in pathEnv.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
            {
                var dir = rawDir.Trim('"');
                if (string.IsNullOrEmpty(dir))
                {
                    continue;
                }

                foreach (var name in candidateNames)
                {
                    var candidate = Path.Combine(dir, name);
                    if (File.Exists(PathSupport.ToIoPath(candidate)))
                    {
                        return Path.GetFullPath(candidate);
                    }
                }
            }
        }

        throw new FileNotFoundException(L.Format("BuildDb_FfrDbNotFound", baseDir));
    }
}

internal static partial class Program
{
    private const string BuildMetadataFileName = "ffr.sqlite.build.json";
    private static readonly string[] BuildDbDatatablePrefixes = ["FFRS/Content/Datatable/"];

    private static void WriteBuildDbStep(int step, int total, string title, bool progressJson)
    {
        if (progressJson)
        {
            Console.WriteLine(ProgressProtocol.FormatProgressLine(step, total, title));
        }

        Console.WriteLine(L.Format("BuildDb_StepLine", step, total, title));
    }

    private static async Task<int> RunBuildDbAsync(string[] args)
    {
        var options = ParseBuildDbArgs(args);
        var totalStopwatch = Stopwatch.StartNew();

        // 1. Validate game directory, input files, ffr-db binary, and output directory guardrails up front.
        var gamePaths = GameVersionDetector.ValidateGameDirectory(options.GameDirectory);

        var usmapFullPath = Path.GetFullPath(options.UsmapPath);
        if (!File.Exists(PathSupport.ToIoPath(usmapFullPath)))
        {
            throw new FileNotFoundException(L.Format("Cli_MappingFileNotFound", options.UsmapPath));
        }

        var ffrDbExecutable = FfrDbLocator.Resolve(options.ExplicitFfrDbPath);

        var effectiveOutputArgument = options.OutputArgument;
        if (!options.HasExplicitOutput)
        {
            var agpResolution = AgpPaths.ResolveWritableAgpRootOrThrow(Console.Out);
            var detectedSteam = GameVersionDetector.ReadSteamBuildInfo(
                gamePaths.GameRoot,
                options.SteamBuildId,
                TextWriter.Null);
            effectiveOutputArgument = AgpPaths.GetDefaultOutputDirectory(
                SteamLocator.ResonanceGame.Code,
                detectedSteam.BuildId,
                agpHomeEnvOverride: agpResolution.AgpRoot);
        }

        var outputPath = PathSupport.TrimTrailingSeparators(Path.GetFullPath(effectiveOutputArgument));
        ValidateBuildDbOutputNotOverlappingGame(gamePaths.GameRoot, outputPath, effectiveOutputArgument);
        PrepareBuildDbOutputDirectory(outputPath, effectiveOutputArgument, options.Force);

        var startedAt = DateTimeOffset.UtcNow;
        var metadataPath = Path.Combine(outputPath, BuildMetadataFileName);
        await WriteBuildDbSentinelAsync(metadataPath, startedAt);

        // 2. Identify game version (Steam buildid, PE FileVersion/ProductVersion, pak fingerprint) and input hashes.
        WriteBuildDbStep(1, 5, L.Get("BuildDb_Step1Title"), options.ProgressJson);
        var versionStopwatch = Stopwatch.StartNew();

        var steamInfo = GameVersionDetector.ReadSteamBuildInfo(gamePaths.GameRoot, options.SteamBuildId, Console.Error);
        var exeInfo = GameVersionDetector.ReadExeVersion(gamePaths.ExePath);
        var pakResult = GameVersionDetector.ComputePakFingerprint(gamePaths.PaksDir);

        versionStopwatch.Stop();
        Console.WriteLine(L.Format(
            "BuildDb_SteamAndExeLine",
            steamInfo.BuildId ?? "null",
            steamInfo.Source ?? L.Get("Common_None"),
            exeInfo.FileVersion ?? "null",
            exeInfo.ProductVersion ?? "null"));
        Console.WriteLine(L.Format(
            "BuildDb_PakFingerprintLine",
            pakResult.Fingerprint,
            pakResult.Files.Length,
            pakResult.HashedBytes,
            pakResult.ElapsedMs));

        var usmapStopwatch = Stopwatch.StartNew();
        var usmapInfo = GameVersionDetector.ComputeUsmapHashes(usmapFullPath);
        usmapStopwatch.Stop();

        Console.WriteLine(L.Format(
            "BuildDb_UsmapHashLine",
            usmapInfo.FileSha256,
            usmapInfo.ContentSha256,
            usmapInfo.TypeCount,
            usmapInfo.EnumCount));

        var ffrDbVersion = await ReadFfrDbVersionAsync(ffrDbExecutable);
        var (toolVersion, gitCommit) = KnownBuildsRegistry.ReadBuildMetadata();

        // 3. Export all 5 JSON stages in-process into <out>/json/ matching .deps/ffr/ layout.
        WriteBuildDbStep(2, 5, L.Get("BuildDb_Step2Title"), options.ProgressJson);
        var jsonRoot = Path.Combine(outputPath, "json");
        var exportDir = Path.Combine(jsonRoot, "export");
        var locresDir = Path.Combine(exportDir, "Localization");
        var inventoryDir = Path.Combine(jsonRoot, "inventory");
        var fullDir = Path.Combine(jsonRoot, "export-full");
        var mapsDir = Path.Combine(fullDir, "maps");

        Directory.CreateDirectory(PathSupport.ToIoPath(exportDir));
        Directory.CreateDirectory(PathSupport.ToIoPath(locresDir));
        Directory.CreateDirectory(PathSupport.ToIoPath(inventoryDir));
        Directory.CreateDirectory(PathSupport.ToIoPath(fullDir));
        Directory.CreateDirectory(PathSupport.ToIoPath(mapsDir));

        var usmapSha256Upper = usmapInfo.FileSha256.ToUpperInvariant();
        var assetsJobs = options.Jobs ?? Math.Clamp(Environment.ProcessorCount, 1, 8);
        var mapsJobs = options.Jobs ?? Math.Clamp(Environment.ProcessorCount, 1, 16);

        long exportDatatableMs;
        long exportLocresMs;
        long exportInventoryMs;
        long exportAssetsMs;
        long exportMapsMs;
        int exportDatatableExit;
        int exportAssetsExit;
        int exportMapsExit;

        using (var provider = CreateProvider(gamePaths.PaksDir, usmapFullPath))
        {
            var stageWatch = Stopwatch.StartNew();
            Console.WriteLine("  [2.1] export (Datatable/)");
            exportDatatableExit = await ExportAssetsWithProviderAsync(
                provider,
                usmapSha256Upper,
                exportDir,
                BuildDbDatatablePrefixes);
            exportDatatableMs = stageWatch.ElapsedMilliseconds;

            stageWatch.Restart();
            Console.WriteLine("  [2.2] locres");
            await ExportLocresWithProviderAsync(provider, locresDir, locresDir, DefaultLanguages);
            exportLocresMs = stageWatch.ElapsedMilliseconds;

            stageWatch.Restart();
            Console.WriteLine("  [2.3] inventory");
            var snapshot = await RunInventoryWithProviderAsync(provider, inventoryDir, inventoryDir);
            exportInventoryMs = stageWatch.ElapsedMilliseconds;

            stageWatch.Restart();
            Console.WriteLine("  [2.4] export-assets");
            exportAssetsExit = await RunExportAssetsWithProviderAsync(
                provider,
                usmapSha256Upper,
                fullDir,
                DefaultFullExportPrefixes,
                assetsJobs,
                force: true,
                preloadedSnapshot: snapshot);
            exportAssetsMs = stageWatch.ElapsedMilliseconds;

            stageWatch.Restart();
            Console.WriteLine("  [2.5] export-maps");
            var exportedMapDataJson = Path.Combine(
                exportDir,
                "FFRS",
                "Content",
                "Datatable",
                "Map",
                "CDT_MapData_Prod.json");
            var mapDataPath = File.Exists(PathSupport.ToIoPath(exportedMapDataJson))
                ? exportedMapDataJson
                : null;
            exportMapsExit = await RunExportMapsWithProviderAsync(
                provider,
                usmapSha256Upper,
                mapsDir,
                mapDataPath,
                [DefaultMapPrefix],
                mapsJobs,
                force: true,
                allowDefaultMapDataFallback: false);
            exportMapsMs = stageWatch.ElapsedMilliseconds;
        }

        // 4. Invoke ffr-db build and ffr-db hash --json.
        WriteBuildDbStep(3, 5, L.Get("BuildDb_Step3Title"), options.ProgressJson);
        var sqlitePath = Path.Combine(outputPath, "ffr.sqlite");
        var dbBuildStopwatch = Stopwatch.StartNew();
        var buildArgs = new List<string>
        {
            "build",
            "--export",
            exportDir,
            "--full",
            fullDir,
            "--inventory",
            inventoryDir,
            "--maps",
            mapsDir,
            "--out",
            sqlitePath,
        };

        var buildResult = await RunExternalProcessAsync(ffrDbExecutable, buildArgs, clearFfrDepsEnv: true);
        dbBuildStopwatch.Stop();
        if (!string.IsNullOrWhiteSpace(buildResult.Stdout))
        {
            Console.Write(buildResult.Stdout);
            if (!buildResult.Stdout.EndsWith('\n'))
            {
                Console.WriteLine();
            }
        }

        if (buildResult.ExitCode != 0)
        {
            throw new InvalidOperationException(L.Format(
                "BuildDb_FfrDbBuildFailed",
                ffrDbExecutable,
                buildResult.ExitCode,
                buildResult.Stderr.Trim()));
        }

        if (!File.Exists(PathSupport.ToIoPath(sqlitePath)))
        {
            throw new FileNotFoundException(L.Format("BuildDb_SqliteMissingAfterBuild", sqlitePath));
        }

        WriteBuildDbStep(4, 5, L.Get("BuildDb_Step4Title"), options.ProgressJson);
        var dbHashStopwatch = Stopwatch.StartNew();
        var dbInfo = await ComputeDatabaseHashInfoAsync(ffrDbExecutable, sqlitePath);
        dbHashStopwatch.Stop();

        Console.WriteLine(L.Format(
            "BuildDb_DbHashLine",
            dbInfo.FileSha256,
            dbInfo.ContentSha256 ?? "null",
            dbInfo.Tables.Length));

        // 5. Compare against known-builds.json.
        WriteBuildDbStep(5, 5, L.Get("BuildDb_Step5Title"), options.ProgressJson);
        var (knownEntries, knownSource) = KnownBuildsRegistry.Load();
        var comparison = KnownBuildsRegistry.Compare(
            knownEntries,
            knownSource,
            pakResult.Fingerprint,
            dbInfo);
        KnownBuildsRegistry.PrintComparison(comparison, Console.Out);

        long cleanupJsonMs = 0;
        if (!options.KeepJson && Directory.Exists(PathSupport.ToIoPath(jsonRoot)))
        {
            var cleanupWatch = Stopwatch.StartNew();
            Directory.Delete(PathSupport.ToIoPath(jsonRoot), recursive: true);
            cleanupWatch.Stop();
            cleanupJsonMs = cleanupWatch.ElapsedMilliseconds;
            Console.WriteLine(L.Format("BuildDb_TempJsonCleaned", jsonRoot, cleanupJsonMs));
        }

        totalStopwatch.Stop();
        var finishedAt = DateTimeOffset.UtcNow;

        var gameVersion = new GameVersionInfo(steamInfo, exeInfo, pakResult.Fingerprint, pakResult.Files);
        var inputsInfo = new BuildInputsInfo(usmapInfo);
        var toolsInfo = new BuildToolsInfo(
            new FfrExportToolInfo(toolVersion, gitCommit, Cue4ParseVersion),
            new FfrDbToolInfo(ffrDbVersion));

        var buildReport = new
        {
            format = 1,
            command = "build-db",
            status = "completed",
            cue4ParseVersion = Cue4ParseVersion,
            started_at = startedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            built_at = finishedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            keep_json = options.KeepJson,
            game_version = gameVersion,
            inputs = inputsInfo,
            tools = toolsInfo,
            database = dbInfo,
            known_build_check = comparison,
            export_exit_codes = new
            {
                datatable = exportDatatableExit,
                assets = exportAssetsExit,
                maps = exportMapsExit,
            },
            timings_ms = new
            {
                game_version = versionStopwatch.ElapsedMilliseconds,
                pak_fingerprint = pakResult.ElapsedMs,
                usmap_hash = usmapStopwatch.ElapsedMilliseconds,
                export_datatable = exportDatatableMs,
                export_locres = exportLocresMs,
                export_inventory = exportInventoryMs,
                export_assets = exportAssetsMs,
                export_maps = exportMapsMs,
                ffr_db_build = dbBuildStopwatch.ElapsedMilliseconds,
                ffr_db_hash = dbHashStopwatch.ElapsedMilliseconds,
                cleanup_json = cleanupJsonMs,
                total = totalStopwatch.ElapsedMilliseconds,
            },
        };

        WriteJsonAtomically(metadataPath, buildReport);
        Console.WriteLine(L.Format(
            "BuildDb_Completed",
            sqlitePath,
            metadataPath,
            totalStopwatch.Elapsed.TotalSeconds.ToString("F2", CultureInfo.InvariantCulture)));

        var hadPartialExportFailure = exportDatatableExit != 0 || exportAssetsExit != 0 || exportMapsExit != 0;
        return hadPartialExportFailure ? 1 : 0;
    }

    internal static BuildDbOptions ParseBuildDbArgs(string[] args)
    {
        string? gameDirectory = null;
        string? usmapPath = null;
        string? explicitOutputArgument = null;
        string? explicitFfrDb = null;
        string? steamBuildId = null;
        var keepJson = false;
        int? jobs = null;
        var force = false;
        var progressJson = false;

        for (var index = 0; index < args.Length; index++)
        {
            var token = args[index];
            if (token.StartsWith("--lang=", StringComparison.Ordinal))
            {
                var langValue = token["--lang=".Length..];
                if (!Localization.TryNormalizeLanguageCode(langValue, out _))
                {
                    throw new ArgumentException(L.Format("Common_UnsupportedLanguage", langValue));
                }

                continue;
            }

            if (string.Equals(token, "--jmap", StringComparison.Ordinal)
                || token.StartsWith("--jmap=", StringComparison.Ordinal))
            {
                throw new ArgumentException(L.Get("BuildDb_JmapRemoved"));
            }

            switch (token)
            {
                case "--game":
                    gameDirectory = RequireValue(args, ref index, "--game");
                    break;
                case "--usmap":
                    usmapPath = RequireValue(args, ref index, "--usmap");
                    break;
                case "--out":
                    explicitOutputArgument = RequireValue(args, ref index, "--out");
                    break;
                case "--ffr-db":
                    explicitFfrDb = RequireValue(args, ref index, "--ffr-db");
                    break;
                case "--steam-buildid":
                    steamBuildId = RequireValue(args, ref index, "--steam-buildid");
                    break;
                case "--keep-json":
                    keepJson = true;
                    break;
                case "--jobs":
                    var rawJobs = RequireValue(args, ref index, "--jobs");
                    if (!int.TryParse(rawJobs, NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsedJobs)
                        || parsedJobs < 1)
                    {
                        throw new ArgumentException(L.Format("ExportAssets_JobsMustBePositive", rawJobs));
                    }

                    jobs = parsedJobs;
                    break;
                case "--force":
                    force = true;
                    break;
                case "--progress-json":
                    progressJson = true;
                    break;
                case "--lang":
                    var rawLang = RequireValue(args, ref index, "--lang");
                    if (!Localization.TryNormalizeLanguageCode(rawLang, out _))
                    {
                        throw new ArgumentException(L.Format("Common_UnsupportedLanguage", rawLang));
                    }

                    break;
                default:
                    throw new ArgumentException(L.Format("Cli_UnknownArg", "build-db", token));
            }
        }

        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            throw new ArgumentException(L.Get("BuildDb_RequiresGame"));
        }

        if (string.IsNullOrWhiteSpace(usmapPath))
        {
            throw new ArgumentException(L.Get("BuildDb_RequiresUsmap"));
        }

        var hasExplicitOutput = !string.IsNullOrWhiteSpace(explicitOutputArgument);
        var resolvedOutputArgument = hasExplicitOutput
            ? explicitOutputArgument!
            : string.Empty;

        return new BuildDbOptions(
            gameDirectory,
            usmapPath,
            resolvedOutputArgument,
            explicitFfrDb,
            steamBuildId,
            keepJson,
            jobs,
            force,
            hasExplicitOutput,
            progressJson);
    }

    private static void ValidateBuildDbOutputNotOverlappingGame(
        string gameRootFullPath,
        string outputFullPath,
        string outputDisplay)
    {
        if (PathSupport.IsSameOrDescendantPath(gameRootFullPath, outputFullPath)
            || PathSupport.IsSameOrDescendantPath(outputFullPath, gameRootFullPath))
        {
            throw new ArgumentException(L.Format("BuildDb_OutOverlapsGame", outputDisplay));
        }
    }

    private static void PrepareBuildDbOutputDirectory(
        string outputPath,
        string outputArgument,
        bool force)
    {
        var ioOutputPath = PathSupport.ToIoPath(outputPath);
        if (Directory.Exists(ioOutputPath) && Directory.EnumerateFileSystemEntries(ioOutputPath).Any())
        {
            if (!IsBuildDbOutputDirectory(ioOutputPath))
            {
                throw new IOException(L.Format("BuildDb_ForeignOutputDir", outputArgument));
            }

            if (!force)
            {
                throw new IOException(L.Format("BuildDb_OutputDirNonEmpty", outputArgument));
            }

            var fileCount = Directory.EnumerateFiles(ioOutputPath, "*", SearchOption.AllDirectories).LongCount();
            Console.WriteLine(L.Format("Cli_ClearingOutputDir", outputArgument, fileCount));
            Directory.Delete(ioOutputPath, recursive: true);
        }

        Directory.CreateDirectory(ioOutputPath);
    }

    private static bool IsBuildDbOutputDirectory(string ioOutputPath)
    {
        var buildMetaPath = Path.Combine(ioOutputPath, BuildMetadataFileName);
        if (!File.Exists(buildMetaPath))
        {
            return false;
        }

        try
        {
            var doc = JObject.Parse(File.ReadAllText(buildMetaPath, Utf8WithoutBom));
            var command = doc.Property("command")?.Value.Value<string>();
            return string.Equals(command, "build-db", StringComparison.Ordinal)
                && doc.Property("cue4ParseVersion") is not null;
        }
        catch (JsonException)
        {
            return false;
        }
    }

    private static async Task WriteBuildDbSentinelAsync(string metadataPath, DateTimeOffset startedAt)
    {
        var sentinel = new
        {
            format = 1,
            command = "build-db",
            status = "in_progress",
            cue4ParseVersion = Cue4ParseVersion,
            started_at = startedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
        };
        await TextOutput.WriteJsonAsync(metadataPath, sentinel, trailingNewline: true);
    }

    private static async Task<string> ReadFfrDbVersionAsync(string ffrDbExecutable)
    {
        var result = await RunExternalProcessAsync(ffrDbExecutable, ["--version"], clearFfrDepsEnv: true);
        if (result.ExitCode != 0)
        {
            return "unknown";
        }

        var line = result.Stdout
            .Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .FirstOrDefault();
        return string.IsNullOrWhiteSpace(line) ? "unknown" : line;
    }

    private static async Task<BuildDatabaseInfo> ComputeDatabaseHashInfoAsync(
        string ffrDbExecutable,
        string sqlitePath)
    {
        var fileSize = new FileInfo(PathSupport.ToIoPath(sqlitePath)).Length;
        var fileName = Path.GetFileName(sqlitePath);

        var hashResult = await RunExternalProcessAsync(
            ffrDbExecutable,
            ["hash", sqlitePath, "--json"],
            clearFfrDepsEnv: true);

        if (hashResult.ExitCode != 0)
        {
            if (IsUnsupportedHashSubcommand(hashResult.Stderr))
            {
                Console.Error.WriteLine(L.Format("BuildDb_FfrDbNoHashSubcommandWarning", ffrDbExecutable));
                var fallbackFileSha = GameVersionDetector.ComputeFileSha256Lower(sqlitePath);
                return new BuildDatabaseInfo(
                    fileName,
                    fileSize,
                    SchemaVersion: null,
                    SqliteVersion: null,
                    FileSha256: fallbackFileSha,
                    ContentSha256: null,
                    HashFormat: null,
                    Tables: []);
            }

            throw new InvalidOperationException(L.Format(
                "BuildDb_FfrDbHashFailed",
                ffrDbExecutable,
                sqlitePath,
                hashResult.ExitCode,
                hashResult.Stderr.Trim()));
        }

        FfrDbHashReport? parsed;
        try
        {
            parsed = JsonConvert.DeserializeObject<FfrDbHashReport>(hashResult.Stdout);
        }
        catch (JsonException exception)
        {
            throw new InvalidDataException(
                L.Format("BuildDb_FfrDbHashInvalidJson", sqlitePath, exception.Message),
                exception);
        }

        if (parsed is null
            || string.IsNullOrWhiteSpace(parsed.FileSha256)
            || string.IsNullOrWhiteSpace(parsed.ContentSha256)
            || parsed.Tables is null)
        {
            throw new InvalidDataException(L.Format("BuildDb_FfrDbHashMissingFields", sqlitePath));
        }

        return new BuildDatabaseInfo(
            fileName,
            fileSize,
            parsed.SchemaVersion,
            parsed.SqliteVersion,
            parsed.FileSha256.ToLowerInvariant(),
            parsed.ContentSha256.ToLowerInvariant(),
            parsed.Format,
            parsed.Tables);
    }

    private static bool IsUnsupportedHashSubcommand(string stderr)
    {
        return stderr.Contains("unrecognized subcommand 'hash'", StringComparison.OrdinalIgnoreCase)
            || stderr.Contains("unknown subcommand 'hash'", StringComparison.OrdinalIgnoreCase);
    }

    private static async Task<ProcessExecutionResult> RunExternalProcessAsync(
        string fileName,
        IReadOnlyList<string> arguments,
        bool clearFfrDepsEnv)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = fileName,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            CreateNoWindow = true,
        };

        foreach (var arg in arguments)
        {
            startInfo.ArgumentList.Add(arg);
        }

        if (clearFfrDepsEnv)
        {
            startInfo.Environment.Remove(DepsEnvVar);
        }

        using var process = Process.Start(startInfo)
            ?? throw new InvalidOperationException(L.Format("BuildDb_StartSubprocessFailed", fileName));

        var stdoutTask = process.StandardOutput.ReadToEndAsync();
        var stderrTask = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();

        return new ProcessExecutionResult(process.ExitCode, await stdoutTask, await stderrTask);
    }
}
