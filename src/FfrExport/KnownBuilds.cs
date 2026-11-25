using System.Globalization;
using System.Reflection;
using System.Text;
using Newtonsoft.Json;

namespace FfrExport;

internal sealed record FfrDbTableHash(
    [property: JsonProperty("name")] string Name,
    [property: JsonProperty("rows")] long Rows,
    [property: JsonProperty("sha256")] string Sha256);

internal sealed record FfrDbHashReport(
    [property: JsonProperty("format")] int Format,
    [property: JsonProperty("file_sha256")] string FileSha256,
    [property: JsonProperty("content_sha256")] string ContentSha256,
    [property: JsonProperty("schema_version")] int SchemaVersion,
    [property: JsonProperty("sqlite_version")] string SqliteVersion,
    [property: JsonProperty("tables")] FfrDbTableHash[] Tables);

internal sealed record BuildInputsInfo(
    [property: JsonProperty("usmap")] UsmapHashInfo Usmap,
    [property: JsonProperty("jmaps")] JmapHashInfo[] Jmaps);

internal sealed record FfrExportToolInfo(
    [property: JsonProperty("version")] string Version,
    [property: JsonProperty("git_commit")] string GitCommit,
    [property: JsonProperty("cue4parse_version")] string Cue4ParseVersion);

internal sealed record FfrDbToolInfo(
    [property: JsonProperty("version")] string Version);

internal sealed record BuildToolsInfo(
    [property: JsonProperty("ffr_export")] FfrExportToolInfo FfrExport,
    [property: JsonProperty("ffr_db")] FfrDbToolInfo FfrDb);

internal sealed record BuildDatabaseInfo(
    [property: JsonProperty("file_name")] string FileName,
    [property: JsonProperty("size")] long Size,
    [property: JsonProperty("schema_version")] int? SchemaVersion,
    [property: JsonProperty("sqlite_version")] string? SqliteVersion,
    [property: JsonProperty("file_sha256")] string FileSha256,
    [property: JsonProperty("content_sha256")] string? ContentSha256,
    [property: JsonProperty("hash_format")] int? HashFormat,
    [property: JsonProperty("tables")] FfrDbTableHash[] Tables);

internal sealed record KnownBuildEntry(
    [property: JsonProperty("registered_at")] string RegisteredAt,
    [property: JsonProperty("note")] string? Note,
    [property: JsonProperty("game_version")] GameVersionInfo GameVersion,
    [property: JsonProperty("inputs")] BuildInputsInfo? Inputs,
    [property: JsonProperty("tools")] BuildToolsInfo? Tools,
    [property: JsonProperty("database")] BuildDatabaseInfo Database);

internal sealed record TableDiffInfo(
    [property: JsonProperty("name")] string Name,
    [property: JsonProperty("kind")] string Kind,
    [property: JsonProperty("expected_rows")] long? ExpectedRows,
    [property: JsonProperty("actual_rows")] long? ActualRows,
    [property: JsonProperty("expected_sha256")] string? ExpectedSha256,
    [property: JsonProperty("actual_sha256")] string? ActualSha256);

internal enum KnownBuildComparisonStatus
{
    Matched,
    NewGameVersion,
    HashMismatch,
}

internal sealed record KnownBuildComparisonResult(
    [property: JsonProperty("status")] string Status,
    [property: JsonProperty("message")] string Message,
    [property: JsonProperty("source")] string Source,
    [property: JsonProperty("matched_registered_at")] string? MatchedRegisteredAt,
    [property: JsonProperty("file_sha256_mismatch_only")] bool FileSha256MismatchOnly,
    [property: JsonProperty("table_diffs")] TableDiffInfo[] TableDiffs)
{
    [JsonIgnore]
    internal KnownBuildComparisonStatus StatusKind { get; init; }
}

/// <summary>
/// Loads known-builds.json (preferring the file next to the executable, falling back to the embedded copy)
/// and compares a newly built SQLite database against registered game versions.
/// </summary>
internal static class KnownBuildsRegistry
{
    internal const string FileName = "known-builds.json";
    internal const string EmbeddedResourceName = "FfrExport.known-builds.json";

    internal const string MessageMatched = "与登记的版本一致";
    internal const string MessageNewGameVersion = "新的游戏版本，未登记";
    internal const string MessageHashMismatch = "游戏版本已登记，但数据库哈希不同";

