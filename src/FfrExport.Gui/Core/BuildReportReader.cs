using System.Globalization;
using System.Text;
using System.Text.Json;

namespace FfrExport.Gui.Core;

internal enum BuildComparisonVerdict
{
    Matched,
    NewVersion,
    HashMismatch,
    Unknown,
}

internal sealed record BuildReportSummary(
    string ReportPath,
    string Status,
    string DatabasePath,
    long? DatabaseSize,
    string FileSha256,
    string? ContentSha256,
    int JmapCount,
    BuildComparisonVerdict Verdict,
    string VerdictSymbol,
    string VerdictTitle,
    string VerdictExplanation,
    string? ComparisonSource,
    string? MatchedRegisteredAt,
    bool FileSha256MismatchOnly,
    int TableDiffCount,
    long? TotalElapsedMs,
    bool HasPartialExportFailure);

/// <summary>
/// Reads structured build results from *.build.json (ffr.sqlite.build.json) without scraping CLI text.
/// </summary>
internal static class BuildReportReader
{
    internal const string DualComboHint =
        "已知版本表里登记了两种输入组合：一种是“只有 .usmap”，一种是“.usmap 加两份 .jmap”，所以无论是否勾选 .jmap，都可能对得上。";

    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    internal static string? FindBuildReportPath(string outputDirectory)
    {
        if (string.IsNullOrWhiteSpace(outputDirectory))
        {
            return null;
        }

        var fullDir = PathSupport.TrimTrailingSeparators(Path.GetFullPath(outputDirectory));
        var ioDir = PathSupport.ToIoPath(fullDir);
        if (!Directory.Exists(ioDir))
        {
            return null;
        }

        var preferred = Path.Combine(fullDir, GuiPaths.BuildMetadataFileName);
        if (File.Exists(PathSupport.ToIoPath(preferred)))
        {
            return preferred;
        }

        string? latest = null;
        DateTime latestWriteUtc = DateTime.MinValue;
        foreach (var file in Directory.EnumerateFiles(ioDir, "*.build.json", SearchOption.TopDirectoryOnly))
        {
            var writeUtc = new FileInfo(file).LastWriteTimeUtc;
            if (latest is null || writeUtc > latestWriteUtc)
            {
                latest = Path.GetFullPath(file);
                latestWriteUtc = writeUtc;
            }
        }

        return latest;
    }

    internal static BuildReportSummary ReadFromOutputDirectory(string outputDirectory)
    {
        var reportPath = FindBuildReportPath(outputDirectory)
            ?? throw new FileNotFoundException(
                $"读取构建结果失败：在输出目录 {outputDirectory} 中找不到 *.build.json 文件。");

        string json;
        try
        {
            json = File.ReadAllText(PathSupport.ToIoPath(reportPath), Utf8WithoutBom);
        }
        catch (Exception exception)
        {
            throw new IOException($"读取构建报告失败（{reportPath}）：{exception.Message}", exception);
        }

        return Parse(json, reportPath, outputDirectory);
    }

