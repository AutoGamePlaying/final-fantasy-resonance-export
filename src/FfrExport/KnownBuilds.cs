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

internal enum UsmapComparisonStatus
{
    ExactMatch,
    ContentMatch,
    NotRegistered,
}

internal sealed record UsmapComparisonResult(
    UsmapComparisonStatus Status,
    string Message,
    string Source,
    string? MatchedRegisteredAt,
    string? MatchedFileName);

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

    internal static string MessageMatched => L.Get("KnownBuilds_Matched");
    internal static string MessageNewGameVersion => L.Get("KnownBuilds_NewGameVersion");
    internal static string MessageHashMismatch => L.Get("KnownBuilds_HashMismatch");

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
            throw new InvalidOperationException(L.Format("KnownBuilds_MissingEmbedded", EmbeddedResourceName));
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
                throw new InvalidDataException(L.Format("KnownBuilds_EmptyOrNotArray", sourceName));
            }

            return parsed;
        }
        catch (JsonException exception)
        {
            throw new InvalidDataException(L.Format("KnownBuilds_ParseFailed", sourceName, exception.Message), exception);
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
        output.WriteLine(L.Format("KnownBuilds_ComparisonHeader", result.Source, result.Message));
        if (result.StatusKind == KnownBuildComparisonStatus.Matched && !string.IsNullOrEmpty(result.MatchedRegisteredAt))
        {
            output.WriteLine(L.Format("KnownBuilds_MatchedDate", result.MatchedRegisteredAt));
            return;
        }

        if (result.StatusKind != KnownBuildComparisonStatus.HashMismatch)
        {
            return;
        }

        if (!string.IsNullOrEmpty(result.MatchedRegisteredAt))
        {
            output.WriteLine(L.Format("KnownBuilds_ReferenceDate", result.MatchedRegisteredAt));
        }

        if (result.FileSha256MismatchOnly)
        {
            output.WriteLine(L.Get("KnownBuilds_FileShaOnlyMismatch"));
            return;
        }

        if (result.TableDiffs.Length == 0)
        {
            output.WriteLine(L.Get("KnownBuilds_NoPerTableHashes"));
            return;
        }

        output.WriteLine(L.Format("KnownBuilds_DiffTableCount", result.TableDiffs.Length));
        foreach (var diff in result.TableDiffs)
        {
            var line = diff.Kind switch
            {
                "added" => L.Format("KnownBuilds_DiffAdded", diff.Name, diff.ActualRows),
                "removed" => L.Format("KnownBuilds_DiffRemoved", diff.Name, diff.ExpectedRows),
                _ => diff.ExpectedRows == diff.ActualRows
                    ? L.Format("KnownBuilds_DiffSameRows", diff.Name, diff.ActualRows)
                    : L.Format("KnownBuilds_DiffDifferentRows", diff.Name, diff.ExpectedRows, diff.ActualRows),
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

    internal static UsmapComparisonResult CompareUsmap(
        IReadOnlyList<KnownBuildEntry> entries,
        string source,
        UsmapHashInfo actualUsmap)
    {
        KnownBuildEntry? contentMatch = null;
        foreach (var entry in entries)
        {
            var registered = entry.Inputs?.Usmap;
            if (registered is null)
            {
                continue;
            }

            var contentEquals = !string.IsNullOrEmpty(registered.ContentSha256)
                && string.Equals(registered.ContentSha256, actualUsmap.ContentSha256, StringComparison.OrdinalIgnoreCase);
            if (!contentEquals)
            {
                continue;
            }

            var fileEquals = !string.IsNullOrEmpty(registered.FileSha256)
                && string.Equals(registered.FileSha256, actualUsmap.FileSha256, StringComparison.OrdinalIgnoreCase);
            if (fileEquals)
            {
                return new UsmapComparisonResult(
                    UsmapComparisonStatus.ExactMatch,
                    L.Get("KnownBuilds_UsmapExactMatch"),
                    source,
                    entry.RegisteredAt,
                    registered.FileName);
            }

            contentMatch ??= entry;
        }

        if (contentMatch is not null)
        {
            return new UsmapComparisonResult(
                UsmapComparisonStatus.ContentMatch,
                L.Get("KnownBuilds_UsmapContentMatch"),
                source,
                contentMatch.RegisteredAt,
                contentMatch.Inputs?.Usmap.FileName);
        }

        return new UsmapComparisonResult(
            UsmapComparisonStatus.NotRegistered,
            L.Get("KnownBuilds_UsmapNotRegistered"),
            source,
            null,
            null);
    }

    internal static void PrintUsmapComparison(UsmapComparisonResult result, TextWriter output)
    {
        var symbol = result.Status == UsmapComparisonStatus.NotRegistered ? "!" : "✓";
        output.WriteLine(L.Format("KnownBuilds_UsmapComparisonHeader", result.Source, symbol, result.Message));
        if (!string.IsNullOrEmpty(result.MatchedRegisteredAt))
        {
            output.WriteLine(L.Format(
                "KnownBuilds_UsmapMatchedDate",
                result.MatchedRegisteredAt,
                result.MatchedFileName ?? L.Get("Common_Unknown")));
        }
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