    internal static (KnownBuildEntry[] Entries, string Source) Load(string? executableDirectoryOverride = null)
    {
        var baseDir = executableDirectoryOverride ?? AppContext.BaseDirectory;
        var adjacentPath = Path.Combine(baseDir, FileName);
        if (File.Exists(PathSupport.ToIoPath(adjacentPath)))
        {
            var json = File.ReadAllText(PathSupport.ToIoPath(adjacentPath), Encoding.UTF8);
            return (ParseEntries(json, adjacentPath), "file");
        }

        var assembly = typeof(KnownBuildsRegistry).Assembly;
        using var stream = assembly.GetManifestResourceStream(EmbeddedResourceName);
        if (stream is null)
        {
            throw new InvalidOperationException($"程序内缺少嵌入资源：{EmbeddedResourceName}");
        }

        using var reader = new StreamReader(stream, Encoding.UTF8);
        var embeddedJson = reader.ReadToEnd();
        return (ParseEntries(embeddedJson, EmbeddedResourceName), "embedded");
    }

    internal static KnownBuildEntry[] ParseEntries(string json, string sourceName)
    {
        try
        {
            var parsed = JsonConvert.DeserializeObject<KnownBuildEntry[]>(json);
            if (parsed is null)
            {
                throw new InvalidDataException($"已知版本登记表为空或格式不是 JSON 数组：{sourceName}");
            }

            return parsed;
        }
        catch (JsonException exception)
        {
            throw new InvalidDataException($"解析已知版本登记表失败：{sourceName}：{exception.Message}", exception);
        }
    }

    internal static KnownBuildComparisonResult Compare(
        IReadOnlyList<KnownBuildEntry> entries,
        string source,
        string pakFingerprint,
        BuildDatabaseInfo actualDb)
    {
        var sameGameEntries = entries
            .Where(entry => entry.GameVersion is not null
                && string.Equals(entry.GameVersion.PakFingerprint, pakFingerprint, StringComparison.OrdinalIgnoreCase))
            .ToArray();

        if (sameGameEntries.Length == 0)
        {
            return new KnownBuildComparisonResult(
                "new_game_version",
                MessageNewGameVersion,
                source,
                null,
                false,
                [])
            {
                StatusKind = KnownBuildComparisonStatus.NewGameVersion,
            };
        }

        foreach (var entry in sameGameEntries)
        {
            if (IsExactDatabaseMatch(entry.Database, actualDb))
            {
                return new KnownBuildComparisonResult(
                    "matched",
                    MessageMatched,
                    source,
                    entry.RegisteredAt,
                    false,
                    [])
                {
                    StatusKind = KnownBuildComparisonStatus.Matched,
                };
            }
        }

        // Pick the closest registered entry for diff reporting: prefer matching schema_version, then latest.
        var reference = sameGameEntries
            .LastOrDefault(entry => entry.Database.SchemaVersion == actualDb.SchemaVersion)
            ?? sameGameEntries[^1];

        var diffs = DiffTables(reference.Database.Tables, actualDb.Tables);
        var contentMatched = !string.IsNullOrEmpty(actualDb.ContentSha256)
            && !string.IsNullOrEmpty(reference.Database.ContentSha256)
            && string.Equals(reference.Database.ContentSha256, actualDb.ContentSha256, StringComparison.OrdinalIgnoreCase);

        return new KnownBuildComparisonResult(
            "hash_mismatch",
            MessageHashMismatch,
            source,
            reference.RegisteredAt,
            contentMatched && diffs.Length == 0,
            diffs)
        {
            StatusKind = KnownBuildComparisonStatus.HashMismatch,
        };
    }

