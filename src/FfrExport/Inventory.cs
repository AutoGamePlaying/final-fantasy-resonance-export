using System.Globalization;
using CUE4Parse.FileProvider;
using CUE4Parse.FileProvider.Objects;
using CUE4Parse.UE4.AssetRegistry;
using CUE4Parse.UE4.AssetRegistry.Objects;
using Newtonsoft.Json;

namespace FfrExport;

internal static partial class Program
{
    private const string RelativeInventoryDirectory = "ffr/inventory";
    private const string PrimaryAssetRegistryPath = "FFRS/AssetRegistry.bin";

    internal static string GetDefaultInventoryDirectory() => ResolveDefaultDepsPath(RelativeInventoryDirectory);

    internal sealed record AssetClassification(
        string Decision,
        string Category,
        string Reason);

    internal sealed record ResolvedRegistryPackage(
        string PackageName,
        string Path,
        string PrimaryClass,
        string[] AssetClasses,
        string AssetName,
        long PackageSize,
        long BulkSize,
        long TotalSize,
        SortedDictionary<string, string> Tags,
        int? DependencyCount,
        bool MatchedInVfs,
        GameFile? VfsFile,
        AssetClassification Classification);

    internal sealed record AssetRegistrySnapshot(
        string SourcePath,
        long SourceSize,
        string HeaderVersion,
        bool FilterEditorOnlyData,
        long BytesRead,
        long TotalBytes,
        int AssetDataCount,
        int DependsNodeCount,
        int PackageDataCount,
        int DistinctVfsFileCount,
        int DuplicateVfsEntryCount,
        List<ResolvedRegistryPackage> Packages,
        List<GameFile> UnindexedVfsPackages,
        List<GameFile> NonPackageVfsFiles,
        Dictionary<string, GameFile> VfsByPathIgnoreCase,
        List<string> Notes);

    private static async Task<int> RunInventoryAsync(string[] args)
    {
        var paks = GetDefaultPaks();
        var outputArgument = GetDefaultInventoryDirectory();
        var force = false;
        for (var index = 0; index < args.Length; index++)
        {
            switch (args[index])
            {
                case "--paks":
                    paks = RequireValue(args, ref index, "--paks");
                    break;
                case "--out":
                    outputArgument = RequireValue(args, ref index, "--out");
                    break;
                case "--force":
                    force = true;
                    break;
                default:
                    throw new ArgumentException($"inventory 收到未知参数：{args[index]}");
            }
        }

        var outputPath = RequireDepsPath(outputArgument, "--out");
        var startedAt = DateTimeOffset.UtcNow;

        using var provider = CreateProvider(paks, null);
        var snapshot = LoadAssetRegistrySnapshot(provider);

        PrepareOutputDirectory(outputPath, outputArgument, force, "inventory");
        await WriteInventoryOutputsAsync(snapshot, outputPath, outputArgument, startedAt);
        return 0;
    }

    private static async Task<AssetRegistrySnapshot> RunInventoryWithProviderAsync(
        DefaultFileProvider provider,
        string outputPath,
        string outputDisplay)
    {
        var startedAt = DateTimeOffset.UtcNow;
        var snapshot = LoadAssetRegistrySnapshot(provider);
        Directory.CreateDirectory(PathSupport.ToIoPath(outputPath));
        await WriteInventoryOutputsAsync(snapshot, outputPath, outputDisplay, startedAt);
        return snapshot;
    }

