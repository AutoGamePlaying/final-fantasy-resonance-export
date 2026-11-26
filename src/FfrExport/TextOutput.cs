using System.Text;
using System.Text.Json;

namespace FfrExport;

/// <summary>
/// Centralized UTF-8 (no BOM) text and System.Text.Json output helper that enforces LF ('\n')
/// line endings on every platform so exported files and manifests are byte-identical across OSes.
/// </summary>
internal static partial class TextOutput
{
    internal const string LineEnding = "\n";
    internal static readonly UTF8Encoding Utf8WithoutBom = new(encoderShouldEmitUTF8Identifier: false);

    internal static void ConfigureLineEnding(TextWriter writer)
    {
        ArgumentNullException.ThrowIfNull(writer);
        writer.NewLine = LineEnding;
    }

    internal static string NormalizeLineEndings(string content)
    {
        ArgumentNullException.ThrowIfNull(content);
        if (content.IndexOf('\r') < 0)
        {
            return content;
        }

        return content
            .Replace("\r\n", LineEnding, StringComparison.Ordinal)
            .Replace("\r", LineEnding, StringComparison.Ordinal);
    }

    internal static JsonSerializerOptions CreateSystemJsonOptions(
        bool writeIndented = true,
        bool propertyNameCaseInsensitive = false)
    {
        return new JsonSerializerOptions
        {
            WriteIndented = writeIndented,
            NewLine = LineEnding,
            PropertyNameCaseInsensitive = propertyNameCaseInsensitive,
        };
    }

    internal static string SerializeSystemJson<T>(
        T value,
        JsonSerializerOptions? options = null,
        bool trailingNewline = false)
    {
        var effectiveOptions = options is null
            ? CreateSystemJsonOptions(writeIndented: false)
            : (string.Equals(options.NewLine, LineEnding, StringComparison.Ordinal)
                ? options
                : new JsonSerializerOptions(options) { NewLine = LineEnding });

        var json = JsonSerializer.Serialize(value, effectiveOptions);
        return trailingNewline ? json + LineEnding : json;
    }

    internal static void WriteText(string path, string content)
    {
        var normalized = NormalizeLineEndings(content);
        File.WriteAllText(PathSupport.ToIoPath(path), normalized, Utf8WithoutBom);
    }

    internal static async Task WriteTextAsync(
        string path,
        string content,
        CancellationToken cancellationToken = default)
    {
        var normalized = NormalizeLineEndings(content);
        await File.WriteAllTextAsync(PathSupport.ToIoPath(path), normalized, Utf8WithoutBom, cancellationToken);
    }

    internal static long WriteTextAtomic(string targetPath, string content)
    {
        var parent = Path.GetDirectoryName(targetPath);
        if (!string.IsNullOrWhiteSpace(parent))
        {
            Directory.CreateDirectory(PathSupport.ToIoPath(parent));
        }

        var normalized = NormalizeLineEndings(content);
        var bytes = Utf8WithoutBom.GetBytes(normalized);
        var ioTarget = PathSupport.ToIoPath(targetPath);
        var tempPath = PathSupport.ToIoPath($"{targetPath}.{Environment.ProcessId}.{Environment.CurrentManagedThreadId}.tmp");
        try
        {
            File.WriteAllBytes(tempPath, bytes);
            File.Move(tempPath, ioTarget, overwrite: true);
            return bytes.LongLength;
        }
        catch
        {
            TryDeleteTempFile(tempPath);
            throw;
        }
    }

    internal static async Task<long> WriteTextAtomicAsync(
        string targetPath,
        string content,
        CancellationToken cancellationToken = default)
    {
        var parent = Path.GetDirectoryName(targetPath);
        if (!string.IsNullOrWhiteSpace(parent))
        {
            Directory.CreateDirectory(PathSupport.ToIoPath(parent));
        }

        var normalized = NormalizeLineEndings(content);
        var bytes = Utf8WithoutBom.GetBytes(normalized);
        var ioTarget = PathSupport.ToIoPath(targetPath);
        var tempPath = PathSupport.ToIoPath($"{targetPath}.{Environment.ProcessId}.{Environment.CurrentManagedThreadId}.tmp");
        try
        {
            await File.WriteAllBytesAsync(tempPath, bytes, cancellationToken);
            File.Move(tempPath, ioTarget, overwrite: true);
            return bytes.LongLength;
        }
        catch
        {
            TryDeleteTempFile(tempPath);
            throw;
        }
    }

    internal static StreamWriter CreateStreamWriter(string path, bool append = false, bool autoFlush = false)
    {
        return new StreamWriter(PathSupport.ToIoPath(path), append, Utf8WithoutBom)
        {
            NewLine = LineEnding,
            AutoFlush = autoFlush,
        };
    }

    internal static StreamWriter CreateStreamWriter(Stream stream, bool autoFlush = false, bool leaveOpen = false)
    {
        return new StreamWriter(stream, Utf8WithoutBom, bufferSize: -1, leaveOpen: leaveOpen)
        {
            NewLine = LineEnding,
            AutoFlush = autoFlush,
        };
    }

    internal static StreamWriter CreateNewFileStreamWriter(
        string path,
        FileShare share = FileShare.ReadWrite,
        bool autoFlush = false)
    {
        var stream = new FileStream(
            PathSupport.ToIoPath(path),
            FileMode.CreateNew,
            FileAccess.Write,
            share);
        try
        {
            return new StreamWriter(stream, Utf8WithoutBom)
            {
                NewLine = LineEnding,
                AutoFlush = autoFlush,
            };
        }
        catch
        {
            stream.Dispose();
            throw;
        }
    }

    internal static void WriteNewTextFile(
        string path,
        string content,
        FileShare share = FileShare.ReadWrite)
    {
        var normalized = NormalizeLineEndings(content);
        using var writer = CreateNewFileStreamWriter(path, share, autoFlush: false);
        writer.Write(normalized);
        writer.Flush();
    }

    internal static void AppendText(
        string path,
        string content,
        FileShare share = FileShare.ReadWrite)
    {
        var normalized = NormalizeLineEndings(content);
        using var stream = new FileStream(
            PathSupport.ToIoPath(path),
            FileMode.Append,
            FileAccess.Write,
            share);
        using var writer = CreateStreamWriter(stream);
        writer.Write(normalized);
        writer.Flush();
    }

    internal static StreamWriter CreateConsoleStreamWriter(Stream stream)
    {
        return new StreamWriter(stream, Utf8WithoutBom) { AutoFlush = true };
    }

    private static void TryDeleteTempFile(string ioTempPath)
    {
        try
        {
            if (File.Exists(ioTempPath))
            {
                File.Delete(ioTempPath);
            }
        }
        catch
        {
            // Ignore temp cleanup errors.
        }
    }
}
