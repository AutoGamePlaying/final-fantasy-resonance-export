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
    private static readonly UTF8Encoding Utf8WithoutBom = TextOutput.Utf8WithoutBom;

    public static async Task<int> Main(string[] args)
    {
        if (Console.IsOutputRedirected || Console.IsErrorRedirected)
        {
            ConfigureRedirectedUtf8Output();
        }

        try
        {
            args = Localization.InitializeFromCli(args);
            return await RunAsync(args);
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine(L.Format("Common_ErrorPrefix", exception.Message));
            return 2;
        }
    }

    private static void ConfigureRedirectedUtf8Output()
    {
        try
        {
            Console.OutputEncoding = Utf8WithoutBom;
        }
        catch (IOException)
        {
            // Fall back to direct UTF-8 stream wrappers if the host console rejects code page changes.
            if (Console.IsOutputRedirected)
            {
                Console.SetOut(TextOutput.CreateConsoleStreamWriter(Console.OpenStandardOutput()));
            }

            if (Console.IsErrorRedirected)
            {
                Console.SetError(TextOutput.CreateConsoleStreamWriter(Console.OpenStandardError()));
            }
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
            return await RunInteractiveWizardAsync();
        }

        switch (args[0])
        {
            case "--help":
            case "-h":
            case "help":
                PrintHelp(Console.Out);
                return 0;
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
            case "build-db":
                return await RunBuildDbAsync(args[1..]);
            case "fetch-usmap":
                return await RunFetchUsmapAsync(args[1..]);
            case "uninstall-ue4ss":
                return await RunUninstallUe4ssAsync(args[1..]);
            default:
                throw new ArgumentException(L.Format("Cli_UnknownSubcommand", args[0]));
        }
    }

    internal static void PrintHelp(TextWriter output)
    {
        foreach (var line in L.Get("Cli_HelpText").Split('\n'))
        {
            output.WriteLine(line);
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
                    throw new ArgumentException(L.Format("Cli_UnknownArg", "list", args[index]));
            }
        }

        AddDefaultPrefixes(prefixes);
        using var provider = CreateProvider(paks, null);
        var count = 0;
        foreach (var file in MatchingFiles(provider, prefixes, false))
        {
            Console.WriteLine(L.Format("Cli_ListItemLine", file.Path, file.Size));
            count++;
        }

        Console.WriteLine(L.Format("Cli_ListTotal", count));
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
                        throw new ArgumentException(L.Format("Cli_RawUnknownOrDuplicateArg", args[index]));
                    }

                    packagePath = args[index];
                    break;
            }
        }

        if (packagePath is null)
        {
            throw new ArgumentException(L.Get("Cli_RawRequiresPackagePath"));
        }

        if (outputArgument is null)
        {
            throw new ArgumentException(L.Get("Cli_RawRequiresOut"));
        }

        var outputPath = RequireDepsPath(outputArgument, "--out");
        using var provider = CreateProvider(paks, null);
        if (!provider.TrySaveAsset(packagePath, out var bytes) || bytes is null)
        {
            throw new InvalidDataException(L.Format("Cli_PackageNotFoundOrUnreadable", packagePath));
        }

        var parent = Path.GetDirectoryName(outputPath);
        if (parent is not null)
        {
            Directory.CreateDirectory(parent);
        }

        await File.WriteAllBytesAsync(outputPath, bytes);
        Console.WriteLine(L.Format("Cli_RawLength", bytes.Length));
        Console.WriteLine(L.Format("Cli_RawSha256", Convert.ToHexString(SHA256.HashData(bytes))));
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
                    throw new ArgumentException(L.Format("Cli_UnknownArg", "export", args[index]));
            }
        }

        if (usmapPath is null)
        {
            throw new ArgumentException(L.Format("Cli_CommandRequiresUsmap", "export"));
        }

        if (!File.Exists(usmapPath))
        {
            throw new FileNotFoundException(L.Format("Cli_MappingFileNotFound", usmapPath));
        }

        AddDefaultPrefixes(prefixes);
        var outputPath = RequireDepsPath(outputArgument, "--out");
        PrepareOutputDirectory(outputPath, outputArgument, force, "export");
        var usmapHash = Convert.ToHexString(SHA256.HashData(await File.ReadAllBytesAsync(PathSupport.ToIoPath(usmapPath))));

        using var provider = CreateProvider(paks, usmapPath);
        return await ExportAssetsWithProviderAsync(provider, usmapHash, outputPath, prefixes);
    }

    private static async Task<int> ExportAssetsWithProviderAsync(
        DefaultFileProvider provider,
        string usmapHash,
        string outputPath,
        IReadOnlyCollection<string> prefixes)
    {
        var startedAt = DateTimeOffset.UtcNow;
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

                var relativePath = file.Path[..^Path.GetExtension(file.Path).Length] + ".json";
                var jsonPath = Path.Combine(outputPath, relativePath.Replace('/', Path.DirectorySeparatorChar));
                var parent = Path.GetDirectoryName(jsonPath);
                if (parent is not null)
                {
                    Directory.CreateDirectory(PathSupport.ToIoPath(parent));
                }

                await TextOutput.WriteJsonAsync(jsonPath, package.GetExports().ToArray());
                successCount++;
            }
            catch (Exception exception)
            {
                failures.Add((file.Path, TruncateError(exception.Message)));
                Console.Error.WriteLine(L.Format("Cli_ExportFailedLine", file.Path, exception.Message));
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
        await TextOutput.WriteJsonAsync(Path.Combine(outputPath, "manifest.json"), manifest);
        Console.WriteLine(L.Format("Cli_ExportSummary", successCount, failures.Count));
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
                    throw new ArgumentException(L.Format("Cli_UnknownArg", "locres", args[index]));
            }
        }

        if (requestedLanguages.Count == 0)
        {
            requestedLanguages.AddRange(DefaultLanguages);
        }

        var outputPath = RequireDepsPath(outputArgument, "--out");

        using var provider = CreateProvider(paks, null);
        var parsedEntries = ParseLocresEntries(provider, requestedLanguages);
        PrepareOutputDirectory(outputPath, outputArgument, force, "locres");
        await WriteLocresOutputsAsync(parsedEntries, outputPath, outputArgument, DateTimeOffset.UtcNow);
    }

    private static async Task ExportLocresWithProviderAsync(
        DefaultFileProvider provider,
        string outputPath,
        string outputDisplay,
        IReadOnlyList<string> requestedLanguages)
    {
        var startedAt = DateTimeOffset.UtcNow;
        var parsedEntries = ParseLocresEntries(provider, requestedLanguages);
        Directory.CreateDirectory(PathSupport.ToIoPath(outputPath));
        await WriteLocresOutputsAsync(parsedEntries, outputPath, outputDisplay, startedAt);
    }

    private static List<(string Language, string SourcePath, string SourceSha256, int NamespaceCount, int EntryCount, SortedDictionary<string, SortedDictionary<string, string>> Data)> ParseLocresEntries(
        DefaultFileProvider provider,
        IReadOnlyList<string> requestedLanguages)
    {
        var availableLanguages = FindAvailableLocresLanguages(provider);
        var parsedEntries = new List<(string Language, string SourcePath, string SourceSha256, int NamespaceCount, int EntryCount, SortedDictionary<string, SortedDictionary<string, string>> Data)>();

        foreach (var language in requestedLanguages)
        {
            var sourcePath = $"{LocalizationGamePrefix}{language}{LocalizationFileSuffix}";
            if (!provider.TrySaveAsset(sourcePath, out var bytes) || bytes is null)
            {
                var available = availableLanguages.Count > 0
                    ? string.Join(L.Get("Common_ListSeparator"), availableLanguages)
                    : L.Get("Common_None");
                throw new InvalidDataException(L.Format("Cli_LocresNotFound", language, sourcePath, available));
            }

            var sourceSha256 = Convert.ToHexString(SHA256.HashData(bytes));
            using var archive = new FByteArchive(sourcePath, bytes);
            var resource = new FTextLocalizationResource(archive);
            var (data, entryCount) = ConvertLocresToDictionary(resource, language);
            parsedEntries.Add((language, sourcePath, sourceSha256, data.Count, entryCount, data));
        }

        return parsedEntries;
    }

    private static async Task WriteLocresOutputsAsync(
        List<(string Language, string SourcePath, string SourceSha256, int NamespaceCount, int EntryCount, SortedDictionary<string, SortedDictionary<string, string>> Data)> parsedEntries,
        string outputPath,
        string outputDisplay,
        DateTimeOffset startedAt)
    {
        foreach (var entry in parsedEntries)
        {
            var jsonPath = Path.Combine(outputPath, $"{entry.Language}.json");
            await TextOutput.WriteJsonAsync(jsonPath, entry.Data);
            Console.WriteLine(L.Format(
                "Cli_LocresLanguageLine",
                entry.Language,
                entry.NamespaceCount,
                entry.EntryCount,
                entry.SourceSha256));
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
        await TextOutput.WriteJsonAsync(Path.Combine(outputPath, "manifest.json"), manifest);
        Console.WriteLine(L.Format("Cli_LocresSummary", parsedEntries.Count, outputDisplay));
    }

    private static void AddLanguages(List<string> target, string rawValue)
    {
        var parts = rawValue.Split(',', StringSplitOptions.TrimEntries);
        if (parts.Length == 0 || parts.Any(string.IsNullOrEmpty))
        {
            throw new ArgumentException(L.Format("Cli_LocresLangEmpty", rawValue));
        }

        foreach (var part in parts)
        {
            if (!SafeLanguageTag.IsMatch(part))
            {
                throw new ArgumentException(L.Format("Cli_LocresLangInvalid", part));
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
                    throw new InvalidDataException(L.Format("Cli_LocresDuplicateKey", language, ns, key));
                }

                bucket[key] = entry.LocalizedString ?? string.Empty;
                entryCount++;
            }
        }

        return (namespaces, entryCount);
    }

    private static DefaultFileProvider CreateProvider(string paks, string? usmapPath)
    {
        var ioPaks = PathSupport.ToIoPath(paks);
        if (!Directory.Exists(ioPaks))
        {
            throw new DirectoryNotFoundException(L.Format("Cli_PaksDirNotFound", paks));
        }

        var provider = new DefaultFileProvider(
            ioPaks,
            SearchOption.TopDirectoryOnly,
            new VersionContainer(EGame.GAME_UE5_6));
        if (usmapPath is not null)
        {
            provider.MappingsContainer = new FileUsmapTypeMappingsProvider(PathSupport.ToIoPath(usmapPath));
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
            throw new ArgumentException(L.Format("Common_OptionRequiresValue", option));
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
            var label = fromEnv ? L.Format("Cli_DepsEnvLabel", depsDisplay) : ".deps/";
            throw new ArgumentException(L.Format("Cli_OutMustBeUnderDeps", option, label, path));
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
                throw new ArgumentException(L.Format("Cli_OutCannotBeProtectedDescendant", option, protectedDisplay, path));
            }
        }

        var ffrRoot = Path.GetFullPath(Path.Combine(depsPath, "ffr"));
        if (Path.GetRelativePath(ffrRoot, fullPath) == ".")
        {
            var ffrDisplay = Path.Combine(depsDisplay, "ffr").Replace('\\', '/');
            throw new ArgumentException(L.Format("Cli_OutCannotBeProtectedSelf", option, ffrDisplay, path));
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
                throw new IOException(L.Get("Cli_OutputDirNonEmpty"));
            }

            var manifest = RequireExportManifest(outputPath, outputArgument);
            EnsureManifestMatchesCommand(manifest, outputArgument, expectedCommand);
            var fileCount = Directory.EnumerateFiles(outputPath, "*", SearchOption.AllDirectories).LongCount();
            Console.WriteLine(L.Format("Cli_ClearingOutputDir", outputArgument, fileCount));
            Directory.Delete(outputPath, true);
        }

        Directory.CreateDirectory(outputPath);
    }

    private static JObject RequireExportManifest(string outputPath, string outputArgument)
    {
        var message = L.Format("Cli_NotFfrExportOutputDir", outputArgument);
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
            throw new IOException(L.Format("Cli_ManifestCommandMismatch", outputArgument, actualCommand, expectedCommand));
        }
    }

    private static string TruncateError(string error)
    {
        return error.Length <= 300 ? error : error[..300];
    }
}
