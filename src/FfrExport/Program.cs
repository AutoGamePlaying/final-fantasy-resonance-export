using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using CUE4Parse.FileProvider;
using CUE4Parse.MappingsProvider.Usmap;
using CUE4Parse.UE4.Assets;
using CUE4Parse.UE4.Localization;
using CUE4Parse.UE4.Readers;
using CUE4Parse.UE4.Versions;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace FfrExport;

internal static partial class Program
{
    internal const string DepsEnvVar = "FFR_DEPS";
    private const string DefaultDepsRelativePath = ".deps";
    private const string Cue4ParseVersion = "1.2.2.202611";
    private const string RelativePaksPath = "ffr/files/FFRS/Content/Paks";
    private const string RelativeExportDirectory = "ffr/export";
    private const string RelativeLocalizationDirectory = "ffr/export/Localization";
    private const string LocalizationGamePrefix = "FFRS/Content/Localization/Game/";
    private const string LocalizationFileSuffix = "/Game.locres";
    private static readonly string[] DefaultPrefixes = ["FFRS/Content/Datatable/", "FFRS/Content/L10N/"];
    private static readonly string[] DefaultLanguages = ["zh-Hans", "en", "ja"];
    private static readonly Regex SafeLanguageTag = new("^[A-Za-z0-9-]+$", RegexOptions.CultureInvariant);
    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    public static async Task<int> Main(string[] args)
    {
        try
        {
            return await RunAsync(args);
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine($"错误：{exception.Message}");
            return 2;
        }
    }

    internal static (string FullPath, string DisplayPath, bool FromEnv) ResolveDepsRoot()
    {
        var envValue = Environment.GetEnvironmentVariable(DepsEnvVar);
        if (!string.IsNullOrWhiteSpace(envValue))
        {
            var trimmed = envValue.Trim();
            return (Path.GetFullPath(trimmed), trimmed, true);
        }

        return (Path.GetFullPath(DefaultDepsRelativePath), DefaultDepsRelativePath, false);
    }

    internal static string ResolveDefaultDepsPath(string relativeSubpath)
    {
        var (_, displayPath, _) = ResolveDepsRoot();
        var normalizedSubpath = relativeSubpath.Replace('/', Path.DirectorySeparatorChar);
        return Path.Combine(displayPath, normalizedSubpath).Replace('\\', '/');
    }

    internal static string GetDefaultPaks() => ResolveDefaultDepsPath(RelativePaksPath);

    internal static string GetDefaultExportDirectory() => ResolveDefaultDepsPath(RelativeExportDirectory);

    internal static string GetDefaultLocalizationDirectory() => ResolveDefaultDepsPath(RelativeLocalizationDirectory);

    private static async Task<int> RunAsync(string[] args)
    {
        if (args.Length == 0)
        {
            throw new ArgumentException("缺少子命令；可用命令：list、raw、export、locres、inventory、export-assets、export-maps");
        }

        switch (args[0])
        {
            case "list":
                ListAssets(args[1..]);
                return 0;
            case "raw":
                await SaveRawAssetAsync(args[1..]);
                return 0;
            case "export":
                return await ExportAssetsAsync(args[1..]);
            case "locres":
                await ExportLocresAsync(args[1..]);
                return 0;
            case "inventory":
                return await RunInventoryAsync(args[1..]);
            case "export-assets":
                return await RunExportAssetsAsync(args[1..]);
            case "export-maps":
                return await RunExportMapsAsync(args[1..]);
            default:
                throw new ArgumentException($"未知子命令：{args[0]}；可用命令：list、raw、export、locres、inventory、export-assets、export-maps");
        }
    }

    private static void ListAssets(string[] args)
    {
        var paks = GetDefaultPaks();
        var prefixes = new List<string>();
        for (var index = 0; index < args.Length; index++)
        {
            switch (args[index])
            {
                case "--paks":
                    paks = RequireValue(args, ref index, "--paks");
                    break;
                case "--prefix":
                    prefixes.Add(RequireValue(args, ref index, "--prefix"));
                    break;
                default:
                    throw new ArgumentException($"list 收到未知参数：{args[index]}");
            }
        }

        AddDefaultPrefixes(prefixes);
        using var provider = CreateProvider(paks, null);
        var count = 0;
        foreach (var file in MatchingFiles(provider, prefixes, false))
        {
            Console.WriteLine($"{file.Path}\t{file.Size} 字节");
            count++;
        }

        Console.WriteLine($"总数：{count}");
    }

