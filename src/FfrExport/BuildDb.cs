using System.Diagnostics;
using System.Globalization;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace FfrExport;

internal sealed record BuildDbOptions(
    string GameDirectory,
    string UsmapPath,
    List<string> JmapPaths,
    string OutputArgument,
    string? ExplicitFfrDbPath,
    string? SteamBuildId,
    bool KeepJson,
    int? Jobs,
    bool Force);

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
                throw new FileNotFoundException($"指定的 --ffr-db 可执行文件不存在：{explicitPath}");
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

        throw new FileNotFoundException(
            $"找不到 ffr-db 可执行文件（已依次检查：--ffr-db 参数、程序同目录 {baseDir}、环境变量 PATH）。" +
            "请通过 --ffr-db <路径> 指定，或将 ffr-db 放到程序同目录 / PATH 中。");
    }
}

internal static partial class Program
{
    private const string DefaultBuildDbOutputDirectory = "ffr-build";
    private const string BuildMetadataFileName = "ffr.sqlite.build.json";
    private static readonly string[] BuildDbDatatablePrefixes = ["FFRS/Content/Datatable/"];

    private static async Task<int> RunBuildDbAsync(string[] args)
    {
        var options = ParseBuildDbArgs(args);
        var totalStopwatch = Stopwatch.StartNew();

        // 1. Validate game directory, input files, ffr-db binary, and output directory guardrails up front.
        var gamePaths = GameVersionDetector.ValidateGameDirectory(options.GameDirectory);

        var usmapFullPath = Path.GetFullPath(options.UsmapPath);
        if (!File.Exists(PathSupport.ToIoPath(usmapFullPath)))
        {
            throw new FileNotFoundException($"映射文件不存在：{options.UsmapPath}");
        }

        var jmapFullPaths = new List<string>(options.JmapPaths.Count);
        foreach (var jmap in options.JmapPaths)
        {
            var fullJmap = Path.GetFullPath(jmap);
            if (!File.Exists(PathSupport.ToIoPath(fullJmap)))
            {
                throw new FileNotFoundException($"反射转储文件（--jmap）不存在：{jmap}");
            }

            jmapFullPaths.Add(fullJmap);
        }

        var ffrDbExecutable = FfrDbLocator.Resolve(options.ExplicitFfrDbPath);

        var outputPath = PathSupport.TrimTrailingSeparators(Path.GetFullPath(options.OutputArgument));
        ValidateBuildDbOutputNotOverlappingGame(gamePaths.GameRoot, outputPath, options.OutputArgument);
        PrepareBuildDbOutputDirectory(outputPath, options.OutputArgument, options.Force);

        var startedAt = DateTimeOffset.UtcNow;
        var metadataPath = Path.Combine(outputPath, BuildMetadataFileName);
        await WriteBuildDbSentinelAsync(metadataPath, startedAt);

        // 2. Identify game version (Steam buildid, PE FileVersion/ProductVersion, pak fingerprint) and input hashes.
        Console.WriteLine("==> 步骤 1/5：识别游戏版本与输入哈希");
        var versionStopwatch = Stopwatch.StartNew();

        var steamInfo = GameVersionDetector.ReadSteamBuildInfo(gamePaths.GameRoot, options.SteamBuildId, Console.Error);
        var exeInfo = GameVersionDetector.ReadExeVersion(gamePaths.ExePath);
        var pakResult = GameVersionDetector.ComputePakFingerprint(gamePaths.PaksDir);

        versionStopwatch.Stop();
        Console.WriteLine(
            $"Steam 构建号：{steamInfo.BuildId ?? "null"}（来源：{steamInfo.Source ?? "无"}）" +
            $"；exe 版本：FileVersion={exeInfo.FileVersion ?? "null"}，ProductVersion={exeInfo.ProductVersion ?? "null"}");
        Console.WriteLine(
            $"游戏包指纹：{pakResult.Fingerprint}（共 {pakResult.Files.Length} 个文件，哈希 {pakResult.HashedBytes} 字节，耗时 {pakResult.ElapsedMs} ms）");

        var usmapStopwatch = Stopwatch.StartNew();
        var usmapInfo = GameVersionDetector.ComputeUsmapHashes(usmapFullPath);
        var jmapInfos = jmapFullPaths.Select(GameVersionDetector.ComputeJmapHash).ToArray();
        usmapStopwatch.Stop();

        Console.WriteLine(
            $".usmap 哈希：file={usmapInfo.FileSha256}，content={usmapInfo.ContentSha256}（{usmapInfo.TypeCount} 类，{usmapInfo.EnumCount} 枚举）");

        var ffrDbVersion = await ReadFfrDbVersionAsync(ffrDbExecutable);
        var (toolVersion, gitCommit) = KnownBuildsRegistry.ReadBuildMetadata();

        // 3. Export all 5 JSON stages in-process into <out>/json/ matching .deps/ffr/ layout.
        Console.WriteLine("==> 步骤 2/5：导出游戏数据为 JSON");
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
        Console.WriteLine("==> 步骤 3/5：调用 ffr-db build 构建 SQLite 数据库");
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
        };
        foreach (var jmap in jmapFullPaths)
        {
            buildArgs.Add("--jmap");
            buildArgs.Add(jmap);
        }

