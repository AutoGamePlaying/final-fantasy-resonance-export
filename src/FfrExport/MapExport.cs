using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;
using System.Security.Cryptography;
using CUE4Parse.FileProvider;
using CUE4Parse.FileProvider.Objects;
using CUE4Parse.UE4.Assets;
using CUE4Parse.UE4.Assets.Exports.Engine;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace FfrExport;

internal static partial class Program
{
    private const string RelativeMapExportDirectory = "ffr/export-full/maps";
    private const string DefaultMapPrefix = "FFRS/Content/";
    private const string MapDataPackagePath = "FFRS/Content/Datatable/Map/CDT_MapData_Prod.uasset";
    private const string RelativeMapDataJsonPath = "ffr/export/FFRS/Content/Datatable/Map/CDT_MapData_Prod.json";

    internal static string GetDefaultMapExportDirectory() => ResolveDefaultDepsPath(RelativeMapExportDirectory);

    internal static string GetDefaultMapDataJsonPath() => ResolveDefaultDepsPath(RelativeMapDataJsonPath);

    private sealed record MapExportOptions(
        string Paks,
        string? UsmapPath,
        string OutputArgument,
        string? MapDataJsonPath,
        string? FromJsonDirectory,
        List<string> Prefixes,
        bool Force,
        int MaxParallelism);

    private sealed record MapFixtureInput(
        [property: JsonProperty("path")] string Path,
        [property: JsonProperty("name")] string? Name,
        [property: JsonProperty("streamingLevels")] List<StreamingLevelDto>? StreamingLevels,
        [property: JsonProperty("actors")] List<ActorDto> Actors);

    private static async Task<int> RunExportMapsAsync(string[] args)
    {
        var options = ParseMapExportArgs(args);
        var outputPath = RequireDepsPath(options.OutputArgument, "--out");
        var startedAt = DateTimeOffset.UtcNow;
        var stopwatch = Stopwatch.StartNew();

        if (options.FromJsonDirectory is not null)
        {
            return await RunExportMapsFromJsonAsync(options, outputPath, startedAt, stopwatch);
        }

        if (options.UsmapPath is null)
        {
            throw new ArgumentException("export-maps 需要 --usmap <文件>（在游戏机上用 UE4SS 导出的 .usmap，见 README.md）");
        }

        if (!File.Exists(options.UsmapPath))
        {
            throw new FileNotFoundException($"映射文件不存在：{options.UsmapPath}");
        }

        PrepareMapExportDirectory(outputPath, options.OutputArgument, options.Force);
        var usmapHash = Convert.ToHexString(SHA256.HashData(await File.ReadAllBytesAsync(PathSupport.ToIoPath(options.UsmapPath))));
        var prefixes = options.Prefixes.Count > 0 ? options.Prefixes : [DefaultMapPrefix];

        using var provider = CreateProvider(options.Paks, options.UsmapPath);
        return await RunExportMapsWithProviderAsync(
            provider,
            usmapHash,
            outputPath,
            options.MapDataJsonPath,
            prefixes,
            options.MaxParallelism,
            options.Force,
            allowDefaultMapDataFallback: true);
    }