    internal static BuildReportSummary Parse(string json, string reportPath, string outputDirectory)
    {
        JsonDocument doc;
        try
        {
            doc = JsonDocument.Parse(json);
        }
        catch (JsonException exception)
        {
            throw new InvalidDataException($"解析构建报告 JSON 失败（{reportPath}）：{exception.Message}", exception);
        }

        using (doc)
        {
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                throw new InvalidDataException($"构建报告格式无效（根节点不是对象）：{reportPath}");
            }

            var status = ReadOptionalString(root, "status") ?? "unknown";
            var fullOutputDir = PathSupport.TrimTrailingSeparators(Path.GetFullPath(outputDirectory));

            var dbFileName = "ffr.sqlite";
            long? dbSize = null;
            string? fileSha256 = null;
            string? contentSha256 = null;
            if (root.TryGetProperty("database", out var dbElement) && dbElement.ValueKind == JsonValueKind.Object)
            {
                dbFileName = ReadOptionalString(dbElement, "file_name") ?? dbFileName;
                dbSize = ReadOptionalInt64(dbElement, "size");
                fileSha256 = ReadOptionalString(dbElement, "file_sha256");
                contentSha256 = ReadOptionalString(dbElement, "content_sha256");
            }

            if (string.IsNullOrWhiteSpace(fileSha256))
            {
                throw new InvalidDataException($"构建报告缺少 database.file_sha256 字段：{reportPath}");
            }

            var jmapCount = 0;
            if (root.TryGetProperty("inputs", out var inputsElement)
                && inputsElement.ValueKind == JsonValueKind.Object
                && inputsElement.TryGetProperty("jmaps", out var jmapsElement)
                && jmapsElement.ValueKind == JsonValueKind.Array)
            {
                jmapCount = jmapsElement.GetArrayLength();
            }

            string? cmpStatusRaw = null;
            string? cmpSource = null;
            string? matchedDate = null;
            var fileShaOnly = false;
            var tableDiffCount = 0;
            if (root.TryGetProperty("known_build_check", out var cmpElement)
                && cmpElement.ValueKind == JsonValueKind.Object)
            {
                cmpStatusRaw = ReadOptionalString(cmpElement, "status");
                cmpSource = ReadOptionalString(cmpElement, "source");
                matchedDate = ReadOptionalString(cmpElement, "matched_registered_at");
                fileShaOnly = ReadOptionalBool(cmpElement, "file_sha256_mismatch_only");
                if (cmpElement.TryGetProperty("table_diffs", out var diffsElement)
                    && diffsElement.ValueKind == JsonValueKind.Array)
                {
                    tableDiffCount = diffsElement.GetArrayLength();
                }
            }

            long? totalElapsedMs = null;
            if (root.TryGetProperty("timings_ms", out var timingsElement)
                && timingsElement.ValueKind == JsonValueKind.Object)
            {
                totalElapsedMs = ReadOptionalInt64(timingsElement, "total");
            }

            var hasPartialExportFailure = false;
            if (root.TryGetProperty("export_exit_codes", out var exitCodesElement)
                && exitCodesElement.ValueKind == JsonValueKind.Object)
            {
                var dtExit = ReadOptionalInt64(exitCodesElement, "datatable") ?? 0;
                var assetsExit = ReadOptionalInt64(exitCodesElement, "assets") ?? 0;
                var mapsExit = ReadOptionalInt64(exitCodesElement, "maps") ?? 0;
                hasPartialExportFailure = dtExit != 0 || assetsExit != 0 || mapsExit != 0;
            }

            var verdict = ClassifyVerdict(cmpStatusRaw);
            var (symbol, title, explanation) = FormatVerdictDisplay(
                verdict,
                jmapCount,
                matchedDate,
                fileShaOnly,
                tableDiffCount,
                hasPartialExportFailure);

            var databasePath = Path.Combine(fullOutputDir, dbFileName);
            return new BuildReportSummary(
                reportPath,
                status,
                databasePath,
                dbSize,
                fileSha256,
                contentSha256,
                jmapCount,
                verdict,
                symbol,
                title,
                explanation,
                cmpSource,
                matchedDate,
                fileShaOnly,
                tableDiffCount,
                totalElapsedMs,
                hasPartialExportFailure);
        }
    }

    internal static BuildComparisonVerdict ClassifyVerdict(string? rawStatus)
    {
        return rawStatus switch
        {
            "matched" => BuildComparisonVerdict.Matched,
            "new_game_version" => BuildComparisonVerdict.NewVersion,
            "hash_mismatch" => BuildComparisonVerdict.HashMismatch,
            _ => BuildComparisonVerdict.Unknown,
        };
    }

    internal static (string Symbol, string Title, string Explanation) FormatVerdictDisplay(
        BuildComparisonVerdict verdict,
        int jmapCount,
        string? matchedRegisteredAt,
        bool fileSha256MismatchOnly,
        int tableDiffCount,
        bool hasPartialExportFailure)
    {
        var inputDesc = jmapCount == 0
            ? "本次输入：仅 .usmap（未包含 .jmap）"
            : string.Create(CultureInfo.InvariantCulture, $"本次输入：.usmap + {jmapCount} 份 .jmap");

        var partialNotice = hasPartialExportFailure
            ? "（提示：导出过程中有少量资源包失败，详见日志与构建报告。）"
            : string.Empty;

        switch (verdict)
        {
            case BuildComparisonVerdict.Matched:
                {
                    var regPart = string.IsNullOrWhiteSpace(matchedRegisteredAt)
                        ? string.Empty
                        : $"，匹配登记日期：{matchedRegisteredAt}";
                    var explanation = $"{inputDesc}{regPart}。{DualComboHint}{partialNotice}";
                    return ("✓", "与登记的版本一致", explanation);
                }

            case BuildComparisonVerdict.NewVersion:
                {
                    var explanation =
                        $"{inputDesc}。当前游戏版本尚未登记在 known-builds.json 中。{DualComboHint}{partialNotice}";
                    return ("✗", "新版本，未登记", explanation);
                }

            case BuildComparisonVerdict.HashMismatch:
                {
                    string detail;
                    if (fileSha256MismatchOnly)
                    {
                        detail = "所有表内容一致（content_sha256 相同），仅数据库文件哈希不同。";
                    }
                    else if (tableDiffCount > 0)
                    {
                        detail = string.Create(
                            CultureInfo.InvariantCulture,
                            $"游戏版本已登记，但有 {tableDiffCount} 张表内容与登记记录不同。");
                    }
                    else
                    {
                        detail = "游戏版本已登记，但数据库哈希与登记记录不同。";
                    }

                    var jmapNote = jmapCount is > 0 and not 2
                        ? string.Create(
                            CultureInfo.InvariantCulture,
                            $"注意：当前提供了 {jmapCount} 份 .jmap，而登记组合为“0 份”或“2 份”。")
                        : string.Empty;

                    var explanation = $"{inputDesc}。{detail}{jmapNote}{DualComboHint}{partialNotice}";
                    return ("✗", "哈希不同", explanation);
                }

            default:
                {
                    var explanation = $"{inputDesc}。构建报告中未包含可识别的版本比对结论。{DualComboHint}{partialNotice}";
                    return ("✗", "比对结论未知", explanation);
                }
        }
    }

    internal static string FormatClipboardHashText(BuildReportSummary summary)
    {
        var sb = new StringBuilder();
        sb.AppendLine($"数据库路径：{summary.DatabasePath}");
        sb.AppendLine($"文件哈希 (file_sha256)：{summary.FileSha256}");
        sb.AppendLine($"内容哈希 (content_sha256)：{summary.ContentSha256 ?? "无"}");
        sb.AppendLine($"比对结论：{summary.VerdictSymbol} {summary.VerdictTitle}");
        return sb.ToString().TrimEnd();
    }

    private static string? ReadOptionalString(JsonElement obj, string propertyName)
    {
        if (!obj.TryGetProperty(propertyName, out var prop) || prop.ValueKind != JsonValueKind.String)
        {
            return null;
        }

        return prop.GetString();
    }

    private static long? ReadOptionalInt64(JsonElement obj, string propertyName)
    {
        if (!obj.TryGetProperty(propertyName, out var prop) || prop.ValueKind != JsonValueKind.Number)
        {
            return null;
        }

        return prop.TryGetInt64(out var value) ? value : null;
    }

    private static bool ReadOptionalBool(JsonElement obj, string propertyName)
    {
        if (!obj.TryGetProperty(propertyName, out var prop))
        {
            return false;
        }

        return prop.ValueKind == JsonValueKind.True;
    }
}
