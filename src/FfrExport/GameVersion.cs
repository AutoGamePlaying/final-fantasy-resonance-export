using System.Buffers.Binary;
using System.Diagnostics;
using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using CUE4Parse.MappingsProvider;
using CUE4Parse.MappingsProvider.Usmap;
using Newtonsoft.Json;

namespace FfrExport;

internal sealed record ValidatedGamePaths(
    string GameRoot,
    string PaksDir,
    string ExePath);

internal sealed record SteamBuildInfo(
    [property: JsonProperty("app_id")] int AppId,
    [property: JsonProperty("build_id")] string? BuildId,
    [property: JsonProperty("name")] string? Name,
    [property: JsonProperty("last_updated")] string? LastUpdated,
    [property: JsonProperty("source")] string? Source);

internal sealed record ExeVersionInfo(
    [property: JsonProperty("relative_path")] string RelativePath,
    [property: JsonProperty("size")] long Size,
    [property: JsonProperty("file_version")] string? FileVersion,
    [property: JsonProperty("product_version")] string? ProductVersion);

internal sealed record PakFileFingerprint(
    [property: JsonProperty("relative_path")] string RelativePath,
    [property: JsonProperty("size")] long Size,
    [property: JsonProperty("sha256")] string? Sha256);

internal sealed record PakFingerprintResult(
    string Fingerprint,
    long ElapsedMs,
    long HashedBytes,
    long TotalBytes,
    PakFileFingerprint[] Files);

internal sealed record GameVersionInfo(
    [property: JsonProperty("steam")] SteamBuildInfo Steam,
    [property: JsonProperty("exe")] ExeVersionInfo Exe,
    [property: JsonProperty("pak_fingerprint")] string PakFingerprint,
    [property: JsonProperty("pak_files")] PakFileFingerprint[] PakFiles);

internal sealed record UsmapHashInfo(
    [property: JsonProperty("file_name")] string FileName,
    [property: JsonProperty("size")] long Size,
    [property: JsonProperty("file_sha256")] string FileSha256,
    [property: JsonProperty("content_sha256")] string ContentSha256,
    [property: JsonProperty("type_count")] int TypeCount,
    [property: JsonProperty("enum_count")] int EnumCount);

internal sealed record PeParsedVersion(
    string? FileVersion,
    string? ProductVersion,
    string? FixedFileVersion,
    string? FixedProductVersion);

/// <summary>
/// Validates the game installation directory and computes cross-platform game version metadata,
/// Steam build ID, PE version info, pak fingerprint, and canonical .usmap content hashes.
/// </summary>
internal static class GameVersionDetector
{
    internal const int SteamAppId = 3259780;
    internal const string AppManifestFileName = "appmanifest_3259780.acf";
    internal const string RelativePaksSubpath = "FFRS/Content/Paks";
    internal const string RelativeExeSubpath = "FFRS/Binaries/Win64/FFRS-Win64-Shipping.exe";

    private const int StreamBufferSize = 1024 * 1024;
    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    internal static ValidatedGamePaths ValidateGameDirectory(string gameDirectory)
    {
        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            throw new ArgumentException(L.Get("Version_GameDirEmpty"));
        }

        var fullRoot = PathSupport.TrimTrailingSeparators(Path.GetFullPath(gameDirectory));
        if (!Directory.Exists(PathSupport.ToIoPath(fullRoot)))
        {
            throw new DirectoryNotFoundException(L.Format("Version_GameDirNotFound", gameDirectory));
        }

        var paksDir = Path.Combine(fullRoot, "FFRS", "Content", "Paks");
        if (!Directory.Exists(PathSupport.ToIoPath(paksDir)))
        {
            throw new DirectoryNotFoundException(L.Format("Version_MissingPaksDir", RelativePaksSubpath, paksDir));
        }

        var exePath = Path.Combine(fullRoot, "FFRS", "Binaries", "Win64", "FFRS-Win64-Shipping.exe");
        if (!File.Exists(PathSupport.ToIoPath(exePath)))
        {
            throw new FileNotFoundException(L.Format("Version_MissingGameExe", RelativeExeSubpath, exePath));
        }

