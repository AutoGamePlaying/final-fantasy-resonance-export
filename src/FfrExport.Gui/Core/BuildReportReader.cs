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
    internal static string DualComboHint => L.Get("GuiCore_DualComboHint");

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
            ?? throw new FileNotFoundException(L.Format("GuiCore_ReportNotFound", outputDirectory));

        string json;
        try
        {
            json = File.ReadAllText(PathSupport.ToIoPath(reportPath), Utf8WithoutBom);
        }
        catch (Exception exception)
        {
            throw new IOException(L.Format("GuiCore_ReportReadFailed", reportPath, exception.Message), exception);
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
            throw new InvalidDataException(L.Format("GuiCore_ReportInvalidJson", reportPath, exception.Message), exception);
        }

        using (doc)
        {
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                throw new InvalidDataException(L.Format("GuiCore_ReportRootNotObject", reportPath));
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
                throw new InvalidDataException(L.Format("GuiCore_ReportMissingFileSha", reportPath));
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
            ? L.Get("GuiCore_InputDescUsmapOnly")
            : L.Format("GuiCore_InputDescWithJmaps", jmapCount);

        var partialNotice = hasPartialExportFailure
            ? L.Get("GuiCore_PartialExportNotice")
            : string.Empty;

        switch (verdict)
        {
            case BuildComparisonVerdict.Matched:
                {
                    var regPart = string.IsNullOrWhiteSpace(matchedRegisteredAt)
                        ? string.Empty
                        : L.Format("GuiCore_MatchedRegDatePart", matchedRegisteredAt);
                    var explanation = L.Format("GuiCore_VerdictMatchedExplanation", inputDesc, regPart, DualComboHint, partialNotice);
                    return ("✓", L.Get("GuiCore_VerdictMatchedTitle"), explanation);
                }

            case BuildComparisonVerdict.NewVersion:
                {
                    var explanation = L.Format("GuiCore_VerdictNewVersionExplanation", inputDesc, DualComboHint, partialNotice);
                    return ("✗", L.Get("GuiCore_VerdictNewVersionTitle"), explanation);
                }

            case BuildComparisonVerdict.HashMismatch:
                {
                    string detail;
                    if (fileSha256MismatchOnly)
                    {
                        detail = L.Get("GuiCore_VerdictFileShaOnlyDetail");
                    }
                    else if (tableDiffCount > 0)
                    {
                        detail = L.Format("GuiCore_VerdictTableDiffDetail", tableDiffCount);
                    }
                    else
                    {
                        detail = L.Get("GuiCore_VerdictGenericMismatchDetail");
                    }

                    var jmapNote = jmapCount is > 0 and not 2
                        ? L.Format("GuiCore_VerdictOddJmapCountNote", jmapCount)
                        : string.Empty;

                    var explanation = L.Format(
                        "GuiCore_VerdictHashMismatchExplanation",
                        inputDesc,
                        detail,
                        jmapNote,
                        DualComboHint,
                        partialNotice);
                    return ("✗", L.Get("GuiCore_VerdictHashMismatchTitle"), explanation);
                }

            default:
                {
                    var explanation = L.Format("GuiCore_VerdictUnknownExplanation", inputDesc, DualComboHint, partialNotice);
                    return ("✗", L.Get("GuiCore_VerdictUnknownTitle"), explanation);
                }
        }
    }

    internal static string FormatClipboardHashText(BuildReportSummary summary)
    {
        var (symbol, title, _) = FormatVerdictDisplay(
            summary.Verdict,
            summary.JmapCount,
            summary.MatchedRegisteredAt,
            summary.FileSha256MismatchOnly,
            summary.TableDiffCount,
            summary.HasPartialExportFailure);
        var sb = new StringBuilder();
        sb.AppendLine(L.Format("GuiCore_ClipboardDbPath", summary.DatabasePath));
        sb.AppendLine(L.Format("GuiCore_ClipboardFileSha", summary.FileSha256));
        sb.AppendLine(L.Format("GuiCore_ClipboardContentSha", summary.ContentSha256 ?? L.Get("Common_None")));
        sb.AppendLine(L.Format("GuiCore_ClipboardVerdict", symbol, title));
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
