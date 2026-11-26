using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;
using System.Security.Cryptography;
using CUE4Parse.FileProvider.Objects;
using CUE4Parse.UE4.Assets;
using CUE4Parse.UE4.Assets.Exports;
using Newtonsoft.Json.Linq;

namespace FfrExport;

internal static partial class Program
{
    private const string RelativeFullExportDirectory = "ffr/export-full";
    private static readonly string[] DefaultFullExportPrefixes = ["FFRS/"];

    internal static string GetDefaultFullExportDirectory() => ResolveDefaultDepsPath(RelativeFullExportDirectory);

    private sealed class ClassExportTracker
    {
        public required string ClassName { get; init; }
        public required string Decision { get; init; }
        public required string Category { get; init; }
        public required string Reason { get; init; }
        public int TotalCount;
        public int ExportedCount;
        public int NewlyExportedCount;
        public int ResumedCount;
        public int FailedCount;
        public int SkippedCount;
        public long SourceBytes;
        public long OutputBytes;
    }

    private static async Task<int> RunExportAssetsAsync(string[] args)
    {
        var paks = GetDefaultPaks();
        var outputArgument = GetDefaultFullExportDirectory();
        string? usmapPath = null;
        var force = false;
        var jobs = Math.Clamp(Environment.ProcessorCount, 1, 8);
        var prefixes = new List<string>();
        var classifyOnly = new List<string>();

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
                case "--jobs":
                    var rawJobs = RequireValue(args, ref index, "--jobs");
                    if (!int.TryParse(rawJobs, NumberStyles.Integer, CultureInfo.InvariantCulture, out jobs) || jobs < 1)
                    {
                        throw new ArgumentException(L.Format("ExportAssets_JobsMustBePositive", rawJobs));
                    }
                    break;
                case "--force":
                    force = true;
                    break;
                case "--classify":
                    AddCommaValues(classifyOnly, RequireValue(args, ref index, "--classify"));
                    break;
                default:
                    throw new ArgumentException(L.Format("Cli_UnknownArg", "export-assets", args[index]));
            }
        }

        if (classifyOnly.Count > 0)
        {
            var results = classifyOnly.Select(className =>
            {
                var c = ClassifyAssetClass(className);
                return new
                {
                    className,
                    decision = c.Decision,
                    category = c.Category,
                    reason = c.Reason,
                };
            }).ToArray();
            Console.WriteLine(TextOutput.SerializeJson(results));
            return 0;
        }

        if (usmapPath is null)
        {
            throw new ArgumentException(L.Format("Cli_CommandRequiresUsmap", "export-assets"));
        }

        if (!File.Exists(usmapPath))
        {
            throw new FileNotFoundException(L.Format("Cli_MappingFileNotFound", usmapPath));
        }

        if (prefixes.Count == 0)
        {
            prefixes.AddRange(DefaultFullExportPrefixes);
        }

        var outputPath = RequireDepsPath(outputArgument, "--out");
        var usmapHash = Convert.ToHexString(SHA256.HashData(await File.ReadAllBytesAsync(PathSupport.ToIoPath(usmapPath))));

        PrepareResumableOutputDirectory(outputPath, outputArgument, force, "export-assets");

        using var provider = CreateProvider(paks, usmapPath);
        return await RunExportAssetsWithProviderAsync(provider, usmapHash, outputPath, prefixes, jobs, force);
    }

    private static async Task<int> RunExportAssetsWithProviderAsync(
        DeterministicFileProvider provider,
        string usmapHash,
        string outputPath,
        IReadOnlyList<string> prefixes,
        int jobs,
        bool force,
        AssetRegistrySnapshot? preloadedSnapshot = null)
    {
        provider.EnsureReadyForDeterministicLoads();
        var startedAt = DateTimeOffset.UtcNow;
        var stopwatch = Stopwatch.StartNew();

        // Write an in-progress manifest sentinel immediately so an interrupted run can safely resume.
        await WriteManifestSentinelAsync(outputPath, usmapHash, prefixes, startedAt);

        var snapshot = preloadedSnapshot ?? LoadAssetRegistrySnapshot(provider);

        var matchedPackages = snapshot.Packages
            .Where(pkg => pkg.MatchedInVfs && MatchesAnyPrefix(pkg.Path, prefixes))
            .ToArray();

        var trackers = new ConcurrentDictionary<string, ClassExportTracker>(StringComparer.Ordinal);
        var toExport = new List<ResolvedRegistryPackage>();

        foreach (var pkg in matchedPackages)
        {
            var tracker = trackers.GetOrAdd(pkg.PrimaryClass, _ => new ClassExportTracker
            {
                ClassName = pkg.PrimaryClass,
                Decision = pkg.Classification.Decision,
                Category = pkg.Classification.Category,
                Reason = pkg.Classification.Reason,
            });

            Interlocked.Increment(ref tracker.TotalCount);
            Interlocked.Add(ref tracker.SourceBytes, pkg.TotalSize);

            if (pkg.Classification.Decision is "export" or "export_blueprint_cdo")
            {
                toExport.Add(pkg);
            }
            else
            {
                Interlocked.Increment(ref tracker.SkippedCount);
            }
        }

        var rawToCopy = new List<GameFile>();
        var skippedNonPackages = new List<GameFile>();
        foreach (var file in snapshot.NonPackageVfsFiles.Where(file => MatchesAnyPrefix(file.Path, prefixes)))
        {
            var classification = ClassifyNonPackageFile(file.Path);
            if (classification.Decision == "copy_raw")
            {
                rawToCopy.Add(file);
            }
            else
            {
                skippedNonPackages.Add(file);
            }
        }

        var failures = new ConcurrentBag<(string Path, string PrimaryClass, string Error)>();
        var readLock = provider.LoadSyncRoot;

        // Export eligible packages in parallel. Provider package loads (including lazy external
        // FPackageId imports resolved during JSON serialization) share provider.LoadSyncRoot while
        // JSON formatting and disk writes run concurrently across worker threads.
        Parallel.ForEach(
            toExport,
            new ParallelOptions { MaxDegreeOfParallelism = jobs },
            pkg =>
            {
                var tracker = trackers[pkg.PrimaryClass];
                var targetPath = BuildExportJsonPath(outputPath, pkg.Path);
                var ioTargetPath = PathSupport.ToIoPath(targetPath);
                if (!force && File.Exists(ioTargetPath))
                {
                    var existingSize = new FileInfo(ioTargetPath).Length;
                    Interlocked.Increment(ref tracker.ExportedCount);
                    Interlocked.Increment(ref tracker.ResumedCount);
                    Interlocked.Add(ref tracker.OutputBytes, existingSize);
                    return;
                }

                try
                {
                    UObject[] exports;
                    lock (readLock)
                    {
                        if (!provider.TryLoadPackage(pkg.Path, out IPackage? loaded) || loaded is null)
                        {
                            throw new InvalidDataException("CUE4Parse 无法加载此包");
                        }

                        exports = FilterExportsForClass(loaded.GetExports(), pkg.PrimaryClass, pkg.Classification.Decision);
                    }

                    var bytesWritten = TextOutput.WriteJsonAtomic(targetPath, exports);
                    Interlocked.Increment(ref tracker.ExportedCount);
                    Interlocked.Increment(ref tracker.NewlyExportedCount);
                    Interlocked.Add(ref tracker.OutputBytes, bytesWritten);
                }
                catch (Exception exception)
                {
                    Interlocked.Increment(ref tracker.FailedCount);
                    failures.Add((pkg.Path, pkg.PrimaryClass, TruncateError(exception.Message)));
                    Console.Error.WriteLine(L.Format("ExportAssets_ExportFailedLine", pkg.Path, pkg.PrimaryClass, exception.Message));
                }
            });

        // Copy Config/*.ini, .uplugin, .uproject verbatim.
        var rawCopiedCount = 0;
        var rawNewlyCopiedCount = 0;
        var rawResumedCount = 0;
        var rawFailedCount = 0;
        long rawSourceBytes = 0;
        long rawOutputBytes = 0;
        var rawByExtension = new ConcurrentDictionary<string, (int Count, long Bytes)>(StringComparer.Ordinal);

        Parallel.ForEach(
            rawToCopy,
            new ParallelOptions { MaxDegreeOfParallelism = jobs },
            file =>
            {
                var targetPath = Path.Combine(outputPath, file.Path.Replace('/', Path.DirectorySeparatorChar));
                var ioTargetPath = PathSupport.ToIoPath(targetPath);
                var ext = NormalizeExtension(file.Path);
                if (!force && File.Exists(ioTargetPath))
                {
                    var existingSize = new FileInfo(ioTargetPath).Length;
                    Interlocked.Increment(ref rawCopiedCount);
                    Interlocked.Increment(ref rawResumedCount);
                    Interlocked.Add(ref rawSourceBytes, file.Size);
                    Interlocked.Add(ref rawOutputBytes, existingSize);
                    rawByExtension.AddOrUpdate(ext, (1, existingSize), (_, cur) => (cur.Count + 1, cur.Bytes + existingSize));
                    return;
                }

                try
                {
                    byte[]? data;
                    lock (readLock)
                    {
                        if (!provider.TrySaveAsset(file.Path, out data) || data is null)
                        {
                            throw new InvalidDataException("无法读取原始文件字节");
                        }
                    }

                    WriteBytesAtomic(targetPath, data);
                    Interlocked.Increment(ref rawCopiedCount);
                    Interlocked.Increment(ref rawNewlyCopiedCount);
                    Interlocked.Add(ref rawSourceBytes, file.Size);
                    Interlocked.Add(ref rawOutputBytes, data.LongLength);
                    rawByExtension.AddOrUpdate(ext, (1, data.LongLength), (_, cur) => (cur.Count + 1, cur.Bytes + data.LongLength));
                }
                catch (Exception exception)
                {
                    Interlocked.Increment(ref rawFailedCount);
                    failures.Add((file.Path, $"raw:{ext}", TruncateError(exception.Message)));
                    Console.Error.WriteLine(L.Format("ExportAssets_RawCopyFailedLine", file.Path, exception.Message));
                }
            });

        stopwatch.Stop();
        var finishedAt = DateTimeOffset.UtcNow;

        var orderedTrackers = trackers.Values
            .OrderByDescending(item => item.TotalCount)
            .ThenBy(item => item.ClassName, StringComparer.Ordinal)
            .ToArray();

        var orderedFailures = failures
            .OrderBy(item => item.Path, StringComparer.Ordinal)
            .Select(item => new { path = item.Path, primaryClass = item.PrimaryClass, error = item.Error })
            .ToArray();

        var byDecision = orderedTrackers
            .GroupBy(item => item.Decision, StringComparer.Ordinal)
            .Select(group => new
            {
                decision = group.Key,
                totalCount = group.Sum(item => item.TotalCount),
                exportedCount = group.Sum(item => item.ExportedCount),
                newlyExportedCount = group.Sum(item => item.NewlyExportedCount),
                resumedCount = group.Sum(item => item.ResumedCount),
                failedCount = group.Sum(item => item.FailedCount),
                skippedCount = group.Sum(item => item.SkippedCount),
                sourceBytes = group.Sum(item => item.SourceBytes),
                outputBytes = group.Sum(item => item.OutputBytes),
            })
            .OrderByDescending(item => item.totalCount)
            .ThenBy(item => item.decision, StringComparer.Ordinal)
            .ToArray();

        var skippedNonPackageByExtension = skippedNonPackages
            .GroupBy(file => NormalizeExtension(file.Path), StringComparer.Ordinal)
            .Select(group => new
            {
                extension = group.Key,
                count = group.Count(),
                totalSize = group.Sum(item => item.Size),
            })
            .OrderByDescending(item => item.count)
            .ThenBy(item => item.extension, StringComparer.Ordinal)
            .ToArray();

        var exportedPackages = orderedTrackers.Sum(item => item.ExportedCount);
        var newlyExportedPackages = orderedTrackers.Sum(item => item.NewlyExportedCount);
        var resumedPackages = orderedTrackers.Sum(item => item.ResumedCount);
        var failedPackages = orderedTrackers.Sum(item => item.FailedCount);
        var skippedPackages = orderedTrackers
            .Where(item => item.Decision is "skip_image" or "skip_binary" or "skip_level")
            .Sum(item => item.SkippedCount);
        var undecidedPackages = orderedTrackers
            .Where(item => item.Decision == "undecided")
            .Sum(item => item.SkippedCount);
        var packageOutputBytes = orderedTrackers.Sum(item => item.OutputBytes);
        var exportedSourceBytes = orderedTrackers
            .Where(item => item.Decision is "export" or "export_blueprint_cdo")
            .Sum(item => item.SourceBytes);
        var skippedSourceBytes = orderedTrackers
            .Where(item => item.Decision is "skip_image" or "skip_binary" or "skip_level")
            .Sum(item => item.SourceBytes);
        var undecidedSourceBytes = orderedTrackers
            .Where(item => item.Decision == "undecided")
            .Sum(item => item.SourceBytes);

        var manifest = new
        {
            cue4ParseVersion = Cue4ParseVersion,
            command = "export-assets",
            status = "completed",
            usmapSha256 = usmapHash,
            prefixes = prefixes.ToArray(),
            jobs,
            startedAt = startedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            finishedAt = finishedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            elapsedSeconds = Math.Round(stopwatch.Elapsed.TotalSeconds, 3),
            totals = new
            {
                consideredPackages = matchedPackages.Length,
                exportedPackages,
                newlyExportedPackages,
                resumedPackages,
                failedPackages,
                skippedPackages,
                undecidedPackages,
                rawCopiedFiles = rawCopiedCount,
                rawNewlyCopiedFiles = rawNewlyCopiedCount,
                rawResumedFiles = rawResumedCount,
                rawFailedFiles = rawFailedCount,
                totalFailures = orderedFailures.Length,
                exportedSourceBytes,
                packageOutputBytes,
                rawOutputBytes,
                totalOutputBytes = packageOutputBytes + rawOutputBytes,
                skippedSourceBytes,
                undecidedSourceBytes,
            },
            byDecision,
            byClass = orderedTrackers.Select(item => new
            {
                className = item.ClassName,
                decision = item.Decision,
                category = item.Category,
                reason = item.Reason,
                totalCount = item.TotalCount,
                exportedCount = item.ExportedCount,
                newlyExportedCount = item.NewlyExportedCount,
                resumedCount = item.ResumedCount,
                failedCount = item.FailedCount,
                skippedCount = item.SkippedCount,
                sourceBytes = item.SourceBytes,
                outputBytes = item.OutputBytes,
            }).ToArray(),
            rawFiles = new
            {
                copiedCount = rawCopiedCount,
                newlyCopiedCount = rawNewlyCopiedCount,
                resumedCount = rawResumedCount,
                failedCount = rawFailedCount,
                sourceBytes = rawSourceBytes,
                outputBytes = rawOutputBytes,
                byExtension = rawByExtension
                    .OrderBy(kv => kv.Key, StringComparer.Ordinal)
                    .Select(kv => new { extension = kv.Key, count = kv.Value.Count, outputBytes = kv.Value.Bytes })
                    .ToArray(),
            },
            skippedNonPackageFiles = new
            {
                totalCount = skippedNonPackages.Count,
                totalSize = skippedNonPackages.Sum(file => file.Size),
                byExtension = skippedNonPackageByExtension,
            },
            failures = orderedFailures,
        };

        await TextOutput.WriteJsonAsync(Path.Combine(outputPath, "manifest.json"), manifest);

        Console.WriteLine(L.Format(
            "ExportAssets_Summary",
            exportedPackages,
            newlyExportedPackages,
            resumedPackages,
            rawCopiedCount,
            rawNewlyCopiedCount,
            rawResumedCount,
            skippedPackages,
            undecidedPackages,
            orderedFailures.Length,
            packageOutputBytes + rawOutputBytes,
            stopwatch.Elapsed.TotalSeconds.ToString("F2", CultureInfo.InvariantCulture)));

        return orderedFailures.Length == 0 ? 0 : 1;
    }

    internal static UObject[] FilterExportsForClass(
        IEnumerable<UObject> exports,
        string primaryClass,
        string decision)
    {
        if (string.Equals(decision, "export_blueprint_cdo", StringComparison.Ordinal))
        {
            var isWidget = string.Equals(primaryClass, "WidgetBlueprintGeneratedClass", StringComparison.Ordinal)
                || string.Equals(primaryClass, "WidgetBlueprint", StringComparison.Ordinal);

            return exports
                .Where(export => ShouldKeepBlueprintExport(export, isWidget))
                .ToArray();
        }

        if (string.Equals(primaryClass, "LevelSequence", StringComparison.Ordinal))
        {
            return exports
                .Where(ShouldKeepLevelSequenceExport)
                .ToArray();
        }

        return exports.ToArray();
    }

    private static bool ShouldKeepBlueprintExport(UObject export, bool isWidget)
    {
        var type = export.ExportType;
        if (string.Equals(type, "Function", StringComparison.Ordinal)
            || type.StartsWith("EdGraph", StringComparison.Ordinal)
            || type.StartsWith("K2Node", StringComparison.Ordinal)
            || string.Equals(type, "BlueprintGraph", StringComparison.Ordinal))
        {
            return false;
        }

        if (isWidget && type.StartsWith("MovieScene", StringComparison.Ordinal))
        {
            return false;
        }

        return true;
    }

    private static bool ShouldKeepLevelSequenceExport(UObject export)
    {
        var type = export.ExportType;
        if (string.Equals(type, "MovieSceneCompiledData", StringComparison.Ordinal)
            || string.Equals(type, "MovieSceneBuiltInEasingFunction", StringComparison.Ordinal)
            || type.StartsWith("EdGraph", StringComparison.Ordinal)
            || type.StartsWith("K2Node", StringComparison.Ordinal))
        {
            return false;
        }

        return true;
    }

    private static void PrepareResumableOutputDirectory(
        string outputPath,
        string outputArgument,
        bool force,
        string expectedCommand)
    {
        if (Directory.Exists(outputPath) && Directory.EnumerateFileSystemEntries(outputPath).Any())
        {
            var manifest = RequireExportManifest(outputPath, outputArgument);
            EnsureManifestMatchesCommand(manifest, outputArgument, expectedCommand);

            if (force)
            {
                long deletedFiles = 0;
                foreach (var entry in Directory.EnumerateFileSystemEntries(outputPath))
                {
                    if (Directory.Exists(entry) && IsOtherCommandSubdirectory(entry, expectedCommand))
                    {
                        Console.WriteLine(L.Format("ExportAssets_KeepOtherCommandDir", Path.GetFileName(entry)));
                        continue;
                    }

                    if (Directory.Exists(entry))
                    {
                        deletedFiles += Directory.EnumerateFiles(entry, "*", SearchOption.AllDirectories).LongCount();
                        Directory.Delete(entry, true);
                    }
                    else
                    {
                        deletedFiles++;
                        File.Delete(entry);
                    }
                }

                Console.WriteLine(L.Format("ExportAssets_ClearedOutputDir", outputArgument, deletedFiles));
            }
            else
            {
                Console.WriteLine(L.Format("ExportAssets_ResumeMode", outputArgument));
            }
        }

        Directory.CreateDirectory(outputPath);
    }

    private static bool IsOtherCommandSubdirectory(string directoryPath, string expectedCommand)
    {
        var manifestPath = Path.Combine(directoryPath, "manifest.json");
        if (!File.Exists(manifestPath))
        {
            return false;
        }

        try
        {
            var manifest = JObject.Parse(File.ReadAllText(manifestPath));
            var command = manifest.Property("command")?.Value.Value<string>();
            return manifest.Property("cue4ParseVersion") is not null
                && !string.Equals(command, expectedCommand, StringComparison.Ordinal);
        }
        catch
        {
            return false;
        }
    }

    private static async Task WriteManifestSentinelAsync(
        string outputPath,
        string usmapHash,
        IReadOnlyList<string> prefixes,
        DateTimeOffset startedAt)
    {
        var sentinel = new
        {
            cue4ParseVersion = Cue4ParseVersion,
            command = "export-assets",
            status = "in_progress",
            usmapSha256 = usmapHash,
            prefixes = prefixes.ToArray(),
            startedAt = startedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
        };
        await TextOutput.WriteJsonAsync(Path.Combine(outputPath, "manifest.json"), sentinel);
    }

    private static string BuildExportJsonPath(string outputRoot, string vfsPath)
    {
        var ext = Path.GetExtension(vfsPath);
        var withoutExt = string.IsNullOrEmpty(ext) ? vfsPath : vfsPath[..^ext.Length];
        return Path.Combine(outputRoot, (withoutExt + ".json").Replace('/', Path.DirectorySeparatorChar));
    }

    private static void WriteBytesAtomic(string targetPath, byte[] bytes)
    {
        var parent = Path.GetDirectoryName(targetPath);
        if (parent is not null)
        {
            Directory.CreateDirectory(PathSupport.ToIoPath(parent));
        }

        var ioTarget = PathSupport.ToIoPath(targetPath);
        var tempPath = PathSupport.ToIoPath($"{targetPath}.{Environment.CurrentManagedThreadId}.tmp");
        File.WriteAllBytes(tempPath, bytes);
        File.Move(tempPath, ioTarget, overwrite: true);
    }

    private static bool MatchesAnyPrefix(string path, IReadOnlyCollection<string> prefixes)
    {
        foreach (var prefix in prefixes)
        {
            if (path.StartsWith(prefix, StringComparison.Ordinal))
            {
                return true;
            }
        }

        return false;
    }

    private static void AddCommaValues(List<string> target, string rawValue)
    {
        var parts = rawValue.Split(',', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries);
        if (parts.Length == 0)
        {
            throw new ArgumentException(L.Format("ExportAssets_ClassifyEmpty", rawValue));
        }

        target.AddRange(parts);
    }
}