    private static async Task<int> RunExportMapsWithProviderAsync(
        DefaultFileProvider provider,
        string usmapHash,
        string outputPath,
        string? mapDataJsonPath,
        IReadOnlyList<string> prefixes,
        int maxParallelism,
        bool force,
        bool allowDefaultMapDataFallback)
    {
        var startedAt = DateTimeOffset.UtcNow;
        var stopwatch = Stopwatch.StartNew();

        // Write an initial running manifest so interrupted runs are still recognized on resume/--force.
        await WriteMapManifestAsync(
            outputPath,
            usmapHash,
            startedAt,
            status: "running",
            totalLevels: 0,
            exportedCount: 0,
            skippedCount: 0,
            failures: [],
            elapsedMs: stopwatch.ElapsedMilliseconds);

        var mapDataEntries = LoadMapDataEntries(provider, mapDataJsonPath, allowDefaultMapDataFallback);
        var mapDataByLevelName = mapDataEntries.ToDictionary(e => e.LevelName, StringComparer.Ordinal);
        var mapDataById = mapDataEntries.ToDictionary(e => e.MapId);

        var umaps = provider.Files.Values
            .Where(file => prefixes.Any(prefix => file.Path.StartsWith(prefix, StringComparison.Ordinal)))
            .Where(file => file.Path.EndsWith(".umap", StringComparison.OrdinalIgnoreCase))
            .OrderBy(file => file.Path, StringComparer.Ordinal)
            .ToArray();

        // Also scan all persistent levels under FFRS/Content/ so filtered --prefix runs still know parent _PL & mapId.
        var allPlFiles = provider.Files.Values
            .Where(file => file.Path.StartsWith(DefaultMapPrefix, StringComparison.Ordinal))
            .Where(file => file.Path.EndsWith(".umap", StringComparison.OrdinalIgnoreCase))
            .Where(file => MapExtractor.ClassifySublevelType(Path.GetFileNameWithoutExtension(file.Path), file.Path) == "PL")
            .OrderBy(file => file.Path, StringComparer.Ordinal)
            .ToArray();

        var ownership = BuildLevelOwnershipIndex(provider, allPlFiles, mapDataEntries);
        var outputNames = AssignMapOutputFilenames(umaps.Select(f => f.Path).ToArray());

        var contributions = new ConcurrentBag<LevelGraphContribution>();
        var failures = new ConcurrentBag<(string Path, string Error)>();
        var exportedCount = 0;
        var skippedCount = 0;

        Parallel.ForEach(
            umaps,
            new ParallelOptions { MaxDegreeOfParallelism = maxParallelism },
            file =>
            {
                try
                {
                    var outputFile = outputNames[file.Path];
                    var jsonPath = Path.Combine(outputPath, outputFile);

                    if (!force && TryLoadExistingMapContribution(jsonPath, out var existing))
                    {
                        contributions.Add(existing!);
                        Interlocked.Increment(ref skippedCount);
                        return;
                    }

                    if (!provider.TryLoadPackage(file.Path, out IPackage? package) || package is null)
                    {
                        throw new InvalidDataException("CUE4Parse 无法加载此关卡包");
                    }

                    var exports = package.GetExports().ToArray();
                    var levelName = Path.GetFileNameWithoutExtension(file.Path);
                    var packagePath = MapExtractor.ToGamePackagePath(file.Path);
                    var sublevelType = MapExtractor.ClassifySublevelType(levelName, file.Path);
                    var streamingLevels = MapExtractor.ExtractStreamingLevels(exports);

                    var (primaryPl, allPls) = ResolvePersistentLevels(
                        file.Path,
                        levelName,
                        packagePath,
                        sublevelType,
                        ownership);
                    var (primaryMapId, allMapIds, mapEntry) = ResolveMapIds(primaryPl, allPls, mapDataByLevelName);

                    var actors = MapExtractor.ExtractActors(package, exports, packagePath);
                    var gameActorCount = actors.Count(a => a.IsGameClass);
                    var metadata = new LevelMetadataDto
                    {
                        Name = levelName,
                        OutputFile = outputFile,
                        Path = file.Path,
                        PackagePath = packagePath,
                        SublevelType = sublevelType,
                        PersistentLevel = primaryPl,
                        PersistentLevels = allPls,
                        MapId = primaryMapId,
                        MapIds = allMapIds,
                        MapRowKey = mapEntry?.RowKey,
                        MapType = mapEntry?.MapType,
                        StreamingLevels = streamingLevels,
                        ActorCount = actors.Count,
                        GameActorCount = gameActorCount,
                        EngineActorCount = actors.Count - gameActorCount,
                    };

                    var summary = MapExtractor.BuildSummary(actors, primaryMapId);
                    var doc = new MapExportDocumentDto
                    {
                        Level = metadata,
                        Actors = actors,
                        Summary = summary,
                    };

                    WriteJsonAtomically(jsonPath, doc);
                    contributions.Add(new LevelGraphContribution(metadata, summary));
                    Interlocked.Increment(ref exportedCount);
                }
                catch (Exception exception)
                {
                    failures.Add((file.Path, TruncateError(exception.Message)));
                    Console.Error.WriteLine($"关卡导出失败：{file.Path}：{exception.Message}");
                }
            });

        var sortedFailures = failures
            .OrderBy(f => f.Path, StringComparer.Ordinal)
            .ToArray();

        var graph = MapGraphBuilder.Build(
            contributions.ToArray(),
            mapDataById,
            totalLevels: umaps.Length,
            failedLevels: sortedFailures.Length);
        WriteJsonAtomically(Path.Combine(outputPath, "_graph.json"), graph);

        stopwatch.Stop();
        await WriteMapManifestAsync(
            outputPath,
            usmapHash,
            startedAt,
            status: "completed",
            totalLevels: umaps.Length,
            exportedCount,
            skippedCount,
            sortedFailures,
            stopwatch.ElapsedMilliseconds);

        Console.WriteLine(
            $"关卡导出完成：总计 {umaps.Length}，新导出 {exportedCount}，跳过 {skippedCount}，失败 {sortedFailures.Length}，" +
            $"映射地图 {graph.Stats.MappedMapCount} 张，耗时 {stopwatch.Elapsed.TotalSeconds:F2}s");
        return sortedFailures.Length == 0 ? 0 : 1;
    }