    private static async Task WriteInventoryOutputsAsync(
        AssetRegistrySnapshot snapshot,
        string outputPath,
        string outputArgument,
        DateTimeOffset startedAt)
    {
        var registryJsonlPath = PathSupport.ToIoPath(Path.Combine(outputPath, "registry.jsonl"));
        await using (var writer = new StreamWriter(registryJsonlPath, false, Utf8WithoutBom))
        {
            foreach (var pkg in snapshot.Packages)
            {
                var record = new
                {
                    path = pkg.Path,
                    packageName = pkg.PackageName,
                    primaryClass = pkg.PrimaryClass,
                    assetClasses = pkg.AssetClasses,
                    assetName = pkg.AssetName,
                    size = pkg.TotalSize,
                    packageSize = pkg.PackageSize,
                    bulkSize = pkg.BulkSize,
                    tags = pkg.Tags,
                    dependencyCount = pkg.DependencyCount,
                    matchedInVfs = pkg.MatchedInVfs,
                    decision = pkg.Classification.Decision,
                    category = pkg.Classification.Category,
                };
                await writer.WriteLineAsync(JsonConvert.SerializeObject(record, Formatting.None));
            }
        }

        var byClass = snapshot.Packages
            .GroupBy(pkg => pkg.PrimaryClass, StringComparer.Ordinal)
            .Select(group =>
            {
                var classification = ClassifyAssetClass(group.Key);
                return new
                {
                    className = group.Key,
                    count = group.Count(),
                    packageSize = group.Sum(item => item.PackageSize),
                    bulkSize = group.Sum(item => item.BulkSize),
                    totalSize = group.Sum(item => item.TotalSize),
                    decision = classification.Decision,
                    category = classification.Category,
                    reason = classification.Reason,
                };
            })
            .OrderByDescending(item => item.count)
            .ThenBy(item => item.className, StringComparer.Ordinal)
            .ToArray();

        var byDirectory = snapshot.Packages
            .GroupBy(pkg => ExtractTopDirectory(pkg.Path), StringComparer.Ordinal)
            .Select(group => new
            {
                directory = group.Key,
                count = group.Count(),
                packageSize = group.Sum(item => item.PackageSize),
                bulkSize = group.Sum(item => item.BulkSize),
                totalSize = group.Sum(item => item.TotalSize),
            })
            .OrderByDescending(item => item.count)
            .ThenBy(item => item.directory, StringComparer.Ordinal)
            .ToArray();

        var byDecision = snapshot.Packages
            .GroupBy(pkg => pkg.Classification.Decision, StringComparer.Ordinal)
            .Select(group => new
            {
                decision = group.Key,
                count = group.Count(),
                packageSize = group.Sum(item => item.PackageSize),
                bulkSize = group.Sum(item => item.BulkSize),
                totalSize = group.Sum(item => item.TotalSize),
            })
            .OrderByDescending(item => item.count)
            .ThenBy(item => item.decision, StringComparer.Ordinal)
            .ToArray();

        var nonPackageByExtension = snapshot.NonPackageVfsFiles
            .GroupBy(file => NormalizeExtension(file.Path), StringComparer.Ordinal)
            .Select(group =>
            {
                var sampleDecision = ClassifyNonPackageFile(group.First().Path);
                return new
                {
                    extension = group.Key,
                    count = group.Count(),
                    totalSize = group.Sum(item => item.Size),
                    ffrsCount = group.Count(item => item.Path.StartsWith("FFRS/", StringComparison.OrdinalIgnoreCase)),
                    ffrsSize = group
                        .Where(item => item.Path.StartsWith("FFRS/", StringComparison.OrdinalIgnoreCase))
                        .Sum(item => item.Size),
                    decision = sampleDecision.Decision,
                    category = sampleDecision.Category,
                };
            })
            .OrderByDescending(item => item.count)
            .ThenBy(item => item.extension, StringComparer.Ordinal)
            .ToArray();

        var unindexedPackages = snapshot.UnindexedVfsPackages
            .Select(file => new { path = file.Path, size = file.Size })
            .ToArray();

        var finishedAt = DateTimeOffset.UtcNow;
        var summary = new
        {
            cue4ParseVersion = Cue4ParseVersion,
            command = "inventory",
            startedAt = startedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            finishedAt = finishedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            assetRegistry = new
            {
                sourcePath = snapshot.SourcePath,
                sourceSize = snapshot.SourceSize,
                headerVersion = snapshot.HeaderVersion,
                filterEditorOnlyData = snapshot.FilterEditorOnlyData,
                bytesRead = snapshot.BytesRead,
                totalBytes = snapshot.TotalBytes,
                unparsedBytes = Math.Max(0L, snapshot.TotalBytes - snapshot.BytesRead),
                assetDataCount = snapshot.AssetDataCount,
                packageCount = snapshot.Packages.Count,
                dependsNodeCount = snapshot.DependsNodeCount,
                packageDataCount = snapshot.PackageDataCount,
                dependencyGraphAvailable = snapshot.DependsNodeCount > 0,
                matchedVfsPackages = snapshot.Packages.Count(pkg => pkg.MatchedInVfs),
                unmatchedRegistryPackages = snapshot.Packages.Count(pkg => !pkg.MatchedInVfs),
                unindexedVfsPackages = unindexedPackages.Length,
            },
            vfsOverview = new
            {
                distinctFileCount = snapshot.DistinctVfsFileCount,
                duplicateEntryCount = snapshot.DuplicateVfsEntryCount,
                distinctTotalSize = snapshot.VfsByPathIgnoreCase.Values.Sum(file => file.Size),
                ffrsDistinctFileCount = snapshot.VfsByPathIgnoreCase.Values.Count(file =>
                    file.Path.StartsWith("FFRS/", StringComparison.OrdinalIgnoreCase)),
                ffrsDistinctTotalSize = snapshot.VfsByPathIgnoreCase.Values
                    .Where(file => file.Path.StartsWith("FFRS/", StringComparison.OrdinalIgnoreCase))
                    .Sum(file => file.Size),
                nonPackageByExtension,
            },
            byDecision,
            byClass,
            byDirectory,
            unindexedPackages,
            unparsedOrLimitations = snapshot.Notes.ToArray(),
        };

        await File.WriteAllTextAsync(
            PathSupport.ToIoPath(Path.Combine(outputPath, "summary.json")),
            JsonConvert.SerializeObject(summary, Formatting.Indented),
            Utf8WithoutBom);

        var manifest = new
        {
            cue4ParseVersion = Cue4ParseVersion,
            command = "inventory",
            startedAt = startedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            finishedAt = finishedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            packageCount = snapshot.Packages.Count,
            classCount = byClass.Length,
            unmatchedRegistryPackages = snapshot.Packages.Count(pkg => !pkg.MatchedInVfs),
            unindexedVfsPackages = unindexedPackages.Length,
        };
        await File.WriteAllTextAsync(
            PathSupport.ToIoPath(Path.Combine(outputPath, "manifest.json")),
            JsonConvert.SerializeObject(manifest, Formatting.Indented),
            Utf8WithoutBom);

        Console.WriteLine(
            $"已索引 {snapshot.Packages.Count} 个包（{byClass.Length} 个主类，{snapshot.AssetDataCount} 条 AssetData），写到 {outputArgument}");
        foreach (var note in snapshot.Notes)
        {
            Console.WriteLine($"说明：{note}");
        }
    }

