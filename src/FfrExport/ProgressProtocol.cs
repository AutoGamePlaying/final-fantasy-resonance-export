using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace FfrExport;

internal sealed record Ue4ssCleanupProtocolMessage(
    string Status,
    string? Path,
    string? Reason);

internal sealed record Ue4ssCleanupResultDocument(
    [property: JsonPropertyName("format")] int Format,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("outcome")] string Outcome,
    [property: JsonPropertyName("exit_code")] int ExitCode,
    [property: JsonPropertyName("removed_files")] int RemovedFiles,
    [property: JsonPropertyName("removed_dirs")] int RemovedDirs,
    [property: JsonPropertyName("remaining_files_in_ue4ss")] int RemainingFilesInUe4ss,
    [property: JsonPropertyName("backup_directory")] string? BackupDirectory = null,
    [property: JsonPropertyName("blocked_path")] string? BlockedPath = null,
    [property: JsonPropertyName("reason")] string? Reason = null,
    [property: JsonPropertyName("message")] string? Message = null,
    [property: JsonPropertyName("messages")] string[]? Messages = null);

/// <summary>
/// Language-independent machine-readable stdout protocol emitted by the CLI
/// and consumed by the WinForms GUI to drive progress bars, .usmap auto-selection,
/// and UE4SS cleanup elevation prompts.
/// </summary>
internal static class ProgressProtocol
{
    internal const string ProgressPrefix = "@@progress ";
    internal const string UsmapPrefix = "@@usmap ";
    internal const string Ue4ssCleanupPrefix = "@@ue4ss-cleanup ";

    internal const int ExitCodeNeedsElevation = 5;
    internal const int ExitCodeFileInUse = 6;

    internal const string CleanupStatusCompleted = "completed";
    internal const string CleanupStatusNotFound = "not_found";
    internal const string CleanupStatusCancelled = "cancelled";
    internal const string CleanupStatusNeedsElevation = "needs_elevation";
    internal const string CleanupStatusInUse = "in_use";
    internal const string CleanupStatusFailed = "failed";

    internal const string CleanupReasonAccessDenied = "access_denied";
    internal const string CleanupReasonInUse = "in_use";

    private static readonly JsonSerializerOptions CompactJsonOptions = new()
    {
        WriteIndented = false,
        NewLine = TextOutput.LineEnding,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
    };

    private static readonly JsonSerializerOptions PrettyJsonOptions = new()
    {
        WriteIndented = true,
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

    private sealed record Ue4ssCleanupPayload(
        [property: JsonPropertyName("status")] string Status,
        [property: JsonPropertyName("path")] string? Path,
        [property: JsonPropertyName("reason")] string? Reason);

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

    internal static string FormatUe4ssCleanupLine(string status, string? path = null, string? reason = null)
    {
        var payload = new Ue4ssCleanupPayload(
            status,
            string.IsNullOrWhiteSpace(path) ? null : path,
            string.IsNullOrWhiteSpace(reason) ? null : reason);
        return Ue4ssCleanupPrefix + TextOutput.SerializeSystemJson(payload, CompactJsonOptions, trailingNewline: false);
    }

    internal static string SerializeUe4ssCleanupResult(Ue4ssCleanupResultDocument document)
    {
        ArgumentNullException.ThrowIfNull(document);
        return TextOutput.SerializeSystemJson(document, PrettyJsonOptions, trailingNewline: true);
    }

    internal static bool TryDeserializeUe4ssCleanupResult(
        string? json,
        out Ue4ssCleanupResultDocument? document,
        out string? errorMessage)
    {
        document = null;
        errorMessage = null;

        if (string.IsNullOrWhiteSpace(json))
        {
            errorMessage = "JSON content is empty.";
            return false;
        }

        try
        {
            using var parsedDoc = JsonDocument.Parse(json);
            if (parsedDoc.RootElement.ValueKind != JsonValueKind.Object)
            {
                errorMessage = "JSON root is not an object.";
                return false;
            }

            if (!parsedDoc.RootElement.TryGetProperty("status", out var statusProp)
                || statusProp.ValueKind != JsonValueKind.String
                || string.IsNullOrWhiteSpace(statusProp.GetString()))
            {
                errorMessage = "Missing required string property 'status'.";
                return false;
            }

            var deserialized = JsonSerializer.Deserialize<Ue4ssCleanupResultDocument>(json);
            if (deserialized is null || string.IsNullOrWhiteSpace(deserialized.Status))
            {
                errorMessage = "Failed to deserialize cleanup result document.";
                return false;
            }

            document = deserialized;
            return true;
        }
        catch (JsonException exception)
        {
            errorMessage = exception.Message;
            return false;
        }
    }

    internal static bool IsMachineProtocolLine(string? line)
    {
        if (string.IsNullOrWhiteSpace(line))
        {
            return false;
        }

        var trimmed = line.TrimStart();
        return trimmed.StartsWith(ProgressPrefix, StringComparison.Ordinal)
            || trimmed.StartsWith(UsmapPrefix, StringComparison.Ordinal)
            || trimmed.StartsWith(Ue4ssCleanupPrefix, StringComparison.Ordinal);
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

    internal static bool TryParseUe4ssCleanupLine(string? line, out Ue4ssCleanupProtocolMessage message)
    {
        message = default!;
        if (string.IsNullOrWhiteSpace(line))
        {
            return false;
        }

        var trimmed = line.Trim();
        if (!trimmed.StartsWith(Ue4ssCleanupPrefix, StringComparison.Ordinal))
        {
            return false;
        }

        var json = trimmed[Ue4ssCleanupPrefix.Length..].Trim();
        if (string.IsNullOrEmpty(json))
        {
            return false;
        }

        try
        {
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !root.TryGetProperty("status", out var statusProp)
                || statusProp.ValueKind != JsonValueKind.String)
            {
                return false;
            }

            var status = statusProp.GetString()?.Trim();
            if (string.IsNullOrWhiteSpace(status))
            {
                return false;
            }

            string? path = null;
            if (root.TryGetProperty("path", out var pathProp) && pathProp.ValueKind == JsonValueKind.String)
            {
                var rawPath = pathProp.GetString()?.Trim();
                if (!string.IsNullOrWhiteSpace(rawPath))
                {
                    path = rawPath;
                }
            }

            string? reason = null;
            if (root.TryGetProperty("reason", out var reasonProp) && reasonProp.ValueKind == JsonValueKind.String)
            {
                var rawReason = reasonProp.GetString()?.Trim();
                if (!string.IsNullOrWhiteSpace(rawReason))
                {
                    reason = rawReason;
                }
            }

            message = new Ue4ssCleanupProtocolMessage(status, path, reason);
            return true;
        }
        catch (JsonException)
        {
            return false;
        }
    }
}