    private static async Task<int> RunExportMapsFromJsonAsync(
        MapExportOptions options,
        string outputPath,
        DateTimeOffset startedAt,
        Stopwatch stopwatch)
    {
        var fixtureDir = options.FromJsonDirectory!;
        if (!Directory.Exists(fixtureDir))
        {
            throw new DirectoryNotFoundException($"夹具目录不存在：{fixtureDir}");
        }

        PrepareMapExportDirectory(outputPath, options.OutputArgument, options.Force);
        await WriteMapManifestAsync(
            outputPath,
            usmapHash: null,
            startedAt,
            status: "running",
            totalLevels: 0,
            exportedCount: 0,
            skippedCount: 0,
            failures: [],
            elapsedMs: stopwatch.ElapsedMilliseconds);

        var mapDataEntries = options.MapDataJsonPath is not null
            ? LoadMapDataEntriesFromJson(options.MapDataJsonPath)
            : [];
        var mapDataByLevelName = mapDataEntries.ToDictionary(e => e.LevelName, StringComparer.Ordinal);
        var mapDataById = mapDataEntries.ToDictionary(e => e.MapId);

        var fixtureFiles = Directory.EnumerateFiles(fixtureDir, "*.json", SearchOption.AllDirectories)
            .OrderBy(p => p, StringComparer.Ordinal)
            .ToArray();

        var loadedFixtures = new List<(string FilePath, MapFixtureInput Input)>();
        var failures = new List<(string Path, string Error)>();
        foreach (var file in fixtureFiles)
        {
            try
            {
                var input = JsonConvert.DeserializeObject<MapFixtureInput>(await File.ReadAllTextAsync(file, Utf8WithoutBom));
                if (input is null || string.IsNullOrWhiteSpace(input.Path))
                {
                    throw new InvalidDataException("夹具缺少 path 字段");
                }

                loadedFixtures.Add((file, input));
            }
            catch (Exception exception)
            {
                failures.Add((file, TruncateError(exception.Message)));
            }
        }

        var ownership = BuildLevelOwnershipIndexFromFixtures(loadedFixtures.Select(f => f.Input).ToArray(), mapDataEntries);
        var outputNames = AssignMapOutputFilenames(loadedFixtures.Select(f => f.Input.Path).ToArray());

        var contributions = new List<LevelGraphContribution>();
        var exportedCount = 0;
        var skippedCount = 0;

        foreach (var (_, input) in loadedFixtures)
        {
            try
            {
                var outputFile = outputNames[input.Path];
                var jsonPath = Path.Combine(outputPath, outputFile);

                if (!options.Force && TryLoadExistingMapContribution(jsonPath, out var existing))
                {
                    contributions.Add(existing!);
                    skippedCount++;
                    continue;
                }

                var levelName = !string.IsNullOrWhiteSpace(input.Name)
                    ? input.Name
                    : Path.GetFileNameWithoutExtension(input.Path);
                var packagePath = MapExtractor.ToGamePackagePath(input.Path);
                var sublevelType = MapExtractor.ClassifySublevelType(levelName, input.Path);
                var streamingLevels = input.StreamingLevels ?? [];

                var (primaryPl, allPls) = ResolvePersistentLevels(
                    input.Path,
                    levelName,
                    packagePath,
                    sublevelType,
                    ownership);
                var (primaryMapId, allMapIds, mapEntry) = ResolveMapIds(primaryPl, allPls, mapDataByLevelName);

                var actors = input.Actors ?? [];
                var gameActorCount = actors.Count(a => a.IsGameClass);
                var metadata = new LevelMetadataDto
                {
                    Name = levelName,
                    OutputFile = outputFile,
                    Path = input.Path,
                    PackagePath = packagePath,
                    SublevelType = sublevelType,
                    PersistentLevel = primaryPl,
                    PersistentLevels = allPls,
                    MapId = primaryMapId,
                    MapIds = allMapIds,
                    MapRowKey = mapEntry?.RowKey,
                    MapType = mapEntry?.MapType,
                    StreamingLevels = streamingLevels,
                    ActorCount = actors.Count,
                    GameActorCount = gameActorCount,
                    EngineActorCount = actors.Count - gameActorCount,
                };

                var summary = MapExtractor.BuildSummary(actors, primaryMapId);
                var doc = new MapExportDocumentDto
                {
                    Level = metadata,
                    Actors = actors,
                    Summary = summary,
                };

                WriteJsonAtomically(jsonPath, doc);
                contributions.Add(new LevelGraphContribution(metadata, summary));
                exportedCount++;
            }
            catch (Exception exception)
            {
                failures.Add((input.Path, TruncateError(exception.Message)));
            }
        }

        var graph = MapGraphBuilder.Build(
            contributions,
            mapDataById,
            totalLevels: fixtureFiles.Length,
            failedLevels: failures.Count);
        WriteJsonAtomically(Path.Combine(outputPath, "_graph.json"), graph);

        stopwatch.Stop();
        await WriteMapManifestAsync(
            outputPath,
            usmapHash: null,
            startedAt,
            status: "completed",
            totalLevels: fixtureFiles.Length,
            exportedCount,
            skippedCount,
            failures,
            stopwatch.ElapsedMilliseconds);

        Console.WriteLine(
            $"关卡夹具汇总完成：总计 {fixtureFiles.Length}，新导出 {exportedCount}，跳过 {skippedCount}，失败 {failures.Count}，" +
            $"映射地图 {graph.Stats.MappedMapCount} 张");
        return failures.Count == 0 ? 0 : 1;
    }