        return new ValidatedGamePaths(fullRoot, paksDir, exePath);
    }

    internal static SteamBuildInfo ReadSteamBuildInfo(
        string gameRoot,
        string? manualBuildId,
        TextWriter? warningOutput = null)
    {
        var warn = warningOutput ?? Console.Error;
        var trimmedManual = string.IsNullOrWhiteSpace(manualBuildId) ? null : manualBuildId.Trim();

        var fullRoot = PathSupport.TrimTrailingSeparators(Path.GetFullPath(gameRoot));
        var steamappsDir = Directory.GetParent(fullRoot)?.Parent?.FullName;
        var manifestPath = steamappsDir is not null
            ? Path.Combine(steamappsDir, AppManifestFileName)
            : null;

        if (manifestPath is not null && File.Exists(PathSupport.ToIoPath(manifestPath)))
        {
            string content;
            try
            {
                content = File.ReadAllText(PathSupport.ToIoPath(manifestPath), Encoding.UTF8);
            }
            catch (Exception exception)
            {
                throw new IOException(L.Format("Version_ReadSteamManifestFailed", manifestPath, exception.Message), exception);
            }

            var fields = ParseAppManifestVdf(content);
            fields.TryGetValue("buildid", out var buildId);
            fields.TryGetValue("name", out var name);
            fields.TryGetValue("LastUpdated", out var lastUpdated);

            if (!string.IsNullOrWhiteSpace(buildId))
            {
                if (trimmedManual is not null
                    && !string.Equals(buildId, trimmedManual, StringComparison.Ordinal))
                {
                    warn.WriteLine(L.Format("Version_SteamBuildIdIgnoredManual", AppManifestFileName, buildId, trimmedManual));
                }

                return new SteamBuildInfo(
                    SteamAppId,
                    buildId.Trim(),
                    string.IsNullOrWhiteSpace(name) ? null : name,
                    string.IsNullOrWhiteSpace(lastUpdated) ? null : lastUpdated.Trim(),
                    "appmanifest");
            }

            if (trimmedManual is not null)
            {
                return new SteamBuildInfo(
                    SteamAppId,
                    trimmedManual,
                    string.IsNullOrWhiteSpace(name) ? null : name,
                    string.IsNullOrWhiteSpace(lastUpdated) ? null : lastUpdated.Trim(),
                    "manual");
            }

            warn.WriteLine(L.Format("Version_SteamManifestMissingBuildId", manifestPath));
            return new SteamBuildInfo(
                SteamAppId,
                null,
                string.IsNullOrWhiteSpace(name) ? null : name,
                string.IsNullOrWhiteSpace(lastUpdated) ? null : lastUpdated.Trim(),
                null);
        }

        if (trimmedManual is not null)
        {
            return new SteamBuildInfo(SteamAppId, trimmedManual, null, null, "manual");
        }

        warn.WriteLine(L.Format("Version_SteamManifestNotFound", AppManifestFileName));
        return new SteamBuildInfo(SteamAppId, null, null, null, null);
    }

    /// <summary>
    /// Parses a Valve KeyValues (VDF) appmanifest file and returns the top-level key/value pairs
    /// inside the root AppState block (brace depth 1).
    /// </summary>
    internal static Dictionary<string, string> ParseAppManifestVdf(string content)
    {
        var result = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var depth = 0;
        var index = 0;
        string? pendingKey = null;

        while (index < content.Length)
        {
            SkipWhitespaceAndComments(content, ref index);
            if (index >= content.Length)
            {
                break;
            }

            var ch = content[index];
            if (ch == '{')
            {
                depth++;
                pendingKey = null;
                index++;
                continue;
            }

            if (ch == '}')
            {
                if (depth > 0)
                {
                    depth--;
                }

                pendingKey = null;
                index++;
                continue;
            }

            var token = ReadVdfToken(content, ref index);
            if (depth == 1)
            {
                if (pendingKey is null)
                {
                    pendingKey = token;
                }
                else
                {
                    result[pendingKey] = token;
                    pendingKey = null;
                }
            }
            else
            {
                pendingKey = null;
            }
        }

        return result;
    }

    private static void SkipWhitespaceAndComments(string content, ref int index)
    {
        while (index < content.Length)
        {
            var ch = content[index];
            if (char.IsWhiteSpace(ch))
            {
                index++;
                continue;
            }

            if (ch == '/' && index + 1 < content.Length && content[index + 1] == '/')
            {
                index += 2;
                while (index < content.Length && content[index] != '\n')
                {
                    index++;
                }

                continue;
            }

            break;
        }
    }

    private static string ReadVdfToken(string content, ref int index)
    {
        if (content[index] == '"')
        {
            index++;
            var sb = new StringBuilder();
            while (index < content.Length)
            {
                var ch = content[index++];
                if (ch == '"')
                {
                    break;
                }

                if (ch == '\\' && index < content.Length)
                {
                    var escaped = content[index++];
                    sb.Append(escaped switch
                    {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '"' => '"',
                        '\\' => '\\',
                        _ => escaped,
                    });
                }
                else
                {
                    sb.Append(ch);
                }
            }

            return sb.ToString();
        }

        var start = index;
        while (index < content.Length
            && !char.IsWhiteSpace(content[index])
            && content[index] != '{'
            && content[index] != '}'
            && content[index] != '"')
        {
            index++;
        }

        return content[start..index];
    }

    internal static ExeVersionInfo ReadExeVersion(string exePath)
    {
        var ioPath = PathSupport.ToIoPath(exePath);
        using var stream = new FileStream(ioPath, FileMode.Open, FileAccess.Read, FileShare.Read);
        var parsed = PeVersionReader.ReadFromStream(stream, exePath);
        return new ExeVersionInfo(
            RelativeExeSubpath,
            stream.Length,
            parsed.FileVersion,
            parsed.ProductVersion);
    }

    internal static PakFingerprintResult ComputePakFingerprint(string paksDir)
    {
        var ioPaksDir = PathSupport.ToIoPath(paksDir);
        if (!Directory.Exists(ioPaksDir))
        {
            throw new DirectoryNotFoundException(L.Format("Version_PakFpDirNotFound", paksDir));
        }

        var stopwatch = Stopwatch.StartNew();
        var candidates = new List<(string RelativePath, string FullPath, string Extension)>();

        foreach (var filePath in Directory.EnumerateFiles(ioPaksDir, "*", SearchOption.AllDirectories))
        {
            var ext = Path.GetExtension(filePath);
            if (!ext.Equals(".utoc", StringComparison.OrdinalIgnoreCase)
                && !ext.Equals(".ucas", StringComparison.OrdinalIgnoreCase)
                && !ext.Equals(".pak", StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }

            var relative = PathSupport.NormalizeRelativePath(ioPaksDir, filePath);
            candidates.Add((relative, filePath, ext));
        }

        if (candidates.Count == 0)
        {
            throw new InvalidDataException(L.Format("Version_PakFpNoFiles", paksDir));
        }

        candidates.Sort((a, b) => StringComparer.Ordinal.Compare(a.RelativePath, b.RelativePath));

        var entries = new PakFileFingerprint[candidates.Count];
        long hashedBytes = 0;
        long totalBytes = 0;

        for (var i = 0; i < candidates.Count; i++)
        {
            var (relativePath, fullPath, ext) = candidates[i];
            var fileSize = new FileInfo(fullPath).Length;
            totalBytes += fileSize;

            string? sha256 = null;
            if (ext.Equals(".utoc", StringComparison.OrdinalIgnoreCase)
                || ext.Equals(".pak", StringComparison.OrdinalIgnoreCase))
            {
                sha256 = ComputeFileSha256Lower(fullPath);
                hashedBytes += fileSize;
            }

            entries[i] = new PakFileFingerprint(relativePath, fileSize, sha256);
        }

        var combined = CombinePakFingerprint(entries);
        stopwatch.Stop();

        return new PakFingerprintResult(
            combined,
            stopwatch.ElapsedMilliseconds,
            hashedBytes,
            totalBytes,
            entries);
    }

    /// <summary>
    /// Combines pak/utoc/ucas metadata into a deterministic SHA-256 fingerprint independent of input order.
    /// </summary>
    internal static string CombinePakFingerprint(IReadOnlyList<PakFileFingerprint> files)
    {
        var ordered = files
            .OrderBy(entry => entry.RelativePath, StringComparer.Ordinal)
            .ToArray();

        using var hasher = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
        AppendUtf8(hasher, "ffr-pak-fingerprint-v1\n");
        foreach (var entry in ordered)
        {
            var line = string.Create(
                CultureInfo.InvariantCulture,
                $"{entry.RelativePath}\t{entry.Size}\t{entry.Sha256 ?? "-"}\n");
            AppendUtf8(hasher, line);
        }

        return Convert.ToHexStringLower(hasher.GetHashAndReset());
    }

    internal static UsmapHashInfo ComputeUsmapHashes(string usmapPath)
    {
        var ioPath = PathSupport.ToIoPath(usmapPath);
        if (!File.Exists(ioPath))
        {
            throw new FileNotFoundException(L.Format("Version_UsmapFileNotFound", usmapPath));
        }

        var size = new FileInfo(ioPath).Length;
        var fileSha256 = ComputeFileSha256Lower(ioPath);

        TypeMappings mappings;
        try
        {
            var provider = new FileUsmapTypeMappingsProvider(ioPath);
            mappings = provider.MappingsForGame
                ?? throw new InvalidDataException(L.Get("Version_UsmapEmptyMappings"));
        }
        catch (Exception exception)
        {
            throw new InvalidDataException(L.Format("Version_UsmapParseFailed", usmapPath, exception.Message), exception);
        }

        var contentSha256 = ComputeUsmapContentHash(mappings);
        return new UsmapHashInfo(
            Path.GetFileName(usmapPath),
            size,
            fileSha256,
            contentSha256,
            mappings.Types.Count,
            mappings.Enums.Count);
    }

    /// <summary>
    /// Computes an order-independent canonical SHA-256 hash of a parsed .usmap TypeMappings object
    /// by sorting enums, types, and properties by name.
    /// </summary>
    internal static string ComputeUsmapContentHash(TypeMappings mappings)
    {
        using var hasher = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
        AppendUtf8(hasher, "ffr-usmap-canonical-v1\n");

        AppendUtf8(hasher, "ENUMS\n");
        foreach (var enumEntry in mappings.Enums.OrderBy(kv => kv.Key, StringComparer.Ordinal))
        {
            AppendUtf8(hasher, $"E\t{enumEntry.Key}\n");
            foreach (var member in enumEntry.Value
                .OrderBy(kv => kv.Key)
                .ThenBy(kv => kv.Value, StringComparer.Ordinal))
            {
                AppendUtf8(
                    hasher,
                    string.Create(CultureInfo.InvariantCulture, $"EV\t{member.Key}\t{member.Value}\n"));
            }
        }

        AppendUtf8(hasher, "TYPES\n");
        foreach (var typeEntry in mappings.Types.OrderBy(kv => kv.Key, StringComparer.Ordinal))
        {
            var structDef = typeEntry.Value;
            AppendUtf8(
                hasher,
                string.Create(
                    CultureInfo.InvariantCulture,
                    $"T\t{typeEntry.Key}\t{structDef.SuperType ?? string.Empty}\t{structDef.PropertyCount}\n"));

            foreach (var propEntry in structDef.Properties
                .OrderBy(kv => kv.Value.Name, StringComparer.Ordinal)
                .ThenBy(kv => kv.Key)
                .ThenBy(kv => kv.Value.Index))
            {
                var info = propEntry.Value;
                var arraySize = info.ArraySize.HasValue
                    ? info.ArraySize.Value.ToString(CultureInfo.InvariantCulture)
                    : string.Empty;
                var typeSignature = CanonicalizePropertyType(info.MappingType);
                AppendUtf8(
                    hasher,
                    string.Create(
                        CultureInfo.InvariantCulture,
                        $"P\t{info.Name}\t{propEntry.Key}\t{info.Index}\t{arraySize}\t{typeSignature}\n"));
            }
        }

        return Convert.ToHexStringLower(hasher.GetHashAndReset());
    }

    private static string CanonicalizePropertyType(PropertyType? propertyType)
    {
        if (propertyType is null)
        {
            return string.Empty;
        }

        var isEnumAsByte = propertyType.IsEnumAsByte.HasValue
            ? (propertyType.IsEnumAsByte.Value ? "1" : "0")
            : string.Empty;
        var boolFlag = propertyType.Bool.HasValue
            ? (propertyType.Bool.Value ? "1" : "0")
            : string.Empty;

        return string.Concat(
            propertyType.Type ?? string.Empty,
            "|st=",
            propertyType.StructType ?? string.Empty,
            "|en=",
            propertyType.EnumName ?? string.Empty,
            "|eb=",
            isEnumAsByte,
            "|b=",
            boolFlag,
            "|in=(",
            CanonicalizePropertyType(propertyType.InnerType),
            ")|val=(",
            CanonicalizePropertyType(propertyType.ValueType),
            ")");
    }

    internal static string ComputeFileSha256Lower(string path)
    {
        var ioPath = PathSupport.ToIoPath(path);
        using var stream = new FileStream(
            ioPath,
            FileMode.Open,
            FileAccess.Read,
            FileShare.Read,
            StreamBufferSize,
            FileOptions.SequentialScan);
        using var hasher = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
        var buffer = new byte[StreamBufferSize];
        while (true)
        {
            var bytesRead = stream.Read(buffer, 0, buffer.Length);
            if (bytesRead == 0)
            {
                break;
            }

            hasher.AppendData(buffer, 0, bytesRead);
        }

        return Convert.ToHexStringLower(hasher.GetHashAndReset());
    }

    private static void AppendUtf8(IncrementalHash hasher, string text)
    {
        var bytes = Utf8WithoutBom.GetBytes(text);
        hasher.AppendData(bytes);
    }
}

