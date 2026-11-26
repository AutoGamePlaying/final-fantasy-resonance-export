using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace FfrExport;

/// <summary>
/// Language-independent machine-readable stdout protocol emitted by the CLI only when requested
/// via --progress-json, and consumed by the WinForms GUI to drive progress bars and auto-selection.
/// </summary>
internal static class ProgressProtocol
{
    internal const string ProgressPrefix = "@@progress ";
    internal const string UsmapPrefix = "@@usmap ";

    private static readonly JsonSerializerOptions CompactJsonOptions = new()
    {
        WriteIndented = false,
        NewLine = TextOutput.LineEnding,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
    };

    private sealed record ProgressPayload(
        [property: JsonPropertyName("step")] int Step,
        [property: JsonPropertyName("total")] int Total,
        [property: JsonPropertyName("message")] string? Message);

    private sealed record UsmapPayload(
        [property: JsonPropertyName("path")] string Path);

    internal static string FormatProgressLine(int step, int total, string? message = null)
    {
        var payload = new ProgressPayload(
            step,
            total,
            string.IsNullOrWhiteSpace(message) ? null : message);
        return ProgressPrefix + TextOutput.SerializeSystemJson(payload, CompactJsonOptions, trailingNewline: false);
    }

    internal static string FormatUsmapLine(string usmapPath)
    {
        var payload = new UsmapPayload(usmapPath);
        return UsmapPrefix + TextOutput.SerializeSystemJson(payload, CompactJsonOptions, trailingNewline: false);
    }

    internal static bool IsMachineProtocolLine(string? line)
    {
        if (string.IsNullOrWhiteSpace(line))
        {
            return false;
        }

        var trimmed = line.TrimStart();
        return trimmed.StartsWith(ProgressPrefix, StringComparison.Ordinal)
            || trimmed.StartsWith(UsmapPrefix, StringComparison.Ordinal);
    }

    internal static bool TryParseProgressLine(
        string? line,
        out int step,
        out int total,
        out string message)
    {
        step = 0;
        total = 0;
        message = string.Empty;

        if (string.IsNullOrWhiteSpace(line))
        {
            return false;
        }

        var trimmed = line.Trim();
        if (!trimmed.StartsWith(ProgressPrefix, StringComparison.Ordinal))
        {
            return false;
        }

        var json = trimmed[ProgressPrefix.Length..].Trim();
        if (string.IsNullOrEmpty(json))
        {
            return false;
        }

        try
        {
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                return false;
            }

            if (!root.TryGetProperty("step", out var stepProp)
                || stepProp.ValueKind != JsonValueKind.Number
                || !stepProp.TryGetInt32(out var parsedStep))
            {
                return false;
            }

            if (!root.TryGetProperty("total", out var totalProp)
                || totalProp.ValueKind != JsonValueKind.Number
                || !totalProp.TryGetInt32(out var parsedTotal))
            {
                return false;
            }

            if (parsedStep <= 0 || parsedTotal <= 0 || parsedStep > parsedTotal)
            {
                return false;
            }

            var parsedMessage = string.Empty;
            if (root.TryGetProperty("message", out var msgProp)
                && msgProp.ValueKind == JsonValueKind.String)
            {
                parsedMessage = msgProp.GetString()?.Trim() ?? string.Empty;
            }

            step = parsedStep;
            total = parsedTotal;
            message = parsedMessage;
            return true;
        }
        catch (JsonException)
        {
            return false;
        }
    }

    internal static bool TryParseUsmapLine(string? line, out string usmapPath)
    {
        usmapPath = string.Empty;
        if (string.IsNullOrWhiteSpace(line))
        {
            return false;
        }

        var trimmed = line.Trim();
        if (!trimmed.StartsWith(UsmapPrefix, StringComparison.Ordinal))
        {
            return false;
        }

        var json = trimmed[UsmapPrefix.Length..].Trim();
        if (string.IsNullOrEmpty(json))
        {
            return false;
        }

        try
        {
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !root.TryGetProperty("path", out var pathProp)
                || pathProp.ValueKind != JsonValueKind.String)
            {
                return false;
            }

            var parsed = pathProp.GetString()?.Trim();
            if (string.IsNullOrWhiteSpace(parsed))
            {
                return false;
            }

            usmapPath = parsed;
            return true;
        }
        catch (JsonException)
        {
            return false;
        }
    }
}