    internal static AssetRegistrySnapshot LoadAssetRegistrySnapshot(DefaultFileProvider provider)
    {
        var (vfsByPathCI, duplicateCount) = BuildDeduplicatedVfsIndex(provider);
        var pluginContentDirs = BuildPluginContentDirectoryMap(vfsByPathCI);

        var arFile = FindAssetRegistryFile(provider, vfsByPathCI);
        if (arFile is null)
        {
            throw new InvalidDataException($"找不到资源注册表文件（期望 {PrimaryAssetRegistryPath}）");
        }

        string headerVersion;
        bool filterEditorOnlyData;
        using (var headerReader = arFile.CreateReader())
        {
            var header = new FAssetRegistryHeader(headerReader);
            headerVersion = header.Version.ToString();
            filterEditorOnlyData = header.bFilterEditorOnlyData;
        }

        FAssetRegistryState state;
        long bytesRead;
        long totalBytes;
        using (var stateReader = arFile.CreateReader())
        {
            state = new FAssetRegistryState(stateReader);
            bytesRead = stateReader.Position;
            totalBytes = stateReader.Length;
        }

        var notes = new List<string>();
        if (bytesRead < totalBytes)
        {
            notes.Add($"AssetRegistry.bin 末尾有 {totalBytes - bytesRead} 字节未被 FAssetRegistryState 读取");
        }

        var dependsByPackage = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
        if (state.PreallocatedDependsNodeDataBuffers.Length > 0)
        {
            foreach (var node in state.PreallocatedDependsNodeDataBuffers)
            {
                var pkgName = node.Identifier?.PackageName.Text;
                if (string.IsNullOrEmpty(pkgName))
                {
                    continue;
                }

                var depCount = node.PackageDependencies?.Length ?? 0;
                if (!dependsByPackage.TryGetValue(pkgName, out var existing) || depCount > existing)
                {
                    dependsByPackage[pkgName] = depCount;
                }
            }
        }
        else
        {
            notes.Add(
                $"AssetRegistry.bin 中 DependsNodes 为 0（HeaderVersion={headerVersion}，bFilterEditorOnlyData={filterEditorOnlyData}），" +
                "cooked 构建已剥离依赖图，registry.jsonl 的 dependencyCount 记为 null");
        }

        if (state.PreallocatedPackageDataBuffers.Length == 0)
        {
            notes.Add("AssetRegistry.bin 中 PackageData 为 0（cooked 构建已剥离），包大小改由 VFS 实测（含伴随 .ubulk）");
        }

        if (duplicateCount > 0)
        {
            notes.Add($"VFS 中存在 {duplicateCount} 个被补丁包（*_P.pak/.utoc）覆写的同名条目，索引已按有效挂载版本去重");
        }

        var matchedVfsPaths = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var packages = new List<ResolvedRegistryPackage>();

        foreach (var group in state.PreallocatedAssetDataBuffers
                     .GroupBy(asset => asset.PackageName.Text, StringComparer.Ordinal))
        {
            var assets = group.ToArray();
            var primary = SelectPrimaryAsset(assets);
            var primaryClass = primary.AssetClass.Text;
            var assetClasses = assets
                .Select(asset => asset.AssetClass.Text)
                .Distinct(StringComparer.Ordinal)
                .OrderBy(name => name, StringComparer.Ordinal)
                .ToArray();

            var tags = MergeAssetTags(primary, assets);
            var vfsFile = ResolveVfsPackageFile(group.Key, vfsByPathCI, pluginContentDirs);
            var resolvedPath = vfsFile?.Path ?? GuessVirtualPath(group.Key, pluginContentDirs);
            long packageSize = 0;
            long bulkSize = 0;
            if (vfsFile is not null)
            {
                matchedVfsPaths.Add(vfsFile.Path);
                packageSize = vfsFile.Size;
                bulkSize = GetCompanionBulkSize(vfsFile.Path, vfsByPathCI);
            }

            int? dependencyCount = state.PreallocatedDependsNodeDataBuffers.Length > 0
                ? (dependsByPackage.TryGetValue(group.Key, out var count) ? count : 0)
                : null;

            packages.Add(new ResolvedRegistryPackage(
                PackageName: group.Key,
                Path: resolvedPath,
                PrimaryClass: primaryClass,
                AssetClasses: assetClasses,
                AssetName: primary.AssetName.Text,
                PackageSize: packageSize,
                BulkSize: bulkSize,
                TotalSize: packageSize + bulkSize,
                Tags: tags,
                DependencyCount: dependencyCount,
                MatchedInVfs: vfsFile is not null,
                VfsFile: vfsFile,
                Classification: ClassifyAssetClass(primaryClass)));
        }

        packages.Sort((left, right) => string.CompareOrdinal(left.Path, right.Path));

        var unindexedVfsPackages = new List<GameFile>();
        var nonPackageVfsFiles = new List<GameFile>();
        foreach (var file in vfsByPathCI.Values.OrderBy(file => file.Path, StringComparer.Ordinal))
        {
            if (IsPackageFile(file.Path))
            {
                if (!matchedVfsPaths.Contains(file.Path))
                {
                    unindexedVfsPackages.Add(file);
                }
            }
            else
            {
                nonPackageVfsFiles.Add(file);
            }
        }

        var unmatchedCount = packages.Count(pkg => !pkg.MatchedInVfs);
        if (unmatchedCount > 0)
        {
            notes.Add($"AssetRegistry 中有 {unmatchedCount} 个包未在当前 VFS 中找到对应 .uasset/.umap 文件");
        }

        if (unindexedVfsPackages.Count > 0)
        {
            notes.Add($"VFS 中有 {unindexedVfsPackages.Count} 个 .uasset/.umap 未出现在 AssetRegistry.bin 中");
        }

        return new AssetRegistrySnapshot(
            SourcePath: arFile.Path,
            SourceSize: arFile.Size,
            HeaderVersion: headerVersion,
            FilterEditorOnlyData: filterEditorOnlyData,
            BytesRead: bytesRead,
            TotalBytes: totalBytes,
            AssetDataCount: state.PreallocatedAssetDataBuffers.Length,
            DependsNodeCount: state.PreallocatedDependsNodeDataBuffers.Length,
            PackageDataCount: state.PreallocatedPackageDataBuffers.Length,
            DistinctVfsFileCount: vfsByPathCI.Count,
            DuplicateVfsEntryCount: duplicateCount,
            Packages: packages,
            UnindexedVfsPackages: unindexedVfsPackages,
            NonPackageVfsFiles: nonPackageVfsFiles,
            VfsByPathIgnoreCase: vfsByPathCI,
            Notes: notes);
    }