/// <summary>
/// Cross-platform pure-C# PE resource reader that extracts FileVersion and ProductVersion
/// from the RT_VERSION (VS_VERSIONINFO) resource without relying on Windows-only FileVersionInfo.
/// </summary>
internal static class PeVersionReader
{
    private const ushort DosMagic = 0x5A4D; // "MZ"
    private const uint PeSignature = 0x00004550; // "PE\0\0"
    private const ushort Pe32Magic = 0x010B;
    private const ushort Pe32PlusMagic = 0x020B;
    private const uint ResourceTypeVersion = 16; // RT_VERSION
    private const uint FixedFileInfoSignature = 0xFEEF04BD;
    private const int MaxVersionResourceBytes = 16 * 1024 * 1024;

    private readonly record struct PeSection(
        uint VirtualAddress,
        uint VirtualSize,
        uint RawPointer,
        uint RawSize);

    private readonly record struct ResourceDirEntry(
        uint NameOrId,
        uint OffsetToData)
    {
        public bool IsNamed => (NameOrId & 0x80000000u) != 0;
        public bool IsDirectory => (OffsetToData & 0x80000000u) != 0;
        public uint SubOffset => OffsetToData & 0x7FFFFFFFu;
    }

    internal static PeParsedVersion ReadFromFile(string exePath)
    {
        var ioPath = PathSupport.ToIoPath(exePath);
        using var stream = new FileStream(ioPath, FileMode.Open, FileAccess.Read, FileShare.Read);
        return ReadFromStream(stream, exePath);
    }