    internal static void PrintComparison(KnownBuildComparisonResult result, TextWriter output)
    {
        output.WriteLine($"已知版本比对（来源：{result.Source}）：{result.Message}");
        if (result.StatusKind == KnownBuildComparisonStatus.Matched && !string.IsNullOrEmpty(result.MatchedRegisteredAt))
        {
            output.WriteLine($"  匹配登记日期：{result.MatchedRegisteredAt}");
            return;
        }

        if (result.StatusKind != KnownBuildComparisonStatus.HashMismatch)
        {
            return;
        }

        if (!string.IsNullOrEmpty(result.MatchedRegisteredAt))
        {
            output.WriteLine($"  对照登记日期：{result.MatchedRegisteredAt}");
        }

        if (result.FileSha256MismatchOnly)
        {
            output.WriteLine("  所有表内容一致（content_sha256 相同），仅数据库文件 file_sha256 不同");
            return;
        }

        if (result.TableDiffs.Length == 0)
        {
            output.WriteLine("  未提供逐表哈希，无法列出表级差异");
            return;
        }

        output.WriteLine($"  差异表共 {result.TableDiffs.Length} 张：");
        foreach (var diff in result.TableDiffs)
        {
            var line = diff.Kind switch
            {
                "added" => string.Create(
                    CultureInfo.InvariantCulture,
                    $"  + {diff.Name}（当前 {diff.ActualRows} 行，登记无此表）"),
                "removed" => string.Create(
                    CultureInfo.InvariantCulture,
                    $"  - {diff.Name}（登记 {diff.ExpectedRows} 行，当前缺少此表）"),
                _ => diff.ExpectedRows == diff.ActualRows
                    ? string.Create(
                        CultureInfo.InvariantCulture,
                        $"  ~ {diff.Name}（行数同为 {diff.ActualRows}，表内容 sha256 不同）")
                    : string.Create(
                        CultureInfo.InvariantCulture,
                        $"  ~ {diff.Name}（行数：登记 {diff.ExpectedRows} -> 当前 {diff.ActualRows}，表内容 sha256 不同）"),
            };
            output.WriteLine(line);
        }
    }

    private static bool IsExactDatabaseMatch(BuildDatabaseInfo registered, BuildDatabaseInfo actual)
    {
        var fileMatches = string.IsNullOrEmpty(registered.FileSha256)
            || string.Equals(registered.FileSha256, actual.FileSha256, StringComparison.OrdinalIgnoreCase);

        if (string.IsNullOrEmpty(actual.ContentSha256))
        {
            return !string.IsNullOrEmpty(registered.FileSha256) && fileMatches;
        }

        var contentMatches = !string.IsNullOrEmpty(registered.ContentSha256)
            && string.Equals(registered.ContentSha256, actual.ContentSha256, StringComparison.OrdinalIgnoreCase);

        return contentMatches && fileMatches;
    }

    private static TableDiffInfo[] DiffTables(
        IReadOnlyList<FfrDbTableHash>? expectedTables,
        IReadOnlyList<FfrDbTableHash>? actualTables)
    {
        var expectedMap = (expectedTables ?? [])
            .ToDictionary(table => table.Name, StringComparer.Ordinal);
        var actualMap = (actualTables ?? [])
            .ToDictionary(table => table.Name, StringComparer.Ordinal);

        var allNames = new SortedSet<string>(expectedMap.Keys, StringComparer.Ordinal);
        allNames.UnionWith(actualMap.Keys);

        var diffs = new List<TableDiffInfo>();
        foreach (var name in allNames)
        {
            var hasExpected = expectedMap.TryGetValue(name, out var expected);
            var hasActual = actualMap.TryGetValue(name, out var actual);

            if (!hasExpected && hasActual && actual is not null)
            {
                diffs.Add(new TableDiffInfo(name, "added", null, actual.Rows, null, actual.Sha256));
                continue;
            }

            if (hasExpected && expected is not null && !hasActual)
            {
                diffs.Add(new TableDiffInfo(name, "removed", expected.Rows, null, expected.Sha256, null));
                continue;
            }

            if (expected is not null
                && actual is not null
                && (expected.Rows != actual.Rows
                    || !string.Equals(expected.Sha256, actual.Sha256, StringComparison.OrdinalIgnoreCase)))
            {
                diffs.Add(new TableDiffInfo(
                    name,
                    "changed",
                    expected.Rows,
                    actual.Rows,
                    expected.Sha256,
                    actual.Sha256));
            }
        }

        return [.. diffs];
    }

    internal static (string Version, string GitCommit) ReadBuildMetadata()
    {
        var assembly = typeof(KnownBuildsRegistry).Assembly;
        var informational = assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion;
        var version = !string.IsNullOrWhiteSpace(informational)
            ? informational.Split('+')[0]
            : assembly.GetName().Version?.ToString(3) ?? "0.1.0";

        var gitCommit = assembly
            .GetCustomAttributes<AssemblyMetadataAttribute>()
            .FirstOrDefault(attr => string.Equals(attr.Key, "GitCommit", StringComparison.Ordinal))
            ?.Value;

        return (version, string.IsNullOrWhiteSpace(gitCommit) ? "unknown" : gitCommit);
    }
}