    internal static (Dictionary<string, GameFile> FilesByPathCI, int DuplicateCount) BuildDeduplicatedVfsIndex(
        DefaultFileProvider provider)
    {
        var filesByPathCI = new Dictionary<string, GameFile>(StringComparer.OrdinalIgnoreCase);
        var totalEntries = 0;
        var seenExactPaths = new HashSet<string>(StringComparer.Ordinal);
        foreach (var entry in provider.Files.Values)
        {
            totalEntries++;
            if (!seenExactPaths.Add(entry.Path))
            {
                continue;
            }

            // Use provider.TryGetGameFile so mounted patch paks take precedence over base paks.
            var effective = provider.TryGetGameFile(entry.Path, out var active) && active is not null
                ? active
                : entry;

            if (!filesByPathCI.ContainsKey(effective.Path))
            {
                filesByPathCI[effective.Path] = effective;
            }
        }

        var duplicateCount = Math.Max(0, totalEntries - filesByPathCI.Count);
        return (filesByPathCI, duplicateCount);
    }

    internal static Dictionary<string, string> BuildPluginContentDirectoryMap(
        Dictionary<string, GameFile> vfsByPathCI)
    {
        var pluginDirs = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (var file in vfsByPathCI.Values)
        {
            if (file.Path.EndsWith(".uplugin", StringComparison.OrdinalIgnoreCase))
            {
                var pluginName = Path.GetFileNameWithoutExtension(file.Path);
                var lastSlash = file.Path.LastIndexOf('/');
                if (lastSlash > 0 && !string.IsNullOrEmpty(pluginName))
                {
                    pluginDirs[pluginName] = file.Path[..lastSlash] + "/Content/";
                }
            }
        }

        // Fallback: infer plugin Content roots from any .../Plugins/.../<Plugin>/Content/ path.
        foreach (var file in vfsByPathCI.Values)
        {
            var contentIndex = file.Path.IndexOf("/Content/", StringComparison.OrdinalIgnoreCase);
            if (contentIndex <= 0)
            {
                continue;
            }

            var prefix = file.Path[..contentIndex];
            if (!prefix.Contains("/Plugins/", StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }

            var lastSlash = prefix.LastIndexOf('/');
            var pluginName = lastSlash >= 0 ? prefix[(lastSlash + 1)..] : prefix;
            if (!string.IsNullOrEmpty(pluginName) && !pluginDirs.ContainsKey(pluginName))
            {
                pluginDirs[pluginName] = prefix + "/Content/";
            }
        }

        return pluginDirs;
    }

    private static GameFile? FindAssetRegistryFile(
        DefaultFileProvider provider,
        Dictionary<string, GameFile> vfsByPathCI)
    {
        if (provider.TryGetGameFile(PrimaryAssetRegistryPath, out var primary) && primary is not null)
        {
            return primary;
        }

        if (vfsByPathCI.TryGetValue(PrimaryAssetRegistryPath, out var ciMatch))
        {
            return ciMatch;
        }

        return vfsByPathCI.Values
            .OrderBy(file => file.Path, StringComparer.Ordinal)
            .FirstOrDefault(file =>
                file.Path.Equals("AssetRegistry.bin", StringComparison.OrdinalIgnoreCase)
                || file.Path.EndsWith("/AssetRegistry.bin", StringComparison.OrdinalIgnoreCase));
    }

    private static FAssetData SelectPrimaryAsset(FAssetData[] assets)
    {
        if (assets.Length == 1)
        {
            return assets[0];
        }

        string[] priorityOrder =
        [
            "World",
            "LevelSequence",
            "WidgetBlueprintGeneratedClass",
            "AnimBlueprintGeneratedClass",
            "BlueprintGeneratedClass",
        ];

        foreach (var preferredClass in priorityOrder)
        {
            foreach (var asset in assets)
            {
                if (string.Equals(asset.AssetClass.Text, preferredClass, StringComparison.Ordinal))
                {
                    return asset;
                }
            }
        }

        var packageLeaf = assets[0].PackageName.Text.Split('/').LastOrDefault();
        foreach (var asset in assets)
        {
            if (string.Equals(asset.AssetName.Text, packageLeaf, StringComparison.OrdinalIgnoreCase))
            {
                return asset;
            }
        }

        return assets[0];
    }

    private static SortedDictionary<string, string> MergeAssetTags(FAssetData primary, FAssetData[] assets)
    {
        var merged = new SortedDictionary<string, string>(StringComparer.Ordinal);
        if (primary.TagsAndValues is not null)
        {
            foreach (var (key, value) in primary.TagsAndValues)
            {
                merged[key.Text] = value ?? string.Empty;
            }
        }

        foreach (var asset in assets)
        {
            if (ReferenceEquals(asset, primary) || asset.TagsAndValues is null)
            {
                continue;
            }

            foreach (var (key, value) in asset.TagsAndValues)
            {
                if (!merged.ContainsKey(key.Text))
                {
                    merged[key.Text] = value ?? string.Empty;
                }
            }
        }

        return merged;
    }

    private static GameFile? ResolveVfsPackageFile(
        string packageName,
        Dictionary<string, GameFile> vfsByPathCI,
        Dictionary<string, string> pluginContentDirs)
    {
        var basePath = MapPackageNameToVfsBase(packageName, pluginContentDirs);
        if (basePath is null)
        {
            return null;
        }

        if (vfsByPathCI.TryGetValue(basePath + ".uasset", out var uasset))
        {
            return uasset;
        }

        if (vfsByPathCI.TryGetValue(basePath + ".umap", out var umap))
        {
            return umap;
        }

        return null;
    }

    private static string GuessVirtualPath(
        string packageName,
        Dictionary<string, string> pluginContentDirs)
    {
        var basePath = MapPackageNameToVfsBase(packageName, pluginContentDirs);
        return basePath is not null ? basePath + ".uasset" : packageName.TrimStart('/');
    }

    private static string? MapPackageNameToVfsBase(
        string packageName,
        Dictionary<string, string> pluginContentDirs)
    {
        var trimmed = packageName.TrimStart('/');
        var slash = trimmed.IndexOf('/');
        if (slash <= 0)
        {
            return null;
        }

        var root = trimmed[..slash];
        var rest = trimmed[(slash + 1)..];
        if (root.Equals("Game", StringComparison.OrdinalIgnoreCase))
        {
            return "FFRS/Content/" + rest;
        }

        if (root.Equals("Engine", StringComparison.OrdinalIgnoreCase))
        {
            return "Engine/Content/" + rest;
        }

        if (pluginContentDirs.TryGetValue(root, out var contentDir))
        {
            return contentDir + rest;
        }

        return null;
    }

    private static long GetCompanionBulkSize(string packagePath, Dictionary<string, GameFile> vfsByPathCI)
    {
        var ext = Path.GetExtension(packagePath);
        if (string.IsNullOrEmpty(ext))
        {
            return 0;
        }

        var basePath = packagePath[..^ext.Length];
        long total = 0;
        if (vfsByPathCI.TryGetValue(basePath + ".ubulk", out var ubulk))
        {
            total += ubulk.Size;
        }

        if (vfsByPathCI.TryGetValue(basePath + ".uptnl", out var uptnl))
        {
            total += uptnl.Size;
        }

        return total;
    }

    internal static bool IsPackageFile(string path)
    {
        return path.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase)
            || path.EndsWith(".umap", StringComparison.OrdinalIgnoreCase);
    }

