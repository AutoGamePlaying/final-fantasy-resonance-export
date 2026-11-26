using System.Globalization;
using System.Text;
using Newtonsoft.Json;

namespace FfrExport;

internal static partial class TextOutput
{
    /// <summary>
    /// Serializes a value using Newtonsoft.Json with an explicit LF ('\n') writer NewLine,
    /// matching JsonConvert.SerializeObject on Linux byte-for-byte on every platform.
    /// </summary>
    internal static string SerializeJson(
        object? value,
        Formatting formatting = Formatting.Indented,
        bool trailingNewline = false,
        JsonSerializerSettings? settings = null)
    {
        var buffer = new StringBuilder(256);
        using var stringWriter = new StringWriter(buffer, CultureInfo.InvariantCulture);
        SerializeJsonToWriter(stringWriter, value, formatting, trailingNewline, settings);
        return buffer.ToString();
    }

    internal static string SerializeJsonCompact(
        object? value,
        JsonSerializerSettings? settings = null)
    {
        return SerializeJson(value, Formatting.None, trailingNewline: false, settings);
    }

    internal static void SerializeJsonToWriter(
        TextWriter writer,
        object? value,
        Formatting formatting = Formatting.Indented,
        bool trailingNewline = false,
        JsonSerializerSettings? settings = null)
    {
        ArgumentNullException.ThrowIfNull(writer);
        ConfigureLineEnding(writer);

        var serializer = JsonSerializer.CreateDefault(settings);
        serializer.Formatting = formatting;

        using (var jsonWriter = new JsonTextWriter(writer) { CloseOutput = false })
        {
            jsonWriter.Formatting = formatting;
            serializer.Serialize(jsonWriter, value);
            jsonWriter.Flush();
        }

        if (trailingNewline)
        {
            writer.Write(LineEnding);
        }
    }

    internal static void WriteJson(
        string path,
        object? value,
        Formatting formatting = Formatting.Indented,
        bool trailingNewline = false,
        JsonSerializerSettings? settings = null)
    {
        WriteText(path, SerializeJson(value, formatting, trailingNewline, settings));
    }

    internal static Task WriteJsonAsync(
        string path,
        object? value,
        Formatting formatting = Formatting.Indented,
        bool trailingNewline = false,
        JsonSerializerSettings? settings = null,
        CancellationToken cancellationToken = default)
    {
        return WriteTextAsync(path, SerializeJson(value, formatting, trailingNewline, settings), cancellationToken);
    }

    internal static long WriteJsonAtomic(
        string targetPath,
        object? value,
        Formatting formatting = Formatting.Indented,
        bool trailingNewline = false,
        JsonSerializerSettings? settings = null)
    {
        return WriteTextAtomic(targetPath, SerializeJson(value, formatting, trailingNewline, settings));
    }

    internal static Task<long> WriteJsonAtomicAsync(
        string targetPath,
        object? value,
        Formatting formatting = Formatting.Indented,
        bool trailingNewline = false,
        JsonSerializerSettings? settings = null,
        CancellationToken cancellationToken = default)
    {
        return WriteTextAtomicAsync(targetPath, SerializeJson(value, formatting, trailingNewline, settings), cancellationToken);
    }
}