    private static MapExportOptions ParseMapExportArgs(string[] args)
    {
        var paks = GetDefaultPaks();
        string? usmapPath = null;
        var outputArgument = GetDefaultMapExportDirectory();
        string? mapDataJsonPath = null;
        string? fromJsonDirectory = null;
        var prefixes = new List<string>();
        var force = false;
        var maxParallelism = Math.Clamp(Environment.ProcessorCount, 1, 16);

        for (var index = 0; index < args.Length; index++)
        {
            switch (args[index])
            {
                case "--paks":
                    paks = RequireValue(args, ref index, "--paks");
                    break;
                case "--usmap":
                    usmapPath = RequireValue(args, ref index, "--usmap");
                    break;
                case "--out":
                    outputArgument = RequireValue(args, ref index, "--out");
                    break;
                case "--prefix":
                    prefixes.Add(RequireValue(args, ref index, "--prefix"));
                    break;
                case "--map-data":
                    mapDataJsonPath = RequireValue(args, ref index, "--map-data");
                    break;
                case "--from-json":
                    fromJsonDirectory = RequireValue(args, ref index, "--from-json");
                    break;
                case "--jobs":
                case "--parallel":
                    var rawJobs = RequireValue(args, ref index, args[index]);
                    if (!int.TryParse(rawJobs, NumberStyles.Integer, CultureInfo.InvariantCulture, out maxParallelism) || maxParallelism < 1)
                    {
                        throw new ArgumentException($"并行度必须是正整数：{rawJobs}");
                    }
                    break;
                case "--force":
                    force = true;
                    break;
                default:
                    throw new ArgumentException($"export-maps 收到未知参数：{args[index]}");
            }
        }

        return new MapExportOptions(
            paks,
            usmapPath,
            outputArgument,
            mapDataJsonPath,
            fromJsonDirectory,
            prefixes,
            force,
            maxParallelism);
    }