    internal static string ExtractTopDirectory(string path)
    {
        var parts = path.Split('/', StringSplitOptions.RemoveEmptyEntries);
        if (parts.Length >= 3)
        {
            return $"{parts[0]}/{parts[1]}/{parts[2]}";
        }

        if (parts.Length == 2)
        {
            return $"{parts[0]}/{parts[1]}";
        }

        return parts.Length == 1 ? parts[0] : "(root)";
    }

    internal static string NormalizeExtension(string path)
    {
        var ext = Path.GetExtension(path);
        return string.IsNullOrEmpty(ext) ? "(none)" : ext.ToLowerInvariant();
    }

    // Classifies an Unreal AssetClass name into export / skip / undecided decisions.
    internal static AssetClassification ClassifyAssetClass(string className)
    {
        if (string.IsNullOrWhiteSpace(className))
        {
            return new AssetClassification("undecided", "unknown", "类名为空，归为待定");
        }

        // 1. Levels (handled separately by task 23b `export-maps`).
        if (string.Equals(className, "World", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_level", "level", "关卡资源（.umap），由 export-maps 任务单独导出");
        }

        // 2. Images, textures, LUT texture atlases, Paper2D sprites, and SpriteStudio 2D projects.
        if (className.StartsWith("Texture", StringComparison.Ordinal)
            || className.EndsWith("Texture", StringComparison.Ordinal)
            || string.Equals(className, "CurveLinearColorAtlas", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_image", "image_texture", "贴图/图像资源（含 LUT 贴图图集），按要求跳过");
        }

        if (className.StartsWith("Paper", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_image", "image_paper2d", "Paper2D 精灵/贴图资源，按要求跳过");
        }

        if (className.StartsWith("Ss6", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_image", "image_spritestudio", "SpriteStudio 6 2D 精灵贴图与帧动画项目，按要求跳过");
        }

        // 3. Large binary / render / audio / video / animation / VFX / material / font assets.
        if (string.Equals(className, "MapBuildDataRegistry", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_binary", "level_build_data", "关卡烘焙光照与可见性构建数据（体积巨大），跳过");
        }

        if (string.Equals(className, "StaticMesh", StringComparison.Ordinal)
            || string.Equals(className, "SkeletalMesh", StringComparison.Ordinal)
            || string.Equals(className, "GeometryCollection", StringComparison.Ordinal)
            || string.Equals(className, "LandscapeLayerInfoObject", StringComparison.Ordinal)
            || string.Equals(className, "ControlRigShapeLibrary", StringComparison.Ordinal)
            || className.StartsWith("FoliageType_", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_binary", "mesh_geometry", "三维网格/几何体/植被分布资源，跳过");
        }

        if (className.StartsWith("Anim", StringComparison.Ordinal)
            || string.Equals(className, "Skeleton", StringComparison.Ordinal)
            || string.Equals(className, "PhysicsAsset", StringComparison.Ordinal)
            || string.Equals(className, "OptimusDeformer", StringComparison.Ordinal)
            || string.Equals(className, "VariableFrameStrippingSettings", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_binary", "animation_rigging", "骨骼、动画序列/蓝图与物理蒙皮资源，跳过");
        }

        if (className.StartsWith("Sound", StringComparison.Ordinal)
            || className.StartsWith("SubmixEffect", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_binary", "audio", "音频与混音效果资源，跳过");
        }

        if (string.Equals(className, "FileManaMovie", StringComparison.Ordinal)
            || string.Equals(className, "PlatformManaMovie", StringComparison.Ordinal)
            || string.Equals(className, "ManaPlayer", StringComparison.Ordinal)
            || className.Contains("MediaSource", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_binary", "video", "视频与媒体播放源资源，跳过");
        }

        if (className.StartsWith("Niagara", StringComparison.Ordinal)
            || string.Equals(className, "ParticleSystem", StringComparison.Ordinal)
            || string.Equals(className, "VectorFieldStatic", StringComparison.Ordinal)
            || string.Equals(className, "WaterWavesAsset", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_binary", "vfx", "粒子/Niagara 特效与水面波形资源，跳过");
        }

        if (className.StartsWith("Material", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_binary", "material", "材质与材质函数/实例资源，跳过");
        }

        if (className.StartsWith("NNE", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_binary", "render_denoiser", "神经网络渲染降噪模型与配置，跳过");
        }

        if (string.Equals(className, "Font", StringComparison.Ordinal)
            || string.Equals(className, "FontFace", StringComparison.Ordinal))
        {
            return new AssetClassification("skip_binary", "font", "字体资源，跳过");
        }

        // 4. Blueprints: export CDO + component defaults + WidgetTree, excluding node graphs & functions.
        if (string.Equals(className, "WidgetBlueprintGeneratedClass", StringComparison.Ordinal)
            || string.Equals(className, "WidgetBlueprint", StringComparison.Ordinal))
        {
            return new AssetClassification(
                "export_blueprint_cdo",
                "widget_blueprint",
                "UI 控件蓝图：导出类元数据、CDO 默认属性与 WidgetTree 控件树（不含节点图/函数/UI动画轨）");
        }

        if (string.Equals(className, "BlueprintGeneratedClass", StringComparison.Ordinal)
            || string.Equals(className, "Blueprint", StringComparison.Ordinal))
        {
            return new AssetClassification(
                "export_blueprint_cdo",
                "blueprint",
                "蓝图类：导出类元数据、类默认对象（CDO）属性与组件默认值（不含节点图与函数）");
        }

        // 5. Pure data, curves, input, sequencer, gameplay tags, data assets, and schemas.
        if (string.Equals(className, "DataTable", StringComparison.Ordinal)
            || string.Equals(className, "CompositeDataTable", StringComparison.Ordinal)
            || string.Equals(className, "StringTable", StringComparison.Ordinal))
        {
            return new AssetClassification("export", "datatable", "数据表/复合数据表/字符串表");
        }

        if (className.StartsWith("Curve", StringComparison.Ordinal))
        {
            return new AssetClassification("export", "curve", "曲线表与数值/颜色曲线");
        }

        if (string.Equals(className, "InputAction", StringComparison.Ordinal)
            || string.Equals(className, "InputMappingContext", StringComparison.Ordinal)
            || string.Equals(className, "PlayerMappableInputConfig", StringComparison.Ordinal)
            || className.StartsWith("InputModifier", StringComparison.Ordinal)
            || className.StartsWith("InputTrigger", StringComparison.Ordinal))
        {
            return new AssetClassification("export", "input", "Enhanced Input 输入动作与映射上下文（R4 用）");
        }

        if (string.Equals(className, "LevelSequence", StringComparison.Ordinal))
        {
            return new AssetClassification("export", "sequencer", "过场与事件演出结构（轨道、绑定、事件与对话引用）");
        }

        if (className.StartsWith("GameplayTag", StringComparison.Ordinal)
            || className.StartsWith("RestrictedGameplayTag", StringComparison.Ordinal))
        {
            return new AssetClassification("export", "gameplay_tags", "GameplayTags 定义与表");
        }

        if (string.Equals(className, "DataAsset", StringComparison.Ordinal)
            || string.Equals(className, "PrimaryDataAsset", StringComparison.Ordinal)
            || string.Equals(className, "PrimaryAssetLabel", StringComparison.Ordinal)
            || string.Equals(className, "CPP_DM_EventShortcutData", StringComparison.Ordinal)
            || className.StartsWith("CPP_DA_", StringComparison.Ordinal)
            || className.StartsWith("CPP_DMI_DA_", StringComparison.Ordinal)
            || className.StartsWith("DA_", StringComparison.Ordinal)
            || className.EndsWith("DataAsset", StringComparison.Ordinal))
        {
            return new AssetClassification("export", "data_asset", "DataAsset 及其游戏自定义子类（纯数据配置）");
        }

        if (string.Equals(className, "UserDefinedEnum", StringComparison.Ordinal)
            || string.Equals(className, "UserDefinedStruct", StringComparison.Ordinal))
        {
            return new AssetClassification("export", "schema", "蓝图自定义枚举/结构体定义");
        }

        // 6. Anything not clearly matched is marked undecided for manual review.
        return new AssetClassification("undecided", "undecided", "归类待定，待审阅后决定是否纳入导出");
    }

    internal static AssetClassification ClassifyNonPackageFile(string path)
    {
        var ext = NormalizeExtension(path);
        if (ext == ".uplugin" || ext == ".uproject")
        {
            return new AssetClassification("copy_raw", "config_project", "工程/插件清单文件，原文件照拷");
        }

        if (ext == ".ini" && path.Contains("/Config/", StringComparison.OrdinalIgnoreCase))
        {
            return new AssetClassification("copy_raw", "config_ini", "Config/*.ini 配置文件（含输入、GameplayTags、引擎配置），原文件照拷");
        }

        if (ext == ".png" || ext == ".svg")
        {
            return new AssetClassification("skip_image", "image_raw", "原始图像文件，按要求跳过");
        }

        if (ext == ".locres" || ext == ".locmeta")
        {
            return new AssetClassification("skip_binary", "localization_handled", "多语言编译资源，已由 locres 子命令单独导出");
        }

        if (ext == ".ubulk" || ext == ".uptnl")
        {
            return new AssetClassification("skip_binary", "bulk_payload", "伴随二进制载荷（主要是贴图/网格 mip 数据），跳过");
        }

        if (ext == ".usm")
        {
            return new AssetClassification("skip_binary", "video_raw", "CriWare USM 视频流，跳过");
        }

        if (ext == ".awb")
        {
            return new AssetClassification("skip_binary", "audio_raw", "CriWare AWB 音频流，跳过");
        }

        if (ext == ".ufont" || ext == ".ttf")
        {
            return new AssetClassification("skip_binary", "font_raw", "字体二进制文件，跳过");
        }

        if (ext == ".ushaderbytecode" || ext == ".stinfo" || ext == ".upipelinecache" || ext == ".bin")
        {
            return new AssetClassification("skip_binary", "shader_or_registry_bin", "着色器缓存或二进制注册表（AssetRegistry 由 inventory 解析），跳过");
        }

        return new AssetClassification("skip_binary", "other_binary", "其他非数据/运行时二进制文件，跳过");
    }
}