    internal static PeParsedVersion ReadFromStream(Stream stream, string sourceName)
    {
        if (!stream.CanSeek)
        {
            throw new InvalidDataException(L.Format("Version_PeNotSeekable", sourceName));
        }

        if (stream.Length < 0x40)
        {
            throw new InvalidDataException(L.Format("Version_PeTooSmall", sourceName));
        }

        var dosHeader = ReadExact(stream, 0, 0x40, sourceName);
        if (BinaryPrimitives.ReadUInt16LittleEndian(dosHeader.AsSpan(0, 2)) != DosMagic)
        {
            throw new InvalidDataException(L.Format("Version_PeMissingMz", sourceName));
        }

        var peOffset = (long)BinaryPrimitives.ReadUInt32LittleEndian(dosHeader.AsSpan(0x3C, 4));
        if (peOffset < 0 || peOffset + 24 > stream.Length)
        {
            throw new InvalidDataException(L.Format("Version_PeHeaderOutOfRange", sourceName));
        }

        var peAndCoff = ReadExact(stream, peOffset, 24, sourceName);
        if (BinaryPrimitives.ReadUInt32LittleEndian(peAndCoff.AsSpan(0, 4)) != PeSignature)
        {
            throw new InvalidDataException(L.Format("Version_PeMissingSignature", sourceName));
        }

        var numberOfSections = BinaryPrimitives.ReadUInt16LittleEndian(peAndCoff.AsSpan(6, 2));
        var sizeOfOptionalHeader = BinaryPrimitives.ReadUInt16LittleEndian(peAndCoff.AsSpan(20, 2));
        var optionalHeaderOffset = peOffset + 24;
        if (sizeOfOptionalHeader < 96 || optionalHeaderOffset + sizeOfOptionalHeader > stream.Length)
        {
            throw new InvalidDataException(L.Format("Version_PeBadOptionalHeader", sourceName));
        }

        var optionalHeader = ReadExact(stream, optionalHeaderOffset, sizeOfOptionalHeader, sourceName);
        var optMagic = BinaryPrimitives.ReadUInt16LittleEndian(optionalHeader.AsSpan(0, 2));
        int numberOfRvaOffset;
        int dataDirectoriesOffset;
        if (optMagic == Pe32Magic)
        {
            numberOfRvaOffset = 92;
            dataDirectoriesOffset = 96;
        }
        else if (optMagic == Pe32PlusMagic)
        {
            numberOfRvaOffset = 108;
            dataDirectoriesOffset = 112;
        }
        else
        {
            throw new InvalidDataException(L.Format(
                "Version_PeUnsupportedMagic",
                sourceName,
                optMagic.ToString("X4", CultureInfo.InvariantCulture)));
        }

        if (sizeOfOptionalHeader < dataDirectoriesOffset + 24)
        {
            return new PeParsedVersion(null, null, null, null);
        }

        var numberOfRvaAndSizes = BinaryPrimitives.ReadUInt32LittleEndian(optionalHeader.AsSpan(numberOfRvaOffset, 4));
        if (numberOfRvaAndSizes <= 2)
        {
            return new PeParsedVersion(null, null, null, null);
        }

        // DataDirectory[2] is the Resource Table.
        var resourceRva = BinaryPrimitives.ReadUInt32LittleEndian(optionalHeader.AsSpan(dataDirectoriesOffset + 16, 4));
        var resourceSize = BinaryPrimitives.ReadUInt32LittleEndian(optionalHeader.AsSpan(dataDirectoriesOffset + 20, 4));
        if (resourceRva == 0 || resourceSize == 0)
        {
            return new PeParsedVersion(null, null, null, null);
        }

        var sectionTableOffset = optionalHeaderOffset + sizeOfOptionalHeader;
        var sectionTableBytes = checked(numberOfSections * 40);
        if (sectionTableOffset + sectionTableBytes > stream.Length)
        {
            throw new InvalidDataException(L.Format("Version_PeSectionTableOutOfRange", sourceName));
        }

        var sectionData = ReadExact(stream, sectionTableOffset, sectionTableBytes, sourceName);
        var sections = new PeSection[numberOfSections];
        for (var i = 0; i < numberOfSections; i++)
        {
            var span = sectionData.AsSpan(i * 40, 40);
            var virtualSize = BinaryPrimitives.ReadUInt32LittleEndian(span.Slice(8, 4));
            var virtualAddress = BinaryPrimitives.ReadUInt32LittleEndian(span.Slice(12, 4));
            var rawSize = BinaryPrimitives.ReadUInt32LittleEndian(span.Slice(16, 4));
            var rawPointer = BinaryPrimitives.ReadUInt32LittleEndian(span.Slice(20, 4));
            sections[i] = new PeSection(virtualAddress, virtualSize, rawPointer, rawSize);
        }

        if (!TryRvaToFileOffset(sections, resourceRva, out var resourceBaseOffset))
        {
            throw new InvalidDataException(L.Format(
                "Version_PeResourceRvaUnmapped",
                sourceName,
                resourceRva.ToString("X", CultureInfo.InvariantCulture)));
        }

        var rootEntries = ReadResourceDirectory(stream, resourceBaseOffset, 0, sourceName);
        ResourceDirEntry? versionTypeEntry = null;
        foreach (var entry in rootEntries)
        {
            if (!entry.IsNamed && entry.NameOrId == ResourceTypeVersion && entry.IsDirectory)
            {
                versionTypeEntry = entry;
                break;
            }
        }

        if (versionTypeEntry is null)
        {
            return new PeParsedVersion(null, null, null, null);
        }

        var nameEntries = ReadResourceDirectory(stream, resourceBaseOffset, versionTypeEntry.Value.SubOffset, sourceName);
        ResourceDirEntry? chosenNameEntry = null;
        foreach (var entry in nameEntries)
        {
            if (!entry.IsDirectory)
            {
                continue;
            }

            if (chosenNameEntry is null || (!entry.IsNamed && entry.NameOrId == 1))
            {
                chosenNameEntry = entry;
                if (!entry.IsNamed && entry.NameOrId == 1)
                {
                    break;
                }
            }
        }

        if (chosenNameEntry is null)
        {
            return new PeParsedVersion(null, null, null, null);
        }

        var langEntries = ReadResourceDirectory(stream, resourceBaseOffset, chosenNameEntry.Value.SubOffset, sourceName);
        ResourceDirEntry? chosenLangEntry = null;
        foreach (var entry in langEntries)
        {
            if (entry.IsDirectory)
            {
                continue;
            }

            if (chosenLangEntry is null || (!entry.IsNamed && entry.NameOrId == 0x0409))
            {
                chosenLangEntry = entry;
                if (!entry.IsNamed && entry.NameOrId == 0x0409)
                {
                    break;
                }
            }
        }

        if (chosenLangEntry is null)
        {
            return new PeParsedVersion(null, null, null, null);
        }

        var dataEntryOffset = resourceBaseOffset + chosenLangEntry.Value.SubOffset;
        var dataEntry = ReadExact(stream, dataEntryOffset, 16, sourceName);
        var dataRva = BinaryPrimitives.ReadUInt32LittleEndian(dataEntry.AsSpan(0, 4));
        var dataSize = BinaryPrimitives.ReadUInt32LittleEndian(dataEntry.AsSpan(4, 4));
        if (dataSize == 0 || dataSize > MaxVersionResourceBytes)
        {
            throw new InvalidDataException(L.Format("Version_PeBadDataSize", sourceName, dataSize));
        }

        if (!TryRvaToFileOffset(sections, dataRva, out var dataFileOffset))
        {
            throw new InvalidDataException(L.Format(
                "Version_PeDataRvaUnmapped",
                sourceName,
                dataRva.ToString("X", CultureInfo.InvariantCulture)));
        }

        var blob = ReadExact(stream, dataFileOffset, (int)dataSize, sourceName);
        return ParseVersionResourceBlob(blob);
    }