    private static void PrepareMapExportDirectory(string outputPath, string outputArgument, bool force)
    {
        if (force)
        {
            PrepareOutputDirectory(outputPath, outputArgument, force: true, "export-maps");
            return;
        }

        if (Directory.Exists(outputPath) && Directory.EnumerateFileSystemEntries(outputPath).Any())
        {
            var manifest = RequireExportManifest(outputPath, outputArgument);
            EnsureManifestMatchesCommand(manifest, outputArgument, "export-maps");
            return;
        }

        Directory.CreateDirectory(outputPath);
    }

    private static List<MapDataEntry> LoadMapDataEntries(
        DefaultFileProvider provider,
        string? mapDataJsonPath,
        bool allowDefaultMapDataFallback = true)
    {
        if (mapDataJsonPath is not null)
        {
            return LoadMapDataEntriesFromJson(mapDataJsonPath);
        }

        if (provider.TryLoadPackage(MapDataPackagePath, out IPackage? package) && package is not null)
        {
            var table = package.GetExports().OfType<UDataTable>().FirstOrDefault();
            if (table is not null)
            {
                var serialized = JObject.Parse(TextOutput.SerializeJsonCompact(table));
                return ParseMapDataTableObject(serialized);
            }
        }

        if (allowDefaultMapDataFallback)
        {
            var defaultMapDataJsonPath = GetDefaultMapDataJsonPath();
            if (File.Exists(PathSupport.ToIoPath(defaultMapDataJsonPath)))
            {
                return LoadMapDataEntriesFromJson(defaultMapDataJsonPath);
            }
        }

        return [];
    }

    private static List<MapDataEntry> LoadMapDataEntriesFromJson(string path)
    {
        var ioPath = PathSupport.ToIoPath(path);
        if (!File.Exists(ioPath))
        {
            throw new FileNotFoundException($"地图数据表文件不存在：{path}");
        }

        var token = JToken.Parse(File.ReadAllText(ioPath, Utf8WithoutBom));
        var tableObj = token switch
        {
            JArray arr when arr.FirstOrDefault() is JObject first => first,
            JObject obj => obj,
            _ => throw new InvalidDataException($"地图数据表 JSON 格式不正确：{path}"),
        };
        return ParseMapDataTableObject(tableObj);
    }

    private static List<MapDataEntry> ParseMapDataTableObject(JObject tableObj)
    {
        var rows = tableObj["Rows"] as JObject;
        if (rows is null)
        {
            return [];
        }

        var result = new List<MapDataEntry>(rows.Count);
        foreach (var prop in rows.Properties())
        {
            if (prop.Value is not JObject row)
            {
                continue;
            }

            var idToken = row["ID"];
            if (idToken is null || idToken.Type != JTokenType.Integer)
            {
                continue;
            }

            var mapId = idToken.Value<int>();
            var levelAssetPath = (row["Level"] as JObject)?["AssetPathName"]?.Value<string>() ?? string.Empty;
            var levelName = MapExtractor.ExtractLevelStem(levelAssetPath);
            if (string.IsNullOrEmpty(levelName))
            {
                continue;
            }

            var mapType = row["MapType"]?.Value<string>() ?? string.Empty;
            var sublevelNames = new List<string>();
            if (row["mainProgressLoadLevelList"] is JArray progList)
            {
                foreach (var entry in progList.OfType<JObject>())
                {
                    if (entry["subLevelNameList"] is JArray names)
                    {
                        foreach (var nameToken in names)
                        {
                            var name = nameToken.Value<string>();
                            if (!string.IsNullOrWhiteSpace(name))
                            {
                                sublevelNames.Add(name);
                            }
                        }
                    }
                }
            }

            result.Add(new MapDataEntry(mapId, prop.Name, levelName, levelAssetPath, mapType, sublevelNames));
        }

        return result;
    }