        buildArgs.Add("--out");
        buildArgs.Add(sqlitePath);

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
            throw new InvalidOperationException(
                $"调用 ffr-db build 失败（可执行文件：{ffrDbExecutable}，退出码：{buildResult.ExitCode}）：{buildResult.Stderr.Trim()}");
        }

        if (!File.Exists(PathSupport.ToIoPath(sqlitePath)))
        {
            throw new FileNotFoundException($"ffr-db build 执行完成但未生成数据库文件：{sqlitePath}");
        }

        Console.WriteLine("==> 步骤 4/5：调用 ffr-db hash 计算数据库哈希");
        var dbHashStopwatch = Stopwatch.StartNew();
        var dbInfo = await ComputeDatabaseHashInfoAsync(ffrDbExecutable, sqlitePath);
        dbHashStopwatch.Stop();

        Console.WriteLine(
            $"数据库哈希：file_sha256={dbInfo.FileSha256}，content_sha256={dbInfo.ContentSha256 ?? "null"}（{dbInfo.Tables.Length} 张表）");

        // 5. Compare against known-builds.json.
        Console.WriteLine("==> 步骤 5/5：比对已知版本登记表并写出构建报告");
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
            Console.WriteLine($"已清理临时导出目录：{jsonRoot}（耗时 {cleanupJsonMs} ms）");
        }

        totalStopwatch.Stop();
        var finishedAt = DateTimeOffset.UtcNow;

        var gameVersion = new GameVersionInfo(steamInfo, exeInfo, pakResult.Fingerprint, pakResult.Files);
        var inputsInfo = new BuildInputsInfo(usmapInfo, jmapInfos);
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
                usmap_and_jmap_hash = usmapStopwatch.ElapsedMilliseconds,
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
        Console.WriteLine(
            $"建库完成：{sqlitePath}（报告：{metadataPath}，总耗时 {totalStopwatch.Elapsed.TotalSeconds:F2}s）");

        var hadPartialExportFailure = exportDatatableExit != 0 || exportAssetsExit != 0 || exportMapsExit != 0;
        return hadPartialExportFailure ? 1 : 0;
    }

    internal static BuildDbOptions ParseBuildDbArgs(string[] args)
    {
        string? gameDirectory = null;
        string? usmapPath = null;
        var jmaps = new List<string>();
        var outputArgument = DefaultBuildDbOutputDirectory;
        string? explicitFfrDb = null;
        string? steamBuildId = null;
        var keepJson = false;
        int? jobs = null;
        var force = false;

        for (var index = 0; index < args.Length; index++)
        {
            switch (args[index])
            {
                case "--game":
                    gameDirectory = RequireValue(args, ref index, "--game");
                    break;
                case "--usmap":
                    usmapPath = RequireValue(args, ref index, "--usmap");
                    break;
                case "--jmap":
                    jmaps.Add(RequireValue(args, ref index, "--jmap"));
                    break;
                case "--out":
                    outputArgument = RequireValue(args, ref index, "--out");
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
                        throw new ArgumentException($"--jobs 必须是正整数：{rawJobs}");
                    }

                    jobs = parsedJobs;
                    break;
                case "--force":
                    force = true;
                    break;
                default:
                    throw new ArgumentException($"build-db 收到未知参数：{args[index]}");
            }
        }

        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            throw new ArgumentException("build-db 需要 --game <游戏目录>");
        }

        if (string.IsNullOrWhiteSpace(usmapPath))
        {
            throw new ArgumentException("build-db 需要 --usmap <文件>");
        }

        return new BuildDbOptions(
            gameDirectory,
            usmapPath,
            jmaps,
            outputArgument,
            explicitFfrDb,
            steamBuildId,
            keepJson,
            jobs,
            force);
    }

    private static void ValidateBuildDbOutputNotOverlappingGame(
        string gameRootFullPath,
        string outputFullPath,
        string outputDisplay)
    {
        if (PathSupport.IsSameOrDescendantPath(gameRootFullPath, outputFullPath)
            || PathSupport.IsSameOrDescendantPath(outputFullPath, gameRootFullPath))
        {
            throw new ArgumentException($"--out 不能与游戏目录重叠：{outputDisplay}");
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
                throw new IOException(
                    $"{outputArgument} 已存在且不是 ffr-export build-db 的输出目录，拒绝写入；换一个空目录或手动清理后重试");
            }

            if (!force)
            {
                throw new IOException(
                    $"输出目录非空（{outputArgument}）；如需清空并重新建库，请加 --force");
            }

            var fileCount = Directory.EnumerateFiles(ioOutputPath, "*", SearchOption.AllDirectories).LongCount();
            Console.WriteLine($"即将清空输出目录：{outputArgument}；文件数：{fileCount}");
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
        await File.WriteAllTextAsync(
            PathSupport.ToIoPath(metadataPath),
            JsonConvert.SerializeObject(sentinel, Formatting.Indented) + "\n",
            Utf8WithoutBom);
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
                Console.Error.WriteLine(
                    $"警告：当前 ffr-db（{ffrDbExecutable}）不支持 hash 子命令，仅计算 file_sha256；content_sha256 与各表哈希记为 null");
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

            throw new InvalidOperationException(
                $"调用 ffr-db hash 失败（可执行文件：{ffrDbExecutable}，数据库：{sqlitePath}，退出码：{hashResult.ExitCode}）：{hashResult.Stderr.Trim()}");
        }

        FfrDbHashReport? parsed;
        try
        {
            parsed = JsonConvert.DeserializeObject<FfrDbHashReport>(hashResult.Stdout);
        }
        catch (JsonException exception)
        {
            throw new InvalidDataException(
                $"解析 ffr-db hash 输出的 JSON 失败（数据库：{sqlitePath}）：{exception.Message}",
                exception);
        }

        if (parsed is null
            || string.IsNullOrWhiteSpace(parsed.FileSha256)
            || string.IsNullOrWhiteSpace(parsed.ContentSha256)
            || parsed.Tables is null)
        {
            throw new InvalidDataException(
                $"ffr-db hash 返回的 JSON 缺少必需字段（file_sha256/content_sha256/tables）：{sqlitePath}");
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
            ?? throw new InvalidOperationException($"无法启动子进程：{fileName}");

        var stdoutTask = process.StandardOutput.ReadToEndAsync();
        var stderrTask = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();

        return new ProcessExecutionResult(process.ExitCode, await stdoutTask, await stderrTask);
    }
}