    internal static PeParsedVersion ParseVersionResourceBlob(ReadOnlySpan<byte> blob)
    {
        if (blob.Length < 6)
        {
            return new PeParsedVersion(null, null, null, null);
        }

        if (!TryReadVersionNodeHeader(blob, 0, blob.Length, out var rootEnd, out var rootValueLen, out _, out _, out var valueOffset))
        {
            return new PeParsedVersion(null, null, null, null);
        }

        string? fixedFileVersion = null;
        string? fixedProductVersion = null;

        if (rootValueLen >= 52 && valueOffset + 52 <= rootEnd)
        {
            var fixedSpan = blob.Slice(valueOffset, 52);
            var sig = BinaryPrimitives.ReadUInt32LittleEndian(fixedSpan.Slice(0, 4));
            if (sig == FixedFileInfoSignature)
            {
                var fileMs = BinaryPrimitives.ReadUInt32LittleEndian(fixedSpan.Slice(8, 4));
                var fileLs = BinaryPrimitives.ReadUInt32LittleEndian(fixedSpan.Slice(12, 4));
                var prodMs = BinaryPrimitives.ReadUInt32LittleEndian(fixedSpan.Slice(16, 4));
                var prodLs = BinaryPrimitives.ReadUInt32LittleEndian(fixedSpan.Slice(20, 4));
                fixedFileVersion = FormatFixedVersion(fileMs, fileLs);
                fixedProductVersion = FormatFixedVersion(prodMs, prodLs);
            }
        }

        string? stringFileVersion = null;
        string? stringProductVersion = null;
        var preferredTableMatched = false;

        var childPos = Align4(valueOffset + rootValueLen);
        while (childPos + 6 <= rootEnd)
        {
            if (!TryReadVersionNodeHeader(blob, childPos, rootEnd, out var childEnd, out var childValLen, out _, out var childKey, out var childValOff))
            {
                break;
            }

            if (string.Equals(childKey, "StringFileInfo", StringComparison.Ordinal))
            {
                var tablePos = Align4(childValOff + childValLen);
                while (tablePos + 6 <= childEnd)
                {
                    if (!TryReadVersionNodeHeader(blob, tablePos, childEnd, out var tableEnd, out var tableValLen, out _, out var tableKey, out var tableValOff))
                    {
                        break;
                    }

                    var isEnUsTable = tableKey.StartsWith("0409", StringComparison.OrdinalIgnoreCase);
                    var tableStrings = ReadStringTable(blob, Align4(tableValOff + tableValLen), tableEnd);
                    tableStrings.TryGetValue("FileVersion", out var fv);
                    tableStrings.TryGetValue("ProductVersion", out var pv);

                    if (!preferredTableMatched || isEnUsTable)
                    {
                        if (!string.IsNullOrWhiteSpace(fv) && (isEnUsTable || stringFileVersion is null))
                        {
                            stringFileVersion = fv;
                        }

                        if (!string.IsNullOrWhiteSpace(pv) && (isEnUsTable || stringProductVersion is null))
                        {
                            stringProductVersion = pv;
                        }

                        if (isEnUsTable)
                        {
                            preferredTableMatched = true;
                        }
                    }

                    var nextTable = Align4(tableEnd);
                    if (nextTable <= tablePos)
                    {
                        break;
                    }

                    tablePos = nextTable;
                }
            }

            var nextChild = Align4(childEnd);
            if (nextChild <= childPos)
            {
                break;
            }

            childPos = nextChild;
        }

        return new PeParsedVersion(
            stringFileVersion ?? fixedFileVersion,
            stringProductVersion ?? fixedProductVersion,
            fixedFileVersion,
            fixedProductVersion);
    }