    private sealed record LevelOwnershipIndex(
        Dictionary<string, List<string>> ByPackagePath,
        Dictionary<string, List<string>> BySublevelName,
        Dictionary<string, string> ByDirectory);

    private static LevelOwnershipIndex BuildLevelOwnershipIndex(
        DefaultFileProvider provider,
        GameFile[] allPlFiles,
        List<MapDataEntry> mapDataEntries)
    {
        var byPackagePath = new ConcurrentDictionary<string, ConcurrentBag<string>>(StringComparer.OrdinalIgnoreCase);
        var bySublevelName = new ConcurrentDictionary<string, ConcurrentBag<string>>(StringComparer.OrdinalIgnoreCase);
        var byDirectory = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);

        foreach (var plFile in allPlFiles)
        {
            var plName = Path.GetFileNameWithoutExtension(plFile.Path);
            var dir = GetParentDirectory(plFile.Path);
            if (!string.IsNullOrEmpty(dir))
            {
                byDirectory[dir] = plName;
            }
        }

        Parallel.ForEach(
            allPlFiles,
            new ParallelOptions { MaxDegreeOfParallelism = Math.Clamp(Environment.ProcessorCount, 1, 16) },
            plFile =>
            {
                var plName = Path.GetFileNameWithoutExtension(plFile.Path);
                if (!provider.TryLoadPackage(plFile.Path, out IPackage? package) || package is null)
                {
                    return;
                }

                var exports = package.GetExports().ToArray();
                foreach (var streaming in MapExtractor.ExtractStreamingLevels(exports))
                {
                    var pkgPath = NormalizeWorldAssetToPackagePath(streaming.WorldAsset);
                    if (!string.IsNullOrEmpty(pkgPath))
                    {
                        byPackagePath.GetOrAdd(pkgPath, _ => []).Add(plName);
                    }

                    bySublevelName.GetOrAdd(streaming.Name, _ => []).Add(plName);
                }
            });

        foreach (var entry in mapDataEntries)
        {
            foreach (var subName in entry.ProgressSublevelNames)
            {
                bySublevelName.GetOrAdd(subName, _ => []).Add(entry.LevelName);
            }
        }