    private static async Task SaveRawAssetAsync(string[] args)
    {
        var paks = GetDefaultPaks();
        string? packagePath = null;
        string? outputArgument = null;
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
                default:
                    if (args[index].StartsWith("-", StringComparison.Ordinal) || packagePath is not null)
                    {
                        throw new ArgumentException($"raw 收到未知或重复参数：{args[index]}");
                    }

                    packagePath = args[index];
                    break;
            }
        }

        if (packagePath is null)
        {
            throw new ArgumentException("raw 需要一个包路径");
        }

        if (outputArgument is null)
        {
            throw new ArgumentException("raw 需要 --out <文件>");
        }

        var outputPath = RequireDepsPath(outputArgument, "--out");
        using var provider = CreateProvider(paks, null);
        if (!provider.TrySaveAsset(packagePath, out var bytes) || bytes is null)
        {
            throw new InvalidDataException($"找不到或无法读取包：{packagePath}");
        }

        var parent = Path.GetDirectoryName(outputPath);
        if (parent is not null)
        {
            Directory.CreateDirectory(parent);
        }

        await File.WriteAllBytesAsync(outputPath, bytes);
        Console.WriteLine($"长度：{bytes.Length} 字节");
        Console.WriteLine($"SHA-256：{Convert.ToHexString(SHA256.HashData(bytes))}");
    }

    private static async Task<int> ExportAssetsAsync(string[] args)
    {
        var paks = GetDefaultPaks();
        var outputArgument = GetDefaultExportDirectory();
        string? usmapPath = null;
        var force = false;
        var prefixes = new List<string>();
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
                case "--force":
                    force = true;
                    break;
                default:
                    throw new ArgumentException($"export 收到未知参数：{args[index]}");
            }
        }

        if (usmapPath is null)
        {
            throw new ArgumentException("export 需要 --usmap <文件>（在游戏机上用 UE4SS 导出的 .usmap，见 README.md）");
        }

        if (!File.Exists(usmapPath))
        {
            throw new FileNotFoundException($"映射文件不存在：{usmapPath}");
        }

        AddDefaultPrefixes(prefixes);
        var outputPath = RequireDepsPath(outputArgument, "--out");
        PrepareOutputDirectory(outputPath, outputArgument, force, "export");
        var startedAt = DateTimeOffset.UtcNow;
        var usmapHash = Convert.ToHexString(SHA256.HashData(await File.ReadAllBytesAsync(usmapPath)));

        using var provider = CreateProvider(paks, usmapPath);
        var failures = new List<(string Path, string Error)>();
        var successCount = 0;
        foreach (var file in MatchingFiles(provider, prefixes, true))
        {
            try
            {
                if (!provider.TryLoadPackage(file.Path, out IPackage? package) || package is null)
                {
                    throw new InvalidDataException("CUE4Parse 无法加载此包");
                }

                var json = JsonConvert.SerializeObject(package.GetExports().ToArray(), Formatting.Indented);
                var relativePath = file.Path[..^Path.GetExtension(file.Path).Length] + ".json";
                var jsonPath = Path.Combine(outputPath, relativePath.Replace('/', Path.DirectorySeparatorChar));
                var parent = Path.GetDirectoryName(jsonPath);
                if (parent is not null)
                {
                    Directory.CreateDirectory(parent);
                }

                await File.WriteAllTextAsync(jsonPath, json, Utf8WithoutBom);
                successCount++;
            }
            catch (Exception exception)
            {
                failures.Add((file.Path, TruncateError(exception.Message)));
                Console.Error.WriteLine($"导出失败：{file.Path}：{exception.Message}");
            }
        }

        var manifest = new
        {
            cue4ParseVersion = Cue4ParseVersion,
            usmapSha256 = usmapHash,
            startedAt = startedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            finishedAt = DateTimeOffset.UtcNow.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            successCount,
            failureCount = failures.Count,
            failures = failures.Select(failure => new { path = failure.Path, error = failure.Error }).ToArray(),
        };
        await File.WriteAllTextAsync(
            Path.Combine(outputPath, "manifest.json"),
            JsonConvert.SerializeObject(manifest, Formatting.Indented),
            Utf8WithoutBom);
        Console.WriteLine($"成功：{successCount}，失败：{failures.Count}");
        return failures.Count == 0 ? 0 : 1;
    }

    private static async Task ExportLocresAsync(string[] args)
    {
        var paks = GetDefaultPaks();
        var outputArgument = GetDefaultLocalizationDirectory();
        var force = false;
        var requestedLanguages = new List<string>();
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
                case "--lang":
                    AddLanguages(requestedLanguages, RequireValue(args, ref index, "--lang"));
                    break;
                case "--force":
                    force = true;
                    break;
                default:
                    throw new ArgumentException($"locres 收到未知参数：{args[index]}");
            }
        }

        if (requestedLanguages.Count == 0)
        {
            requestedLanguages.AddRange(DefaultLanguages);
        }

        var outputPath = RequireDepsPath(outputArgument, "--out");
        var startedAt = DateTimeOffset.UtcNow;

        using var provider = CreateProvider(paks, null);
        var availableLanguages = FindAvailableLocresLanguages(provider);
        var parsedEntries = new List<(string Language, string SourcePath, string SourceSha256, int NamespaceCount, int EntryCount, SortedDictionary<string, SortedDictionary<string, string>> Data)>();

        foreach (var language in requestedLanguages)
        {
            var sourcePath = $"{LocalizationGamePrefix}{language}{LocalizationFileSuffix}";
            if (!provider.TrySaveAsset(sourcePath, out var bytes) || bytes is null)
            {
                var available = availableLanguages.Count > 0
                    ? string.Join("、", availableLanguages)
                    : "无";
                throw new InvalidDataException($"找不到语言 {language} 的本地化文件（{sourcePath}）；包内可用语言：{available}");
            }

            var sourceSha256 = Convert.ToHexString(SHA256.HashData(bytes));
            using var archive = new FByteArchive(sourcePath, bytes);
            var resource = new FTextLocalizationResource(archive);
            var (data, entryCount) = ConvertLocresToDictionary(resource, language);
            parsedEntries.Add((language, sourcePath, sourceSha256, data.Count, entryCount, data));
        }

        PrepareOutputDirectory(outputPath, outputArgument, force, "locres");

        foreach (var entry in parsedEntries)
        {
            var jsonPath = Path.Combine(outputPath, $"{entry.Language}.json");
            var json = JsonConvert.SerializeObject(entry.Data, Formatting.Indented);
            await File.WriteAllTextAsync(jsonPath, json, Utf8WithoutBom);
            Console.WriteLine(
                $"{entry.Language}：{entry.NamespaceCount} 个命名空间，{entry.EntryCount} 条，SHA-256：{entry.SourceSha256}");
        }

        var manifest = new
        {
            cue4ParseVersion = Cue4ParseVersion,
            command = "locres",
            startedAt = startedAt.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            finishedAt = DateTimeOffset.UtcNow.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            languages = parsedEntries.Select(entry => new
            {
                language = entry.Language,
                sourcePath = entry.SourcePath,
                sourceSha256 = entry.SourceSha256,
                namespaceCount = entry.NamespaceCount,
                entryCount = entry.EntryCount,
            }).ToArray(),
        };
        await File.WriteAllTextAsync(
            Path.Combine(outputPath, "manifest.json"),
            JsonConvert.SerializeObject(manifest, Formatting.Indented),
            Utf8WithoutBom);
        Console.WriteLine($"已导出 {parsedEntries.Count} 种语言到 {outputArgument}");
    }

    private static void AddLanguages(List<string> target, string rawValue)
    {
        var parts = rawValue.Split(',', StringSplitOptions.TrimEntries);
        if (parts.Length == 0 || parts.Any(string.IsNullOrEmpty))
        {
            throw new ArgumentException($"--lang 不能为空或包含空语言代码：{rawValue}");
        }

        foreach (var part in parts)
        {
            if (!SafeLanguageTag.IsMatch(part))
            {
                throw new ArgumentException($"--lang 包含不合法的语言代码：{part}");
            }

            if (!target.Contains(part, StringComparer.Ordinal))
            {
                target.Add(part);
            }
        }
    }

    private static List<string> FindAvailableLocresLanguages(DefaultFileProvider provider)
    {
        var languages = new SortedSet<string>(StringComparer.Ordinal);
        foreach (var file in provider.Files.Values)
        {
            if (!file.Path.StartsWith(LocalizationGamePrefix, StringComparison.Ordinal)
                || !file.Path.EndsWith(LocalizationFileSuffix, StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }

            var middle = file.Path[LocalizationGamePrefix.Length..^LocalizationFileSuffix.Length];
            if (middle.Length > 0 && !middle.Contains('/', StringComparison.Ordinal))
            {
                languages.Add(middle);
            }
        }

        return [.. languages];
    }

    private static (SortedDictionary<string, SortedDictionary<string, string>> Data, int EntryCount) ConvertLocresToDictionary(
        FTextLocalizationResource resource,
        string language)
    {
        var namespaces = new SortedDictionary<string, SortedDictionary<string, string>>(StringComparer.Ordinal);
        var entryCount = 0;
        foreach (var (nsKey, entries) in resource.Entries)
        {
            var ns = nsKey.Str ?? string.Empty;
            if (!namespaces.TryGetValue(ns, out var bucket))
            {
                bucket = new SortedDictionary<string, string>(StringComparer.Ordinal);
                namespaces[ns] = bucket;
            }

            foreach (var (textKey, entry) in entries)
            {
                var key = textKey.Str ?? string.Empty;
                if (bucket.ContainsKey(key))
                {
                    throw new InvalidDataException($"语言 {language} 的 locres 存在重复键：{ns}/{key}");
                }

                bucket[key] = entry.LocalizedString ?? string.Empty;
                entryCount++;
            }
        }

        return (namespaces, entryCount);
    }

    private static DefaultFileProvider CreateProvider(string paks, string? usmapPath)
    {
        if (!Directory.Exists(paks))
        {
            throw new DirectoryNotFoundException($"资源目录不存在：{paks}");
        }

        var provider = new DefaultFileProvider(
            paks,
            SearchOption.TopDirectoryOnly,
            new VersionContainer(EGame.GAME_UE5_6));
        if (usmapPath is not null)
        {
            provider.MappingsContainer = new FileUsmapTypeMappingsProvider(usmapPath);
        }

        provider.Initialize();
        provider.Mount();
        return provider;
    }

    private static IEnumerable<CUE4Parse.FileProvider.Objects.GameFile> MatchingFiles(
        DefaultFileProvider provider,
        IReadOnlyCollection<string> prefixes,
        bool packagesOnly)
    {
        return provider.Files.Values
            .Where(file => prefixes.Any(prefix => file.Path.StartsWith(prefix, StringComparison.Ordinal)))
            .Where(file => !packagesOnly || file.Path.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase))
            .OrderBy(file => file.Path, StringComparer.Ordinal);
    }

    private static string RequireValue(string[] args, ref int index, string option)
    {
        if (index + 1 >= args.Length)
        {
            throw new ArgumentException($"选项 {option} 需要一个值");
        }

        index++;
        return args[index];
    }

    private static void AddDefaultPrefixes(List<string> prefixes)
    {
        if (prefixes.Count == 0)
        {
            prefixes.AddRange(DefaultPrefixes);
        }
    }

    internal static string RequireDepsPath(string path, string option)
    {
        var (depsPath, depsDisplay, fromEnv) = ResolveDepsRoot();
        var fullPath = Path.GetFullPath(path);
        var relativePath = Path.GetRelativePath(depsPath, fullPath);
        var outside = relativePath == ".."
            || relativePath.StartsWith($"..{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
            || relativePath.StartsWith($"..{Path.AltDirectorySeparatorChar}", StringComparison.Ordinal)
            || Path.IsPathRooted(relativePath);
        if (relativePath == "." || outside)
        {
            var label = fromEnv ? $"FFR_DEPS（{depsDisplay}）" : ".deps/";
            throw new ArgumentException($"{option} 必须是 {label} 下的路径：{path}");
        }

        string[] protectedSubpaths = [
            "ffr/files",
            "r1",
            "dotnet",
            "nuget",
            "dotnet-home",
            "nuget-http",
        ];
        foreach (var subpath in protectedSubpaths)
        {
            var protectedFull = Path.GetFullPath(Path.Combine(depsPath, subpath));
            if (IsSameOrDescendant(protectedFull, fullPath))
            {
                var protectedDisplay = Path.Combine(depsDisplay, subpath).Replace('\\', '/');
                throw new ArgumentException($"{option} 不能指向受保护目录 {protectedDisplay} 或其内部：{path}");
            }
        }

        var ffrRoot = Path.GetFullPath(Path.Combine(depsPath, "ffr"));
        if (Path.GetRelativePath(ffrRoot, fullPath) == ".")
        {
            var ffrDisplay = Path.Combine(depsDisplay, "ffr").Replace('\\', '/');
            throw new ArgumentException($"{option} 不能指向受保护目录 {ffrDisplay} 本身：{path}");
        }

        return fullPath;
    }

    private static bool IsSameOrDescendant(string parentPath, string candidatePath)
    {
        var relativePath = Path.GetRelativePath(parentPath, candidatePath);
        return relativePath == "."
            || (relativePath != ".."
                && !relativePath.StartsWith($"..{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                && !relativePath.StartsWith($"..{Path.AltDirectorySeparatorChar}", StringComparison.Ordinal)
                && !Path.IsPathRooted(relativePath));
    }

    private static void PrepareOutputDirectory(
        string outputPath,
        string outputArgument,
        bool force,
        string expectedCommand)
    {
        if (Directory.Exists(outputPath) && Directory.EnumerateFileSystemEntries(outputPath).Any())
        {
            if (!force)
            {
                throw new IOException("输出目录非空；如需清空并重新导出，请加 --force");
            }

            var manifest = RequireExportManifest(outputPath, outputArgument);
            EnsureManifestMatchesCommand(manifest, outputArgument, expectedCommand);
            var fileCount = Directory.EnumerateFiles(outputPath, "*", SearchOption.AllDirectories).LongCount();
            Console.WriteLine($"即将清空输出目录：{outputArgument}；文件数：{fileCount}");
            Directory.Delete(outputPath, true);
        }

        Directory.CreateDirectory(outputPath);
    }

    private static JObject RequireExportManifest(string outputPath, string outputArgument)
    {
        var message = $"{outputArgument} 不是 ffr-export 的输出目录，不清空；换一个空目录或删掉后重试";
        var manifestPath = Path.Combine(outputPath, "manifest.json");
        if (!File.Exists(manifestPath))
        {
            throw new IOException(message);
        }

        try
        {
            var manifest = JObject.Parse(File.ReadAllText(manifestPath));
            if (manifest.Property("cue4ParseVersion") is null)
            {
                throw new IOException(message);
            }

            return manifest;
        }
        catch (JsonException exception)
        {
            throw new IOException(message, exception);
        }
    }

    private static void EnsureManifestMatchesCommand(
        JObject manifest,
        string outputArgument,
        string expectedCommand)
    {
        var manifestCommand = manifest.Property("command")?.Value.Value<string>();
        var actualCommand = manifestCommand
            ?? (manifest.Property("languages") is not null ? "locres" : "export");
        if (!string.Equals(actualCommand, expectedCommand, StringComparison.Ordinal))
        {
            throw new IOException(
                $"{outputArgument} 是 ffr-export {actualCommand} 的输出目录，不能由 {expectedCommand} --force 清空");
        }
    }

    private static string TruncateError(string error)
    {
        return error.Length <= 300 ? error : error[..300];
    }
}