    private static Dictionary<string, string> ReadStringTable(ReadOnlySpan<byte> blob, int start, int tableEnd)
    {
        var values = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var pos = start;
        while (pos + 6 <= tableEnd)
        {
            if (!TryReadVersionNodeHeader(blob, pos, tableEnd, out var entryEnd, out var valLen, out var wType, out var key, out var valOff))
            {
                break;
            }

            if (!string.IsNullOrEmpty(key) && valLen > 0 && valOff < entryEnd)
            {
                var byteLen = wType == 1
                    ? Math.Min(entryEnd - valOff, valLen * 2)
                    : Math.Min(entryEnd - valOff, valLen);
                byteLen -= byteLen % 2;
                if (byteLen > 0)
                {
                    var raw = Encoding.Unicode.GetString(blob.Slice(valOff, byteLen));
                    var nulIdx = raw.IndexOf('\0');
                    var cleaned = (nulIdx >= 0 ? raw[..nulIdx] : raw).Trim();
                    if (!string.IsNullOrEmpty(cleaned) && !values.ContainsKey(key))
                    {
                        values[key] = cleaned;
                    }
                }
            }

            var next = Align4(entryEnd);
            if (next <= pos)
            {
                break;
            }

            pos = next;
        }

        return values;
    }

    private static bool TryReadVersionNodeHeader(
        ReadOnlySpan<byte> blob,
        int offset,
        int limit,
        out int nodeEnd,
        out int valueLength,
        out ushort valueType,
        out string key,
        out int valueOffset)
    {
        nodeEnd = offset;
        valueLength = 0;
        valueType = 0;
        key = string.Empty;
        valueOffset = offset;

        if (offset + 6 > limit)
        {
            return false;
        }

        var wLength = BinaryPrimitives.ReadUInt16LittleEndian(blob.Slice(offset, 2));
        valueLength = BinaryPrimitives.ReadUInt16LittleEndian(blob.Slice(offset + 2, 2));
        valueType = BinaryPrimitives.ReadUInt16LittleEndian(blob.Slice(offset + 4, 2));
        if (wLength < 6)
        {
            return false;
        }

        nodeEnd = Math.Min(limit, offset + wLength);
        var keyStart = offset + 6;
        var keyPos = keyStart;
        while (keyPos + 2 <= nodeEnd)
        {
            var codeUnit = BinaryPrimitives.ReadUInt16LittleEndian(blob.Slice(keyPos, 2));
            keyPos += 2;
            if (codeUnit == 0)
            {
                key = Encoding.Unicode.GetString(blob.Slice(keyStart, keyPos - keyStart - 2));
                valueOffset = Align4(keyPos);
                return true;
            }
        }

        return false;
    }