        return new LevelOwnershipIndex(
            byPackagePath.ToDictionary(
                kv => kv.Key,
                kv => kv.Value.Distinct(StringComparer.Ordinal).OrderBy(s => s, StringComparer.Ordinal).ToList(),
                StringComparer.OrdinalIgnoreCase),
            bySublevelName.ToDictionary(
                kv => kv.Key,
                kv => kv.Value.Distinct(StringComparer.Ordinal).OrderBy(s => s, StringComparer.Ordinal).ToList(),
                StringComparer.OrdinalIgnoreCase),
            byDirectory);
    }

    private static LevelOwnershipIndex BuildLevelOwnershipIndexFromFixtures(
        MapFixtureInput[] fixtures,
        List<MapDataEntry> mapDataEntries)
    {
        var byPackagePath = new Dictionary<string, HashSet<string>>(StringComparer.OrdinalIgnoreCase);
        var bySublevelName = new Dictionary<string, HashSet<string>>(StringComparer.OrdinalIgnoreCase);
        var byDirectory = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);

        foreach (var fixture in fixtures)
        {
            var name = !string.IsNullOrWhiteSpace(fixture.Name)
                ? fixture.Name
                : Path.GetFileNameWithoutExtension(fixture.Path);
            if (MapExtractor.ClassifySublevelType(name, fixture.Path) != "PL")
            {
                continue;
            }

            var dir = GetParentDirectory(fixture.Path);
            if (!string.IsNullOrEmpty(dir))
            {
                byDirectory[dir] = name;
            }

            if (fixture.StreamingLevels is not null)
            {
                foreach (var streaming in fixture.StreamingLevels)
                {
                    var pkgPath = NormalizeWorldAssetToPackagePath(streaming.WorldAsset);
                    if (!string.IsNullOrEmpty(pkgPath))
                    {
                        if (!byPackagePath.TryGetValue(pkgPath, out var set))
                        {
                            set = new HashSet<string>(StringComparer.Ordinal);
                            byPackagePath[pkgPath] = set;
                        }

                        set.Add(name);
                    }

                    if (!bySublevelName.TryGetValue(streaming.Name, out var nameSet))
                    {
                        nameSet = new HashSet<string>(StringComparer.Ordinal);
                        bySublevelName[streaming.Name] = nameSet;
                    }

                    nameSet.Add(name);
                }
            }
        }

        foreach (var entry in mapDataEntries)
        {
            foreach (var subName in entry.ProgressSublevelNames)
            {
                if (!bySublevelName.TryGetValue(subName, out var nameSet))
                {
                    nameSet = new HashSet<string>(StringComparer.Ordinal);
                    bySublevelName[subName] = nameSet;
                }

                nameSet.Add(entry.LevelName);
            }
        }

        return new LevelOwnershipIndex(
            byPackagePath.ToDictionary(kv => kv.Key, kv => kv.Value.OrderBy(s => s, StringComparer.Ordinal).ToList(), StringComparer.OrdinalIgnoreCase),
            bySublevelName.ToDictionary(kv => kv.Key, kv => kv.Value.OrderBy(s => s, StringComparer.Ordinal).ToList(), StringComparer.OrdinalIgnoreCase),
            byDirectory);
    }

    private static (string? PrimaryPl, List<string> AllPls) ResolvePersistentLevels(
        string pakPath,
        string levelName,
        string packagePath,
        string sublevelType,
        LevelOwnershipIndex ownership)
    {
        if (sublevelType == "PL")
        {
            return (levelName, [levelName]);
        }

        var dirPl = FindEnclosingPersistentLevel(pakPath, ownership.ByDirectory);

        if (ownership.ByPackagePath.TryGetValue(packagePath, out var byPkg) && byPkg.Count > 0)
        {
            var primary = dirPl is not null && byPkg.Contains(dirPl, StringComparer.Ordinal)
                ? dirPl
                : byPkg[0];
            return (primary, byPkg);
        }

        if (ownership.BySublevelName.TryGetValue(levelName, out var byName) && byName.Count > 0)
        {
            var primary = dirPl is not null && byName.Contains(dirPl, StringComparer.Ordinal)
                ? dirPl
                : byName[0];
            return (primary, byName);
        }

        if (dirPl is not null)
        {
            return (dirPl, [dirPl]);
        }

        return (null, []);
    }

    private static (int? PrimaryMapId, List<int> AllMapIds, MapDataEntry? PrimaryEntry) ResolveMapIds(
        string? primaryPl,
        List<string> allPls,
        Dictionary<string, MapDataEntry> mapDataByLevelName)
    {
        MapDataEntry? primaryEntry = null;
        if (primaryPl is not null)
        {
            mapDataByLevelName.TryGetValue(primaryPl, out primaryEntry);
        }

        var mapIds = new List<int>();
        foreach (var pl in allPls)
        {
            if (mapDataByLevelName.TryGetValue(pl, out var entry) && !mapIds.Contains(entry.MapId))
            {
                mapIds.Add(entry.MapId);
            }
        }

        mapIds.Sort();
        var primaryMapId = primaryEntry?.MapId ?? (mapIds.Count > 0 ? mapIds[0] : null);
        return (primaryMapId, mapIds, primaryEntry);
    }

    private static string? FindEnclosingPersistentLevel(string pakPath, Dictionary<string, string> byDirectory)
    {
        var current = GetParentDirectory(pakPath);
        while (!string.IsNullOrEmpty(current))
        {
            if (byDirectory.TryGetValue(current, out var plName))
            {
                return plName;
            }

            current = GetParentDirectory(current);
        }

        return null;
    }

    private static string GetParentDirectory(string path)
    {
        var normalized = path.Replace('\\', '/');
        var slash = normalized.LastIndexOf('/');
        return slash > 0 ? normalized[..slash] : string.Empty;
    }

    private static string NormalizeWorldAssetToPackagePath(string worldAsset)
    {
        var trimmed = worldAsset.Trim();
        var dot = trimmed.LastIndexOf('.');
        var slash = trimmed.LastIndexOf('/');
        return dot > slash ? trimmed[..dot] : trimmed;
    }

    internal static Dictionary<string, string> AssignMapOutputFilenames(string[] pakPaths)
    {
        var groups = pakPaths
            .GroupBy(p => Path.GetFileNameWithoutExtension(p), StringComparer.OrdinalIgnoreCase);

        var result = new Dictionary<string, string>(pakPaths.Length, StringComparer.Ordinal);
        var used = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

        foreach (var group in groups)
        {
            var items = group.OrderBy(p => p, StringComparer.Ordinal).ToArray();
            if (items.Length == 1)
            {
                var fileName = $"{Path.GetFileNameWithoutExtension(items[0])}.json";
                result[items[0]] = fileName;
                used.Add(fileName);
                continue;
            }

            // Disambiguate colliding basenames (e.g., a copied KD/SD level in another map folder).
            foreach (var item in items)
            {
                var stem = Path.GetFileNameWithoutExtension(item);
                var folderTag = ExtractMapFolderTag(item);
                string candidate;
                if (!string.IsNullOrEmpty(folderTag) && stem.Contains(folderTag, StringComparison.OrdinalIgnoreCase) && !used.Contains($"{stem}.json"))
                {
                    candidate = $"{stem}.json";
                }
                else if (!string.IsNullOrEmpty(folderTag) && !used.Contains($"{folderTag}__{stem}.json"))
                {
                    candidate = $"{folderTag}__{stem}.json";
                }
                else
                {
                    var slug = item.Replace('/', '_').Replace('\\', '_');
                    candidate = $"{Path.GetFileNameWithoutExtension(slug)}.json";
                }

                result[item] = candidate;
                used.Add(candidate);
            }
        }

        return result;
    }

    private static string ExtractMapFolderTag(string pakPath)
    {
        var parts = pakPath.Replace('\\', '/').Split('/');
        // Typical path: FFRS/Content/Map/Dng/05Old/05Old_45/KD/Dng_01Gra_44_01_KD.umap -> "05Old_45"
        if (parts.Length >= 3)
        {
            return parts[^3];
        }

        return parts.Length >= 2 ? parts[^2] : string.Empty;
    }

    private static bool TryLoadExistingMapContribution(string jsonPath, out LevelGraphContribution? contribution)
    {
        contribution = null;
        var ioPath = PathSupport.ToIoPath(jsonPath);
        if (!File.Exists(ioPath))
        {
            return false;
        }

        try
        {
            var text = File.ReadAllText(ioPath, Utf8WithoutBom);
            if (string.IsNullOrWhiteSpace(text))
            {
                return false;
            }

            var doc = JsonConvert.DeserializeObject<MapExportDocumentDto>(text);
            if (doc?.Level is null || doc.Summary is null)
            {
                return false;
            }

            contribution = new LevelGraphContribution(doc.Level, doc.Summary);
            return true;
        }
        catch (Exception)
        {
            return false;
        }
    }

    private static void WriteJsonAtomically(string targetPath, object value)
    {
        TextOutput.WriteJsonAtomic(targetPath, value, trailingNewline: true);
    }

    private static async Task WriteMapManifestAsync(
        string outputPath,
        string? usmapHash,
        DateTimeOffset startedAt,
        string status,
        int totalLevels,
        int exportedCount,
        int skippedCount,
        IReadOnlyCollection<(string Path, string Error)> failures,
        long elapsedMs)
    {
        var manifest = new
        {
            cue4ParseVersion = Cue4ParseVersion,
            command = "export-maps",
            status,
            usmapSha256 = usmapHash,
            startedAt = startedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            finishedAt = DateTimeOffset.UtcNow.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            elapsedMs,
            totalLevels,
            exportedCount,
            skippedCount,
            successCount = exportedCount + skippedCount,
            failureCount = failures.Count,
            graphFile = "_graph.json",
            failures = failures.Select(f => new { path = f.Path, error = f.Error }).ToArray(),
        };

        await TextOutput.WriteJsonAtomicAsync(
            Path.Combine(outputPath, "manifest.json"),
            manifest,
            trailingNewline: true);
    }
}