    private static string FormatFixedVersion(uint ms, uint ls)
    {
        return string.Create(
            CultureInfo.InvariantCulture,
            $"{ms >> 16}.{ms & 0xFFFFu}.{ls >> 16}.{ls & 0xFFFFu}");
    }

    private static int Align4(int offset) => (offset + 3) & ~3;

    private static ResourceDirEntry[] ReadResourceDirectory(
        Stream stream,
        long resourceBaseOffset,
        uint relativeOffset,
        string sourceName)
    {
        var dirOffset = resourceBaseOffset + relativeOffset;
        var header = ReadExact(stream, dirOffset, 16, sourceName);
        var namedCount = BinaryPrimitives.ReadUInt16LittleEndian(header.AsSpan(12, 2));
        var idCount = BinaryPrimitives.ReadUInt16LittleEndian(header.AsSpan(14, 2));
        var total = namedCount + idCount;
        if (total == 0)
        {
            return [];
        }

        var rawEntries = ReadExact(stream, dirOffset + 16, checked(total * 8), sourceName);
        var entries = new ResourceDirEntry[total];
        for (var i = 0; i < total; i++)
        {
            var span = rawEntries.AsSpan(i * 8, 8);
            var nameOrId = BinaryPrimitives.ReadUInt32LittleEndian(span.Slice(0, 4));
            var offsetToData = BinaryPrimitives.ReadUInt32LittleEndian(span.Slice(4, 4));
            entries[i] = new ResourceDirEntry(nameOrId, offsetToData);
        }

        return entries;
    }

    private static bool TryRvaToFileOffset(PeSection[] sections, uint rva, out long fileOffset)
    {
        foreach (var section in sections)
        {
            var span = Math.Max(section.VirtualSize, section.RawSize);
            if (rva >= section.VirtualAddress && rva < section.VirtualAddress + span)
            {
                var delta = rva - section.VirtualAddress;
                if (delta < section.RawSize)
                {
                    fileOffset = (long)section.RawPointer + delta;
                    return true;
                }
            }
        }

        fileOffset = 0;
        return false;
    }

    private static byte[] ReadExact(Stream stream, long offset, int count, string sourceName)
    {
        if (offset < 0 || count < 0 || offset + count > stream.Length)
        {
            throw new InvalidDataException(L.Format(
                "Version_PeReadOutOfRange",
                sourceName,
                offset.ToString("X", CultureInfo.InvariantCulture),
                count));
        }

        stream.Seek(offset, SeekOrigin.Begin);
        var buffer = new byte[count];
        var readTotal = 0;
        while (readTotal < count)
        {
            var n = stream.Read(buffer, readTotal, count - readTotal);
            if (n == 0)
            {
                throw new InvalidDataException(L.Format("Version_PeUnexpectedEof", sourceName));
            }

            readTotal += n;
        }

        return buffer;
    }
}
